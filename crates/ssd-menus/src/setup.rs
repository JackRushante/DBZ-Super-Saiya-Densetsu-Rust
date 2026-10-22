//! The first-launch screens of the Android build: choose the ROM (and the patch of a Japanese ROM), choose the
//! mode, install the HD pack; and, on every platform, the language of the text before the first Enhanced launch's
//! first frame. They follow the approved boards: a cream window in an orange frame over a dark
//! ground, one orange button, the quiet ones in dark red. Pure Rust: the page and its status in, a draw list and
//! the buttons (with their rectangles, for touch) out. The app drives it and does the work (picker, copy, install).
use crate::draw::{Align, Color, DrawList, Op, Rect};
use crate::language::Language;
use crate::layout::{ADVANCE, palette};
use crate::nav::Key;
use crate::screens::wrap;

/// The logical space of the screens: the proportions of a phone held sideways.
pub const WIDTH: f32 = 932.0;
pub const HEIGHT: f32 = 430.0;

/// The window, in the logical space.
const WINDOW: Rect = Rect::new(116.0, 24.0, 700.0, 382.0);
/// The size of the HD pack once installed, for the note.
pub const PACK_SIZE_NOTE: &str = "0.9 GB";

const GROUND_TOP: Color = Color::rgb(0x1A, 0x0F, 0x0C);
const GROUND_BOTTOM: Color = Color::rgb(0x0B, 0x07, 0x06);
const INK: Color = Color::rgb(0x3A, 0x00, 0x00);
const BUTTON_INK: Color = Color::rgb(0x1A, 0x00, 0x00);
const QUIET: Color = Color::rgb(0x5A, 0x2A, 0x2A);
const OFF_FILL: Color = Color::rgb(0xD8, 0xC9, 0xA6);
const OFF_LINE: Color = Color::rgb(0x7A, 0x6A, 0x55);
const OFF_INK: Color = Color::rgb(0x4A, 0x3A, 0x3A);

/// The pages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    /// Choose the ROM file.
    Rom,
    /// A Japanese Rev 1 ROM needs the patch file.
    Patch,
    /// Classic or Enhanced.
    Mode,
    /// Choose hd-pack.zip, or skip.
    Pack,
    /// The language of the game's text (Enhanced): English or Traditional Chinese (Hong Kong). Shown by the game's
    /// own window before the first frame, on every platform, while no language is saved.
    Language,
}

/// What the line under the buttons says.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// Nothing happened yet.
    Idle,
    /// Work in progress: the step, and the percent when it has one (a bar is drawn on the pack page).
    Busy { text: String, percent: Option<u8> },
    /// The file was accepted (or the pack installed): the next button is open.
    Done(String),
    /// The file was refused, or the work failed: the reason. The page is the choice again.
    Error(String),
    /// A message that leaves the page's choice open (a mod picked on the pack page went to the mods folder).
    Note(String),
}

/// What a button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// Open the system file picker for this page's file.
    Pick,
    /// Go on to the next page (or start the game from the pack page).
    Continue,
    /// Back to the ROM page from the patch page.
    Back,
    /// Skip the HD pack: Classic only.
    Skip,
    /// The mode chosen: `true` is Enhanced.
    Mode { enhanced: bool },
    /// The language chosen.
    Language(Language),
}

/// How a button looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Look {
    /// The orange main button, with the cursor.
    Primary,
    /// A dark-red quiet button.
    Quiet,
    /// The plain button of the foot (Continue, Start game).
    Plain,
}

/// A button: where it is (in the logical space), what it says and does, and whether it answers.
#[derive(Debug, Clone, PartialEq)]
pub struct Button {
    pub rect: Rect,
    pub label: String,
    pub command: Command,
    pub look: Look,
    pub enabled: bool,
}

/// One screen: the page, the status, and the button the keys have.
#[derive(Debug, Clone, PartialEq)]
pub struct Setup {
    page: Page,
    status: Status,
    /// The index of the button the keys have.
    focus: usize,
}

impl Setup {
    /// The screen of `page`, nothing said yet.
    #[must_use]
    pub fn new(page: Page) -> Self {
        let mut setup = Self { page, status: Status::Idle, focus: 0 };
        setup.focus_first();
        setup
    }

    #[must_use]
    pub fn page(&self) -> Page {
        self.page
    }

    #[must_use]
    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Goes to `page` with the status cleared.
    pub fn go(&mut self, page: Page) {
        self.page = page;
        self.status = Status::Idle;
        self.focus_first();
    }

