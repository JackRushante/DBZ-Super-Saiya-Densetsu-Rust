//! The HD Order screen (Menu > Order): the party in its order, one member to a row, with a portrait and the slot number.
//! A grabs a member, Up and Down carry it to its new place, A drops it; "Apply order" at the foot commits the order. The
//! positions are the approved design's (variant B, in the 1280 x 720 space; the whole screen lies inside its 4:3 core).
//!
//! The screen holds the new order while it is edited and writes nothing: the game's own order changes only when
//! "Apply order" is chosen, and then the original's own Order screen takes it, pick by pick (`menu/order.rs`). Slot 1
//! is the member who leads the party on the map.

use crate::draw::{Align, Color, DrawList, Op, Rect};
use crate::layout::{LARGE_TEXT, LINE, SMALL_TEXT, palette};
use crate::nav::{Action, Key};
use crate::view::Picture;

/// The most members the original's Order screen lists (its five rows and five slots).
pub const SLOTS: usize = 5;

/// The cursor and the order of the Order screen: the party as the original lists it when the screen opens
/// (`characters`, one id per row of its list), and the order the player has made (`order`, the rows of that list,
/// slot by slot).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct OrderNav {
    characters: Vec<u8>,
    order: Vec<u8>,
    cursor: usize,
    held: Option<Grab>,
    undo: Vec<Vec<u8>>,
}

/// A member held: the order as it was when it was grabbed, for B to put it back.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Grab {
    from: Vec<u8>,
}

impl OrderNav {
    /// The screen opens on `characters`, the party in the rows of the original's list, in that order.
    pub fn open(&mut self, characters: Vec<u8>) {
        self.order = (0..characters.len()).filter_map(|row| u8::try_from(row).ok()).collect();
        self.characters = characters;
        self.cursor = 0;
        self.held = None;
        self.undo.clear();
    }

    /// The character ids in the rows of the original's list.
    #[must_use]
    pub fn characters(&self) -> &[u8] {
        &self.characters
    }

    /// The order made: for each slot (first to last), the row of the original's list of the member that goes there.
    #[must_use]
    pub fn order(&self) -> &[u8] {
        &self.order
    }

    /// The character ids in the order made.
    #[must_use]
    pub fn members(&self) -> Vec<u8> {
        self.order
            .iter()
            .filter_map(|&row| self.characters.get(usize::from(row)).copied())
            .collect()
    }

    /// The row the cursor is on: a slot of the order, or the Apply row after the last one.
    #[must_use]
    pub const fn cursor(&self) -> usize {
        self.cursor
    }

    /// The slot of the member held, if one is.
    #[must_use]
    pub fn held(&self) -> Option<usize> {
        self.held.is_some().then_some(self.cursor)
    }

    /// Whether the order differs from the original's.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.order.iter().enumerate().any(|(slot, &row)| usize::from(row) != slot)
    }

    /// Whether a move can be undone.
    #[must_use]
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    /// What `key` does. Up and Down move the cursor (wrapping over the slots and the Apply row), or carry the member held;
    /// A grabs the member under the cursor, drops the one held, or applies the order on the Apply row; B puts back the
    /// member held, undoes the last move, or goes back when nothing was moved.
    pub fn key(&mut self, key: Key) -> Action {
        let members = self.order.len();
        match key {
            Key::Up | Key::Down => {
                if self.held.is_some() {
                    let to = if key == Key::Up {
                        self.cursor.saturating_sub(1)
                    } else {
                        (self.cursor + 1).min(members.saturating_sub(1))
                    };
                    if to != self.cursor {
                        self.order.swap(self.cursor, to);
                        self.cursor = to;
                    }
                } else {
                    let rows = members + 1;
                    self.cursor = if key == Key::Up {
                        (self.cursor + rows - 1) % rows
                    } else {
                        (self.cursor + 1) % rows
                    };
                }
                Action::None
            }
            Key::Confirm if self.cursor == members => {
                if self.changed() && self.held.is_none() {
                    Action::OrderApply
                } else {
                    Action::None
                }
            }
            Key::Confirm => {
                match self.held.take() {
                    Some(grab) => {
                        if grab.from != self.order {
                            self.undo.push(grab.from);
                        }
                    }
                    None => self.held = Some(Grab { from: self.order.clone() }),
                }
                Action::None
            }
            Key::Cancel => {
                if let Some(grab) = self.held.take() {
                    self.order = grab.from;
                } else if let Some(before) = self.undo.pop() {
                    self.order = before;
                } else {
                    return Action::Back;
                }
                Action::None
            }
            Key::Left
            | Key::Right
            | Key::Tab
            | Key::TabBack
            | Key::ResetPage
            | Key::ResetAll
            | Key::Apply => Action::None,
        }
    }
}

