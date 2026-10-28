//! The HD Save screen's plan and its facts. The HD screen holds the choice (a slot, and the answer to "Overwrite slot
//! 0N?" when the slot holds a game) and writes nothing: the save is the original's own. Its first question, "Save?", is
//! not asked by the HD; the plan answers it with the original's own A, then moves the slot list's cursor to the slot with
//! the d-pad and presses A, which is what the original's screen does with its choice (`03:E556` writes the battery save,
//! then the screen shows "Completed!"). The bag, the cheats and the mod state ride along as they always did: the driver
//! follows the original's Save in work RAM (the slot list's cursor, the completion step), whatever pressed the buttons.
//!
//! What the screen shows is read from the cartridge's save (the slots) and from work RAM (the game the player is in).

use super::presses::{Button, Step, Until};
use super::{MenuProbe, keeper};
use ssd_menus::Labels;
use ssd_menus::bag::Sidecar;
use ssd_menus::nav::ORIGINAL_MENU_ROWS;
use ssd_menus::screens::{SAVE_SLOTS as SLOTS, SlotState, Summary, place_name};
use ssd_save::{SaveFile, Slot, SlotStatus};

/// The row of Save in the original's Menu list (Cards, Status, Order, Text, Save).
const MENU_ROW: u8 = 4;
/// The original's Save screen (`menu_screen`).
pub(super) const SCREEN: u8 = super::SAVE_SCREEN;
/// Frames the original's Save screen runs once it is drawn, before it takes the press (a press in the first frames after a
/// draw is lost).
const SETTLE: u32 = 30;

/// The steps of saving to `slot` (0 to 2): the Save entry of the original's Menu list, the A that answers its first
/// question (Yes is where its cursor starts), the slot list's cursor on `slot`, and the A that writes the slot. The plan
/// ends on the original's "Completed!".
pub(super) fn save_steps(slot: u8) -> Vec<Step> {
    vec![
        Step::Row { row: MENU_ROW, rows: ORIGINAL_MENU_ROWS as u8 },
        Step::Press { button: Button::A, until: Until::Screen(SCREEN) },
        Step::Wait { until: Until::Ready },
        Step::Pause { frames: SETTLE },
        Step::Press { button: Button::A, until: Until::SaveList },
        Step::Wait { until: Until::Ready },
        Step::Pause { frames: SETTLE },
        Step::Row { row: slot.min(SLOTS as u8 - 1), rows: SLOTS as u8 },
        Step::Press { button: Button::A, until: Until::SaveDone },
    ]
}

/// The steps that take the game back to the field from the original's "Completed!": its A closes the menu.
pub(super) fn close_steps() -> Vec<Step> {
    vec![Step::Press { button: Button::A, until: Until::Closed }]
}

/// The steps that leave the Menu list for the field with nothing saved: the original's own B back to its command window,
/// and B again, which closes the menu (Start on the HD Save list).
pub(super) fn leave_steps() -> Vec<Step> {
    vec![
        Step::Press { button: Button::B, until: Until::Command },
        Step::Press { button: Button::B, until: Until::Closed },
    ]
}

/// The three slots as the cartridge's save `sram` has them (`None`: the host gave no save, so every slot counts as
/// holding a game and a save asks first). The bag of a slot is the sidecar's for the slot's checksum, when it has one, so
/// the items are counted as the bag counts them.
pub(super) fn slots(sram: Option<&[u8]>, labels: &Labels, sidecar: &Sidecar) -> [SlotState; SLOTS] {
    let Some(file) = sram.and_then(|sram| SaveFile::from_bytes(sram).ok()) else {
        return std::array::from_fn(|_| SlotState::Used(None));
    };
    std::array::from_fn(|index| match file.status(index) {
        Ok(SlotStatus::Empty) => SlotState::Empty,
        Ok(SlotStatus::Good | SlotStatus::FromBackup) => match file.slot(index) {
            Ok(slot) => {
                let units = u8::try_from(index).ok().and_then(|number| {
                    let checksum = keeper::good_checksum(sram?, number)?;
                    sidecar.bag_for(number, checksum).map(ssd_menus::bag::Bag::units)
                });
                SlotState::Used(Some(summary_of(&slot, labels, units)))
            }
            Err(_) => SlotState::Used(None),
        },
        Ok(SlotStatus::Corrupt) | Err(_) => SlotState::Used(None),
    })
}

