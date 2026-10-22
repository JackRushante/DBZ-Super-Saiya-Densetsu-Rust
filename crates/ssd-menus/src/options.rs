//! The options (M7): the table of rows of each category, the values they are bound to, and the two text
//! files that keep them. Pure Rust: no emulator, no ROM.
//!
//! A row is a [`RowDef`]: a label, a help text, a [`Kind`] (how it looks and moves) and a [`Source`] (where its
//! value lives). Each category has a file of its own with its rows, so a later task adds rows without touching
//! the others. A row whose value is a small stored number lives in a [`Values`] map under the row's key; the
//! category says which file keeps the map ([`Category::per_game`]): the slot file (`ssd.options`) for the
//! categories saved with the game, the PC file (`ssd.settings`) for the rest. A row can also be a view of a
//! value that lives elsewhere (the cheat block, the original's text speed), so there is one truth.

use crate::cheats::{Cheats, Setting};
use std::collections::BTreeMap;
use std::fmt::Write as _;

pub mod accessibility;
pub mod audio;
pub mod controls;
pub mod difficulty;
pub mod game;
pub mod gameplay;
pub mod video;

/// The categories of the Options screen, in the order of its list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Difficulty,
    Gameplay,
    Video,
    Audio,
    Controls,
    Accessibility,
    /// The game's own setup (Android only): the mode and the HD pack. Hidden unless [`show_game_page`] turned it on.
    Game,
}

#[cfg(test)]
pub(crate) static GAME_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

static GAME_PAGE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LANGUAGE_ROW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static GAME_INFO: std::sync::Mutex<(String, String)> =
    std::sync::Mutex::new((String::new(), String::new()));

/// Shows (or hides) the Game page of the Options: the app turns it on where the platform has the setup to open
/// (Android). Off by default, and the desktop never turns it on.
pub fn show_game_page(on: bool) {
    GAME_PAGE.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Shows (or hides) the Accessibility page's Language row: the app turns it on where the player has a translation
/// (its files in the data folder). Off by default, so without one the page is as it was.
pub fn show_language_row(on: bool) {
    LANGUAGE_ROW.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether the player has a translation: the Language row shows, and the language can be chosen.
#[must_use]
pub fn language_row_shown() -> bool {
    LANGUAGE_ROW.load(std::sync::atomic::Ordering::Relaxed)
}

/// The texts of the Game page's two read-only rows: the current mode, and the HD pack's state.
pub fn set_game_info(mode: &str, pack: &str) {
    *GAME_INFO.lock().unwrap_or_else(std::sync::PoisonError::into_inner) =
        (mode.to_owned(), pack.to_owned());
}

impl Category {
    /// The categories the Options shows: all of them where the Game page is on, else without it.
    #[must_use]
    pub fn shown() -> &'static [Category] {
        if GAME_PAGE.load(std::sync::atomic::Ordering::Relaxed) {
            &Self::ALL
        } else {
            &Self::ALL[..Self::ALL.len() - 1]
        }
    }

    /// Every category, the Game page last.
    pub const ALL: [Category; 7] = [
        Category::Difficulty,
        Category::Gameplay,
        Category::Video,
        Category::Audio,
        Category::Controls,
        Category::Accessibility,
        Category::Game,
    ];

    /// The name in the list.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Difficulty => "Difficulty",
            Self::Gameplay => "Gameplay",
            Self::Video => "Video",
            Self::Audio => "Audio",
            Self::Controls => "Controls",
            Self::Accessibility => "Accessibility",
            Self::Game => "Game",
        }
    }

    /// Whether the category is saved with the game (per save slot) rather than kept for this PC.
    #[must_use]
    pub const fn per_game(self) -> bool {
        matches!(self, Self::Difficulty | Self::Gameplay)
    }

    /// The small label under the name: where the category is kept.
    #[must_use]
    pub const fn scope_label(self) -> &'static str {
        if self.per_game() {
            "Saved with the game"
        } else if matches!(self, Self::Game) {
            "This device"
        } else {
            "This PC"
        }
    }

    /// The rows of the category, in the order they show.
    #[must_use]
    pub fn rows(self) -> &'static [RowDef] {
        match self {
            Self::Difficulty => difficulty::ROWS,
            Self::Gameplay => gameplay::ROWS,
            Self::Video => video::ROWS,
            Self::Audio => audio::ROWS,
            Self::Controls => controls::ROWS,
            Self::Accessibility if language_row_shown() => accessibility::ROWS_WITH_LANGUAGE,
            Self::Accessibility => accessibility::ROWS,
            Self::Game => game::ROWS,
        }
    }

    /// The line under the rows, if the category has one.
    #[must_use]
    pub const fn note(self) -> Option<&'static str> {
        match self {
            Self::Difficulty => difficulty::NOTE,
            Self::Gameplay => gameplay::NOTE,
            Self::Video => video::NOTE,
            Self::Audio => audio::NOTE,
            Self::Controls => controls::NOTE,
            Self::Accessibility => accessibility::NOTE,
            Self::Game => game::NOTE,
        }
    }

    /// The key hints of the help strip for the row under the cursor (`None` when the page has no rows).
    #[must_use]
    pub fn hints(self, row: Option<&RowDef>) -> Vec<&'static str> {
        let first = match row.map(|row| row.kind) {
            Some(Kind::Button(_)) => "A  Open",
            Some(Kind::Key) => "A  Remap",
            _ => "Left/Right  Change",
        };
        vec![first, "B  Back", "Y  Reset page", "Select  Reset all", "L/R  Page"]
    }

    /// The category after this one (wrapping).
    #[must_use]
    pub fn next(self) -> Self {
        {
            let shown = Self::shown();
            shown[(self.index() + 1) % shown.len()]
        }
    }

    /// The category before this one (wrapping).
    #[must_use]
    pub fn previous(self) -> Self {
        {
            let shown = Self::shown();
            shown[(self.index() + shown.len() - 1) % shown.len()]
        }
    }

    /// The place in [`Category::ALL`].
    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&category| category == self).unwrap_or(0)
    }
}

