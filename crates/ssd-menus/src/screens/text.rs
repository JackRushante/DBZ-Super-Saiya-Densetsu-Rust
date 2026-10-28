//! The HD Text screen (Menu > Text): the five speeds of the messages, drawn as the Options screen's bars, with a sample
//! line that writes itself at the speed under the cursor. The positions are the approved design's (the 1280 x 720
//! space; the whole screen lies inside its 4:3 core).
//!
//! The speed is the original's own setting (`text_speed`, `$01D0`; the Options > Gameplay > Text speed row is a view of
//! the same byte), counted as the game counts it: 0 the fastest and 4 the slowest (`docs/re/data/gameplay.md`). The
//! screen shows them as 1 (Fast) to 5 (Slow), left to right, as the original's list does.

use crate::draw::{Align, Color, DrawList, Op, Rect};
use crate::layout::{ADVANCE, LARGE_TEXT, LINE, SMALL_TEXT, palette};
use crate::nav::{Action, Key};

/// The speeds the original offers.
pub const SPEEDS: u8 = 5;
/// The speed the game starts with (the middle one).
pub const DEFAULT_SPEED: u8 = 2;
/// The sample line of the preview.
pub const SAMPLE: &str = "Goku: The Dragon Balls are close. Let's go!";

/// The characters the message engine writes after `frames` frames at the speed `speed` (the byte of `$01D0`): three,
/// two or one a frame for the first three speeds, then one every second and every fourth frame
/// (`docs/re/data/gameplay.md`, the table of the handlers). A speed past the slowest writes at the slowest.
#[must_use]
pub fn typed(speed: u8, frames: u32) -> usize {
    let frames = frames as usize;
    match speed {
        0 => frames * 3,
        1 => frames * 2,
        2 => frames,
        3 => frames / 2,
        _ => frames / 4,
    }
}

/// The cursor of the Text screen: the speed it holds and the frames its preview has run since the last move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextNav {
    speed: u8,
    frames: u32,
}

impl TextNav {
    /// The screen opens on the game's speed `speed`, with the preview from its start.
    pub fn open(&mut self, speed: u8) {
        self.speed = speed.min(SPEEDS - 1);
        self.frames = 0;
    }

    /// The speed under the cursor (the byte `$01D0` would hold).
    #[must_use]
    pub const fn speed(&self) -> u8 {
        self.speed
    }

    /// The frames the preview has run.
    #[must_use]
    pub const fn frames(&self) -> u32 {
        self.frames
    }

    /// One frame of the preview.
    pub fn tick(&mut self) {
        self.frames = self.frames.saturating_add(1);
    }

    /// What `key` does: Left and Right move the cursor (to the fastest and to the slowest, no further) and start the
    /// preview again; A keeps the speed under the cursor and B goes back.
    pub fn key(&mut self, key: Key) -> Action {
        match key {
            Key::Left | Key::Right => {
                let moved = if key == Key::Left {
                    self.speed.saturating_sub(1)
                } else {
                    (self.speed + 1).min(SPEEDS - 1)
                };
                if moved != self.speed {
                    self.speed = moved;
                    self.frames = 0;
                }
                Action::None
            }
            Key::Confirm => Action::TextKeep(self.speed),
            Key::Cancel => Action::Back,
            Key::Up
            | Key::Down
            | Key::Tab
            | Key::TabBack
            | Key::ResetPage
            | Key::ResetAll
            | Key::Apply => Action::None,
        }
    }
}

/// What the Text screen shows: the speed under the cursor and the frames its preview has run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextView {
    /// The speed under the cursor, as the byte of `$01D0`: 0 the fastest to 4 the slowest.
    pub speed: u8,
    /// The frames the preview has run at that speed.
    pub frames: u32,
}

