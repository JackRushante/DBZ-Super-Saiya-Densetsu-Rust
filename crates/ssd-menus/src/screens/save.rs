//! The HD Save screen (Menu > Save): the three save slots with what each one holds, a strip for the game the player is in,
//! and the questions around the save. The positions are the approved design's (the 1280 x 720 space; the whole screen lies
//! inside its 4:3 core).
//!
//! The screen holds the choice and writes nothing: the save is the original's own. A on a slot has the driver play the
//! original's presses (its "Save?" Yes, the slot, A), so the original's code writes the battery save, the bag and the
//! cheats that ride along with it keep working as they do, and the game comes back to the field with a third A.
//!
//! A slot that holds a game is marked as one the save would replace, and A on it asks "Overwrite slot 0N?" with the
//! cursor on No. An empty slot saves at once.

use crate::draw::{Align, Color, DrawList, Op, Rect};
use crate::layout::{CURSOR_HEIGHT, LARGE_TEXT, LINE, SMALL_TEXT, palette, text_width};
use crate::nav::{Action, Key};
use crate::view::Picture;

/// The save slots of the cartridge.
pub const SLOTS: usize = 3;
/// The most portraits a row shows.
pub const MAX_FACES: usize = 5;

/// What a save slot, or the game the player is in, holds: the party's faces, the place, the leader's level, the money and
/// the number of items.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Summary {
    /// The corner face of each member of the party, in party order (`None`: a character with no face).
    pub faces: Vec<Option<u8>>,
    /// The place, when the project has a name for the map (see [`place_name`]).
    pub place: Option<&'static str>,
    /// The leader's level.
    pub level: u8,
    /// The money, in zeni.
    pub zeni: u16,
    /// The items the party carries.
    pub items: u16,
}

impl Summary {
    /// The line of the money and the items: "LV 44  ·  120P  ·  9 items".
    #[must_use]
    pub fn stats(&self) -> String {
        let items = if self.items == 1 { "item" } else { "items" };
        format!("LV {}  \u{b7}  {}P  \u{b7}  {} {items}", self.level, self.zeni, self.items)
    }

    /// The place and the level, for the question: "First island, LV 10" (the level alone without a place).
    #[must_use]
    pub fn headline(&self) -> String {
        match self.place {
            Some(place) => format!("{place}, LV {}", self.level),
            None => format!("LV {}", self.level),
        }
    }
}

/// A save slot as the cartridge has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotState {
    /// No game in it.
    Empty,
    /// A game, with what it holds when the slot could be read (`None`: a slot that is marked but cannot be read, or one
    /// the host could not look at).
    Used(Option<Summary>),
}

impl SlotState {
    /// Whether the slot holds a game: a save to it replaces that game.
    #[must_use]
    pub const fn used(&self) -> bool {
        matches!(self, Self::Used(_))
    }
}

/// The name of a place of the first chapters, by map number, from the notes of the maps (`docs/re/data/scenes.md`, "Map
/// ids", and `save-format.md`, "Maps"). The save stores the map number only, and the names are ours: only the maps the
/// notes name with some certainty have one, and the rest show no place.
#[must_use]
pub const fn place_name(map: u8) -> Option<&'static str> {
    match map {
        0 => Some("First island"),
        1 | 7 => Some("King Kai's planet"),
        2 => Some("Island cave"),
        3 => Some("Piccolo's cave"),
        4 | 5 => Some("Earth"),
        6 | 8..=19 | 22..=24 => Some("Namek"),
        20 | 21 => Some("Spaceship"),
        _ => None,
    }
}

/// Where the screen is: choosing a slot (with the overwrite question open or not), the original saving, or the save done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SavePhase {
    /// Choosing a slot.
    #[default]
    Browse,
    /// The original is saving to this slot (0 to 2): its presses run.
    Saving(u8),
    /// The original saved to this slot and shows its "Completed!": A takes the game back to the field.
    Done(u8),
}

/// The cursor of the Save screen: the slot, the overwrite question while it is open, and the phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveNav {
    cursor: u8,
    /// While "Overwrite slot 0N?" asks: the choice under the cursor, 0 Yes and 1 No.
    ask: Option<u8>,
    /// The slots that hold a game, as the host last saw the cartridge (all of them until it has).
    used: [bool; SLOTS],
    phase: SavePhase,
}