    /// Sets the status; a finished one puts the keys on the button that goes on.
    pub fn set_status(&mut self, status: Status) {
        self.status = status;
        let done = matches!(self.status, Status::Done(_));
        let buttons = self.buttons();
        let target = if done {
            buttons.iter().position(|b| b.enabled && b.command == Command::Continue)
        } else {
            buttons.iter().position(|b| b.enabled)
        };
        self.focus = target.unwrap_or(0);
    }

    fn busy(&self) -> bool {
        matches!(self.status, Status::Busy { .. })
    }

    fn focus_first(&mut self) {
        self.focus = self.buttons().iter().position(|b| b.enabled).unwrap_or(0);
    }

    /// The buttons of the page, in the order the keys move through them.
    #[must_use]
    pub fn buttons(&self) -> Vec<Button> {
        let busy = self.busy();
        let done = matches!(self.status, Status::Done(_));
        let button = |x, y, w, h, label: &str, command, look, enabled| Button {
            rect: inner(x, y, w, h),
            label: label.to_owned(),
            command,
            look,
            enabled,
        };
        match self.page {
            Page::Rom => vec![
                button(
                    24.0,
                    112.0,
                    330.0,
                    56.0,
                    "Select ROM file...",
                    Command::Pick,
                    Look::Primary,
                    !busy,
                ),
                button(480.0, 304.0, 194.0, 48.0, "Continue", Command::Continue, Look::Plain, done),
            ],
            Page::Patch => vec![
                button(
                    24.0,
                    112.0,
                    330.0,
                    56.0,
                    "Select patch file...",
                    Command::Pick,
                    Look::Primary,
                    !busy,
                ),
                button(372.0, 116.0, 150.0, 48.0, "Back", Command::Back, Look::Quiet, !busy),
                button(480.0, 304.0, 194.0, 48.0, "Continue", Command::Continue, Look::Plain, done),
            ],
            Page::Mode => vec![
                button(
                    24.0,
                    112.0,
                    330.0,
                    56.0,
                    "Classic",
                    Command::Mode { enhanced: false },
                    Look::Primary,
                    true,
                ),
                button(
                    24.0,
                    188.0,
                    330.0,
                    56.0,
                    "Enhanced",
                    Command::Mode { enhanced: true },
                    Look::Primary,
                    true,
                ),
            ],
            Page::Pack => vec![
                button(
                    24.0,
                    116.0,
                    330.0,
                    56.0,
                    "Select hd-pack.zip...",
                    Command::Pick,
                    Look::Primary,
                    !busy && !done,
                ),
                button(
                    24.0,
                    316.0,
                    330.0,
                    48.0,
                    "Skip, play Classic only",
                    Command::Skip,
                    Look::Quiet,
                    !busy && !done,
                ),
                button(
                    480.0,
                    316.0,
                    194.0,
                    48.0,
                    "Start game",
                    Command::Continue,
                    Look::Plain,
                    done,
                ),
            ],
            Page::Language => Language::ALL
                .iter()
                .enumerate()
                .map(|(n, &language)| {
                    button(
                        24.0,
                        112.0 + 76.0 * n as f32,
                        330.0,
                        56.0,
                        language.native_name(),
                        Command::Language(language),
                        Look::Primary,
                        true,
                    )
                })
                .collect(),
        }
    }

    /// A key of the keyboard, the gamepad or the d-pad. What it chose, if it chose a button.
    pub fn key(&mut self, key: Key) -> Option<Command> {
        let buttons = self.buttons();
        let open: Vec<usize> =
            buttons.iter().enumerate().filter(|(_, b)| b.enabled).map(|(i, _)| i).collect();
        let count = open.len().max(1);
        let at = open.iter().position(|&i| i == self.focus);
        match key {
            Key::Down | Key::Right | Key::Tab => {
                let next = at.map_or(0, |at| (at + 1) % count);
                self.focus = open.get(next).copied().unwrap_or(self.focus);
            }
            Key::Up | Key::Left | Key::TabBack => {
                let previous = at.map_or(0, |at| (at + count - 1) % count);
                self.focus = open.get(previous).copied().unwrap_or(self.focus);
            }
            Key::Confirm => {
                return buttons.get(self.focus).filter(|b| b.enabled).map(|b| b.command);
            }
            Key::Cancel => {
                return buttons
                    .iter()
                    .find(|b| b.enabled && b.command == Command::Back)
                    .map(|b| b.command);
            }
            Key::ResetPage | Key::ResetAll | Key::Apply => {}
        }
        None
    }

