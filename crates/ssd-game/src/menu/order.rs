//! The HD Order screen's side of the original: reading the party as the original's own Order screen lists it, and the
//! plan that applies the order the HD holds. The HD never writes the order: it plays the original's own presses, the
//! way a player orders the party, and the original's code copies the records into the new order (the party list at `$0190` is written only by a Save).
//!
//! The original's Order screen (screen 8, `docs/re/data/menus.md`) lists the party's records in their order (one row
//! each) and fills five numbered slots in turn: A on a row copies that record to the next slot and takes the row out of
//! the list (the cursor skips it from then on, and moves to the next row left); when one row is left it is taken by
//! itself, and a Yes or No asks for the order (the cursor starts on Yes). Yes writes the order back and closes the menu,
//! No empties the slots. B takes back the last pick, or leaves the screen when there is none.

use super::MenuProbe;
use super::presses::{Button, Step, Until};
use crate::data as d;
use ssd_menus::Labels;
use ssd_menus::nav::ORIGINAL_MENU_ROWS;
use ssd_menus::screens::{OrderMember, OrderNav, OrderView};

/// The row of Order in the original's Menu list (Cards, Status, Order, Text, Save).
const MENU_ROW: u8 = 2;
/// The original's Order screen (`menu_screen`).
pub(super) const SCREEN: u8 = 8;
/// The step that takes the picks, and the step that asks Yes or No.
const PICKING: u8 = 1;
pub(super) const ASKING: u8 = 2;
/// The number of members in the party (`$01BB`) and the number of slots filled so far (`$0D6B`).
const SIZE: u32 = 0x7E_01BB;
const PICKED: u32 = 0x7E_0D6B;
/// The rows of the Yes or No question.
const ANSWERS: u8 = 2;
/// Frames the original runs once its Order screen is drawn, and after each pick, before it takes the next press (a press
/// in the first frames after a draw is lost).
const SETTLE: u32 = 30;
const PICK_SETTLE: u32 = 10;
/// The records of the party, and the id of the character in each.
const RECORDS: u32 = 9;
const RECORD_LENGTH: u32 = 32;
const RECORD_FLAGS: u32 = 0x00;
const RECORD_CHARACTER: u32 = 0x1C;
const RECORD_IN_USE: u8 = 0x80;

impl MenuProbe<'_> {
    /// The characters of the party in the rows of the original's Order list: the records in their order. `None` when the
    /// records do not make a party of the size the game holds (the HD does not offer the screen then).
    #[must_use]
    pub fn order_rows(&self) -> Option<Vec<u8>> {
        let size = u32::from(self.byte(SIZE)?);
        if size == 0 || size > RECORDS {
            return None;
        }
        (0..size)
            .map(|row| {
                let record = d::PARTY_RECORDS + row * RECORD_LENGTH;
                let used = self.byte(record + RECORD_FLAGS)? & RECORD_IN_USE != 0;
                used.then(|| self.byte(record + RECORD_CHARACTER)).flatten()
            })
            .collect()
    }

    /// The slots the original's Order screen has filled (`$0D6B`).
    #[must_use]
    pub(super) fn order_picked(&self) -> u8 {
        self.byte(PICKED).unwrap_or(0)
    }
}

/// What the Order screen shows: the members in the order made, with their names and faces.
#[must_use]
pub fn view(nav: &OrderNav, labels: &Labels) -> OrderView {
    let members = nav
        .members()
        .into_iter()
        .map(|character| {
            let label = labels.characters.get(usize::from(character));
            OrderMember {
                name: label.map_or_else(|| "?".to_owned(), |label| label.name.clone()),
                face: label.and_then(|label| label.face),
            }
        })
        .collect();
    OrderView {
        members,
        cursor: nav.cursor(),
        held: nav.held(),
        changed: nav.changed(),
        can_undo: nav.can_undo(),
    }
}