impl Default for SaveNav {
    fn default() -> Self {
        Self { cursor: 0, ask: None, used: [true; SLOTS], phase: SavePhase::Browse }
    }
}

impl SaveNav {
    /// The screen opens on the first slot, with nothing asked.
    pub fn open(&mut self) {
        *self = Self { used: self.used, ..Self::default() };
    }

    /// The slot under the cursor (0 to 2).
    #[must_use]
    pub const fn cursor(&self) -> u8 {
        self.cursor
    }

    /// The overwrite question's choice while it asks: 0 Yes and 1 No.
    #[must_use]
    pub const fn ask(&self) -> Option<u8> {
        self.ask
    }

    /// Whether the slot list is what shows: nothing asked, nothing saving or saved.
    #[must_use]
    pub const fn is_browsing(&self) -> bool {
        matches!(self.phase, SavePhase::Browse) && self.ask.is_none()
    }

    /// Where the screen is.
    #[must_use]
    pub const fn phase(&self) -> SavePhase {
        self.phase
    }

    /// The host tells which slots hold a game (every frame the screen is up).
    pub fn set_used(&mut self, used: [bool; SLOTS]) {
        self.used = used;
    }

    /// The original starts saving to `slot`.
    pub fn saving(&mut self, slot: u8) {
        self.ask = None;
        self.phase = SavePhase::Saving(slot);
    }

    /// The original saved to `slot`.
    pub fn done(&mut self, slot: u8) {
        self.ask = None;
        self.phase = SavePhase::Done(slot);
    }

    /// What `key` does. Up and Down move the cursor over the slots (wrapping); A on an empty slot saves to it, on a slot
    /// that holds a game it asks first, with the cursor on No; B goes back. While the question asks, Left and Right
    /// choose Yes or No, A answers (Yes saves, No closes the question) and B closes it. Once the save is done A or B take
    /// the game back to the field; while the original saves the keys do nothing.
    pub fn key(&mut self, key: Key) -> Action {
        match self.phase {
            SavePhase::Saving(_) => Action::None,
            SavePhase::Done(_) => match key {
                Key::Confirm | Key::Cancel => Action::SaveClose,
                _ => Action::None,
            },
            SavePhase::Browse => match self.ask {
                Some(choice) => self.key_question(key, choice),
                None => self.key_list(key),
            },
        }
    }

    fn key_list(&mut self, key: Key) -> Action {
        let last = SLOTS as u8 - 1;
        match key {
            Key::Up => self.cursor = if self.cursor == 0 { last } else { self.cursor - 1 },
            Key::Down => self.cursor = if self.cursor == last { 0 } else { self.cursor + 1 },
            Key::Confirm if self.used[usize::from(self.cursor)] => self.ask = Some(1),
            Key::Confirm => return Action::SaveSlot(self.cursor),
            Key::Cancel => return Action::Back,
            _ => {}
        }
        Action::None
    }

    fn key_question(&mut self, key: Key, choice: u8) -> Action {
        match key {
            Key::Left => self.ask = Some(0),
            Key::Right => self.ask = Some(1),
            Key::Up | Key::Down | Key::Tab | Key::TabBack => self.ask = Some(choice ^ 1),
            Key::Confirm => {
                self.ask = None;
                if choice == 0 {
                    return Action::SaveSlot(self.cursor);
                }
            }
            Key::Cancel => self.ask = None,
            Key::ResetPage | Key::ResetAll | Key::Apply => {}
        }
        Action::None
    }
}

/// What the Save screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveView {
    /// The game the player is in.
    pub now: Summary,
    /// The three slots.
    pub slots: [SlotState; SLOTS],
    /// The slots saved with a cheat on (their CHEATS badge).
    pub badges: [bool; SLOTS],
    /// The slot under the cursor.
    pub cursor: u8,
    /// The overwrite question's choice while it asks: 0 Yes and 1 No.
    pub ask: Option<u8>,
    pub phase: SavePhase,
}

const WINDOW: Rect = Rect::new(176.0, 30.0, 928.0, 580.0);
const HELP_WINDOW: Rect = Rect::new(176.0, 626.0, 928.0, 84.0);
const LEFT: f32 = 206.0;
const RIGHT: f32 = 1074.0;
const RULE_WIDTH: f32 = RIGHT - LEFT;
/// Where the faces and the texts of a row start.
const FACES_X: f32 = 330.0;
const TEXT_X: f32 = 720.0;
/// The vertical middle of the strip and of the three rows, and the rules between them.
const NOW_MID: f32 = 156.0;
const ROW_MID: [f32; SLOTS] = [278.0, 402.0, 526.0];
const RULES: [f32; 3] = [208.0, 340.0, 464.0];
const ROW_TINT: Color = Color([0xFF, 0x52, 0x00, 40]);
const PLACE_TEXT: f32 = 24.0;
const HELP_TEXT: f32 = 22.0;

