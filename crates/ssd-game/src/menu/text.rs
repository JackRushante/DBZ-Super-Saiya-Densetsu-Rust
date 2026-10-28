//! The HD Text screen's plan. The HD screen holds the speed the player moves to, and writes nothing: the game's own
//! setting (`text_speed`, `$01D0`, also the Options > Gameplay > Text speed row) changes only when A keeps it, and
//! then the original's own Text screen takes it. The plan opens that screen from the Menu list the original never
//! left, moves its cursor to the speed with the d-pad and presses A, which is what the original's screen does with
//! its choice (it stores the cursor in `$01D0` and closes the menu). B writes nothing, so the old speed stays.

use super::presses::{Button, Step, Until};
use ssd_menus::nav::ORIGINAL_MENU_ROWS;

/// The row of Text in the original's Menu list (Cards, Status, Order, Text, Save).
const MENU_ROW: u8 = 3;
/// The original's Text screen (`menu_screen`).
pub(super) const SCREEN: u8 = 9;
/// The speeds the original's screen offers: its cursor runs from 0 (the fastest) to 4.
const SPEEDS: u8 = 5;
/// Frames the original's Text screen runs once it is drawn, before it takes the press (a press in the first frames
/// after a draw is lost).
const SETTLE: u32 = 30;

/// The steps of keeping `speed` (0 the fastest to 4 the slowest, the byte of `$01D0`): the Text entry of the original's
/// Menu list, the cursor of its Text screen on `speed`, and A, which closes the menu.
pub(super) fn keep_steps(speed: u8) -> Vec<Step> {
    vec![
        Step::Row { row: MENU_ROW, rows: ORIGINAL_MENU_ROWS as u8 },
        Step::Press { button: Button::A, until: Until::Screen(SCREEN) },
        Step::Wait { until: Until::Ready },
        Step::Pause { frames: SETTLE },
        Step::Column { column: speed.min(SPEEDS - 1), columns: SPEEDS },
        Step::Press { button: Button::A, until: Until::Closed },
    ]
}

#[cfg(test)]
mod tests {
    use super::super::driver::{Curtain, Driver};
    use super::super::testing::FakeMenu;
    use super::*;
    use ssd_menus::{Key, Labels, Screen};
    use ssd_oracle::Joypad;

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
        for _ in 0..2_000 {
            pads.push(frame(driver, menu, Joypad::default()));
            if !driver.busy() {
                return pads;
            }
        }
        panic!("the plan did not end");
    }

    /// The HD Text screen up over the original's Menu list, with the game's speed `speed`.
    fn on_text(speed: u8) -> (Driver, FakeMenu) {
        let mut driver = Driver::new(Labels::default());
        let mut menu = FakeMenu::default();
        menu.text_speed = speed;
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
        for _ in 0..3 {
            driver.key(Key::Down, &menu.wram());
        }
        choose(&mut driver, &mut menu, Key::Confirm);
        (driver, menu)
    }

    #[test]
    fn the_menu_lists_text_opens_the_hd_screen_on_the_games_speed_and_sends_nothing_to_the_original()
     {
        for speed in 0..5 {
            let (driver, menu) = on_text(speed);
            assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Text));
            assert_eq!(driver.nav().text().speed(), speed, "the cursor is on the game's speed");
            assert_eq!(menu.screen, 4, "the original keeps its Menu list: no press went to it");
            assert!(!driver.busy(), "no plan runs for the screen itself");
            assert!(driver.scene(&menu.wram()).is_some(), "the HD draws the screen");
        }
    }

    #[test]
    fn moving_the_speed_writes_nothing_and_b_leaves_the_old_speed_and_the_menu_list() {
        let (mut driver, mut menu) = on_text(2);
        let old = menu.wram();
        driver.key(Key::Right, &menu.wram());
        driver.key(Key::Right, &menu.wram());
        assert_eq!(driver.nav().text().speed(), 4);
        for _ in 0..20 {
            let frame = driver.frame(&menu.wram(), None, Joypad::default());
            assert!(frame.events.is_empty(), "the HD screen writes no byte of the game");
            assert_eq!(frame.pad, Joypad::default(), "and presses no button");
            menu.tick(frame.pad);
        }
        assert_eq!(menu.wram(), old, "the game is untouched while the speed is being chosen");
        assert!(driver.nav().text().frames() >= 20, "the preview runs");
        let pads = choose(&mut driver, &mut menu, Key::Cancel);
        assert!(pads.iter().all(|pad| *pad == Joypad::default()), "B presses nothing either");
        assert_eq!(menu.text_speed, 2, "the old speed stays");
        assert_eq!((driver.curtain(), driver.nav().screen()), (Curtain::Up, Screen::Menu));
        assert_eq!(menu.screen, 4);
        // Opened again, the screen starts on the game's speed, not on the one that was dropped.
        choose(&mut driver, &mut menu, Key::Confirm);
        assert_eq!(driver.nav().text().speed(), 2);
    }

    #[test]
    fn a_has_the_originals_text_screen_take_the_speed_with_its_own_buttons_and_close_the_menu() {
        for (from, to) in [(2, 4), (2, 0), (0, 4), (4, 0), (3, 3), (1, 2)] {
            let (mut driver, mut menu) = on_text(from);
            for _ in 0..from.abs_diff(to) {
                driver.key(if to > from { Key::Right } else { Key::Left }, &menu.wram());
            }
            assert_eq!(driver.nav().text().speed(), to);
            let pads = choose(&mut driver, &mut menu, Key::Confirm);
            assert_eq!(menu.text_speed, to, "the original stored the speed {from} -> {to}");
            assert_eq!(menu.field, 0, "the original's Text screen closes the menu");
            assert_eq!(driver.curtain(), Curtain::Down);
            // The presses are the original's own: Down to the Text row, A, the d-pad along the speeds, A.
            let edges = |pick: fn(&Joypad) -> bool| {
                (usize::from(pads.first().is_some_and(pick))
                    + pads.windows(2).filter(|w| !pick(&w[0]) && pick(&w[1])).count())
                    as u8
            };
            let (rights, lefts) = (edges(|pad| pad.right), edges(|pad| pad.left));
            let (want_right, want_left) = (to.saturating_sub(from), from.saturating_sub(to));
            assert_eq!((rights, lefts), (want_right, want_left), "{from} -> {to}");
            assert_eq!(edges(|pad| pad.a), 2, "A into the Text screen and A to keep the speed");
        }
    }

    #[test]
    fn the_plan_walks_the_cursor_the_short_way_and_never_off_the_list() {
        let steps = keep_steps(9);
        assert!(
            steps.contains(&Step::Column { column: 4, columns: 5 }),
            "a speed past the slowest is the slowest: {steps:?}"
        );
        assert_eq!(
            steps.first(),
            Some(&Step::Row { row: 3, rows: 5 }),
            "the Text row of the Menu list"
        );
    }
}
