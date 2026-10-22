//! The menu probe of the HD menus: the state of the original menu, read from a snapshot of work RAM
//! and joined with the names of the catalogue. A pure function of the bytes: nothing is written,
//! and the original menu logic keeps running as it does. The HD screens draw what it returns.
//!
//! The addresses are those of `re/symbols.toml` (`docs/re/data/menus.md`): `field_state` ($25) says
//! a menu is opening or open, `menu_screen` ($0D5F) which screen, `menu_busy` ($0D60) whether it is
//! busy, `map_entry_value` and `map_entry_0d63` the cursor and the page of a list, `item_list` the
//! entries, `party_order` and `party_records` the party. [`presses`] plays the original's input and
//! [`driver`] turns the HD's choices into it.

pub mod driver;
pub mod keeper;
pub mod presses;
#[cfg(test)]
mod testing;

use crate::data as d;
use ssd_menus::{Labels, Member, MenuView, ObjectLabel, Screen, Selected, Slot};

/// The bus address of the first byte of work RAM (`$7E:0000`).
const WRAM_BASE: u32 = 0x7E_0000;
/// The command window on the field (screen 0).
const SCREEN_COMMANDS: u8 = 0;
/// The command window in a town (screen 13: no Fly).
const SCREEN_TOWN: u8 = 13;
/// The Item screen (screen 2).
const SCREEN_ITEMS: u8 = 2;
/// The command window of a battle (screen 17: Atk., Item, Run, Menu). Its Item entry opens the Item screen.
const SCREEN_BATTLE_COMMAND: u8 = 17;
/// The battle's action screen (screen 21): a battle item's effect runs on it after its Use (`docs/re/data/menus.md`).
const SCREEN_BATTLE_EFFECT: u8 = 21;
/// The Menu list (screen 4): Cards, Status, Order, Text, Save.
const SCREEN_MENU: u8 = 4;
/// The Save (screen 10). Its list of save slots is step 1: the cursor (`map_entry_value`, `$0D62`) holds the slot
/// chosen, 0 to 2. The A that chooses the slot writes the slot, and the Save moves to its completion (step 2) in the
/// next frame; the cursor is back at 0 by then, so the slot is read on the list's frames. Step 0 is the question
/// before the list.
pub const SAVE_SCREEN: u8 = 10;
pub const SAVE_LIST_STEP: u8 = 1;
pub const SAVE_DONE_STEP: u8 = 2;
/// The steps of the Item screen in which a screen takes presses: the list (1), the Use or Drop
/// choice (2), the party list of a use (5), and the Yes or No of a drop (6).
const ITEMS_LIST_STEP: u8 = 1;
const ITEMS_CHOICE_STEP: u8 = 2;
/// The message after a use (3), the effect that runs after it (4), and the result's message (7): A
/// dismisses each message once it waits for A.
const ITEMS_MESSAGE_STEP: u8 = 3;
const ITEMS_EFFECT_STEP: u8 = 4;
const ITEMS_RESULT_STEP: u8 = 7;
const ITEMS_MEMBER_STEP: u8 = 5;
const ITEMS_DROP_STEP: u8 = 6;
/// `field_state` while a menu is opening, and once it is open.
const STATE_OPENING: u8 = 0x0F;
const STATE_OPEN: u8 = 1;
/// `travel_mode` while the party flies (1 is walking).
const TRAVEL_FLYING: u8 = 2;
/// `game_mode` of a town or an interior (`$24`; `docs/re/data/menus.md`).
const TOWN_MODE: u8 = 0x01;
/// `game_mode` of the field, a chapter map walked or flown (`$24`).
const FIELD_MODE: u8 = 0x06;
/// The bit of `$0D67` that is set while the window queue of the menu is empty (everything drawn).
const DRAW_QUEUE_EMPTY: u8 = 0x80;
/// Rows of the item list on one page (the original's, eight a page).
const ITEM_PAGE: usize = 8;
/// The object id of Porunga: his price box shows the wishes he has left.
pub(crate) const PORUNGA: u8 = 15;
/// Entries of the item list (two bytes each).
const ITEM_LIST_LENGTH: usize = 24;
/// Bytes of one character record of the party, and the offsets the view reads.
const RECORD_LENGTH: u32 = 32;
const RECORD_FLAGS: u32 = 0x00;
const RECORD_LEVEL: u32 = 0x02;
const RECORD_HP: u32 = 0x03;
const RECORD_KI: u32 = 0x05;
const RECORD_BP: u32 = 0x07;
const RECORD_MAX_HP: u32 = 0x0A;
const RECORD_MAX_KI: u32 = 0x0C;
const RECORD_CHARACTER: u32 = 0x1C;
/// A record is in use when bit 7 of its flags is set.
const RECORD_IN_USE: u8 = 0x80;
/// The end of the party order.
const PARTY_END: u8 = 0xFF;
/// Bytes of the party order.
const PARTY_ORDER_LENGTH: u32 = 16;
/// Records of the party.
const RECORDS: u32 = 9;

