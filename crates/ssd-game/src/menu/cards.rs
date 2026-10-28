//! The HD Cards screen's side of the original: reading the hand the original's own Cards screen deals, and the plans that
//! leave that screen. Nothing here writes game state: the original deals the cards, and the HD only reads them and
//! presses the original's own button.
//!
//! The original's Cards screen (screen 6, `docs/re/data/menus.md`) marks the five slots of `card_hand` face down (flag
//! `$40`), then turns one over every 16 frames (the flag goes to 0 and the card is drawn), and waits for A, which closes
//! the whole menu. A card is 4 x 6 tile words of the tile map, row by row; the slot's position word says where the
//! card's first tile word is, and the first value, the second value and the symbol of the card are in the slot.

use super::MenuProbe;
use super::presses::{Button, Step, Until};
use crate::data as d;
use ssd_menus::nav::{Entry, command_rows};
use ssd_menus::screens::cards::{Card, CardsView, Fighter, HAND, Place};

/// Bytes of one slot of the hand: the flag, the position word, the two values and the symbol.
const SLOT: u32 = 6;
/// The flag byte of the first slot, one before `card_hand` (which starts at the position word).
const FLAGS: u32 = d::CARD_HAND - 1;
/// A slot that is face down: its back is drawn until the original turns it over.
const FACE_DOWN: u8 = 0x40;
/// A slot that is empty (its card was played).
const EMPTY: u8 = 0x80;
/// The tile map the original draws the hand into (`$7E:3000`, 32 tile words a row), and the card's size in tile words.
const TILE_MAP: u32 = 0x7E_3000;
const ROW_BYTES: u32 = 0x40;
const COLUMNS: u32 = 4;
const ROWS: u32 = 6;
/// Frames the original runs after its menu closes before it takes the next press (see `driver::SETTLE_WINDOW`).
const SETTLE: u32 = 120;

impl MenuProbe<'_> {
    /// The five places of the hand as the original shows them now: a card turned over, or its back while the original
    /// has not dealt it (or its slot is empty).
    #[must_use]
    pub fn hand(&self) -> Vec<Place> {
        (0..HAND as u32).map(|slot| self.place(slot)).collect()
    }

    fn place(&self, slot: u32) -> Place {
        let base = FLAGS + SLOT * slot;
        let card = || {
            let flag = self.byte(base)?;
            if flag & (FACE_DOWN | EMPTY) != 0 {
                return None;
            }
            let at = u32::from(self.word(base + 1)?);
            let tiles = (0..ROWS)
                .flat_map(|row| (0..COLUMNS).map(move |column| (row, column)))
                .map(|(row, column)| self.word(TILE_MAP + at + row * ROW_BYTES + column * 2))
                .collect::<Option<Vec<u16>>>()?;
            Some(Card {
                attack: self.byte(base + 3)?,
                defence: self.byte(base + 4)?,
                symbol: self.byte(base + 5)?,
                tiles,
            })
        };
        card().map_or(Place::Back, Place::Card)
    }

    /// The character ids of the party, in party order, of the characters that have a record in use.
    fn party_characters(&self) -> Vec<u8> {
        self.party_ids()
            .into_iter()
            .filter(|&character| self.record_of(character).is_some())
            .collect()
    }

    /// The Cards screen as the HD shows it: the hand the original dealt so far and the party's fighters.
    #[must_use]
    pub fn cards_view(&self, cursor: usize) -> CardsView {
        let party = self
            .party_characters()
            .into_iter()
            .map(|character| {
                let label = self.labels.characters.get(usize::from(character));
                Fighter {
                    name: label.map_or_else(|| "?".to_owned(), |label| label.name.clone()),
                    face: label.and_then(|label| label.face),
                    class: label.and_then(|label| label.class),
                }
            })
            .collect();
        CardsView { hand: self.hand(), cursor: cursor.min(HAND - 1), party }
    }
}

/// The steps that close the menu from the original's Cards screen: A, which it takes once the hand is dealt.
#[must_use]
pub fn close_steps() -> Vec<Step> {
    vec![Step::Press { button: Button::A, until: Until::Closed }]
}

/// The steps that go back to the Menu list from the original's Cards screen. The original's screen takes only A, which
/// closes the whole menu, so the plan closes it and opens it again on the Menu list, as a player would.
#[must_use]
pub fn back_steps(flying: bool, town: bool) -> Vec<Step> {
    let rows = command_rows(flying, town);
    let row = rows.iter().position(|&entry| entry == Entry::Menu).unwrap_or(rows.len() - 1);
    vec![
        Step::Press { button: Button::A, until: Until::Closed },
        Step::Pause { frames: SETTLE },
        Step::Press { button: Button::A, until: Until::Open },
        Step::Wait { until: Until::Ready },
        Step::Pause { frames: SETTLE },
        Step::Row { row: row as u8, rows: rows.len() as u8 },
        Step::Press { button: Button::A, until: Until::Screen(4) },
    ]
}