/// The label a slot number carries: "01" to "03".
fn number(slot: u8) -> String {
    format!("{:02}", slot + 1)
}

/// The Save screen.
#[must_use]
pub fn save_screen(view: &SaveView) -> DrawList {
    let mut list = DrawList::new();
    list.push(Op::Backdrop);
    match (view.phase, view.ask) {
        (SavePhase::Saving(slot), _) => {
            message(&mut list, &format!("Saving slot {}...", number(slot)), None);
        }
        (SavePhase::Done(slot), _) => {
            message(&mut list, &format!("Saved to slot {}.", number(slot)), Some("A  OK"));
        }
        (SavePhase::Browse, Some(choice)) => question(&mut list, view, choice),
        (SavePhase::Browse, None) => slots(&mut list, view),
    }
    list
}

/// The list: the title, the strip of the game the player is in, the three slots and the help window.
fn slots(list: &mut DrawList, view: &SaveView) {
    list.push(Op::Window { rect: WINDOW });
    list.text(LEFT, 69.0, "Save", LARGE_TEXT, palette::BLACK, Align::Left);
    list.text(RIGHT, 69.0, "Choose a slot", SMALL_TEXT, palette::DARK_RED, Align::Right);
    list.fill(Rect::new(LEFT, 96.0, RULE_WIDTH, LINE), palette::DARK_RED);
    // The game the player is in: a strip to compare the slots with.
    list.text(214.0, NOW_MID, "Now", 20.0, palette::DARK_RED, Align::Left);
    faces(list, &view.now.faces, Rect::new(FACES_X, 128.0, 56.0, 56.0), 68.0);
    summary_text(list, &view.now, NOW_MID - 10.0, 26.0);
    for rule in RULES {
        list.fill(Rect::new(LEFT, rule, RULE_WIDTH, LINE), palette::DARK_RED);
    }
    for (index, slot) in view.slots.iter().enumerate() {
        let slot_number = index as u8;
        let mid = ROW_MID[index];
        let on = slot_number == view.cursor;
        if on {
            let (top, bottom) =
                (RULES[index] + 2.0, RULES.get(index + 1).map_or(588.0, |rule| *rule));
            list.fill(Rect::new(190.0, top, 900.0, bottom - top), ROW_TINT);
            list.push(Op::Cursor { x: 194.0, y: mid - CURSOR_HEIGHT / 2.0 });
        }
        list.text(230.0, mid, &number(slot_number), LARGE_TEXT, palette::BLACK, Align::Left);
        match slot {
            SlotState::Empty => {
                list.text(FACES_X, mid, "Empty", PLACE_TEXT, palette::GREY, Align::Left);
            }
            SlotState::Used(Some(summary)) => {
                let shown =
                    faces(list, &summary.faces, Rect::new(FACES_X, mid - 32.0, 64.0, 64.0), 74.0);
                summary_text(list, summary, mid - 12.0, 28.0);
                if on {
                    replaced(list, shown, mid);
                }
            }
            SlotState::Used(None) => {
                list.text(FACES_X, mid, "Saved game", PLACE_TEXT, palette::BLACK, Align::Left);
                if on {
                    replaced(list, 0, mid);
                }
            }
        }
        if view.badges[index] {
            badge(list, mid);
        }
    }
    list.push(Op::Window { rect: HELP_WINDOW });
    let line = match view.slots[usize::from(view.cursor.min(SLOTS as u8 - 1))] {
        SlotState::Empty => {
            format!("Slot {} is empty. Saving puts the game you are in there.", number(view.cursor))
        }
        SlotState::Used(_) => format!(
            "Slot {} holds a game. Saving puts the one you are in over it.",
            number(view.cursor)
        ),
    };
    list.text(LEFT, 655.0, &line, HELP_TEXT, palette::BLACK, Align::Left);
    for (x, text) in [(LEFT, "A  Save"), (326.0, "B  Back"), (446.0, "Start  Close")] {
        list.text(x, 688.0, text, SMALL_TEXT, palette::DARK_RED, Align::Left);
    }
}