/// The probe of one snapshot of work RAM (128 KiB, from `$7E:0000`), with the names to show.
pub struct MenuProbe<'a> {
    wram: &'a [u8],
    labels: &'a Labels,
}

impl<'a> MenuProbe<'a> {
    /// A probe of the work RAM `wram` with the labels of the catalogue.
    #[must_use]
    pub fn new(wram: &'a [u8], labels: &'a Labels) -> Self {
        Self { wram, labels }
    }

    /// Whether a menu is open or opening: `field_state` is not 0.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.byte(d::FIELD_STATE).is_some_and(|state| state != 0)
    }

    /// Whether the menu is opening or open, as opposed to a field hold (a landing, a scene). A battle's Item list is
    /// an open menu too: a battle holds the field (`field_state` 2) while it runs.
    #[must_use]
    pub fn menu_open(&self) -> bool {
        self.byte(d::FIELD_STATE).is_some_and(|state| state == STATE_OPENING || state == STATE_OPEN)
            || self.battle_list()
    }

    /// Whether a battle runs: `context_61` is 1 in a battle and 2 in the second kind of battle, and 0 on the map.
    #[must_use]
    pub fn in_battle(&self) -> bool {
        self.byte(d::CONTEXT_61).is_some_and(|context| context != 0)
    }

    /// Whether the Item list of a battle is open (screen 2 in a battle): the list the HD's Items screen goes over.
    #[must_use]
    pub fn battle_list(&self) -> bool {
        self.in_battle() && self.screen() == Some(SCREEN_ITEMS)
    }

    /// Whether a battle's item effect runs on its action screen (screen 21 in a battle), after the Use of an item.
    #[must_use]
    pub fn battle_effect(&self) -> bool {
        self.in_battle() && self.screen() == Some(SCREEN_BATTLE_EFFECT)
    }

    /// Whether the command window of a battle is up and idle (screen 17 in a battle): where the battle's Item list
    /// returns to with B, and where a use ends.
    #[must_use]
    pub fn battle_command(&self) -> bool {
        self.in_battle() && self.screen() == Some(SCREEN_BATTLE_COMMAND) && !self.busy()
    }

    /// Whether the menu is opening (`field_state` is `$0F`): its screen byte is not set yet, and
    /// still holds the screen the menu was last on (the Dragon Radar closes the menu from screen 2).
    #[must_use]
    pub fn opening(&self) -> bool {
        self.byte(d::FIELD_STATE) == Some(STATE_OPENING)
    }

    /// Whether the field is free: no menu is open and nothing holds the party.
    #[must_use]
    pub fn field_free(&self) -> bool {
        self.byte(d::FIELD_STATE) == Some(0)
    }

    /// The screen the original menu is on (`menu_screen`), if the snapshot holds it.
    #[must_use]
    pub fn screen(&self) -> Option<u8> {
        self.byte(d::MENU_SCREEN)
    }

    /// Whether the original menu is busy: a message, an effect or a screen being drawn.
    #[must_use]
    pub fn busy(&self) -> bool {
        self.byte(d::MENU_BUSY).is_some_and(|busy| busy != 0)
    }

    /// The step inside the screen (`menu_step`).
    #[must_use]
    pub fn step(&self) -> Option<u8> {
        self.byte(d::MENU_STEP)
    }

    /// Whether an item effect already runs on the Use or Drop choice step (`effect_pending` bit 7):
    /// an effect with a screen of its own, the Dragon Radar, which no message or list comes before.
    #[must_use]
    pub fn effect_runs(&self) -> bool {
        self.menu_open()
            && self.screen() == Some(SCREEN_ITEMS)
            && self.step() == Some(ITEMS_CHOICE_STEP)
            && self.byte(d::EFFECT_PENDING).is_some_and(|pending| pending & 0x80 != 0)
    }

    /// The cursor of the open list (`map_entry_value`).
    #[must_use]
    pub fn cursor(&self) -> Option<u8> {
        self.byte(d::MAP_ENTRY_VALUE)
    }

    /// Whether the game is in the field or in a town (`game_mode` is the field's or the town's): a
    /// save is loaded, or a new game is played, and the bag may bind to it.
    #[must_use]
    pub fn field_mode(&self) -> bool {
        self.byte(d::GAME_MODE).is_some_and(|mode| mode == FIELD_MODE || mode == TOWN_MODE)
    }

    /// The members of the party.
    #[must_use]
    pub fn party_size(&self) -> usize {
        self.party().len()
    }

    /// Whether the party flies (`travel_mode`).
    #[must_use]
    pub fn flying(&self) -> bool {
        self.byte(d::TRAVEL_MODE) == Some(TRAVEL_FLYING)
    }

    /// Whether the party is in a town or an interior: `game_mode` says so before the menu opens,
    /// while its command window (screen 13) is set a couple of frames after the menu is drawn.
    #[must_use]
    pub fn town(&self) -> bool {
        self.byte(d::GAME_MODE) == Some(TOWN_MODE) || self.screen() == Some(SCREEN_TOWN)
    }

    /// The entries of the item list in use (`item_count`, at most the original's 24).
    #[must_use]
    pub fn item_count(&self) -> usize {
        self.byte(d::ITEM_COUNT).map_or(0, |count| usize::from(count).min(ITEM_LIST_LENGTH))
    }

    /// Whether the open menu takes the next press: the field is free (a press on it opens the
    /// menu), or the menu is drawn and idle. A press before the windows are drawn is lost: the
    /// draw queue is empty when its bit is set (`$0D67`). On the Item list the step says idle, since
    /// the list stays busy while it is shown.
    #[must_use]
    pub fn ready(&self) -> bool {
        if self.field_free() {
            return true;
        }
        // A battle's item effect waits on its action screen (idle, with its own press), as an open menu does.
        if !self.menu_open() && !self.battle_effect() {
            return false;
        }
        if self.byte(d::MENU_DRAW_DONE).is_none_or(|done| done & DRAW_QUEUE_EMPTY == 0) {
            return false;
        }
        match self.screen() {
            // A message after a use takes A once its text waits for it (`$0D60` = 1, as opposed to 3
            // while the text is written).
            Some(SCREEN_ITEMS) => match self.step() {
                Some(ITEMS_MESSAGE_STEP | ITEMS_EFFECT_STEP | ITEMS_RESULT_STEP) => {
                    self.byte(d::MENU_BUSY) == Some(1)
                }
                Some(ITEMS_LIST_STEP | ITEMS_CHOICE_STEP | ITEMS_MEMBER_STEP | ITEMS_DROP_STEP) => {
                    true
                }
                _ => false,
            },
            _ => !self.busy(),
        }
    }

    /// Whether a message of the Item screen waits for A (`menu_busy` is 1 while its text waits).
    #[must_use]
    pub fn message_waits(&self) -> bool {
        self.byte(d::MENU_BUSY) == Some(1)
    }

    /// The view of the open menu as the original shows it, when it is a screen the HD draws (the
    /// command window of the field or of a town, the Menu list, or the item list). `None` on the
    /// field, on the other screens of the menu, and for a snapshot too short to hold work RAM.
    #[must_use]
    pub fn view(&self) -> Option<MenuView> {
        if !self.is_open() {
            return None;
        }
        let (screen, town) = match self.screen()? {
            SCREEN_COMMANDS => (Screen::Command, false),
            SCREEN_TOWN => (Screen::Command, true),
            SCREEN_ITEMS => (Screen::Items, false),
            SCREEN_MENU => (Screen::Menu, false),
            _ => return None,
        };
        let flying = screen == Screen::Command && !town && self.flying();
        self.view_as(screen, town, flying, self.cursor()?, self.byte(d::MAP_ENTRY_0D63)?)
    }

    /// The view the HD draws: the screen and the cursors are the HD's own, and the party, the money
    /// and the item list are the game's. `flying` and `town` pick the command rows.
    #[must_use]
    pub fn view_as(
        &self,
        screen: Screen,
        town: bool,
        flying: bool,
        cursor: u8,
        page: u8,
    ) -> Option<MenuView> {
        let items = self.items()?;
        let selected = match screen {
            Screen::Items => self.selected(&items, cursor, page),
            Screen::Command | Screen::Menu | Screen::Cheats | Screen::Options | Screen::Mods => {
                None
            }
        };
        Some(MenuView {
            bag: None,
            screen,
            town,
            flying,
            cursor,
            page,
            zeni: self.word(d::ZENI)?,
            party: self.party(),
            items,
            selected,
        })
    }

    /// The original's text speed (`text_speed`, `$01D0`), the cursor of its Text screen: 0 when the snapshot lacks it.
    #[must_use]
    pub fn text_speed(&self) -> u8 {
        self.byte(d::TEXT_SPEED).unwrap_or(0)
    }

    /// The byte at a bus address of work RAM.
    fn byte(&self, address: u32) -> Option<u8> {
        let index = usize::try_from(address.checked_sub(WRAM_BASE)?).ok()?;
        self.wram.get(index).copied()
    }

    /// The little-endian word at a bus address of work RAM.
    fn word(&self, address: u32) -> Option<u16> {
        Some(u16::from_le_bytes([self.byte(address)?, self.byte(address + 1)?]))
    }

    /// The entries of the item list, in order.
    fn items(&self) -> Option<Vec<Slot>> {
        (0..self.item_count())
            .map(|index| {
                let entry = d::ITEM_LIST + 2 * u32::try_from(index).ok()?;
                let object = self.byte(entry)?;
                let attribute = self.byte(entry + 1)?;
                let name = self.object_label(object).map(|label| label.name.clone());
                Some(Slot { object, attribute, name: name.unwrap_or_else(unknown) })
            })
            .collect()
    }

    /// The party in party order: the record of each character id of `party_order`.
    fn party(&self) -> Vec<Member> {
        (0..PARTY_ORDER_LENGTH)
            .map_while(|index| self.byte(d::PARTY_ORDER + index).filter(|&id| id != PARTY_END))
            .filter_map(|character| self.member(character))
            .collect()
    }

    /// The member of a character: its record in the party, if it has one.
    fn member(&self, character: u8) -> Option<Member> {
        let record = self.record_of(character)?;
        let label = self.labels.characters.get(usize::from(character));
        Some(Member {
            name: label.map_or_else(unknown, |label| label.name.clone()),
            level: self.byte(record + RECORD_LEVEL)?,
            hp: self.word(record + RECORD_HP)?,
            max_hp: self.word(record + RECORD_MAX_HP)?,
            ki: self.word(record + RECORD_KI)?,
            max_ki: self.word(record + RECORD_MAX_KI)?,
            bp: u32::from(self.word(record + RECORD_BP)?)
                | u32::from(self.byte(record + RECORD_BP + 2)?) << 16,
            face: label.and_then(|label| label.face),
        })
    }

    /// The bus address of the party record of a character, if one is in use.
    fn record_of(&self, character: u8) -> Option<u32> {
        (0..RECORDS).map(|index| d::PARTY_RECORDS + index * RECORD_LENGTH).find(|&record| {
            self.byte(record + RECORD_FLAGS).is_some_and(|flags| flags & RECORD_IN_USE != 0)
                && self.byte(record + RECORD_CHARACTER) == Some(character)
        })
    }

    /// The object under the cursor of the item list, if the cursor is on an entry.
    fn selected(&self, items: &[Slot], cursor: u8, page: u8) -> Option<Selected> {
        let index = usize::from(page) * ITEM_PAGE + usize::from(cursor);
        let slot = items.get(index)?;
        let label = self.object_label(slot.object)?;
        // Porunga's box shows the wishes he has left, a byte of the entry's state word, where the
        // others show their price
        let price = if slot.object == PORUNGA {
            self.byte(d::ITEM_EXTRA + 2 * index as u32 + 1)
        } else {
            label.price
        };
        Some(Selected {
            object: slot.object,
            name: label.name.clone(),
            price,
            help: label.help.clone(),
            battle: label.battle,
            field: label.field,
        })
    }

    /// The label of an object id, if the catalogue has one.
    fn object_label(&self, object: u8) -> Option<&'a ObjectLabel> {
        self.labels.objects.get(usize::from(object))
    }
}