/// How a row looks and moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The options side by side, the current one highlighted; Left and Right move it, wrapping.
    Choice(&'static [&'static str]),
    /// An arrow, the value and an arrow; Left and Right move it, wrapping.
    Cycle(&'static [&'static str]),
    /// A button that opens something (`A`); the text is on the button.
    Button(&'static str),
    /// A bar of `steps` values from empty to full (a volume): Left and Right move one step and stop at the ends.
    Slider { steps: u8 },
    /// A read-only value (no cursor, no change): its text comes from [`info_text`].
    Info,
    /// A key box (the Controls page): the stored number is the code of a key or button; A waits for the next one.
    Key,
    /// A line of small text under a group (no cursor, no value).
    Note(&'static str),
}

/// What a button opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opens {
    /// The Cheats screen.
    Cheats,
    /// The mode choice of the setup (Android): the game restarts into it.
    Mode,
    /// The HD pack install of the setup (Android): the game restarts into it.
    Pack,
    /// The Data packs screen (M10): the installed packs, the set the game runs with, and the chapters.
    Packs,
    /// The touch controls' places and sizes go back to the design (M9).
    ResetTouchLayout,
}

/// Where a row's value lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// A small stored number (the index of the choice), under the row's key, in the category's file.
    Stored { default: u8 },
    /// The original's text speed (`$01D0`, five speeds): the same setting as the Menu's Text entry.
    TextSpeed,
    /// The encounter rate of the Cheats: Off is "random battles off", the rest the rate x1/4 to x4.
    Encounters,
    /// The Difficulty preset: Easy, Normal or Hard, read off the cheats it sets (enemy power, EXP and money). Custom (none
    /// of them) is the index past the last choice. A step writes the three.
    Preset,
    /// A value of the cheats that the page only shows.
    View(View),
    /// No value: a note.
    None,
    /// Not a value: the row opens a screen.
    Opens(Opens),
    /// A key or button code (0 is none), under the row's key, in the PC file; `default` is today's key.
    Key { default: u8 },
    /// Not a value: a read-only line worked out from the other rows of the page.
    Info,
}

/// A cheat value an info row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Power,
    Exp,
    Money,
}

/// One row of a category.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowDef {
    /// The key of the stored value (`category.name`); also the identity of the row.
    pub key: &'static str,
    /// The label at the left of the row.
    pub label: &'static str,
    /// The title of the group the row is in (a new title is drawn where it changes).
    pub group: &'static str,
    /// The help in the strip while the row is under the cursor.
    pub help: &'static str,
    pub kind: Kind,
    pub source: Source,
}