/// The party's faces in a row of boxes of the size of `first`, `step` apart, up to [`MAX_FACES`]. Returns how many.
fn faces(list: &mut DrawList, faces: &[Option<u8>], first: Rect, step: f32) -> usize {
    let shown = faces.len().min(MAX_FACES);
    for (index, face) in faces.iter().take(MAX_FACES).enumerate() {
        let rect = Rect::new(first.x + step * index as f32, first.y, first.w, first.h);
        list.push(Op::Portrait { rect, picture: face.map(Picture::Face) });
    }
    shown
}

/// The place (when there is one) and the stats of a summary: the place on the line at `y`, the stats `gap` pixels under
/// it. Without a place the stats take the place's line.
fn summary_text(list: &mut DrawList, summary: &Summary, y: f32, gap: f32) {
    match summary.place {
        Some(place) => {
            list.text(TEXT_X, y, place, PLACE_TEXT, palette::BLACK, Align::Left);
            list.text(
                TEXT_X,
                y + gap,
                &summary.stats(),
                SMALL_TEXT,
                palette::DARK_RED,
                Align::Left,
            );
        }
        None => list.text(
            TEXT_X,
            y + gap / 2.0,
            &summary.stats(),
            SMALL_TEXT,
            palette::DARK_RED,
            Align::Left,
        ),
    }
}

/// "Replaced when you save", on the used slot under the cursor: beside the faces when they leave room, else under the
/// texts. `shown` is the number of faces of the row.
fn replaced(list: &mut DrawList, shown: usize, mid: f32) {
    let text = "Replaced when you save";
    let width = text_width(text, SMALL_TEXT);
    let x = FACES_X + 74.0 * shown as f32 + 12.0;
    if shown > 0 && x + width <= TEXT_X - 10.0 {
        list.text(x, mid, text, SMALL_TEXT, palette::DARK_RED, Align::Left);
    } else {
        list.text(TEXT_X, mid + 44.0, text, SMALL_TEXT, palette::DARK_RED, Align::Left);
    }
}

/// The CHEATS tag of a slot saved with a cheat on, at the right end of its row.
fn badge(list: &mut DrawList, mid: f32) {
    let rect = Rect::new(RIGHT - 80.0, mid - 11.0, 80.0, 22.0);
    list.fill(rect, palette::CREAM);
    list.push(Op::Outline { rect, width: LINE, color: palette::ORANGE });
    list.text(
        rect.x + rect.w / 2.0,
        rect.middle_y(),
        "CHEATS",
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Centre,
    );
}

/// "Overwrite slot 0N?" over the field: what the slot holds, and Yes and No with the cursor on `choice`.
fn question(list: &mut DrawList, view: &SaveView, choice: u8) {
    list.push(Op::Window { rect: Rect::new(380.0, 250.0, 520.0, 200.0) });
    list.text(
        640.0,
        306.0,
        &format!("Overwrite slot {}?", number(view.cursor)),
        LARGE_TEXT,
        palette::BLACK,
        Align::Centre,
    );
    if let SlotState::Used(Some(summary)) =
        &view.slots[usize::from(view.cursor.min(SLOTS as u8 - 1))]
    {
        list.text(640.0, 344.0, &summary.headline(), SMALL_TEXT, palette::DARK_RED, Align::Centre);
    }
    for (index, (centre, label)) in [(520.0, "Yes"), (740.0, "No")].into_iter().enumerate() {
        let on = index as u8 == choice;
        if on {
            let chip = Rect::new(centre - 60.0, 380.0, 120.0, 44.0);
            list.fill(chip, palette::ORANGE);
            list.push(Op::Outline { rect: chip, width: LINE, color: palette::DARK_RED });
        }
        let color = if on { palette::CREAM } else { palette::BLACK };
        list.text(centre, 402.0, label, 24.0, color, Align::Centre);
    }
}