/// The text of a name the catalogue does not have.
fn unknown() -> String {
    "?".to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_menus::{CharacterLabel, ObjectLabel};

    /// Work RAM of zeros with `bytes` at bus addresses.
    fn wram(bytes: &[(u32, &[u8])]) -> Vec<u8> {
        let mut ram = vec![0_u8; 0x2_0000];
        for &(address, data) in bytes {
            let index = (address - WRAM_BASE) as usize;
            ram[index..index + data.len()].copy_from_slice(data);
        }
        ram
    }

    fn labels() -> Labels {
        let object = |name: &str, price: Option<u8>, battle: bool, field: bool| ObjectLabel {
            name: name.to_owned(),
            price,
            help: vec![format!("{name} helps."), "Second line.".to_owned()],
            battle,
            field,
            ..ObjectLabel::default()
        };
        let mut objects = vec![ObjectLabel::default(); 40];
        objects[2] = object("Bulma", Some(3), true, true);
        objects[19] = object("Vegeta", None, true, false);
        objects[13] = object("Oolong", Some(7), false, true);
        objects[28] = object("Goku", None, false, false);
        let characters = vec![
            CharacterLabel { name: "Goku".into(), face: Some(28) },
            CharacterLabel { name: "Gohan".into(), face: None },
            CharacterLabel { name: "Piccolo".into(), face: Some(29) },
        ];
        Labels { characters, objects }
    }

    /// A field menu on the Item screen: three items, the cursor on the second, Goku and Piccolo in
    /// the party (Piccolo first in the party order).
    fn item_screen() -> Vec<u8> {
        let party_records = {
            let mut record = [0_u8; 32];
            record[0] = 0xC0;
            record[0x02] = 10;
            record[0x03..0x05].copy_from_slice(&190_u16.to_le_bytes());
            record[0x0A..0x0C].copy_from_slice(&200_u16.to_le_bytes());
            record[0x1C] = 0;
            record
        };
        let piccolo = {
            let mut record = [0_u8; 32];
            record[0] = 0xC0;
            record[0x02] = 2;
            record[0x03..0x05].copy_from_slice(&84_u16.to_le_bytes());
            record[0x0A..0x0C].copy_from_slice(&84_u16.to_le_bytes());
            record[0x1C] = 2;
            record
        };
        wram(&[
            (d::FIELD_STATE, &[1]),
            (d::MENU_SCREEN, &[SCREEN_ITEMS]),
            (d::MENU_STEP, &[ITEMS_LIST_STEP]),
            (d::MENU_DRAW_DONE, &[DRAW_QUEUE_EMPTY]),
            (d::MAP_ENTRY_VALUE, &[1]),
            (d::MAP_ENTRY_0D63, &[0]),
            (d::ITEM_COUNT, &[3]),
            (d::ITEM_LIST, &[2, 0xC0, 19, 0x80, 13, 0x40]),
            (d::ZENI, &40_u16.to_le_bytes()),
            (d::PARTY_ORDER, &[2, 0, PARTY_END]),
            (d::PARTY_RECORDS, &party_records),
            (d::PARTY_RECORDS + 32, &piccolo),
        ])
    }

    fn set(ram: &mut [u8], address: u32, value: u8) {
        ram[(address - WRAM_BASE) as usize] = value;
    }

    #[test]
    fn the_item_screen_is_read_with_its_names_cursor_and_selection() {
        let ram = item_screen();
        let labels = labels();
        let probe = MenuProbe::new(&ram, &labels);
        assert!(probe.is_open() && probe.menu_open() && !probe.field_free());
        let view = probe.view().expect("the item list");
        assert_eq!(view.screen, Screen::Items);
        assert_eq!((view.cursor, view.page, view.zeni), (1, 0, 40));
        let names: Vec<&str> = view.items.iter().map(|slot| slot.name.as_str()).collect();
        assert_eq!(names, ["Bulma", "Vegeta", "Oolong"]);
        assert_eq!(view.items[1].attribute, 0x80);
        let selected = view.selected.expect("the cursor is on Vegeta");
        assert_eq!((selected.object, selected.name.as_str()), (19, "Vegeta"));
        assert_eq!((selected.price, selected.battle, selected.field), (None, true, false));
    }

    #[test]
    fn porungas_box_shows_the_wishes_he_has_left_not_a_price() {
        let mut ram = item_screen();
        let mut labels = labels();
        labels.objects[15] =
            ObjectLabel { name: "Porunga".to_owned(), price: Some(150), ..ObjectLabel::default() };
        // The list is Bulma, Vegeta, Porunga with the cursor on the third; his state word is
        // 0x0200 (the second byte is the wishes left)
        for (at, byte) in [(4_u32, 15_u8), (5, 0x40)] {
            set(&mut ram, d::ITEM_LIST + at, byte);
        }
        set(&mut ram, d::MAP_ENTRY_VALUE, 2);
        set(&mut ram, d::ITEM_EXTRA + 2 * 2 + 1, 2);
        let probe = MenuProbe::new(&ram, &labels);
        let selected = probe.view().unwrap().selected.expect("the cursor is on Porunga");
        assert_eq!((selected.object, selected.price), (15, Some(2)));
        // Another object keeps its price
        set(&mut ram, d::MAP_ENTRY_VALUE, 0);
        let probe = MenuProbe::new(&ram, &labels);
        assert_eq!(probe.view().unwrap().selected.unwrap().price, Some(3));
    }

    #[test]
    fn the_party_is_in_party_order_with_its_records() {
        let ram = item_screen();
        let labels = labels();
        let view = MenuProbe::new(&ram, &labels).view().unwrap();
        let members: Vec<(&str, u8, u16, u16, Option<u8>)> =
            view.party.iter().map(|m| (m.name.as_str(), m.level, m.hp, m.max_hp, m.face)).collect();
        assert_eq!(members, [("Piccolo", 2, 84, 84, Some(29)), ("Goku", 10, 190, 200, Some(28))]);
    }

    #[test]
    fn a_town_is_its_game_mode_or_its_command_screen() {
        let labels = labels();
        let mut ram = item_screen();
        set(&mut ram, d::MENU_SCREEN, SCREEN_COMMANDS);
        assert!(!MenuProbe::new(&ram, &labels).town(), "the field");
        set(&mut ram, d::GAME_MODE, TOWN_MODE);
        assert!(MenuProbe::new(&ram, &labels).town(), "a town, before its screen is set");
        set(&mut ram, d::GAME_MODE, 0x06);
        set(&mut ram, d::MENU_SCREEN, SCREEN_TOWN);
        assert!(MenuProbe::new(&ram, &labels).town(), "the town's command screen");
    }

    #[test]
    fn the_command_window_knows_flying_and_town() {
        let labels = labels();
        let mut ram = item_screen();
        set(&mut ram, d::MENU_SCREEN, SCREEN_COMMANDS);
        let view = MenuProbe::new(&ram, &labels).view().unwrap();
        assert_eq!(
            (view.screen, view.town, view.flying, view.selected),
            (Screen::Command, false, false, None)
        );
        set(&mut ram, d::TRAVEL_MODE, TRAVEL_FLYING);
        assert!(MenuProbe::new(&ram, &labels).view().unwrap().flying);
        set(&mut ram, d::MENU_SCREEN, SCREEN_TOWN);
        let view = MenuProbe::new(&ram, &labels).view().unwrap();
        assert!(view.town && !view.flying, "a town has no flying");
    }

    #[test]
    fn the_menu_list_is_a_view_and_the_other_screens_are_not() {
        let labels = labels();
        let mut ram = item_screen();
        set(&mut ram, d::MENU_SCREEN, SCREEN_MENU);
        let view = MenuProbe::new(&ram, &labels).view().expect("the Menu list");
        assert_eq!(view.screen, Screen::Menu);
        set(&mut ram, d::MENU_SCREEN, 6);
        assert!(MenuProbe::new(&ram, &labels).view().is_none(), "Cards is the original's");
        set(&mut ram, d::FIELD_STATE, 0);
        set(&mut ram, d::MENU_SCREEN, SCREEN_MENU);
        assert!(MenuProbe::new(&ram, &labels).view().is_none(), "no menu open");
        assert!(MenuProbe::new(&ram[..0x100], &labels).view().is_none(), "too short");
    }

    #[test]
    fn the_menu_is_ready_when_it_is_idle_and_on_the_list_at_its_step() {
        let labels = labels();
        let mut ram = item_screen();
        assert!(MenuProbe::new(&ram, &labels).ready(), "the Item list at its step");
        set(&mut ram, d::MENU_STEP, 0);
        assert!(!MenuProbe::new(&ram, &labels).ready(), "the list is not drawn yet");
        set(&mut ram, d::MENU_STEP, ITEMS_LIST_STEP);
        set(&mut ram, d::MENU_DRAW_DONE, 0);
        assert!(!MenuProbe::new(&ram, &labels).ready(), "the window queue is still drawing");
        set(&mut ram, d::MENU_DRAW_DONE, DRAW_QUEUE_EMPTY);
        set(&mut ram, d::MENU_SCREEN, SCREEN_COMMANDS);
        set(&mut ram, d::MENU_BUSY, 1);
        assert!(!MenuProbe::new(&ram, &labels).ready(), "busy");
        set(&mut ram, d::MENU_BUSY, 0);
        assert!(MenuProbe::new(&ram, &labels).ready(), "idle");
        set(&mut ram, d::FIELD_STATE, 0);
        assert!(MenuProbe::new(&ram, &labels).ready(), "a free field takes a press (to open)");
        assert!(!MenuProbe::new(&ram, &labels).menu_open());
    }

    #[test]
    fn the_view_as_takes_the_hd_cursor_and_page_not_the_originals() {
        let labels = labels();
        let ram = item_screen();
        let probe = MenuProbe::new(&ram, &labels);
        let view = probe.view_as(Screen::Items, false, false, 0, 0).unwrap();
        assert_eq!(view.selected.map(|s| s.object), Some(2), "the HD cursor is on Bulma");
        let view = probe.view_as(Screen::Menu, false, false, 3, 0).unwrap();
        assert_eq!((view.screen, view.cursor, view.selected), (Screen::Menu, 3, None));
    }

    #[test]
    fn the_field_and_the_other_screens_are_not_hd_views() {
        let labels = labels();
        let mut ram = item_screen();
        ram[(d::FIELD_STATE - WRAM_BASE) as usize] = 0;
        assert!(!MenuProbe::new(&ram, &labels).is_open());
        assert!(MenuProbe::new(&ram, &labels).view().is_none(), "no menu open");
        ram[(d::FIELD_STATE - WRAM_BASE) as usize] = 1;
        ram[(d::MENU_SCREEN - WRAM_BASE) as usize] = 7;
        assert!(MenuProbe::new(&ram, &labels).view().is_none(), "Status is the original's");
        assert!(MenuProbe::new(&ram[..0x100], &labels).view().is_none(), "too short for work RAM");
    }

    #[test]
    fn an_entry_without_a_name_is_a_question_mark() {
        let labels = Labels::default();
        let ram = item_screen();
        let view = MenuProbe::new(&ram, &labels).view().unwrap();
        assert!(view.items.iter().all(|slot| slot.name == "?"));
        assert!(view.selected.is_none());
        let names: Vec<&str> = view.party.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["?", "?"], "two members in the party, without names or faces");
        assert!(view.party.iter().all(|m| m.face.is_none()));
    }
}