const WINDOW: Rect = Rect::new(220.0, 90.0, 840.0, 400.0);
const HELP_WINDOW: Rect = Rect::new(220.0, 524.0, 840.0, 100.0);
const LEFT: f32 = 250.0;
const RIGHT: f32 = 1030.0;
const BAR_LEFT: f32 = 328.0;
const BAR_STEP: f32 = 126.0;
const BAR_WIDTH: f32 = 120.0;
const BAR_HEIGHT: f32 = 28.0;
const BAR_TOP: f32 = 218.0;
const NUMBER_TEXT: f32 = 24.0;
const HELP_TEXT: f32 = 22.0;
/// The paper of the preview box, and the line not written yet.
const PAPER: Color = Color::rgb(0xFF, 0xF8, 0xE8);
const UNWRITTEN: Color = Color::rgb(0xD9, 0xC3, 0xA0);

/// The Text screen: the window with the five speeds and the preview, and the help window under it.
#[must_use]
pub fn text_screen(view: &TextView) -> DrawList {
    let mut list = DrawList::new();
    list.push(Op::Backdrop);
    list.push(Op::Window { rect: WINDOW });
    list.text(LEFT, 129.0, "Text speed", LARGE_TEXT, palette::BLACK, Align::Left);
    list.text(RIGHT, 129.0, "Saved with the game", SMALL_TEXT, palette::DARK_RED, Align::Right);
    list.fill(Rect::new(LEFT, 156.0, RIGHT - LEFT, LINE), palette::DARK_RED);
    speeds(&mut list, view.speed);
    preview(&mut list, *view);
    list.push(Op::Window { rect: HELP_WINDOW });
    list.text(
        LEFT,
        557.0,
        "How fast messages are written. Also in Options > Gameplay.",
        HELP_TEXT,
        palette::BLACK,
        Align::Left,
    );
    for (x, text) in [(250.0, "Left/Right  Speed"), (470.0, "A  OK"), (570.0, "B  Back")] {
        list.text(x, 598.0, text, SMALL_TEXT, palette::DARK_RED, Align::Left);
    }
    list
}

/// The five bars with the arrows at their ends, their numbers, the cell of the chosen one and the three captions.
fn speeds(list: &mut DrawList, chosen: u8) {
    let arrow_y = BAR_TOP + (BAR_HEIGHT - 18.0) / 2.0;
    list.push(Op::Arrow { x: 292.0, y: arrow_y, left: true });
    list.push(Op::Arrow { x: 972.0, y: arrow_y, left: false });
    for index in 0..SPEEDS {
        let bar = Rect::new(BAR_LEFT + BAR_STEP * f32::from(index), BAR_TOP, BAR_WIDTH, BAR_HEIGHT);
        // The bar as the Options sliders draw it: a dark red line, a white edge, a black well; the chosen one is full.
        list.fill(bar.grow(1.0), palette::DARK_RED);
        list.fill(bar, palette::WHITE);
        let well = bar.grow(-2.0);
        list.fill(well, palette::BLACK);
        let on = index == chosen;
        if on {
            list.fill(well, palette::HP);
        }
        let centre = bar.x + bar.w / 2.0;
        if on {
            let chip = Rect::new(centre - 22.0, 262.0, 44.0, 32.0);
            list.fill(chip, palette::ORANGE);
            list.push(Op::Outline { rect: chip, width: LINE, color: palette::DARK_RED });
        }
        let colour = if on { palette::CREAM } else { palette::BLACK };
        list.text(centre, 278.0, &(index + 1).to_string(), NUMBER_TEXT, colour, Align::Centre);
    }
    let last = BAR_LEFT + BAR_STEP * f32::from(SPEEDS - 1) + BAR_WIDTH;
    list.text(BAR_LEFT, 316.0, "Fast", SMALL_TEXT, palette::DARK_RED, Align::Left);
    let middle = BAR_LEFT + BAR_STEP * f32::from(DEFAULT_SPEED) + BAR_WIDTH / 2.0;
    list.text(middle, 316.0, "Default", SMALL_TEXT, palette::DARK_RED, Align::Centre);
    list.text(last, 316.0, "Slow", SMALL_TEXT, palette::DARK_RED, Align::Right);
}