/// The text of the read-only row `key` of `category`, given the values of the category's rows (as the screen shows them).
#[must_use]
pub fn info_text(category: Category, key: &str, shown: &[usize]) -> String {
    match (category, key) {
        (Category::Video, "video.size") => video::size_text(shown),
        (Category::Game, key) => game::info_text(key),
        (Category::Difficulty, key) => difficulty::info_text(key, shown),
        _ => String::new(),
    }
}

/// The small caption left of a read-only row's value.
#[must_use]
pub fn info_caption(category: Category, key: &str) -> &'static str {
    match (category, key) {
        (Category::Video, "video.size") => "pixels at this scale",
        (Category::Difficulty, key) => difficulty::info_caption(key),
        _ => "",
    }
}

/// The last text speed of the game's byte (`$01D0` counts 0, the fastest, to 4, the slowest); the row shows `4 - byte`, slowest first.
pub const TEXT_SPEED_LAST: u8 = 4;
/// The default of the text speed (the middle of the five).
pub const TEXT_SPEED_DEFAULT: u8 = 2;
/// The place of "x1" in the encounter rate's choices.
const ENCOUNTERS_NEUTRAL: usize = 3;

impl RowDef {
    /// Whether the row is a read-only line: it has no cursor.
    #[must_use]
    pub const fn is_info(&self) -> bool {
        matches!(self.kind, Kind::Info | Kind::Note(_))
    }

    /// The number of values the row has.
    #[must_use]
    pub const fn count(&self) -> usize {
        match self.kind {
            Kind::Choice(labels) | Kind::Cycle(labels) => labels.len(),
            Kind::Slider { steps } => steps as usize,
            Kind::Button(_) | Kind::Info | Kind::Note(_) => 1,
            Kind::Key => 256,
        }
    }

    /// The text of value `index`.
    #[must_use]
    pub fn text(&self, index: usize) -> &'static str {
        match self.kind {
            Kind::Choice(labels) | Kind::Cycle(labels) => labels.get(index).copied().unwrap_or("?"),
            Kind::Button(text) | Kind::Note(text) => text,
            Kind::Slider { .. } | Kind::Info | Kind::Key => "",
        }
    }

    /// The index of the row's value, clamped to its choices: the stored number (or the default), or the
    /// view of the cheats or of the text speed.
    #[must_use]
    pub fn index(&self, values: &Values, cheats: &Cheats, text_speed: u8) -> usize {
        let last = self.count().saturating_sub(1);
        let index = match self.source {
            Source::Stored { default } | Source::Key { default } => values
                .get(self.key)
                .and_then(|value| usize::try_from(value).ok())
                .unwrap_or(usize::from(default)),
            Source::TextSpeed => usize::from(TEXT_SPEED_LAST - text_speed.min(TEXT_SPEED_LAST)),
            Source::Encounters => {
                if cheats.random_battles_off() {
                    0
                } else {
                    1 + cheats.index(Setting::Encounters)
                }
            }
            Source::Preset => {
                // Custom is the index past the last choice: no choice shows as chosen.
                return difficulty::preset_of(cheats).unwrap_or(self.count());
            }
            // An info row has one value of its own; the cheat's step it shows is not clamped to it.
            Source::View(View::Power) => return cheats.index(Setting::Power),
            Source::View(View::Exp) => return cheats.index(Setting::Exp),
            Source::View(View::Money) => return cheats.money_index(),
            Source::Opens(_) | Source::Info | Source::None => 0,
        };
        index.min(last)
    }

    /// The default index of the row.
    #[must_use]
    pub fn default_index(&self) -> usize {
        match self.source {
            Source::Stored { default } | Source::Key { default } => {
                usize::from(default).min(self.count().saturating_sub(1))
            }
            Source::TextSpeed => usize::from(TEXT_SPEED_LAST - TEXT_SPEED_DEFAULT),
            Source::Encounters => ENCOUNTERS_NEUTRAL,
            Source::Preset => difficulty::NORMAL,
            Source::View(_) | Source::Opens(_) | Source::Info | Source::None => 0,
        }
    }

    /// Puts the row on value `index` (clamped) and says what changed.
    pub fn set(&self, values: &mut Values, cheats: &mut Cheats, index: usize) -> Effect {
        let index = index.min(self.count().saturating_sub(1));
        match self.source {
            Source::Stored { .. } | Source::Key { .. } => {
                values.set(self.key, index as i64);
                Effect::Stored
            }
            Source::TextSpeed => Effect::TextSpeed(
                TEXT_SPEED_LAST - u8::try_from(index).unwrap_or(0).min(TEXT_SPEED_LAST),
            ),
            Source::Encounters => {
                cheats.set_random_battles_off(index == 0);
                if index > 0 {
                    cheats.set_index(Setting::Encounters, index - 1);
                }
                Effect::Cheats
            }
            Source::Preset => {
                difficulty::apply(cheats, index);
                Effect::Cheats
            }
            Source::View(_) | Source::None | Source::Info => Effect::None,
            Source::Opens(opens) => Effect::Open(opens),
        }
    }

    /// Moves the row by `by` values (-1 Left, 1 Right), wrapping at both ends. A button opens its screen.
    pub fn step(&self, values: &mut Values, cheats: &mut Cheats, text_speed: u8, by: i8) -> Effect {
        match self.kind {
            Kind::Button(_) => return self.set(values, cheats, 0),
            Kind::Info | Kind::Note(_) => return Effect::None,
            _ => {}
        }
        let count = self.count() as i64;
        let from = self.index(values, cheats, text_speed) as i64;
        let next = if matches!(self.kind, Kind::Slider { .. }) {
            // A bar stops at its ends (a volume does not jump from full to silence).
            (from + i64::from(by)).clamp(0, count.max(1) - 1) as usize
        } else {
            // Custom (a preset row past its last choice) steps to the first choice on Right and to the last on Left.
            let from = if from >= count { if by > 0 { -1 } else { count } } else { from };
            (from + i64::from(by)).rem_euclid(count.max(1)) as usize
        };
        self.set(values, cheats, next)
    }

    /// Puts the row back to its default.
    pub fn reset(&self, values: &mut Values, cheats: &mut Cheats) -> Effect {
        match self.source {
            Source::Opens(_) | Source::Info | Source::View(_) | Source::None => Effect::None,
            // A stored row forgets its number: the file only holds what differs from the defaults.
            Source::Stored { .. } | Source::Key { .. } => {
                values.remove(self.key);
                Effect::Stored
            }
            _ => self.set(values, cheats, self.default_index()),
        }
    }
}