/// A member of the screen: the name and the corner face of the character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderMember {
    pub name: String,
    pub face: Option<u8>,
}

/// What the Order screen shows: the members in the order made, the cursor (a slot, or the Apply row after the last), the
/// slot held and whether the order differs from the game's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderView {
    pub members: Vec<OrderMember>,
    pub cursor: usize,
    pub held: Option<usize>,
    pub changed: bool,
    pub can_undo: bool,
}

const WINDOW: Rect = Rect::new(296.0, 40.0, 688.0, 640.0);
const LEFT: f32 = 326.0;
const RIGHT: f32 = 954.0;
/// The middle of the first row, and the distance between two rows.
const FIRST_MID: f32 = 160.0;
const PITCH: f32 = 84.0;
/// Where the rows stop: the Apply row sits under them.
const ROWS_BOTTOM: f32 = 544.0;
const NUMBER_X: f32 = 340.0;
const PORTRAIT_X: f32 = 386.0;
const NAME_X: f32 = 466.0;
const APPLY: Rect = Rect::new(326.0, 552.0, 628.0, 48.0);
const APPLY_TEXT: f32 = 22.0;
const KEYS_Y: f32 = 640.0;
/// The tint of the row under the cursor, and the paper of the member held.
const TINT: Color = Color([0xFF, 0x52, 0x00, 40]);
const HELD_PAPER: Color = Color::rgb(0xFF, 0xE2, 0xBF);

/// The Order screen: the window with the members in their slots, the Apply row, and the keys at the foot.
#[must_use]
pub fn order_screen(view: &OrderView) -> DrawList {
    let mut list = DrawList::new();
    list.push(Op::Backdrop);
    list.push(Op::Window { rect: WINDOW });
    list.text(LEFT, 79.0, "Order", LARGE_TEXT, palette::BLACK, Align::Left);
    let note = match view.held.and_then(|slot| view.members.get(slot)) {
        Some(member) => format!("Holding: {}", member.name),
        None => "Slot 1 leads the party on the map".to_owned(),
    };
    list.text(RIGHT, 79.0, &note, SMALL_TEXT, palette::DARK_RED, Align::Right);
    list.fill(Rect::new(LEFT, 106.0, RIGHT - LEFT, LINE), palette::DARK_RED);
    let count = view.members.len().max(1) as f32;
    let pitch = PITCH.min((ROWS_BOTTOM - (FIRST_MID - PITCH / 2.0)) / count);
    let mid = |slot: usize| FIRST_MID - PITCH / 2.0 + pitch / 2.0 + pitch * slot as f32;
    // The member held is drawn last, over the rows around it.
    for (slot, member) in view.members.iter().enumerate() {
        if view.held != Some(slot) {
            row(&mut list, view, slot, member, mid(slot), pitch);
        }
    }
    if let Some((slot, member)) = view.held.and_then(|s| view.members.get(s).map(|m| (s, m))) {
        row(&mut list, view, slot, member, mid(slot), pitch);
    }
    apply(&mut list, view);
    keys(&mut list, view);
    list
}