#[cfg(test)]
mod tests {
    use super::super::driver::{Curtain, Driver};
    use super::super::presses::Outcome;
    use super::super::testing::FakeMenu;
    use super::*;
    use ssd_menus::screens::cards::favours;
    use ssd_menus::view::Screen;
    use ssd_menus::{CharacterLabel, Key, Labels};
    use ssd_oracle::Joypad;

    /// Where the original puts the first slot's card in the tile map (row 23, column 10).
    const FIRST: u16 = 0x05D4;
    /// The record flags and the character id of a party record.
    const RECORD_FLAGS: u32 = 0x00;
    const RECORD_CHARACTER: u32 = 0x1C;

    fn labels() -> Labels {
        let class = [1, 2, 2];
        Labels {
            characters: ["Goku", "Gohan", "Piccolo"]
                .iter()
                .zip(class)
                .map(|(name, class)| CharacterLabel {
                    name: (*name).to_owned(),
                    face: None,
                    class: Some(class),
                })
                .collect(),
            ..Labels::default()
        }
    }

    fn put(ram: &mut [u8], address: u32, bytes: &[u8]) {
        let at = (address - 0x7E_0000) as usize;
        ram[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// Work RAM with a hand of five, the first `dealt` of them turned over (the others face down), the tile words of
    /// slot `n` being `n * 100 + the place in the card`, and the party Goku, Gohan and Piccolo.
    fn with_hand(mut ram: Vec<u8>, dealt: usize) -> Vec<u8> {
        for slot in 0..HAND as u32 {
            let base = FLAGS + SLOT * slot;
            let position = FIRST + 8 * slot as u16;
            put(&mut ram, base, &[if (slot as usize) < dealt { 0 } else { FACE_DOWN }]);
            put(&mut ram, base + 1, &position.to_le_bytes());
            let symbol = [2, 1, 6, 4, 7][slot as usize];
            put(&mut ram, base + 3, &[slot as u8 + 1, 8 - slot as u8, symbol]);
            for row in 0..ROWS {
                for column in 0..COLUMNS {
                    let word = slot as u16 * 100 + (row * COLUMNS + column) as u16;
                    let at = TILE_MAP + u32::from(position) + row * ROW_BYTES + column * 2;
                    put(&mut ram, at, &word.to_le_bytes());
                }
            }
        }
        for (index, character) in [0_u8, 1, 2].into_iter().enumerate() {
            put(&mut ram, d::PARTY_ORDER + index as u32, &[character]);
            let record = d::PARTY_RECORDS + 32 * index as u32;
            put(&mut ram, record + RECORD_FLAGS, &[0xC0]);
            put(&mut ram, record + RECORD_CHARACTER, &[character]);
        }
        put(&mut ram, d::PARTY_ORDER + 3, &[0xFF]);
        ram
    }

    fn hand(dealt: usize) -> Vec<Place> {
        let labels = labels();
        let ram = with_hand(vec![0; 0x2_0000], dealt);
        MenuProbe::new(&ram, &labels).hand()
    }

    #[test]
    fn the_hand_is_read_with_its_values_symbols_and_tile_words() {
        let hand = hand(5);
        let Place::Card(first) = &hand[0] else { panic!("dealt") };
        assert_eq!((first.attack, first.defence, first.symbol), (1, 8, 2));
        assert_eq!(first.tiles.len(), 24);
        assert_eq!(first.tiles[..5], [0, 1, 2, 3, 4], "row by row from the slot's position");
        assert_eq!(first.tiles[23], 23);
        let Place::Card(last) = &hand[4] else { panic!("dealt") };
        assert_eq!((last.attack, last.defence, last.symbol), (5, 4, 7));
        assert_eq!(last.tiles[0], 400, "each slot is read at its own position");
    }

    #[test]
    fn a_slot_the_original_has_not_turned_over_yet_is_a_back() {
        let hand = hand(2);
        assert!(matches!(hand[1], Place::Card(_)));
        assert_eq!(hand[2..], [Place::Back, Place::Back, Place::Back]);
    }

    #[test]
    fn an_empty_slot_is_a_back_too() {
        let labels = labels();
        let mut ram = with_hand(vec![0; 0x2_0000], 5);
        put(&mut ram, FLAGS + SLOT * 3, &[EMPTY]);
        assert_eq!(MenuProbe::new(&ram, &labels).hand()[3], Place::Back);
    }

    #[test]
    fn the_party_is_the_fighters_with_their_classes_so_the_favoured_are_the_games() {
        let labels = labels();
        let ram = with_hand(vec![0; 0x2_0000], 5);
        let view = MenuProbe::new(&ram, &labels).cards_view(0);
        let names: Vec<&str> = view.party.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Goku", "Gohan", "Piccolo"]);
        let favoured = |slot: usize| -> Vec<&str> {
            let Place::Card(card) = &view.hand[slot] else { panic!("dealt") };
            view.party.iter().filter(|f| favours(card, f)).map(|f| f.name.as_str()).collect()
        };
        // Symbol 2 (Demon): Gohan and Piccolo, not Goku; symbol 1 (God): Goku; 6 (Super): nobody; 7: everyone
        assert_eq!(favoured(0), ["Gohan", "Piccolo"]);
        assert_eq!(favoured(1), ["Goku"]);
        assert_eq!(favoured(2), [] as [&str; 0]);
        assert_eq!(favoured(4), ["Goku", "Gohan", "Piccolo"]);
    }

    #[test]
    fn a_party_record_that_is_not_in_use_is_not_in_the_row() {
        let labels = labels();
        let mut ram = with_hand(vec![0; 0x2_0000], 5);
        put(&mut ram, d::PARTY_RECORDS + 32 + RECORD_FLAGS, &[0x00]);
        let view = MenuProbe::new(&ram, &labels).cards_view(0);
        assert_eq!(view.party.len(), 2);
    }

    /// One frame: the driver gives the joypad, the fake menu runs it (with the hand in work RAM).
    fn frame(driver: &mut Driver, menu: &mut FakeMenu) {
        let wram = with_hand(menu.wram(), 5);
        let pad = driver.frame(&wram, None, Joypad::default()).pad;
        menu.tick(pad);
    }

    fn settle(driver: &mut Driver, menu: &mut FakeMenu) {
        for _ in 0..2_000 {
            frame(driver, menu);
            if !driver.busy() {
                return;
            }
        }
        panic!("the plan did not end");
    }

    fn key(driver: &mut Driver, menu: &mut FakeMenu, key: Key) {
        driver.key(key, &with_hand(menu.wram(), 5));
        settle(driver, menu);
    }

    /// The menu open on the Menu list, the HD cursor on Cards.
    fn on_menu_list() -> (Driver, FakeMenu) {
        let mut driver = Driver::new(labels());
        let mut menu = FakeMenu::default();
        driver.open();
        settle(&mut driver, &mut menu);
        for _ in 0..4 {
            key(&mut driver, &mut menu, Key::Down);
        }
        key(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.nav().screen(), Screen::Menu);
        (driver, menu)
    }

    #[test]
    fn cards_opens_the_hd_screen_over_the_originals_and_the_cursor_walks_the_hand() {
        let (mut driver, mut menu) = on_menu_list();
        key(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(menu.screen, 6, "the original is on its Cards screen");
        assert_eq!((driver.curtain(), driver.screen()), (Curtain::Up, Some(Screen::Cards)));
        assert!(driver.scene(&with_hand(menu.wram(), 5)).is_some());
        let cursor = |driver: &Driver| driver.nav().cursor();
        assert_eq!(cursor(&driver), 0);
        key(&mut driver, &mut menu, Key::Right);
        key(&mut driver, &mut menu, Key::Right);
        assert_eq!(cursor(&driver), 2);
        key(&mut driver, &mut menu, Key::Left);
        assert_eq!(cursor(&driver), 1);
        key(&mut driver, &mut menu, Key::Left);
        key(&mut driver, &mut menu, Key::Left);
        assert_eq!(cursor(&driver), 4, "the cursor wraps");
        assert_eq!(menu.screen, 6, "the original never moved: its screen takes only A");
        key(&mut driver, &mut menu, Key::Confirm);
        key(&mut driver, &mut menu, Key::Up);
        assert_eq!(driver.screen(), Some(Screen::Cards), "A and Up do nothing here");
    }

    #[test]
    fn b_goes_back_to_the_menu_list_through_the_originals_own_close_and_reopening() {
        let (mut driver, mut menu) = on_menu_list();
        key(&mut driver, &mut menu, Key::Confirm);
        key(&mut driver, &mut menu, Key::Right);
        key(&mut driver, &mut menu, Key::Cancel);
        assert_eq!(driver.last_outcome(), Some(Outcome::Reached));
        assert_eq!((driver.curtain(), driver.screen()), (Curtain::Up, Some(Screen::Menu)));
        assert_eq!(menu.screen, 4, "the original is on its Menu list again");
        assert_eq!(driver.nav().cursor(), 0, "on Cards, as it was left");
        // and the Cards can be opened again, with the cursor on the first card
        key(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.screen(), Some(Screen::Cards));
        assert_eq!(driver.nav().cursor(), 0);
    }

    #[test]
    fn start_closes_the_menu_with_the_originals_a() {
        let (mut driver, mut menu) = on_menu_list();
        key(&mut driver, &mut menu, Key::Confirm);
        driver.close_menu();
        settle(&mut driver, &mut menu);
        assert_eq!(menu.field, 0, "the field is free");
        assert_eq!(driver.last_outcome(), Some(Outcome::Reached));
        assert_eq!(driver.curtain(), Curtain::Down);
    }

    #[test]
    fn start_does_nothing_off_the_cards_screen() {
        let (mut driver, mut menu) = on_menu_list();
        driver.close_menu();
        assert!(!driver.busy());
        settle(&mut driver, &mut menu);
        assert_eq!((driver.curtain(), driver.screen()), (Curtain::Up, Some(Screen::Menu)));
    }
}
