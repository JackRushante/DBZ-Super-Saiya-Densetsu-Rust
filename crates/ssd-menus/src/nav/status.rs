//! The Status screen's keys: Up and Down move the cursor over the party's members (wrapping, as the original's lists do),
//! B goes back to the Menu list (Start is the driver's `close_menu`). The right panel reads whichever member the cursor is on, so
//! nothing is chosen and nothing is sent to the original but the way out.

use super::{Action, Key, Nav, next, previous};

impl Nav {
    /// What `key` does on the Status screen, whose party has `members` members.
    pub fn key_status(&mut self, key: Key, members: usize) -> Action {
        self.status = self.status.min(members.saturating_sub(1));
        match key {
            Key::Up if members > 0 => self.status = previous(self.status, members),
            Key::Down if members > 0 => self.status = next(self.status, members),
            Key::Cancel => return Action::Back,
            _ => {}
        }
        Action::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Screen;

    fn on_status() -> Nav {
        let mut nav = Nav::default();
        nav.enter(Screen::Status);
        nav
    }

    #[test]
    fn up_and_down_move_over_the_members_and_wrap() {
        let mut nav = on_status();
        assert_eq!(nav.cursor(), 0);
        assert_eq!(nav.key_status(Key::Down, 3), Action::None);
        assert_eq!(nav.key_status(Key::Down, 3), Action::None);
        assert_eq!(nav.cursor(), 2);
        nav.key_status(Key::Down, 3);
        assert_eq!(nav.cursor(), 0, "Down wraps after the last member");
        nav.key_status(Key::Up, 3);
        assert_eq!(nav.cursor(), 2, "Up wraps before the first");
    }

    #[test]
    fn b_goes_back_and_nothing_else_does_anything() {
        let mut nav = on_status();
        assert_eq!(nav.key_status(Key::Cancel, 5), Action::Back);
        for key in [Key::Confirm, Key::Left, Key::Right, Key::Tab, Key::Apply] {
            assert_eq!(nav.key_status(key, 5), Action::None, "{key:?}");
        }
        assert_eq!(nav.cursor(), 0);
    }

    #[test]
    fn the_cursor_never_leaves_a_party_that_shrank_and_an_empty_party_has_none() {
        let mut nav = on_status();
        nav.key_status(Key::Down, 5);
        nav.key_status(Key::Down, 5);
        nav.key_status(Key::Down, 5);
        assert_eq!(nav.cursor(), 3);
        nav.key_status(Key::Up, 2);
        assert_eq!(nav.cursor(), 0, "clamped to the last member (1), then Up goes to 0");
        nav.key_status(Key::Down, 0);
        assert_eq!(nav.cursor(), 0);
    }

    #[test]
    fn entering_the_screen_puts_the_cursor_on_the_first_member() {
        let mut nav = on_status();
        nav.key_status(Key::Down, 4);
        nav.enter(Screen::Status);
        assert_eq!(nav.cursor(), 0);
    }
}