/// One slot: its number, the portrait and the name, with the cursor or the grip when it is the one under the cursor.
fn row(
    list: &mut DrawList,
    view: &OrderView,
    slot: usize,
    member: &OrderMember,
    mid: f32,
    pitch: f32,
) {
    let held = view.held == Some(slot);
    let under = view.cursor == slot;
    let band = Rect::new(LEFT - 4.0, mid - pitch / 2.0 + 4.0, RIGHT - LEFT + 8.0, pitch - 8.0);
    if held {
        list.fill(band.grow(3.0), palette::ORANGE);
        list.fill(band, HELD_PAPER);
    } else if under {
        list.fill(band, TINT);
        list.push(Op::Cursor { x: LEFT - 8.0, y: mid - 9.0 });
    }
    list.text(NUMBER_X, mid, &(slot + 1).to_string(), LARGE_TEXT, palette::DARK_RED, Align::Left);
    let size = 64.0_f32.min(pitch - 12.0);
    let frame = Rect::new(PORTRAIT_X, mid - size / 2.0, size, size);
    list.push(Op::Portrait { rect: frame, picture: member.face.map(Picture::Face) });
    list.text(NAME_X, mid, &member.name, LARGE_TEXT, palette::BLACK, Align::Left);
    if held {
        triangle(list, 900.0, mid - 14.0, true);
        triangle(list, 900.0, mid + 4.0, false);
    }
}

/// An orange triangle with a dark red edge, 18 wide and 10 high, pointing up or down, built of rows two pixels high.
fn triangle(list: &mut DrawList, x: f32, y: f32, up: bool) {
    for row in 0..5 {
        let width = if up { 4.0 + 4.0 * row as f32 } else { 20.0 - 4.0 * row as f32 };
        list.fill(
            Rect::new(x + 9.0 - width / 2.0, y + 2.0 * row as f32, width, 2.0),
            palette::DARK_RED,
        );
        if width > 6.0 {
            let inner = width - 6.0;
            list.fill(
                Rect::new(x + 9.0 - inner / 2.0, y + 2.0 * row as f32, inner, 2.0),
                palette::ORANGE,
            );
        }
    }
}

/// The Apply row: a box with its words, greyed while the order is the game's, tinted under the cursor.
fn apply(list: &mut DrawList, view: &OrderView) {
    let on = view.cursor == view.members.len() && view.held.is_none();
    list.fill(APPLY, palette::CREAM);
    if on {
        list.fill(APPLY, TINT);
    }
    list.push(Op::Outline { rect: APPLY, width: LINE, color: palette::DARK_RED });
    let colour = if view.changed { palette::BLACK } else { palette::GREY };
    list.text(
        APPLY.x + APPLY.w / 2.0,
        APPLY.middle_y(),
        "Apply order",
        APPLY_TEXT,
        colour,
        Align::Centre,
    );
    if on {
        list.push(Op::Cursor { x: APPLY.x + 12.0, y: APPLY.middle_y() - 9.0 });
    }
}