/// What a change of a row asks the host to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Effect {
    /// Nothing.
    None,
    /// A stored number changed: its file is to be written.
    Stored,
    /// The cheats changed: the cheat block is to be written.
    Cheats,
    /// The text speed is to be written to the game.
    TextSpeed(u8),
    /// The row opens a screen.
    Open(Opens),
}

/// The integers the options keep, by key. Keys the game does not know are kept and written back.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Values(BTreeMap<String, i64>);

impl Values {
    /// An empty map: every row is at its default.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The value under `key`.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<i64> {
        self.0.get(key).copied()
    }

    /// Stores `value` under `key`.
    pub fn set(&mut self, key: &str, value: i64) {
        self.0.insert(key.to_owned(), value);
    }

    /// Forgets the value of `key` (the row is at its default again).
    pub fn remove(&mut self, key: &str) {
        self.0.remove(key);
    }

    /// Forgets every key that starts with `prefix`.
    pub fn remove_prefix(&mut self, prefix: &str) {
        self.retain(|key| !key.starts_with(prefix));
    }

    /// Keeps the keys `keep` says yes to.
    pub fn retain(&mut self, mut keep: impl FnMut(&str) -> bool) {
        self.0.retain(|key, _| keep(key));
    }

    /// Whether nothing is stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// The keys that belong to `category`'s rows are forgotten.
    pub fn reset_category(&mut self, category: Category) {
        for row in category.rows() {
            self.remove(row.key);
        }
    }
}

/// The first word of the PC file.
const PC_MAGIC: &str = "ssd-settings";
/// The first word of the slot file.
const SLOTS_MAGIC: &str = "ssd-options";
/// The version of both files. A file of a newer version is read as far as it is understood.
pub const VERSION: u32 = 1;

/// The `key=value` lines of `text` after its header line, the header being `magic` and a version number.
fn lines<'a>(text: &'a str, magic: &str) -> Result<impl Iterator<Item = (&'a str, i64)>, String> {
    let mut all = text.lines();
    let header = all.next().unwrap_or("").trim();
    let version = header
        .strip_prefix(magic)
        .map(str::trim)
        .and_then(|version| version.parse::<u32>().ok())
        .filter(|&version| version > 0)
        .ok_or_else(|| format!("not a {magic} file"))?;
    let _ = version;
    Ok(all.filter_map(|line| {
        let (key, value) = line.split_once('=')?;
        Some((key.trim(), value.trim().parse::<i64>().ok()?))
    }))
}