/// The steps of applying `order` (for each slot, the row of the original's list of the member that goes there), the
/// original's own way: the Order entry of its Menu list, A on each row in turn (the cursor goes down to the row, which
/// skips the rows already taken; the last member is taken by the original), and A on Yes, which closes the menu.
#[must_use]
pub fn apply_steps(order: &[u8]) -> Vec<Step> {
    let rows = u8::try_from(order.len()).unwrap_or(u8::MAX);
    let mut steps = vec![
        Step::Row { row: MENU_ROW, rows: ORIGINAL_MENU_ROWS as u8 },
        Step::Press { button: Button::A, until: Until::Screen(SCREEN) },
        Step::Wait { until: Until::OrderStep(PICKING) },
        Step::Wait { until: Until::Ready },
        Step::Pause { frames: SETTLE },
    ];
    let picks = order.len().saturating_sub(1).max(1);
    for (slot, &row) in order.iter().take(picks).enumerate() {
        // The pick before the last one has the original take the last member too, and ask.
        let until = if slot + 1 >= picks {
            Until::OrderStep(ASKING)
        } else {
            Until::Picked(slot as u8 + 1)
        };
        steps.push(Step::Seek { row, rows });
        steps.push(Step::Press { button: Button::A, until });
        steps.push(Step::Wait { until: Until::Ready });
        steps.push(Step::Pause { frames: PICK_SETTLE });
    }
    steps.push(Step::Row { row: 0, rows: ANSWERS });
    steps.push(Step::Press { button: Button::A, until: Until::Closed });
    steps
}

#[cfg(test)]
mod tests {
    use super::super::driver::{Curtain, Driver};
    use super::super::presses::{Next, Outcome, Player};
    use super::super::testing::FakeMenu;
    use super::*;
    use ssd_menus::{CharacterLabel, Key, Screen};
    use ssd_oracle::Joypad;

    fn labels() -> Labels {
        Labels {
            characters: ["Goku", "Gohan", "Piccolo", "Krillin", "Tien"]
                .iter()
                .map(|name| CharacterLabel { name: (*name).to_owned(), face: None, class: None })
                .collect(),
            ..Labels::default()
        }
    }

    /// One frame: the driver gives the joypad, the fake menu runs it. Returns the joypad.
    fn frame(driver: &mut Driver, menu: &mut FakeMenu, live: Joypad) -> Joypad {
        let pad = driver.frame(&menu.wram(), None, live).pad;
        menu.tick(pad);
        pad
    }

    /// A key of the HD, then the frames of the plan it starts; the joypads they ran with.
    fn choose(driver: &mut Driver, menu: &mut FakeMenu, key: Key) -> Vec<Joypad> {
        driver.key(key, &menu.wram());
        let mut pads = Vec::new();
        for _ in 0..3_000 {
            pads.push(frame(driver, menu, Joypad::default()));
            if !driver.busy() {
                return pads;
            }
        }
        panic!("the plan did not end");
    }