    /// The command of the enabled button at the logical point, if any.
    #[must_use]
    pub fn hit(&self, x: f32, y: f32) -> Option<Command> {
        self.buttons()
            .iter()
            .find(|b| {
                b.enabled
                    && x >= b.rect.x
                    && x <= b.rect.right()
                    && y >= b.rect.y
                    && y <= b.rect.bottom()
            })
            .map(|b| b.command)
    }

    /// The draw list of the screen, in [`WIDTH`] x [`HEIGHT`].
    #[must_use]
    pub fn scene(&self) -> DrawList {
        let mut list = DrawList::new();
        ground(&mut list);
        window(&mut list);
        let (title, step) = match self.page {
            Page::Rom => ("CHOOSE YOUR ROM", Some("STEP 1 OF 2")),
            Page::Patch => ("CHOOSE THE PATCH", Some("STEP 1 OF 2")),
            Page::Mode => ("CHOOSE A MODE", None),
            Page::Pack => ("INSTALL THE HD PACK", Some("STEP 2 OF 2")),
            Page::Language => ("CHOOSE A LANGUAGE", Some("語言")),
        };
        text(&mut list, 24.0, 18.0, 24.0, title, INK, Align::Left);
        if let Some(step) = step {
            text(&mut list, 676.0, 24.0, 13.0, step, palette::DARK_RED, Align::Right);
        }
        match self.page {
            Page::Rom => self.rom_page(&mut list),
            Page::Patch => self.patch_page(&mut list),
            Page::Mode => mode_page(&mut list),
            Page::Pack => self.pack_page(&mut list),
            Page::Language => language_page(&mut list),
        }
        for (index, button) in self.buttons().iter().enumerate() {
            draw_button(&mut list, button, index == self.focus);
        }
        list
    }

    fn rom_page(&self, list: &mut DrawList) {
        paragraph(
            list,
            24.0,
            62.0,
            640.0,
            15.0,
            "This game needs your own copy of Super Saiya Densetsu. The file stays on this device and is never \
             uploaded.",
        );
        text(list, 372.0, 128.0, 13.0, "Opens the system file picker.", QUIET, Align::Left);
        let rect = inner(24.0, 186.0, 650.0, 104.0);
        list.push(Op::Outline { rect, width: 2.0, color: palette::DARK_RED });
        text(list, 38.0, 196.0, 15.0, "Accepted ROMs", INK, Align::Left);
        text(list, 38.0, 221.5, 15.0, "English v1.02 - CRC-32 A8CF4F2A", INK, Align::Left);
        text(list, 38.0, 247.0, 15.0, "Japanese Rev 1 + patch", INK, Align::Left);
        self.foot_status(list, "No file selected yet.");
    }

    fn patch_page(&self, list: &mut DrawList) {
        paragraph(
            list,
            24.0,
            62.0,
            640.0,
            15.0,
            "This is the Japanese Rev 1 ROM. It needs the English v1.02 patch, the file dbz102.ips. Pick it to \
             go on.",
        );
        let rect = inner(24.0, 186.0, 650.0, 104.0);
        list.push(Op::Outline { rect, width: 2.0, color: palette::DARK_RED });
        text(list, 38.0, 196.0, 15.0, "Needed", INK, Align::Left);
        text(list, 38.0, 221.5, 15.0, "dbz102.ips - the v1.02 patch", INK, Align::Left);
        text(
            list,
            38.0,
            247.0,
            15.0,
            "It is applied once; the patched ROM is kept.",
            INK,
            Align::Left,
        );
        self.foot_status(list, "No patch selected yet.");
    }

    /// The status line at the foot of the ROM and patch pages, left of Continue.
    fn foot_status(&self, list: &mut DrawList, idle: &str) {
        let (message, color) = match &self.status {
            Status::Idle => (idle.to_owned(), INK),
            Status::Busy { text, .. } | Status::Done(text) | Status::Note(text) => {
                (text.clone(), INK)
            }
            Status::Error(text) => (text.clone(), palette::DARK_RED),
        };
        let chars = (440.0 / (ADVANCE * 14.0)) as usize;
        for (line, part) in wrap(&message, chars).iter().take(3).enumerate() {
            text(list, 24.0, 304.0 + 20.0 * line as f32, 14.0, part, color, Align::Left);
        }
    }