/// The preview: the sample line written so far, the mark after it, the rest of the line pale, and the note under it.
fn preview(list: &mut DrawList, view: TextView) {
    list.text(LEFT, 358.0, "Preview", SMALL_TEXT, palette::DARK_RED, Align::Left);
    let box_ = Rect::new(LEFT, 374.0, RIGHT - LEFT, 64.0);
    list.fill(box_, PAPER);
    list.push(Op::Outline { rect: box_, width: LINE, color: palette::DARK_RED });
    let chars: Vec<char> = SAMPLE.chars().collect();
    let written = typed(view.speed, view.frames).min(chars.len());
    let done: String = chars[..written].iter().collect();
    let rest: String = chars[written..].iter().collect();
    let advance = NUMBER_TEXT * ADVANCE;
    let x = box_.x + 20.0;
    list.text(x, 406.0, &done, NUMBER_TEXT, palette::BLACK, Align::Left);
    // The mark and the line still to come start after the last visible character: a space is not one.
    let end = x + done.trim_end().chars().count() as f32 * advance;
    down_mark(list, end + 11.0, 406.0);
    if !rest.is_empty() {
        list.text(end + 22.0, 406.0, rest.trim_start(), NUMBER_TEXT, UNWRITTEN, Align::Left);
    }
    list.text(
        LEFT,
        464.0,
        &format!(
            "Types at speed {}. It starts again each time you change the speed.",
            view.speed + 1
        ),
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
}

/// The orange mark that points down (the game's "more" mark), 18 wide and 16 high, centred on (`x`, `y`): a dark red
/// triangle with an orange one in it, built of rows two pixels high.
fn down_mark(list: &mut DrawList, x: f32, y: f32) {
    for row in 0..8 {
        let width = 18.0 - 2.0 * row as f32;
        list.fill(
            Rect::new(x - width / 2.0, y - 8.0 + 2.0 * row as f32, width, 2.0),
            palette::DARK_RED,
        );
    }
    for row in 0..5 {
        let width = 12.0 - 2.0 * row as f32;
        list.fill(
            Rect::new(x - width / 2.0, y - 5.0 + 2.0 * row as f32, width, 2.0),
            palette::ORANGE,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(list: &DrawList) -> Vec<String> {
        list.texts().map(|(_, _, text)| text.to_owned()).collect()
    }

    #[test]
    fn the_five_speeds_write_as_the_game_does_three_two_one_a_frame_then_every_second_and_fourth() {
        assert_eq!((0..5).map(|speed| typed(speed, 8)).collect::<Vec<_>>(), vec![24, 16, 8, 4, 2]);
        assert_eq!(typed(9, 8), 2, "a byte past the slowest writes at the slowest");
        assert_eq!(typed(0, 0), 0);
        let length = SAMPLE.chars().count();
        assert_eq!(typed(2, length as u32), length, "the sample takes a frame a character at 3");
        assert!(typed(4, length as u32) < length, "and four frames a character at 5");
    }

    #[test]
    fn left_and_right_move_the_cursor_to_the_ends_and_restart_the_preview() {
        let mut nav = TextNav::default();
        nav.open(2);
        for _ in 0..30 {
            nav.tick();
        }
        assert_eq!(nav.key(Key::Right), Action::None);
        assert_eq!((nav.speed(), nav.frames()), (3, 0), "slower, and the line starts again");
        nav.tick();
        nav.key(Key::Right);
        nav.key(Key::Right);
        assert_eq!(nav.speed(), 4, "no further than the slowest");
        nav.tick();
        assert_eq!(nav.key(Key::Right), Action::None);
        assert_eq!(nav.frames(), 1, "a press that moves nothing leaves the preview running");
        for _ in 0..9 {
            nav.key(Key::Left);
        }
        assert_eq!(nav.speed(), 0, "no further than the fastest");
        assert_eq!(nav.key(Key::Up), Action::None);
    }

    #[test]
    fn a_keeps_the_speed_under_the_cursor_and_b_goes_back() {
        let mut nav = TextNav::default();
        nav.open(4);
        nav.key(Key::Left);
        assert_eq!(nav.key(Key::Confirm), Action::TextKeep(3));
        assert_eq!(nav.key(Key::Cancel), Action::Back);
        nav.open(9);
        assert_eq!(nav.speed(), 4, "a byte the screen has no cell for opens on the slowest");
    }

    #[test]
    fn the_screen_is_the_design_the_numbers_the_captions_and_one_chosen_cell() {
        let list = text_screen(&TextView { speed: 2, frames: 0 });
        let text = texts(&list);
        for label in [
            "Text speed",
            "Saved with the game",
            "Fast",
            "Default",
            "Slow",
            "Preview",
            "How fast messages are written. Also in Options > Gameplay.",
            "A  OK",
            "B  Back",
            "Types at speed 3. It starts again each time you change the speed.",
        ] {
            assert!(text.iter().any(|t| t == label), "{label} in {text:?}");
        }
        for number in ["1", "2", "3", "4", "5"] {
            assert!(text.iter().any(|t| t == number), "{number}");
        }
        let chips: Vec<&Op> = list
            .ops()
            .iter()
            .filter(|op| matches!(op, Op::Fill { color, .. } if *color == palette::ORANGE))
            .collect();
        let full: Vec<&Op> = list
            .ops()
            .iter()
            .filter(|op| matches!(op, Op::Fill { color, .. } if *color == palette::HP))
            .collect();
        assert_eq!(full.len(), 1, "one bar is full: the chosen speed");
        assert!(!chips.is_empty(), "and its number has the orange cell");
        if let Some(Op::Fill { rect, .. }) = full.first() {
            // The third bar (centre 640) sits in the middle of the window.
            assert!((rect.x + rect.w / 2.0 - 640.0).abs() < 3.0, "{rect:?}");
        }
        let arrows = list.ops().iter().filter(|op| matches!(op, Op::Arrow { .. })).count();
        assert_eq!(arrows, 2, "an arrow at each end of the bars");
    }

    #[test]
    fn the_preview_writes_the_sample_at_the_speed_and_keeps_the_rest_pale() {
        let at = |speed, frames| text_screen(&TextView { speed, frames });
        let written = |list: &DrawList| {
            list.texts()
                .find(|(_, y, _)| (*y - 406.0).abs() < 0.5)
                .map(|(_, _, t)| t.to_owned())
                .unwrap_or_default()
        };
        assert_eq!(written(&at(2, 0)), "", "nothing yet at frame 0");
        assert_eq!(written(&at(2, 4)), "Goku");
        assert_eq!(written(&at(0, 4)), "Goku: The Dr", "the fastest writes three a frame");
        assert_eq!(written(&at(4, 16)), "Goku");
        assert_eq!(written(&at(2, 500)), SAMPLE, "the whole line, kept");
        // The pale rest is drawn with the written part: both are in the box.
        let design = at(2, 26);
        assert_eq!(written(&design), "Goku: The Dragon Balls are");
        let rest = design.texts().find(|(_, _, t)| *t == "close. Let's go!");
        let (x, _, _) = rest.expect("the rest of the line");
        // The design: the written part ends at 607 and the pale rest starts 22 px after it.
        assert!((x - (270.0 + 26.0 * 24.0 * ADVANCE + 22.0)).abs() < 0.01, "{x}");
        assert!(at(2, 500).texts().all(|(_, _, t)| t != "close. Let's go!"), "nothing left pale");
    }

    #[test]
    fn the_whole_screen_lies_in_the_four_by_three_core() {
        let list = text_screen(&TextView { speed: 4, frames: 100 });
        let (left, right) = (160.0, 1120.0);
        for op in list.ops() {
            let rect = match op {
                Op::Window { rect } | Op::Fill { rect, .. } | Op::Outline { rect, .. } => *rect,
                Op::Arrow { x, y, .. } => Rect::new(*x, *y, 16.0, 18.0),
                _ => continue,
            };
            assert!(rect.x >= left && rect.right() <= right, "{op:?} leaves the 4:3 core");
        }
        for (x, _, text) in list.texts() {
            assert!(x >= left && x <= right, "{text}: {x}");
        }
    }
}