/// A small window with one line over the field: "Saved to slot 02." and its key, or "Saving slot 02...".
fn message(list: &mut DrawList, line: &str, key: Option<&str>) {
    let height = if key.is_some() { 160.0 } else { 120.0 };
    list.push(Op::Window { rect: Rect::new(380.0, 250.0, 520.0, height) });
    let y = if key.is_some() { 312.0 } else { 310.0 };
    list.text(640.0, y, line, LARGE_TEXT, palette::BLACK, Align::Centre);
    if let Some(key) = key {
        list.text(640.0, 366.0, key, SMALL_TEXT, palette::DARK_RED, Align::Centre);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(place: Option<&'static str>, faces: usize) -> Summary {
        Summary {
            faces: (0..faces as u8).map(Some).collect(),
            place,
            level: 44,
            zeni: 120,
            items: 9,
        }
    }

    fn view(slots: [SlotState; SLOTS], cursor: u8) -> SaveView {
        SaveView {
            now: summary(Some("Namek"), 5),
            slots,
            badges: [false; SLOTS],
            cursor,
            ask: None,
            phase: SavePhase::Browse,
        }
    }

    fn texts(list: &DrawList) -> Vec<String> {
        list.texts().map(|(_, _, text)| text.to_owned()).collect()
    }

    fn nav(used: [bool; SLOTS]) -> SaveNav {
        let mut nav = SaveNav::default();
        nav.set_used(used);
        nav.open();
        nav
    }

    #[test]
    fn a_summary_reads_as_the_design_the_level_the_money_and_the_items() {
        assert_eq!(summary(None, 2).stats(), "LV 44  \u{b7}  120P  \u{b7}  9 items");
        let one = Summary { items: 1, ..summary(None, 1) };
        assert!(one.stats().ends_with("1 item"), "{}", one.stats());
        assert_eq!(summary(Some("Namek"), 1).headline(), "Namek, LV 44");
        assert_eq!(summary(None, 1).headline(), "LV 44", "no place, no comma");
    }

    #[test]
    fn the_place_names_are_the_notes_and_the_maps_they_do_not_name_show_none() {
        assert_eq!(place_name(0), Some("First island"));
        assert_eq!(place_name(7), Some("King Kai's planet"));
        assert_eq!(place_name(5), Some("Earth"));
        assert_eq!(place_name(11), Some("Namek"));
        assert_eq!(place_name(20), Some("Spaceship"));
        assert_eq!(place_name(40), None, "the caves of the late maps have no name in the notes");
        assert_eq!(place_name(255), None);
    }

    #[test]
    fn a_on_an_empty_slot_saves_at_once() {
        let mut nav = nav([true, false, false]);
        nav.key(Key::Down);
        assert_eq!(nav.cursor(), 1);
        assert_eq!(nav.key(Key::Confirm), Action::SaveSlot(1));
        assert_eq!(nav.ask(), None, "an empty slot asks nothing");
    }

    #[test]
    fn a_on_a_used_slot_asks_to_overwrite_with_the_cursor_on_no_and_no_saves_nothing() {
        let mut nav = nav([true, true, false]);
        assert_eq!(nav.key(Key::Confirm), Action::None, "A only asks");
        assert_eq!(nav.ask(), Some(1), "the cursor starts on No");
        // A on No, B, and a second A on No all close the question and save nothing.
        assert_eq!(nav.key(Key::Confirm), Action::None);
        assert_eq!(nav.ask(), None);
        nav.key(Key::Confirm);
        assert_eq!(nav.key(Key::Cancel), Action::None);
        assert_eq!(nav.ask(), None);
        assert_eq!(nav.phase(), SavePhase::Browse);
        // Left chooses Yes and A then saves to the slot the question was about.
        nav.key(Key::Down);
        nav.key(Key::Confirm);
        assert_eq!(nav.key(Key::Left), Action::None);
        assert_eq!(nav.ask(), Some(0));
        assert_eq!(nav.key(Key::Confirm), Action::SaveSlot(1));
        assert_eq!(nav.ask(), None);
        // Right goes back to No, Up and Down swap them.
        nav.key(Key::Confirm);
        nav.key(Key::Left);
        nav.key(Key::Right);
        assert_eq!(nav.ask(), Some(1));
        nav.key(Key::Up);
        assert_eq!(nav.ask(), Some(0));
        nav.key(Key::Down);
        assert_eq!(nav.ask(), Some(1));
    }

    #[test]
    fn the_cursor_wraps_b_goes_back_and_the_keys_of_the_question_do_not_move_the_cursor() {
        let mut nav = nav([false; SLOTS]);
        nav.key(Key::Up);
        assert_eq!(nav.cursor(), 2);
        nav.key(Key::Down);
        assert_eq!(nav.cursor(), 0);
        assert_eq!(nav.key(Key::Cancel), Action::Back);
        let mut used = SaveNav::default();
        used.open();
        used.key(Key::Down);
        used.key(Key::Confirm);
        used.key(Key::Down);
        assert_eq!(used.cursor(), 1, "Down answers the question's choice, not the slot");
    }

    #[test]
    fn while_the_original_saves_the_keys_do_nothing_and_a_done_save_is_closed_with_a_or_b() {
        let mut nav = nav([false; SLOTS]);
        nav.saving(2);
        for key in [Key::Confirm, Key::Cancel, Key::Down, Key::Left] {
            assert_eq!(nav.key(key), Action::None);
        }
        assert_eq!(nav.phase(), SavePhase::Saving(2));
        nav.done(2);
        assert_eq!(nav.key(Key::Down), Action::None);
        assert_eq!(
            nav.key(Key::Cancel),
            Action::SaveClose,
            "the game is saved: B does not undo it"
        );
        assert_eq!(nav.key(Key::Confirm), Action::SaveClose);
    }

    #[test]
    fn a_screen_opened_again_starts_on_the_first_slot_with_nothing_asked() {
        let mut nav = nav([true; SLOTS]);
        nav.key(Key::Down);
        nav.key(Key::Confirm);
        nav.done(1);
        nav.open();
        assert_eq!((nav.cursor(), nav.ask(), nav.phase()), (0, None, SavePhase::Browse));
    }

    #[test]
    fn the_list_is_the_design_the_strip_the_slots_and_the_marks() {
        let slots = [
            SlotState::Used(Some(summary(Some("Namek"), 5))),
            SlotState::Used(Some(summary(Some("First island"), 2))),
            SlotState::Empty,
        ];
        let list = save_screen(&view(slots, 1));
        let text = texts(&list);
        for label in [
            "Save",
            "Choose a slot",
            "Now",
            "01",
            "02",
            "03",
            "Empty",
            "First island",
            "Replaced when you save",
            "Slot 02 holds a game. Saving puts the one you are in over it.",
            "A  Save",
            "B  Back",
            "Start  Close",
        ] {
            assert!(text.iter().any(|t| t == label), "{label} in {text:?}");
        }
        assert_eq!(
            text.iter().filter(|t| t.starts_with("LV ")).count(),
            3,
            "the strip and two slots"
        );
        let portraits = list.ops().iter().filter(|op| matches!(op, Op::Portrait { .. })).count();
        assert_eq!(
            portraits,
            5 + 5 + 2,
            "the strip's five, the first slot's five, the second's two"
        );
        let cursors: Vec<&Op> =
            list.ops().iter().filter(|op| matches!(op, Op::Cursor { .. })).collect();
        assert_eq!(cursors, vec![&Op::Cursor { x: 194.0, y: 393.0 }], "the cursor is on slot 02");
        // On the empty slot the help says it is empty, and nothing is marked as replaced.
        let empty = save_screen(&view([SlotState::Empty, SlotState::Empty, SlotState::Empty], 2));
        let text = texts(&empty);
        assert!(text.iter().any(|t| t.starts_with("Slot 03 is empty")), "{text:?}");
        assert!(text.iter().all(|t| t != "Replaced when you save"));
    }

    #[test]
    fn the_replaced_mark_sits_beside_two_faces_and_under_the_texts_beside_five() {
        let mark = |faces| {
            let list = save_screen(&view(
                [
                    SlotState::Used(Some(summary(Some("Namek"), faces))),
                    SlotState::Empty,
                    SlotState::Empty,
                ],
                0,
            ));
            list.texts()
                .find(|(_, _, t)| *t == "Replaced when you save")
                .map(|(x, y, _)| (x, y))
                .expect("marked")
        };
        let (x, y) = mark(2);
        assert!(
            (x - 490.0).abs() < 0.01 && (y - 278.0).abs() < 0.01,
            "the design's place: {x} {y}"
        );
        let (x, y) = mark(5);
        assert!((x - TEXT_X).abs() < 0.01 && y > 278.0, "no room beside five faces: {x} {y}");
    }

    #[test]
    fn a_slot_with_a_cheat_badge_shows_it_and_an_unreadable_slot_says_it_holds_a_game() {
        let mut shown = view([SlotState::Used(None), SlotState::Empty, SlotState::Empty], 0);
        shown.badges = [true, false, false];
        let list = save_screen(&shown);
        let text = texts(&list);
        assert!(text.iter().any(|t| t == "CHEATS"));
        assert!(text.iter().any(|t| t == "Saved game"));
        assert!(
            text.iter().any(|t| t == "Replaced when you save"),
            "an unreadable slot is replaced too"
        );
    }

    #[test]
    fn the_question_is_the_design_with_the_cursor_on_the_choice_and_the_slots_place() {
        let slots = [
            SlotState::Used(Some(summary(Some("Namek"), 5))),
            SlotState::Used(Some(Summary { level: 10, ..summary(Some("First island"), 2) })),
            SlotState::Empty,
        ];
        let mut asked = view(slots, 1);
        asked.ask = Some(1);
        let list = save_screen(&asked);
        let text = texts(&list);
        for label in ["Overwrite slot 02?", "First island, LV 10", "Yes", "No"] {
            assert!(text.iter().any(|t| t == label), "{label} in {text:?}");
        }
        assert!(text.iter().all(|t| t != "Save"), "the list is gone under the question");
        let chip = list.ops().iter().find_map(|op| match op {
            Op::Fill { rect, color } if *color == palette::ORANGE => Some(*rect),
            _ => None,
        });
        let chip = chip.expect("the chosen answer has the orange chip");
        assert!((chip.x + chip.w / 2.0 - 740.0).abs() < 0.01, "on No: {chip:?}");
        asked.ask = Some(0);
        let list = save_screen(&asked);
        let chip = list.ops().iter().find_map(|op| match op {
            Op::Fill { rect, color } if *color == palette::ORANGE => Some(*rect),
            _ => None,
        });
        assert!((chip.expect("chip").x + 60.0 - 520.0).abs() < 0.01, "on Yes");
    }

    #[test]
    fn saving_and_done_are_small_windows_with_the_slot_and_the_key() {
        let slots = [SlotState::Empty, SlotState::Empty, SlotState::Empty];
        let mut shown = view(slots, 1);
        shown.phase = SavePhase::Saving(1);
        assert!(texts(&save_screen(&shown)).iter().any(|t| t == "Saving slot 02..."));
        shown.phase = SavePhase::Done(1);
        let text = texts(&save_screen(&shown));
        assert!(text.iter().any(|t| t == "Saved to slot 02."), "{text:?}");
        assert!(text.iter().any(|t| t == "A  OK"));
    }

    #[test]
    fn every_screen_lies_in_the_four_by_three_core() {
        let slots = [
            SlotState::Used(Some(summary(Some("King Kai's planet"), 5))),
            SlotState::Used(None),
            SlotState::Empty,
        ];
        let mut shown = view(slots, 0);
        shown.badges = [true; SLOTS];
        let mut screens = vec![save_screen(&shown)];
        shown.ask = Some(0);
        screens.push(save_screen(&shown));
        shown.ask = None;
        shown.phase = SavePhase::Done(0);
        screens.push(save_screen(&shown));
        shown.phase = SavePhase::Saving(0);
        screens.push(save_screen(&shown));
        let (left, right) = (160.0, 1120.0);
        for list in screens {
            for op in list.ops() {
                let rect = match op {
                    Op::Window { rect }
                    | Op::Fill { rect, .. }
                    | Op::Outline { rect, .. }
                    | Op::Portrait { rect, .. } => *rect,
                    Op::Cursor { x, y } => Rect::new(*x, *y, 16.0, 18.0),
                    _ => continue,
                };
                assert!(rect.x >= left && rect.right() <= right, "{op:?} leaves the 4:3 core");
            }
            for (x, _, text) in list.texts() {
                assert!(x >= left && x <= right, "{text}: {x}");
            }
        }
    }

    #[test]
    fn no_text_runs_into_the_one_beside_it_on_a_row() {
        let slots = [
            SlotState::Used(Some(summary(Some("King Kai's planet"), 5))),
            SlotState::Used(Some(summary(Some("Piccolo's cave"), 2))),
            SlotState::Empty,
        ];
        let list = save_screen(&view(slots, 1));
        for op in list.ops() {
            if let Op::Text { x, text, size, align: Align::Left, .. } = op
                && *x >= TEXT_X
            {
                assert!(
                    x + text_width(text, *size) <= RIGHT - 80.0 + 0.5,
                    "{text} reaches the badge's place"
                );
            }
        }
    }
}