    fn pack_page(&self, list: &mut DrawList) {
        paragraph(
            list,
            24.0,
            62.0,
            640.0,
            15.0,
            &format!(
                "Enhanced mode uses the HD art pack. Pick the file hd-pack.zip. It takes about \
                 {PACK_SIZE_NOTE} of free space once installed. A mod (a .ssdpack file) picked here goes \
                 to the mods."
            ),
        );
        match &self.status {
            Status::Idle => {
                text(
                    list,
                    24.0,
                    192.0,
                    15.0,
                    "Pick the file to install the pack.",
                    INK,
                    Align::Left,
                );
            }
            Status::Busy { text: step, percent } => {
                text(list, 24.0, 192.0, 15.0, step, INK, Align::Left);
                bar(list, *percent);
            }
            Status::Done(message) => {
                text(list, 24.0, 192.0, 15.0, message, INK, Align::Left);
                bar(list, Some(100));
            }
            Status::Error(message) | Status::Note(message) => {
                let colour =
                    if matches!(self.status, Status::Error(_)) { palette::DARK_RED } else { INK };
                let chars = (650.0 / (ADVANCE * 14.0)) as usize;
                for (line, part) in wrap(message, chars).iter().take(4).enumerate() {
                    let top = 190.0 + 20.0 * line as f32;
                    text(list, 24.0, top, 14.0, part, colour, Align::Left);
                }
            }
        }
        text(
            list,
            24.0,
            282.0,
            13.0,
            "Keep the app open while it installs. You can install the pack later from Options.",
            QUIET,
            Align::Left,
        );
    }
}

fn mode_page(list: &mut DrawList) {
    paragraph(
        list,
        24.0,
        62.0,
        640.0,
        15.0,
        "How do you want to play? Your saves stay either way.",
    );
    for (top, lines) in [
        (112.0, ["The original game and its menus,", "as it was on the console."]),
        (188.0, ["HD menus, art and extra options.", "Needs the HD pack (about 0.9 GB)."]),
    ] {
        for (n, line) in lines.iter().enumerate() {
            text(list, 372.0, top + 14.0 + 20.0 * n as f32, 14.0, line, QUIET, Align::Left);
        }
    }
    let note = "Change it later: Options > Game in Enhanced, or hold Select and Start for 2 seconds in Classic.";
    let chars = (650.0 / (ADVANCE * 13.0)) as usize;
    for (n, line) in wrap(note, chars).iter().enumerate() {
        text(list, 24.0, 268.0 + 19.0 * n as f32, 13.0, line, QUIET, Align::Left);
    }
}

fn language_page(list: &mut DrawList) {
    paragraph(list, 24.0, 62.0, 640.0, 15.0, "The language of the story's text. 故事文字的語言。");
    for (top, lines) in [
        (112.0, ["The game's own English text.", ""]),
        (188.0, ["Traditional Chinese (Hong Kong),", "in the HD windows of Enhanced."]),
    ] {
        for (n, line) in lines.iter().enumerate() {
            text(list, 372.0, top + 14.0 + 20.0 * n as f32, 14.0, line, QUIET, Align::Left);
        }
    }
    let note = "Change it later: Options > Accessibility > Language. 之後可在「選項」中更改。";
    let chars = (650.0 / (ADVANCE * 13.0)) as usize;
    for (n, line) in wrap(note, chars).iter().enumerate() {
        text(list, 24.0, 280.0 + 19.0 * n as f32, 13.0, line, QUIET, Align::Left);
    }
}

/// A rectangle given in window coordinates, as a rectangle of the space.
fn inner(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::new(WINDOW.x + 4.0 + x, WINDOW.y + 4.0 + y, w, h)
}

/// A text run whose line box starts at `top` (the web boards' line height is 1.45), at window coordinates.
fn text(list: &mut DrawList, x: f32, top: f32, size: f32, text: &str, color: Color, align: Align) {
    let at = inner(x, top, 0.0, 0.0);
    list.text(at.x, at.y + size * 0.725, text, size, color, align);
}

/// Wrapped text of `width` pixels from `top`.
fn paragraph(list: &mut DrawList, x: f32, top: f32, width: f32, size: f32, words: &str) {
    let chars = (width / (ADVANCE * size)) as usize;
    for (line, part) in wrap(words, chars).iter().enumerate() {
        text(list, x, top + size * 1.45 * line as f32, size, part, INK, Align::Left);
    }
}