impl Values {
    /// The PC file: a header line, then `key=value` lines.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = format!("{PC_MAGIC} {VERSION}\n");
        for (key, value) in &self.0 {
            let _ = writeln!(text, "{key}={value}");
        }
        text
    }

    /// Reads [`Values::to_text`]. A line that is not `key=number` is skipped; a key nobody knows is kept.
    ///
    /// # Errors
    ///
    /// The text is not a settings file.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut values = Self::new();
        for (key, value) in lines(text, PC_MAGIC)? {
            if !key.is_empty() {
                values.set(key, value);
            }
        }
        Ok(values)
    }
}

/// The options of one save slot, as the slot file keeps them: the slot's checksum word says which save they
/// belong to (as the cheats' sidecar does).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotEntry {
    pub checksum: u16,
    pub values: Values,
}

/// The slot file (`ssd.options`): one entry per save slot.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SlotOptions {
    entries: BTreeMap<u8, SlotEntry>,
}

impl SlotOptions {
    /// A file with no entries.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// The values saved for `slot` when its checksum word was `checksum`; none for a slot changed since.
    #[must_use]
    pub fn entry_for(&self, slot: u8, checksum: u16) -> Option<&Values> {
        self.entries
            .get(&slot)
            .filter(|entry| entry.checksum == checksum)
            .map(|entry| &entry.values)
    }

    /// Keeps `values` as the options of `slot`, saved when its checksum word was `checksum`.
    pub fn store(&mut self, slot: u8, checksum: u16, values: Values) {
        self.entries.insert(slot, SlotEntry { checksum, values });
    }

    /// The file: a header line, then for each slot `s<slot>.sum=<checksum>` and `s<slot>.<key>=<value>` lines.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = format!("{SLOTS_MAGIC} {VERSION}\n");
        for (slot, entry) in &self.entries {
            let _ = writeln!(text, "s{slot}.sum={}", entry.checksum);
            for (key, value) in &entry.values.0 {
                let _ = writeln!(text, "s{slot}.{key}={value}");
            }
        }
        text
    }

    /// Reads [`SlotOptions::to_text`]. A line that is not understood is skipped, and a slot with no checksum
    /// word is left out.
    ///
    /// # Errors
    ///
    /// The text is not an options file.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut sums: BTreeMap<u8, u16> = BTreeMap::new();
        let mut values: BTreeMap<u8, Values> = BTreeMap::new();
        for (key, value) in lines(text, SLOTS_MAGIC)? {
            let Some((slot, name)) = key.strip_prefix('s').and_then(|rest| rest.split_once('.'))
            else {
                continue;
            };
            let Ok(slot) = slot.parse::<u8>() else { continue };
            if name == "sum" {
                if let Ok(sum) = u16::try_from(value) {
                    sums.insert(slot, sum);
                }
            } else if !name.is_empty() {
                values.entry(slot).or_default().set(name, value);
            }
        }
        let entries = sums
            .into_iter()
            .map(|(slot, checksum)| {
                (slot, SlotEntry { checksum, values: values.remove(&slot).unwrap_or_default() })
            })
            .collect();
        Ok(Self { entries })
    }
}

/// The options of a run: the values saved with the game (the bound slot's) and the ones kept for this PC.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Options {
    pub game: Values,
    pub pc: Values,
}

impl Options {
    /// The map `category` is kept in.
    #[must_use]
    pub fn values(&self, category: Category) -> &Values {
        if category.per_game() { &self.game } else { &self.pc }
    }

    /// The map `category` is kept in, to change.
    pub fn values_mut(&mut self, category: Category) -> &mut Values {
        if category.per_game() { &mut self.game } else { &mut self.pc }
    }