/// The three keys of the foot, as the state of the screen has them.
fn keys(list: &mut DrawList, view: &OrderView) {
    let on_apply = view.cursor == view.members.len() && view.held.is_none();
    let first = match (view.held.is_some(), on_apply) {
        (true, _) => "A  Drop",
        (false, true) => "A  Apply order",
        (false, false) => "A  Grab",
    };
    let second = if on_apply { "Up/Down  Choose" } else { "Up/Down  Move" };
    let third = match (view.held.is_some(), view.can_undo) {
        (true, _) => "B  Put back",
        (false, true) => "B  Undo move",
        (false, false) => "B  Back",
    };
    for (x, text) in [(LEFT, first), (520.0, second), (724.0, third)] {
        list.text(x, KEYS_Y, text, SMALL_TEXT, palette::DARK_RED, Align::Left);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn nav(count: u8) -> OrderNav {
        let mut nav = OrderNav::default();
        nav.open((10..10 + count).collect());
        nav
    }

    fn press(nav: &mut OrderNav, keys: &[Key]) -> Action {
        keys.iter().fold(Action::None, |_, &key| nav.key(key))
    }

    #[test]
    fn it_opens_on_the_games_order_with_nothing_changed() {
        let nav = nav(4);
        assert_eq!(nav.members(), vec![10, 11, 12, 13]);
        assert_eq!(nav.order(), [0, 1, 2, 3]);
        assert_eq!(
            (nav.cursor(), nav.held(), nav.changed(), nav.can_undo()),
            (0, None, false, false)
        );
    }

    #[test]
    fn a_grabs_a_member_up_and_down_carry_it_and_a_drops_it() {
        let mut nav = nav(5);
        press(&mut nav, &[Key::Down, Key::Confirm]);
        assert_eq!(nav.held(), Some(1));
        press(&mut nav, &[Key::Down, Key::Down]);
        assert_eq!((nav.held(), nav.members()), (Some(3), vec![10, 12, 13, 11, 14]));
        assert_eq!(nav.order(), [0, 2, 3, 1, 4], "the rows of the original's list, slot by slot");
        press(&mut nav, &[Key::Up]);
        assert_eq!(nav.members(), vec![10, 12, 11, 13, 14]);
        press(&mut nav, &[Key::Confirm]);
        assert_eq!(
            (nav.held(), nav.cursor(), nav.changed(), nav.can_undo()),
            (None, 2, true, true)
        );
    }

    #[test]
    fn a_member_held_stops_at_the_ends_and_the_cursor_wraps_over_the_apply_row() {
        let mut nav = nav(3);
        press(&mut nav, &[Key::Confirm, Key::Up, Key::Up]);
        assert_eq!(
            (nav.held(), nav.order()),
            (Some(0), [0, 1, 2].as_slice()),
            "no further than slot 1"
        );
        press(&mut nav, &[Key::Down, Key::Down, Key::Down, Key::Down]);
        assert_eq!(
            (nav.held(), nav.order()),
            (Some(2), [1, 2, 0].as_slice()),
            "nor past the last slot"
        );
        press(&mut nav, &[Key::Confirm]);
        press(&mut nav, &[Key::Down]);
        assert_eq!(nav.cursor(), 3, "the Apply row follows the last slot");
        press(&mut nav, &[Key::Down]);
        assert_eq!(nav.cursor(), 0, "and the list wraps");
        press(&mut nav, &[Key::Up]);
        assert_eq!(nav.cursor(), 3);
    }

    #[test]
    fn b_puts_back_the_member_held_then_undoes_moves_one_by_one_then_goes_back() {
        let mut nav = nav(4);
        press(&mut nav, &[Key::Confirm, Key::Down, Key::Down]);
        assert_eq!(nav.key(Key::Cancel), Action::None);
        assert_eq!(
            (nav.held(), nav.order()),
            (None, [0, 1, 2, 3].as_slice()),
            "put back where it was"
        );
        assert!(!nav.can_undo(), "a move cancelled is no move");
        nav.cursor = 0;
        press(&mut nav, &[Key::Confirm, Key::Down, Key::Confirm]);
        nav.cursor = 2;
        press(&mut nav, &[Key::Confirm, Key::Down, Key::Confirm]);
        assert_eq!(nav.order(), [1, 0, 3, 2]);
        assert_eq!(nav.key(Key::Cancel), Action::None);
        assert_eq!(nav.order(), [1, 0, 2, 3], "the last move is undone");
        assert_eq!(nav.key(Key::Cancel), Action::None);
        assert_eq!(nav.order(), [0, 1, 2, 3]);
        assert_eq!(nav.key(Key::Cancel), Action::Back, "nothing moved: B goes back");
    }

    #[test]
    fn a_drop_where_it_was_taken_is_not_a_move() {
        let mut nav = nav(3);
        press(&mut nav, &[Key::Confirm, Key::Down, Key::Up, Key::Confirm]);
        assert_eq!((nav.changed(), nav.can_undo()), (false, false));
    }

    #[test]
    fn apply_is_offered_only_for_a_changed_order_with_nothing_held() {
        let mut nav = nav(3);
        press(&mut nav, &[Key::Up]);
        assert_eq!(nav.cursor(), 3);
        assert_eq!(
            nav.key(Key::Confirm),
            Action::None,
            "the order is the game's: nothing to apply"
        );
        press(&mut nav, &[Key::Down, Key::Confirm, Key::Down, Key::Confirm]);
        press(&mut nav, &[Key::Down, Key::Down]);
        assert_eq!(nav.cursor(), 3);
        assert_eq!(nav.key(Key::Confirm), Action::OrderApply);
        assert_eq!(nav.members(), vec![11, 10, 12], "the screen keeps the order it applies");
    }

    fn view(count: usize, cursor: usize, held: Option<usize>, changed: bool) -> OrderView {
        OrderView {
            members: ["Goku", "Gohan", "Piccolo", "Krillin", "Tien"]
                .iter()
                .take(count)
                .enumerate()
                .map(|(index, name)| OrderMember {
                    name: (*name).to_owned(),
                    face: Some(index as u8),
                })
                .collect(),
            cursor,
            held,
            changed,
            can_undo: changed,
        }
    }

    fn texts(list: &DrawList) -> Vec<String> {
        list.texts().map(|(_, _, text)| text.to_owned()).collect()
    }

    #[test]
    fn the_screen_is_the_design_numbers_names_portraits_apply_and_keys() {
        let list = order_screen(&view(5, 0, None, false));
        let text = texts(&list);
        for label in [
            "Order",
            "Slot 1 leads the party on the map",
            "1",
            "5",
            "Goku",
            "Tien",
            "Apply order",
            "A  Grab",
            "Up/Down  Move",
            "B  Back",
        ] {
            assert!(text.iter().any(|t| t == label), "{label} in {text:?}");
        }
        let portraits = list.ops().iter().filter(|op| matches!(op, Op::Portrait { .. })).count();
        assert_eq!(portraits, 5);
        // The design's rows: the middles are 160, 244, 328, 412 and 496.
        let numbers: Vec<f32> = list
            .texts()
            .filter(|(x, _, _)| (*x - NUMBER_X).abs() < 0.01)
            .map(|(_, y, _)| y)
            .collect();
        assert_eq!(numbers, vec![160.0, 244.0, 328.0, 412.0, 496.0]);
        assert!(
            list.ops().iter().any(|op| matches!(op, Op::Cursor { .. })),
            "the cursor is on a row"
        );
    }

    #[test]
    fn a_member_held_is_named_in_the_title_and_carries_its_arrows() {
        let list = order_screen(&view(5, 1, Some(1), true));
        let text = texts(&list);
        assert!(text.iter().any(|t| t == "Holding: Gohan"), "{text:?}");
        for label in ["A  Drop", "B  Put back"] {
            assert!(text.iter().any(|t| t == label), "{label} in {text:?}");
        }
        assert!(
            list.ops()
                .iter()
                .any(|op| matches!(op, Op::Fill { color, .. } if *color == HELD_PAPER)),
            "the held row has its paper"
        );
        assert!(
            !list.ops().iter().any(|op| matches!(op, Op::Cursor { .. })),
            "the grip replaces the cursor"
        );
    }

    #[test]
    fn the_apply_row_is_greyed_until_the_order_changes_and_has_the_cursor_when_chosen() {
        let apply_colour = |list: &DrawList| {
            list.ops().iter().find_map(|op| match op {
                Op::Text { text, color, .. } if text == "Apply order" => Some(*color),
                _ => None,
            })
        };
        assert_eq!(apply_colour(&order_screen(&view(3, 0, None, false))), Some(palette::GREY));
        assert_eq!(apply_colour(&order_screen(&view(3, 3, None, true))), Some(palette::BLACK));
        let on = order_screen(&view(3, 3, None, true));
        assert!(texts(&on).iter().any(|t| t == "A  Apply order"));
        assert!(texts(&on).iter().any(|t| t == "B  Undo move"));
    }

    #[test]
    fn the_whole_screen_lies_in_the_four_by_three_core_and_in_the_window() {
        for count in [2, 5, 9] {
            let list = order_screen(&view(count.min(5), 0, Some(0), true));
            for op in list.ops() {
                let rect = match op {
                    Op::Window { rect }
                    | Op::Fill { rect, .. }
                    | Op::Outline { rect, .. }
                    | Op::Portrait { rect, .. } => *rect,
                    Op::Cursor { x, y } => Rect::new(*x, *y, 16.0, 18.0),
                    _ => continue,
                };
                assert!(rect.x >= 160.0 && rect.right() <= 1120.0, "{op:?} leaves the 4:3 core");
                assert!(rect.y >= 40.0 && rect.bottom() <= 680.0, "{op:?} leaves the window");
            }
        }
    }

    #[test]
    fn a_long_party_squeezes_its_rows_above_the_apply_row() {
        let mut big = view(5, 0, None, false);
        let first = big.members[0].clone();
        big.members = (0..9).map(|_| first.clone()).collect();
        let list = order_screen(&big);
        let last = list
            .texts()
            .filter(|(x, _, _)| (*x - NUMBER_X).abs() < 0.01)
            .map(|(_, y, _)| y)
            .fold(0.0_f32, f32::max);
        assert!(last < APPLY.y, "the ninth row ({last}) sits above the Apply row");
    }
}