fn ground(list: &mut DrawList) {
    let steps = 8;
    for step in 0..steps {
        let t = step as f32 / (steps - 1) as f32;
        let mix = |a: u8, b: u8| (f32::from(a) + (f32::from(b) - f32::from(a)) * t) as u8;
        let color = Color::rgb(
            mix(GROUND_TOP.0[0], GROUND_BOTTOM.0[0]),
            mix(GROUND_TOP.0[1], GROUND_BOTTOM.0[1]),
            mix(GROUND_TOP.0[2], GROUND_BOTTOM.0[2]),
        );
        let h = HEIGHT / steps as f32;
        list.fill(Rect::new(0.0, h * step as f32, WIDTH, h + 1.0), color);
    }
}

/// The window of the boards: a 4 px orange frame, a 2 px dark-red line inside it, cream paper.
fn window(list: &mut DrawList) {
    list.fill(WINDOW, palette::ORANGE);
    list.fill(WINDOW.grow(-4.0), palette::DARK_RED);
    list.fill(WINDOW.grow(-6.0), palette::CREAM);
}

fn bar(list: &mut DrawList, percent: Option<u8>) {
    let rect = inner(24.0, 218.0, 650.0, 28.0);
    list.fill(rect, palette::DARK_RED);
    let well = rect.grow(-2.0);
    list.fill(well, OFF_FILL);
    if let Some(percent) = percent {
        let filled = well.w * f32::from(percent.min(100)) / 100.0;
        list.fill(Rect::new(well.x, well.y, filled, well.h), palette::ORANGE);
        text(list, 24.0, 254.0, 14.0, &format!("{percent}%"), INK, Align::Left);
    }
}