/// What a slot of the cartridge holds. `units` are the units of the slot's bag when the sidecar has it.
fn summary_of(slot: &Slot, labels: &Labels, units: Option<u32>) -> Summary {
    let party = slot.party();
    let faces = party
        .iter()
        .map(|&id| labels.characters.get(usize::from(id)).and_then(|label| label.face))
        .collect();
    let level = party
        .first()
        .and_then(|&leader| slot.record(leader).ok())
        .map_or(0, |record| record.level());
    let items = units.map_or_else(|| slot.items().len(), |units| units as usize);
    Summary {
        faces,
        place: place_name(slot.map_id()),
        level,
        zeni: slot.zeni(),
        items: u16::try_from(items).unwrap_or(u16::MAX),
    }
}

impl MenuProbe<'_> {
    /// The game the player is in, from work RAM: the party, the place, the leader's level, the money and the items
    /// (`units`: the units of the bag, when the bag is bound; else the entries of the original's list).
    pub(super) fn save_now(&self, units: Option<u32>) -> Summary {
        let party = self.party();
        let items = units.map_or_else(|| self.item_count(), |units| units as usize);
        Summary {
            faces: party.iter().map(|member| member.face).collect(),
            place: self.byte(crate::data::MAP_ID).and_then(place_name),
            level: party.first().map_or(0, |member| member.level),
            zeni: self.word(crate::data::ZENI).unwrap_or(0),
            items: u16::try_from(items).unwrap_or(u16::MAX),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::driver::{Curtain, Driver};
    use super::super::testing::FakeMenu;
    use super::*;
    use ssd_menus::{CharacterLabel, Key, Screen};
    use ssd_oracle::Joypad;
    use ssd_save::Record;

    /// A cartridge whose slots `used` hold a game: Goku (level `10 + slot`) and Piccolo, `20 + slot` zeni, on map `slot`
    /// (slot 0 is the first island, slot 1 King Kai's planet).
    fn cartridge(used: &[usize]) -> Vec<u8> {
        let mut file = SaveFile::blank();
        for &index in used {
            let mut slot = Slot::empty();
            for (id, level) in [(0_u8, 10 + index as u8), (2, 5)] {
                let mut record = Record::blank();
                record.set_flags(0xC0);
                record.set_level(level);
                record.set_character_id(id);
                slot.set_record(id, &record).unwrap();
            }
            slot.set_party(&[0, 2]).unwrap();
            slot.set_zeni(20 + index as u16).unwrap();
            slot.set_page_byte(0xA0, index as u8);
            file.set_slot(index, &slot).unwrap();
        }
        file.as_bytes().to_vec()
    }

    fn labels() -> Labels {
        let characters = (0..12_u8)
            .map(|id| CharacterLabel { name: format!("C{id}"), face: Some(id + 100), class: None })
            .collect();
        Labels { characters, ..Labels::default() }
    }

    #[test]
    fn the_plan_is_the_originals_menu_entry_its_yes_the_slot_and_its_a_and_nothing_else() {
        let steps = save_steps(2);
        assert_eq!(
            steps.first(),
            Some(&Step::Row { row: 4, rows: 5 }),
            "the Save row of the Menu list"
        );
        assert!(
            steps.contains(&Step::Row { row: 2, rows: 3 }),
            "the slot's row of the list: {steps:?}"
        );
        assert_eq!(
            steps.last(),
            Some(&Step::Press { button: Button::A, until: Until::SaveDone }),
            "the A that writes the slot is the last"
        );
        let presses = steps.iter().filter(|step| matches!(step, Step::Press { .. })).count();
        assert_eq!(presses, 3, "A into the Save, A on its question, A on the slot");
        assert_eq!(
            save_steps(9).iter().find(|step| matches!(step, Step::Row { rows: 3, .. })),
            Some(&Step::Row { row: 2, rows: 3 }),
            "a slot past the third is the third"
        );
    }

    #[test]
    fn a_slot_shows_what_the_save_holds_the_faces_the_place_the_level_the_money_and_the_items() {
        let sram = cartridge(&[0, 1]);
        let states = slots(Some(&sram), &labels(), &Sidecar::empty());
        let SlotState::Used(Some(first)) = &states[0] else { panic!("{:?}", states[0]) };
        assert_eq!(
            first.faces,
            vec![Some(100), Some(102)],
            "Goku's and Piccolo's faces, in party order"
        );
        assert_eq!((first.level, first.zeni, first.items), (10, 20, 0), "the leader's level");
        assert_eq!(first.place, Some("First island"));
        let SlotState::Used(Some(second)) = &states[1] else { panic!("{:?}", states[1]) };
        assert_eq!((second.level, second.zeni, second.place), (11, 21, Some("King Kai's planet")));
        assert_eq!(states[2], SlotState::Empty, "a slot with no mark is empty");
        // No save to look at: every slot counts as a game, so a save asks first.
        let blind = slots(None, &labels(), &Sidecar::empty());
        assert!(blind.iter().all(|state| *state == SlotState::Used(None)));
    }

    #[test]
    fn a_slot_the_checksum_refuses_is_marked_as_a_game_without_details() {
        let mut sram = cartridge(&[0]);
        let at = ssd_save::SLOT_BASES[0] + 5;
        sram[at] ^= 0xFF;
        let at = ssd_save::SLOT_BASES[0] + ssd_save::BACKUP_OFFSET + 5;
        sram[at] ^= 0xFF;
        assert_eq!(slots(Some(&sram), &labels(), &Sidecar::empty())[0], SlotState::Used(None));
    }

    /// One frame: the driver gives the joypad, the fake menu runs it.
    fn frame(driver: &mut Driver, menu: &mut FakeMenu, sram: &[u8], live: Joypad) -> Joypad {
        let pad = driver.frame(&menu.wram(), Some(sram), live).pad;
        menu.tick(pad);
        pad
    }

    /// A key of the HD, then the frames of the plan it starts; the joypads they ran with.
    fn press(driver: &mut Driver, menu: &mut FakeMenu, sram: &[u8], key: Key) -> Vec<Joypad> {
        driver.key(key, &menu.wram());
        let mut pads = Vec::new();
        for _ in 0..3_000 {
            pads.push(frame(driver, menu, sram, Joypad::default()));
            if !driver.busy() {
                return pads;
            }
        }
        panic!("the plan did not end");
    }

    /// The player's A opens the menu, the Menu list's Save entry is chosen: the HD Save screen is up, on the cartridge
    /// `sram`.
    fn open_save(driver: &mut Driver, menu: &mut FakeMenu, sram: &[u8]) {
        let mut a = Joypad { a: true, ..Joypad::default() };
        for _ in 0..40 {
            frame(driver, menu, sram, a);
            a = Joypad::default();
        }
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        press(driver, menu, sram, Key::Confirm);
        assert_eq!(driver.nav().screen(), Screen::Menu);
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        press(driver, menu, sram, Key::Confirm);
        // The slots are read on the next frame.
        frame(driver, menu, sram, Joypad::default());
    }

    fn on_save(sram: &[u8]) -> (Driver, FakeMenu) {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        open_save(&mut driver, &mut menu, sram);
        (driver, menu)
    }

    /// The presses of A in a run of joypads.
    fn a_presses(pads: &[Joypad]) -> usize {
        usize::from(pads.first().is_some_and(|pad| pad.a))
            + pads.windows(2).filter(|w| !w[0].a && w[1].a).count()
    }

    fn texts(driver: &Driver, menu: &FakeMenu) -> Vec<String> {
        driver
            .scene(&menu.wram())
            .map(|list| list.texts().map(|(_, _, text)| text.to_owned()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn the_menu_lists_save_opens_the_hd_screen_with_no_question_and_no_press_to_the_original() {
        let sram = cartridge(&[0, 1]);
        let (driver, menu) = on_save(&sram);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Save));
        assert_eq!(menu.screen, 4, "the original keeps its Menu list: its \"Save?\" is not asked");
        assert!(!driver.busy(), "no plan runs for the screen itself");
        let text = texts(&driver, &menu);
        assert!(text.iter().any(|t| t == "Choose a slot"), "{text:?}");
        assert!(text.iter().any(|t| t == "Empty"), "slot 03 is marked empty: {text:?}");
        assert_eq!(menu.saved, Vec::<u8>::new());
    }

    #[test]
    fn a_on_an_empty_slot_has_the_original_save_it_with_its_own_presses_then_a_closes_the_menu() {
        let sram = cartridge(&[0, 1]);
        let (mut driver, mut menu) = on_save(&sram);
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Down, &menu.wram());
        assert_eq!(driver.nav().save().cursor(), 2);
        let pads = press(&mut driver, &mut menu, &sram, Key::Confirm);
        assert_eq!(menu.saved, vec![2], "the original wrote slot 03, and only it");
        assert_eq!(a_presses(&pads), 3, "its entry, its question's Yes and the slot");
        assert!(pads.iter().all(|pad| !pad.b), "no B");
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Save));
        assert!(texts(&driver, &menu).iter().any(|t| t == "Saved to slot 03."));
        assert_eq!(
            (menu.screen, menu.step),
            (10, 2),
            "the original shows its \"Completed!\" under it"
        );
        // A takes the game back to the field with the original's own last A.
        let pads = press(&mut driver, &mut menu, &sram, Key::Confirm);
        assert_eq!(a_presses(&pads), 1);
        assert_eq!((menu.field, driver.curtain()), (0, Curtain::Down));
        assert_eq!(menu.saved, vec![2], "nothing more is written");
    }

    #[test]
    fn a_on_a_used_slot_asks_with_the_cursor_on_no_and_sends_nothing_until_yes() {
        let sram = cartridge(&[0, 1]);
        let (mut driver, mut menu) = on_save(&sram);
        let before = menu.wram();
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.nav().save().ask(), Some(1), "the question opens on No");
        assert!(texts(&driver, &menu).iter().any(|t| t == "Overwrite slot 01?"));
        assert!(!driver.busy());
        for _ in 0..30 {
            assert_eq!(frame(&mut driver, &mut menu, &sram, Joypad::default()), Joypad::default());
        }
        assert_eq!(menu.wram(), before, "asking writes nothing in the game");
        // A on No closes the question; nothing was saved, the original did not move.
        let pads = press(&mut driver, &mut menu, &sram, Key::Confirm);
        assert!(pads.iter().all(|pad| *pad == Joypad::default()));
        assert_eq!((driver.nav().save().ask(), menu.screen, menu.saved.len()), (None, 4, 0));
        // Yes saves the slot the question was about, slot 02 here.
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        driver.key(Key::Left, &menu.wram());
        assert_eq!(driver.nav().save().ask(), Some(0));
        let pads = press(&mut driver, &mut menu, &sram, Key::Confirm);
        assert_eq!(menu.saved, vec![1]);
        assert_eq!(a_presses(&pads), 3);
        assert_eq!(driver.nav().save().phase(), ssd_menus::screens::SavePhase::Done(1));
    }

    #[test]
    fn b_goes_back_to_the_menu_list_with_nothing_saved_and_no_press() {
        let sram = cartridge(&[0, 1]);
        let (mut driver, mut menu) = on_save(&sram);
        let pads = press(&mut driver, &mut menu, &sram, Key::Cancel);
        assert!(pads.iter().all(|pad| *pad == Joypad::default()), "B presses nothing");
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Menu));
        assert_eq!((menu.screen, menu.saved.len()), (4, 0));
        // Opened again (the Menu list's cursor stays on Save), a question closed with B leaves the list and the original
        // alone too.
        press(&mut driver, &mut menu, &sram, Key::Confirm);
        assert_eq!(driver.nav().screen(), Screen::Save);
        driver.key(Key::Confirm, &menu.wram());
        driver.key(Key::Cancel, &menu.wram());
        assert_eq!((driver.nav().screen(), driver.nav().save().ask()), (Screen::Save, None));
        assert_eq!(menu.saved, Vec::<u8>::new());
    }

    #[test]
    fn the_slots_are_read_from_the_cartridge_as_it_is_now_so_a_slot_the_save_wrote_asks_next_time()
    {
        let before = cartridge(&[0, 1]);
        let (mut driver, menu) = on_save(&before);
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(
            driver.nav().save().ask(),
            None,
            "slot 03 is empty on this cartridge: it saves at once"
        );
        // The same screen on the cartridge as the save left it: slot 03 holds a game.
        let (mut driver, mut menu) = on_save(&before);
        let after = cartridge(&[0, 1, 2]);
        frame(&mut driver, &mut menu, &after, Joypad::default());
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.nav().save().ask(), Some(1), "slot 03 holds a game now: it asks");
    }

    #[test]
    fn with_no_cartridge_to_look_at_every_slot_asks_first() {
        let sram = cartridge(&[]);
        let (mut driver, mut menu) = on_save(&sram);
        let pad = driver.frame(&menu.wram(), None, Joypad::default()).pad;
        menu.tick(pad);
        driver.key(Key::Down, &menu.wram());
        driver.key(Key::Confirm, &menu.wram());
        assert_eq!(driver.nav().save().ask(), Some(1));
    }

    #[test]
    fn start_on_the_list_closes_the_menu_with_the_originals_own_b_and_saves_nothing_but_not_over_a_question()
     {
        let sram = cartridge(&[0, 1]);
        let (mut driver, mut menu) = on_save(&sram);
        driver.key(Key::Confirm, &menu.wram());
        driver.close_menu();
        assert!(!driver.busy(), "Start does nothing while the overwrite question asks");
        driver.key(Key::Cancel, &menu.wram());
        driver.close_menu();
        assert!(driver.busy());
        let mut pads = Vec::new();
        for _ in 0..3_000 {
            pads.push(frame(&mut driver, &mut menu, &sram, Joypad::default()));
            if !driver.busy() {
                break;
            }
        }
        assert_eq!((menu.field, driver.curtain()), (0, Curtain::Down), "the menu is closed");
        assert_eq!(menu.saved, Vec::<u8>::new());
        assert!(pads.iter().all(|pad| !pad.a), "only B");
    }
}