    /// The indices of the rows of `category`, as the screen shows them.
    #[must_use]
    pub fn indices(&self, category: Category, cheats: &Cheats, text_speed: u8) -> Vec<usize> {
        category
            .rows()
            .iter()
            .map(|row| row.index(self.values(category), cheats, text_speed))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_category_has_a_unique_key_per_row_and_a_default_inside_its_choices() {
        let mut keys = Vec::new();
        for category in Category::ALL {
            for row in category.rows() {
                assert!(!keys.contains(&row.key), "{} twice", row.key);
                keys.push(row.key);
                assert!(row.default_index() < row.count(), "{}", row.key);
                let prefix = if category == Category::Accessibility {
                    "access".to_owned()
                } else {
                    category.name().to_lowercase()
                };
                assert!(row.key.starts_with(&prefix), "{}", row.key);
            }
        }
        assert!(keys.contains(&"video.scale") && keys.contains(&"gameplay.text_speed"));
    }

    #[test]
    fn the_categories_saved_with_the_game_are_difficulty_and_gameplay() {
        let per_game: Vec<_> = Category::ALL.iter().filter(|c| c.per_game()).collect();
        assert_eq!(per_game, [&Category::Difficulty, &Category::Gameplay]);
        assert_eq!(Category::Video.scope_label(), "This PC");
        assert_eq!(Category::Gameplay.scope_label(), "Saved with the game");
        assert_eq!(Category::Accessibility.next(), Category::Difficulty);
        assert_eq!(Category::Difficulty.previous(), Category::Accessibility);
    }

    #[test]
    fn a_stored_row_reads_its_default_then_its_value_and_clamps_a_wild_one() {
        let row = &video::ROWS[1];
        let mut values = Values::new();
        let mut cheats = Cheats::neutral();
        assert_eq!(row.index(&values, &cheats, 0), 2, "4x by default");
        assert_eq!(row.set(&mut values, &mut cheats, 6), Effect::Stored);
        assert_eq!(row.index(&values, &cheats, 0), 6);
        values.set(row.key, 99);
        assert_eq!(row.index(&values, &cheats, 0), 6, "clamped to the last choice");
        values.set(row.key, -4);
        assert_eq!(row.index(&values, &cheats, 0), 2, "a negative number is the default");
    }

    #[test]
    fn stepping_wraps_and_a_reset_goes_back_to_the_default() {
        let row = &video::ROWS[1];
        let (mut values, mut cheats) = (Values::new(), Cheats::neutral());
        row.step(&mut values, &mut cheats, 0, -1);
        row.step(&mut values, &mut cheats, 0, -1);
        row.step(&mut values, &mut cheats, 0, -1);
        assert_eq!(row.index(&values, &cheats, 0), 6, "left from 2x wraps to 8x");
        row.step(&mut values, &mut cheats, 0, 1);
        assert_eq!(row.index(&values, &cheats, 0), 0);
        row.reset(&mut values, &mut cheats);
        assert_eq!(row.index(&values, &cheats, 0), 2);
    }

    #[test]
    fn the_encounter_row_is_a_view_of_the_cheats() {
        let row = gameplay::ROWS.iter().find(|row| row.key == "gameplay.encounters").unwrap();
        let (mut values, mut cheats) = (Values::new(), Cheats::neutral());
        assert_eq!(row.index(&values, &cheats, 0), 3, "x1");
        assert_eq!(row.step(&mut values, &mut cheats, 0, 1), Effect::Cheats);
        assert_eq!((row.index(&values, &cheats, 0), cheats.index(Setting::Encounters)), (4, 3));
        row.set(&mut values, &mut cheats, 0);
        assert!(cheats.random_battles_off(), "Off is the cheat random battles off");
        assert_eq!(row.index(&values, &cheats, 0), 0);
        row.set(&mut values, &mut cheats, 1);
        assert!(!cheats.random_battles_off());
        assert_eq!(cheats.multiplier(Setting::Encounters), crate::cheats::Multiplier::new(1, 4));
        assert!(values.is_empty(), "nothing is stored: the cheats are the one truth");
        row.reset(&mut values, &mut cheats);
        assert!(cheats.is_neutral());
    }

    #[test]
    fn the_text_speed_row_is_a_view_of_the_game_and_asks_the_host_to_write_it() {
        let row = &gameplay::ROWS[0];
        let (mut values, mut cheats) = (Values::new(), Cheats::neutral());
        // The byte counts from the fastest (0); the row shows slowest first.
        assert_eq!(row.index(&values, &cheats, 4), 0, "byte 4 is the slowest");
        assert_eq!(row.index(&values, &cheats, 0), 4, "byte 0 is the fastest");
        assert_eq!(row.step(&mut values, &mut cheats, 4, 1), Effect::TextSpeed(3), "faster");
        assert_eq!(
            row.step(&mut values, &mut cheats, 0, 1),
            Effect::TextSpeed(4),
            "wraps to the slowest"
        );
        assert_eq!(row.step(&mut values, &mut cheats, 0, -1), Effect::TextSpeed(1), "slower");
        assert_eq!(row.index(&values, &cheats, 200), 0, "a wild byte is clamped to the slowest");
        assert_eq!(row.reset(&mut values, &mut cheats), Effect::TextSpeed(TEXT_SPEED_DEFAULT));
    }

    #[test]
    fn the_cheats_button_opens_the_cheats_and_has_no_value() {
        let row = difficulty::ROWS.last().unwrap();
        let (mut values, mut cheats) = (Values::new(), Cheats::neutral());
        assert_eq!(row.step(&mut values, &mut cheats, 0, 1), Effect::Open(Opens::Cheats));
        assert_eq!(row.reset(&mut values, &mut cheats), Effect::None);
        assert_eq!(row.text(0), "Open Cheats");
    }

    #[test]
    fn the_pc_file_round_trips_keeps_unknown_keys_and_skips_damaged_lines() {
        let mut values = Values::new();
        values.set("video.scale", 4);
        values.set("future.thing", 7);
        let text = values.to_text();
        assert!(text.starts_with("ssd-settings 1\n"));
        assert_eq!(Values::from_text(&text).unwrap(), values);
        let rough = "ssd-settings 3\nvideo.scale = 5\nnonsense\nvideo.look=x\n=4\naudio.music=-2\n";
        let read = Values::from_text(rough).unwrap();
        assert_eq!(
            read.get("video.scale"),
            Some(5),
            "a newer version is read as far as it is understood"
        );
        assert_eq!(read.get("video.look"), None);
        assert_eq!(read.get("audio.music"), Some(-2));
        assert_eq!(read.get(""), None);
        assert!(Values::from_text("").is_err() && Values::from_text("ssd-options 1\n").is_err());
        assert!(
            Values::from_text("ssd-settings\n").is_err()
                && Values::from_text("ssd-settings 0\n").is_err()
        );
        assert_eq!(
            Values::from_text("ssd-settings 1\n").unwrap(),
            Values::new(),
            "no keys: all defaults"
        );
    }

    #[test]
    fn the_slot_file_keeps_each_slots_values_by_its_checksum_word() {
        let mut slots = SlotOptions::empty();
        let mut values = Values::new();
        values.set("difficulty.preset", 2);
        slots.store(1, 0xBEEF, values.clone());
        slots.store(0, 7, Values::new());
        let text = slots.to_text();
        assert!(text.starts_with("ssd-options 1\n"));
        let read = SlotOptions::from_text(&text).unwrap();
        assert_eq!(read, slots);
        assert_eq!(read.entry_for(1, 0xBEEF), Some(&values));
        assert_eq!(read.entry_for(1, 0xBEEE), None, "a slot changed since has no entry");
        assert_eq!(read.entry_for(0, 7), Some(&Values::new()));
        assert_eq!(read.entry_for(2, 0), None);
    }

    #[test]
    fn a_slot_file_with_missing_unknown_or_damaged_lines_still_reads() {
        let text = "ssd-options 2\ns0.gameplay.x=1\ns1.sum=5\ns1.new.key=9\ns1.bad=oops\nsx.sum=3\ns2.sum=70000\njunk\n";
        let read = SlotOptions::from_text(text).unwrap();
        assert_eq!(read.entry_for(0, 0), None, "no checksum word: left out");
        assert_eq!(read.entry_for(1, 5).and_then(|v| v.get("new.key")), Some(9));
        assert_eq!(read.entry_for(2, 0), None, "a checksum word out of range is no entry");
        assert!(SlotOptions::from_text("hello").is_err());
    }

    #[test]
    fn the_options_pick_the_map_by_the_category_and_reset_one_category_only() {
        let mut options = Options::default();
        options.values_mut(Category::Gameplay).set("gameplay.a", 1);
        options.values_mut(Category::Video).set("video.scale", 5);
        assert_eq!(options.game.get("gameplay.a"), Some(1));
        assert_eq!(options.pc.get("video.scale"), Some(5));
        options.pc.reset_category(Category::Video);
        assert_eq!(options.pc.get("video.scale"), None);
        assert_eq!(options.game.get("gameplay.a"), Some(1));
        let indices = options.indices(Category::Video, &Cheats::neutral(), 0);
        assert_eq!(indices, [0, 2, 0, 0, 0, 0, 0, 0], "the page was reset: the defaults");
    }
}