fn draw_button(list: &mut DrawList, button: &Button, focused: bool) {
    let (fill, line, ink) = if button.enabled {
        match button.look {
            Look::Primary | Look::Plain => (palette::ORANGE, palette::DARK_RED, BUTTON_INK),
            Look::Quiet => (palette::DARK_RED, palette::ORANGE, palette::CREAM),
        }
    } else {
        (OFF_FILL, OFF_LINE, OFF_INK)
    };
    let rect = button.rect;
    if focused && button.enabled {
        list.fill(rect.grow(4.0), INK);
    }
    list.fill(rect, line);
    list.fill(rect.grow(-2.0), fill);
    let size = if button.look == Look::Primary { 19.0 } else { 17.0 };
    match button.look {
        Look::Primary => {
            if button.enabled {
                list.push(Op::Cursor { x: rect.x + 12.0, y: rect.middle_y() - 9.0 });
            }
            list.text(rect.x + 40.0, rect.middle_y(), &button.label, size, ink, Align::Left);
        }
        Look::Quiet => {
            list.text(rect.x + 12.0, rect.middle_y(), &button.label, size, ink, Align::Left);
        }
        Look::Plain => {
            let x = rect.x + rect.w / 2.0;
            list.text(x, rect.middle_y(), &button.label, size, ink, Align::Centre);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(list: &DrawList) -> Vec<String> {
        list.texts().map(|(_, _, t)| t.to_owned()).collect()
    }

    #[test]
    fn every_button_is_a_touch_target_inside_the_window() {
        for page in [Page::Rom, Page::Patch, Page::Mode, Page::Pack, Page::Language] {
            for button in Setup::new(page).buttons() {
                assert!(button.rect.h >= 48.0, "{} is {} high", button.label, button.rect.h);
                assert!(button.rect.w >= 48.0);
                assert!(button.rect.x >= WINDOW.x && button.rect.right() <= WINDOW.right());
                assert!(button.rect.y >= WINDOW.y && button.rect.bottom() <= WINDOW.bottom());
            }
        }
    }

    #[test]
    fn the_rom_page_matches_the_board_and_continue_waits_for_an_accepted_file() {
        let mut setup = Setup::new(Page::Rom);
        let shown = texts(&setup.scene());
        for line in
            ["CHOOSE YOUR ROM", "STEP 1 OF 2", "Select ROM file...", "Accepted ROMs", "Continue"]
        {
            assert!(shown.iter().any(|t| t == line), "{line} in {shown:?}");
        }
        assert!(shown.iter().any(|t| t == "No file selected yet."));
        assert_eq!(setup.key(Key::Confirm), Some(Command::Pick));
        assert_eq!(setup.key(Key::Down), None);
        assert_eq!(setup.key(Key::Confirm), Some(Command::Pick), "Continue is closed: focus stays");
        setup.set_status(Status::Done("ROM accepted.".to_owned()));
        assert_eq!(setup.key(Key::Confirm), Some(Command::Continue), "the focus moves to Continue");
    }

    #[test]
    fn a_refused_file_says_why_and_the_choice_is_open_again() {
        let mut setup = Setup::new(Page::Rom);
        setup.set_status(Status::Busy { text: "Checking the file...".to_owned(), percent: None });
        assert_eq!(setup.key(Key::Confirm), None, "nothing is chosen while it works");
        setup.set_status(Status::Error("This file is not one of the accepted ROMs.".to_owned()));
        assert!(texts(&setup.scene()).iter().any(|t| t.contains("not one of the accepted")));
        assert_eq!(setup.key(Key::Confirm), Some(Command::Pick));
    }

    #[test]
    fn the_pack_page_shows_the_steps_and_a_bar_and_start_waits_for_the_install() {
        let mut setup = Setup::new(Page::Pack);
        setup.set_status(Status::Busy {
            text: "Unpacking the art...".to_owned(),
            percent: Some(62),
        });
        let shown = texts(&setup.scene());
        assert!(shown.contains(&"62%".to_owned()));
        assert!(shown.contains(&"Unpacking the art...".to_owned()));
        assert!(setup.buttons().iter().all(|b| !b.enabled), "nothing answers while it installs");
        setup.set_status(Status::Done("Installed.".to_owned()));
        let start = inner(500.0, 330.0, 0.0, 0.0);
        assert_eq!(setup.hit(start.x, start.y), Some(Command::Continue));
    }

    #[test]
    fn a_mod_installed_from_the_pack_page_leaves_the_pick_open() {
        let mut setup = Setup::new(Page::Pack);
        setup.set_status(Status::Note("The mod Free progression is installed.".to_owned()));
        assert!(texts(&setup.scene()).iter().any(|t| t.contains("Free progression")));
        let pick = setup.buttons().into_iter().find(|b| b.command == Command::Pick).unwrap();
        assert!(pick.enabled, "the HD pack can still be picked");
    }

    #[test]
    fn the_language_page_offers_each_language_by_keys_and_by_touch() {
        let mut setup = Setup::new(Page::Language);
        let shown = texts(&setup.scene());
        assert!(shown.iter().any(|t| t == "English"));
        assert!(shown.iter().any(|t| t == "繁體中文（香港）"));
        assert_eq!(setup.key(Key::Confirm), Some(Command::Language(Language::English)));
        assert_eq!(setup.key(Key::Down), None);
        assert_eq!(setup.key(Key::Confirm), Some(Command::Language(Language::ZhHk)));
        assert_eq!(setup.key(Key::Down), None, "wraps to the first");
        assert_eq!(setup.key(Key::Confirm), Some(Command::Language(Language::English)));
        assert_eq!(setup.key(Key::Cancel), None, "there is no going back: a language is needed");
        let second = setup.buttons()[1].rect;
        assert_eq!(
            setup.hit(second.x + 10.0, second.y + 10.0),
            Some(Command::Language(Language::ZhHk))
        );
    }

    #[test]
    fn the_patch_page_goes_back_with_cancel() {
        let mut setup = Setup::new(Page::Patch);
        assert_eq!(setup.key(Key::Cancel), Some(Command::Back));
        let back = inner(400.0, 130.0, 0.0, 0.0);
        assert_eq!(setup.hit(back.x, back.y), Some(Command::Back));
    }

    #[test]
    fn every_text_lies_inside_the_window() {
        let mut setups = Vec::new();
        for page in [Page::Rom, Page::Patch, Page::Mode, Page::Pack, Page::Language] {
            let mut setup = Setup::new(page);
            setups.push(setup.clone());
            setup.set_status(Status::Error(
                "This file was not accepted: the cached ROM is damaged and the reason is long enough to wrap \
                 onto several lines of the window."
                    .to_owned(),
            ));
            setups.push(setup);
        }
        for setup in setups {
            for op in setup.scene().ops() {
                if let Op::Text { x, y, text, size, align, .. } = op {
                    let width = text.chars().count() as f32 * ADVANCE * size;
                    let (left, right) = match align {
                        Align::Left => (*x, x + width),
                        Align::Centre => (x - width / 2.0, x + width / 2.0),
                        Align::Right => (x - width, *x),
                    };
                    let inside = left >= WINDOW.x
                        && right <= WINDOW.right() - 6.0
                        && *y < WINDOW.bottom() - 6.0;
                    assert!(inside, "{text:?} at {x},{y} on {:?}", setup.page());
                }
            }
        }
    }
}