    /// The HD Order screen up over the original's Menu list, with the party `party` in the original's records.
    fn on_order(party: &[u8]) -> (Driver, FakeMenu) {
        let mut driver = Driver::new(labels());
        let mut menu = FakeMenu::with_party(party);
        let mut a = Joypad { a: true, ..Joypad::default() };
        for _ in 0..40 {
            frame(&mut driver, &mut menu, a);
            a = Joypad::default();
        }
        for _ in 0..4 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.nav().screen(), Screen::Menu);
        for _ in 0..2 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        (driver, menu)
    }

    fn keys(driver: &mut Driver, menu: &FakeMenu, keys: &[Key]) {
        for &key in keys {
            driver.key(key, &menu.wram());
        }
    }

    /// The edges of a button over the joypads of a run: its presses.
    fn edges(pads: &[Joypad], pick: fn(&Joypad) -> bool) -> usize {
        usize::from(pads.first().is_some_and(pick))
            + pads.windows(2).filter(|w| !pick(&w[0]) && pick(&w[1])).count()
    }

    #[test]
    fn the_menu_lists_order_opens_the_hd_screen_on_the_party_and_sends_nothing_to_the_original() {
        let (driver, menu) = on_order(&[0, 1, 2, 3, 4]);
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Order));
        assert_eq!(driver.order().members(), vec![0, 1, 2, 3, 4]);
        assert_eq!(menu.screen, 4, "the original keeps its Menu list: no press went to it");
        assert!(!driver.busy(), "no plan runs for the screen itself");
        assert!(driver.scene(&menu.wram()).is_some(), "the HD draws the screen");
    }

    #[test]
    fn editing_the_order_writes_nothing_and_presses_nothing() {
        let (mut driver, mut menu) = on_order(&[0, 1, 2, 3, 4]);
        let old = menu.wram();
        keys(&mut driver, &menu, &[Key::Confirm, Key::Down, Key::Down, Key::Confirm]);
        assert_eq!(driver.order().members(), vec![1, 2, 0, 3, 4]);
        for _ in 0..20 {
            let frame = driver.frame(&menu.wram(), None, Joypad::default());
            assert!(frame.events.is_empty(), "the HD screen writes no byte of the game");
            assert_eq!(frame.pad, Joypad::default(), "and presses no button");
            menu.tick(frame.pad);
        }
        assert_eq!(menu.wram(), old, "the game is untouched while the order is being made");
    }

    #[test]
    fn b_undoes_moves_and_then_goes_back_to_the_menu_list_with_nothing_pressed() {
        let (mut driver, mut menu) = on_order(&[0, 1, 2]);
        keys(&mut driver, &menu, &[Key::Confirm, Key::Down, Key::Confirm]);
        assert_eq!(driver.order().members(), vec![1, 0, 2]);
        keys(&mut driver, &menu, &[Key::Cancel]);
        assert_eq!(driver.order().members(), vec![0, 1, 2], "the move is undone");
        assert_eq!(driver.nav().screen(), Screen::Order);
        let pads = choose(&mut driver, &mut menu, Key::Cancel);
        assert!(pads.iter().all(|pad| *pad == Joypad::default()), "B presses nothing either");
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Menu));
        assert_eq!((menu.screen, menu.party.clone()), (4, vec![0, 1, 2]));
        // Opened again, the screen starts on the game's order.
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.order().members(), vec![0, 1, 2]);
    }

    #[test]
    fn apply_with_no_change_does_nothing() {
        let (mut driver, mut menu) = on_order(&[0, 1, 2]);
        keys(&mut driver, &menu, &[Key::Up]);
        let pads = choose(&mut driver, &mut menu, Key::Confirm);
        assert!(pads.iter().all(|pad| *pad == Joypad::default()));
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Order));
    }

    #[test]
    fn apply_order_has_the_original_take_the_members_and_close_the_menu() {
        let (mut driver, mut menu) = on_order(&[0, 1, 2, 3, 4]);
        // Gohan to the front, then Tien (now on the fifth place) to the second place.
        keys(&mut driver, &menu, &[Key::Down, Key::Confirm, Key::Up, Key::Confirm]);
        keys(&mut driver, &menu, &[Key::Down, Key::Down, Key::Down, Key::Down, Key::Confirm]);
        keys(&mut driver, &menu, &[Key::Up, Key::Up, Key::Up, Key::Confirm]);
        assert_eq!(driver.order().members(), vec![1, 4, 0, 2, 3]);
        keys(&mut driver, &menu, &[Key::Down, Key::Down, Key::Down, Key::Down]);
        let pads = choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(menu.party, vec![1, 4, 0, 2, 3], "the original wrote the order");
        assert_eq!(menu.field, 0, "its Order screen closed the menu");
        assert_eq!(driver.curtain(), Curtain::Down);
        // The presses are the original's own: A into the screen, four picks, A on Yes; Down only walks the cursor.
        assert_eq!(edges(&pads, |pad| pad.a), 1 + 4 + 1);
        assert_eq!(edges(&pads, |pad| pad.b | pad.up | pad.left | pad.right), 0);
    }

    /// The party the original ends with after `apply_steps(order)` is played on its Order screen.
    fn played(party: &[u8], order: &[u8]) -> (FakeMenu, Vec<Joypad>) {
        let mut menu = FakeMenu::with_party(party);
        (menu.field, menu.screen) = (1, 4);
        let labels = labels();
        let mut player = Player::new(apply_steps(order));
        let mut pads = Vec::new();
        for _ in 0..5_000 {
            let ram = menu.wram();
            match player.next(&MenuProbe::new(&ram, &labels)) {
                Next::Frame(pad) => {
                    pads.push(pad);
                    menu.tick(pad);
                }
                Next::Done(outcome) => {
                    assert_eq!(outcome, Outcome::Reached, "{order:?}");
                    return (menu, pads);
                }
                Next::Stage(_) | Next::Lift => {}
            }
        }
        panic!("the plan did not end for {order:?}");
    }

    fn permutations(items: &[u8]) -> Vec<Vec<u8>> {
        if items.len() <= 1 {
            return vec![items.to_vec()];
        }
        let mut all = Vec::new();
        for (at, &item) in items.iter().enumerate() {
            let mut rest = items.to_vec();
            rest.remove(at);
            for mut tail in permutations(&rest) {
                tail.insert(0, item);
                all.push(tail);
            }
        }
        all
    }

    #[test]
    fn the_plan_gives_the_original_every_order_of_two_to_five_members() {
        for size in 2_u8..=5 {
            let party: Vec<u8> = (10..10 + size).collect();
            let rows: Vec<u8> = (0..size).collect();
            for order in permutations(&rows) {
                let (menu, pads) = played(&party, &order);
                let wanted: Vec<u8> = order.iter().map(|&row| party[usize::from(row)]).collect();
                assert_eq!(menu.party, wanted, "the original's records after {order:?}");
                assert_eq!(menu.field, 0, "Yes closes the menu: {order:?}");
                // A into the screen, a pick for each member but the last (the original takes it), and Yes.
                assert_eq!(edges(&pads, |pad| pad.a), 1 + (usize::from(size) - 1) + 1, "{order:?}");
                assert_eq!(edges(&pads, |pad| pad.b), 0, "{order:?}");
            }
        }
    }

    #[test]
    fn the_order_the_game_has_is_picked_without_a_down_past_the_menu_list() {
        let party = [10, 11, 12];
        let (menu, pads) = played(&party, &[0, 1, 2]);
        assert_eq!(menu.party, party);
        let downs = edges(&pads, |pad| pad.down);
        assert_eq!(
            downs, 2,
            "the Menu list's two Downs: the original moves its cursor after each pick"
        );
    }

    #[test]
    fn after_an_order_the_hd_reads_the_party_from_the_records_not_from_the_list_the_game_saves() {
        let (mut driver, mut menu) = on_order(&[0, 1, 2]);
        keys(&mut driver, &menu, &[Key::Confirm, Key::Down, Key::Down, Key::Confirm]);
        keys(&mut driver, &menu, &[Key::Down]);
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(menu.party, vec![1, 2, 0]);
        assert_eq!(menu.listed, vec![0, 1, 2], "the original leaves its saved list alone");
        let labels = labels();
        let ram = menu.wram();
        let probe = MenuProbe::new(&ram, &labels);
        let names: Vec<String> = probe.cards_view(0).party.into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["Gohan", "Piccolo", "Goku"], "the Cards screen's party row");
        assert_eq!(probe.party_size(), 3);
        let members: Vec<String> = probe.party().into_iter().map(|member| member.name).collect();
        assert_eq!(members, ["Gohan", "Piccolo", "Goku"], "the corners and the use-on list");
    }

    #[test]
    fn a_party_the_records_do_not_describe_keeps_the_originals_screen() {
        let labels = labels();
        let mut menu = FakeMenu::with_party(&[0, 1, 2]);
        let mut ram = menu.wram();
        // The size says four, the records hold three.
        ram[0x01BB] = 4;
        assert_eq!(MenuProbe::new(&ram, &labels).order_rows(), None);
        ram[0x01BB] = 3;
        assert_eq!(MenuProbe::new(&ram, &labels).order_rows(), Some(vec![0, 1, 2]));
        menu.party.clear();
        assert_eq!(MenuProbe::new(&menu.wram(), &labels).order_rows(), None, "no party");
    }
}
