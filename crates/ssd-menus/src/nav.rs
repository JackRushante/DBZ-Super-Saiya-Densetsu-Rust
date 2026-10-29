//! The HD navigation: the cursors of the HD screens and what a key chooses. Pure: the entries and
//! the cursors are plain values. `ssd-game`'s menu driver turns a choice into the presses the
//! original menu needs, so the original keeps every consequence; this module only moves the HD
//! cursors and reports what was chosen.

mod status;

use crate::cheats::Row;
use crate::options::controls::{self, Device};
use crate::options::{Category, Kind, Opens};
use crate::screens::cards::HAND;
use crate::screens::{PAGE_SIZE, SaveNav, TextNav};
use crate::view::{Screen, Sheet, Tab};

/// The line A shows on a tab with no stack: there is nothing under the cursor to use or drop.
const EMPTY_TAB: &str = "Nothing on this tab.";

/// A navigation key: the host maps its keyboard and gamepad to these.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    /// Chooses the entry under the cursor (the SNES A button).
    Confirm,
    /// Goes back: one level up, or closes the menu from the command window (the SNES B button).
    Cancel,
    /// The next tab of the bag, or the next page of the options (the shoulder button R).
    Tab,
    /// The previous page of the options (the shoulder button L); the bag treats it as Tab.
    TabBack,
    /// The Y button: on the options, Reset this page.
    ResetPage,
    /// The Select button: on the options, Reset all.
    ResetAll,
    /// The X button: on the Data packs screen, Apply the set.
    Apply,
}

/// An entry of the command window or of the Menu list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Entry {
    Talk,
    Look,
    Fly,
    Land,
    Item,
    Menu,
    Cards,
    Status,
    Order,
    Text,
    Save,
    /// The cheats screen (M5).
    Cheats,
    /// The options screen (M7).
    Options,
}

impl Entry {
    /// The label the window shows.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Talk => "Talk",
            Self::Look => "Look",
            Self::Fly => "Fly",
            Self::Land => "Land",
            Self::Item => "Item",
            Self::Menu => "Menu",
            Self::Cards => "Cards",
            Self::Status => "Status",
            Self::Order => "Order",
            Self::Text => "Text",
            Self::Save => "Save",
            Self::Cheats => "Cheats",
            Self::Options => "Options",
        }
    }

    /// The help line shown while the entry is under the cursor.
    #[must_use]
    pub const fn help(self) -> &'static str {
        match self {
            Self::Talk => "Talk to the person ahead.",
            Self::Look => "Search the ground nearby.",
            Self::Fly => "Take off and fly the map.",
            Self::Land => "Land where the party flies.",
            Self::Item => "Use the items you carry.",
            Self::Menu => "Cards, status and saves.",
            Self::Cards => "Look at the party's cards.",
            Self::Status => "See each fighter's stats.",
            Self::Order => "Change the party order.",
            Self::Text => "Set the text speed.",
            Self::Save => "Save the game in a slot.",
            Self::Cheats => "Set the cheats of this run.",
            Self::Options => "Game, picture, sound and keys.",
        }
    }

    /// Whether the entry does something: a greyed entry chooses nothing. Every entry does, since the options exist.
    #[must_use]
    pub const fn enabled(self) -> bool {
        let _ = self;
        true
    }
}

/// The rows of the command window, in the original's order: the field's five commands, a town's
/// four (no Fly), and the flying three (Land first).
#[must_use]
pub const fn command_rows(flying: bool, town: bool) -> &'static [Entry] {
    if flying {
        &FLYING_ROWS
    } else if town {
        &TOWN_ROWS
    } else {
        &FIELD_ROWS
    }
}

const FIELD_ROWS: [Entry; 5] = [Entry::Talk, Entry::Look, Entry::Fly, Entry::Item, Entry::Menu];
const TOWN_ROWS: [Entry; 4] = [Entry::Talk, Entry::Look, Entry::Item, Entry::Menu];
const FLYING_ROWS: [Entry; 3] = [Entry::Land, Entry::Item, Entry::Menu];

/// The rows of the Menu list: the original's five (Cards to Save, in its order), then Cheats and
/// Options.
pub const MENU_ROWS: [Entry; 7] = [
    Entry::Cards,
    Entry::Status,
    Entry::Order,
    Entry::Text,
    Entry::Save,
    Entry::Cheats,
    Entry::Options,
];

/// The rows the original's Menu list has: the first ones of [`MENU_ROWS`].
pub const ORIGINAL_MENU_ROWS: usize = 5;

/// The rows of the item list's left or right column on a page.
const COLUMN: usize = PAGE_SIZE / 2;

/// One stack of the bag's list, as the navigation needs it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemRow {
    /// The stack's id in the bag.
    pub id: u32,
    /// Usable here (on the map): Use is offered.
    pub usable: bool,
    /// Can be dropped: Drop is offered.
    pub droppable: bool,
}

/// What a key chooses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Nothing: the cursor moved, or the key does nothing on this screen.
    None,
    /// The enabled entry under the cursor is chosen.
    Choose(Entry),
    /// Back: from the command window the menu closes; from a list the command window comes back.
    Back,
    /// Use one unit of the stack `stack`: the original takes the unit, and asks for nothing.
    Use { stack: u32 },
    /// Drop one unit of the stack `stack`.
    Drop { stack: u32 },
    /// A line for the screen's foot: the choice does nothing, and says why.
    Notice(&'static str),
    /// On the cheats screen, the choice of `row` moves by `by` steps (-1 Left, 1 Right or A).
    Adjust { row: Row, by: i8 },
    /// On the cheats screen, Reset all: every cheat is off.
    ResetAll,
    /// On the item spawner, `count` units of `object` go into the bag.
    Spawn { object: u8, count: u8 },
    /// On the options screen, the row `row` of `category` moves by `by` steps (-1 Left, 1 Right or A).
    OptionChange { category: Category, row: usize, by: i8 },
    /// On the options screen, Reset this page: every row of `category` goes back to its default.
    OptionsResetPage(Category),
    /// On the options screen, Reset all was confirmed: every option goes back to its default.
    OptionsResetAll,
    /// On the options screen, a button opens another screen.
    OptionsOpen(Opens),
    /// On the options screen, "Apply video changes?" was answered: Keep (`true`) or Revert (`false`).
    OptionsVideo { keep: bool },
    /// On the Text screen, A kept the speed (the byte of `$01D0`: 0 the fastest to 4 the slowest).
    TextKeep(u8),
    /// On the Save screen, the slot `0` (0 to 2) was chosen, and the overwrite question, if there was one, answered Yes.
    SaveSlot(u8),
    /// On the Save screen, the save is done and A (or B) takes the game back to the field.
    SaveClose,
    /// On the Order screen, "Apply order" was chosen: the order the screen holds goes to the original.
    OrderApply,
    /// On the Data packs screen, the pack at place `at` of the list is switched on or off.
    PackToggle(usize),
    /// On the Data packs screen, the pack at place `at` moves by `by` places (-1 up, 1 down) in the order.
    PackMove { at: usize, by: i8 },
    /// On the Data packs screen, Apply was confirmed: the set is applied.
    PacksApply,
    /// On the Data packs screen, "Install a pack..." was chosen: the host opens the file picker.
    PacksInstall,
    /// On the Data packs screen, the chapter on the line `line` of the list is started (the question was confirmed).
    ChapterStart(usize),
    /// On the Data packs screen, "Main game" was confirmed: the main saves are back.
    ChapterMain,
}

/// A question the Data packs screen asks. Its choice is 0 for the confirm button and 1 for Cancel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModsAsk {
    /// "Apply the packs?"
    Apply,
    /// "Start this chapter?", for the chapter on the list's line with this index.
    Chapter(usize),
    /// "Back to the main game?"
    Main,
}

/// A line of the Data packs screen's list, as the cursor moves through it: the packs first, then "Install a pack...", then
/// "Main game" (in a chapter session) and the chapters of the packs that are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModsLine {
    /// An installed pack. A refused pack cannot be switched on.
    Pack { refused: bool },
    /// "Install a pack...": the file picker, for a pack to copy into the mods folder.
    Install,
    /// "Main game": the main saves, from a chapter session.
    Main,
    /// A chapter: locked until its condition holds.
    Chapter { locked: bool },
}

/// The cursor, the question and the held Y of the Data packs screen (see [`Nav::key_mods`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ModsNav {
    /// The line under the cursor: an index of the lines of the list.
    cursor: usize,
    /// The question on the screen, and the choice under the cursor (0 the confirm button, 1 Cancel).
    ask: Option<(ModsAsk, u8)>,
    /// Y is held: Up and Down move the pack under the cursor in the order, instead of the cursor.
    moving: bool,
}

/// The cursors of the HD screens: the command window, the Menu list, the bag and the cheats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Nav {
    screen: Screen,
    command: usize,
    menu: usize,
    bag: Bag,
    cheats: usize,
    /// The item spawner of the cheats screen.
    spawner: Spawner,
    options: OptionsNav,
    /// The Cheats screen was opened from the options, and goes back there.
    cheats_from_options: bool,
    /// The Data packs screen's cursor and question.
    mods: ModsNav,
    /// The card under the cursor of the Cards screen: an index of the hand.
    cards: usize,
    /// The member under the cursor of the Status screen.
    status: usize,
    /// The Text screen's speed and preview.
    text: TextNav,
    /// The Save screen's cursor, question and phase.
    save: SaveNav,
}

/// The line A shows on a refused pack: it stays off.
const REFUSED_PACK: &str = "A refused pack stays off.";
/// The line A shows on a locked chapter.
const LOCKED_CHAPTER: &str = "Locked: the game must be finished first.";

/// A key the Controls page refused: where the cursor was, the action that holds the key, and the frames the
/// row still flashes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refusal {
    pub position: usize,
    pub by: &'static str,
    pub flash: u16,
}

/// The frames the refused row flashes (a second).
pub const REFUSAL_FLASH: u16 = 60;

/// The objects of the game the item spawner lists (ids 0 to 39).
pub const SPAWNER_OBJECTS: u8 = 40;
/// The rows of a page of the item spawner's list.
pub const SPAWNER_PAGE: u8 = 11;
/// The most units the spawner adds at once (a stack of the bag); a story object is added one at a time.
pub const SPAWN_MAX: u8 = 99;

/// The item spawner of the cheats screen: open or not, the object under the cursor, how many to add, and the story
/// warning's choice while it asks (0 Add anyway, 1 Cancel).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Spawner {
    open: bool,
    object: u8,
    count: u8,
    confirm: Option<u8>,
}

impl Spawner {
    /// The object under the cursor.
    #[must_use]
    pub const fn object(&self) -> u8 {
        self.object
    }

    /// How many units the next add puts in the bag (at least 1).
    #[must_use]
    pub fn count(&self) -> u8 {
        self.count.max(1)
    }

    /// The story warning's choice while it asks: 0 Add anyway, 1 Cancel.
    #[must_use]
    pub const fn confirm(&self) -> Option<u8> {
        self.confirm
    }
}

/// The cursors of the options screen: the category, the row in it, and Reset all's question while it is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OptionsNav {
    category: usize,
    row: usize,
    /// While Reset all asks: the choice under the cursor, 0 Yes and 1 No.
    confirm: Option<u8>,
    /// While "Apply video changes?" asks: the choice under the cursor, 0 Keep and 1 Revert.
    video_confirm: Option<u8>,
    /// The Controls page: the device it shows (0 keyboard, 1 gamepad), told by the host from the stored value.
    device: u8,
    /// The Controls page waits for the next key or button.
    listening: bool,
    /// The Controls page refused the last key.
    refused: Option<Refusal>,
}

/// The cursors and the sheet of the bag's screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Bag {
    tab: Tab,
    /// The cursor: an index of the stacks the tab lists.
    cursor: usize,
    sheet: Sheet,
    /// On Choose: 0 Use, 1 Drop; on Confirm: 0 Yes, 1 No.
    choice: u8,
}

impl Nav {
    /// The screen the HD shows.
    #[must_use]
    pub const fn screen(&self) -> Screen {
        self.screen
    }

    /// The cursor as a view shows it: the row of the command window or of the Menu list, or the row
    /// on the page of the bag.
    #[must_use]
    pub fn cursor(&self) -> u8 {
        let row = match self.screen {
            Screen::Command => self.command,
            Screen::Menu => self.menu,
            Screen::Items => self.bag.cursor % PAGE_SIZE,
            Screen::Cheats => self.cheats,
            Screen::Options => self.options_row(),
            Screen::Mods => self.mods.cursor,
            Screen::Cards => self.cards,
            Screen::Status => self.status,
            Screen::Text => usize::from(self.text.speed()),
            Screen::Save => usize::from(self.save.cursor()),
            // The Order screen's cursor is its own (`OrderNav`, kept by the driver).
            Screen::Order => 0,
        };
        u8::try_from(row).unwrap_or(0)
    }

    /// The Text screen's speed and preview.
    #[must_use]
    pub const fn text(&self) -> &TextNav {
        &self.text
    }

    /// The Save screen's cursor, question and phase.
    #[must_use]
    pub const fn save(&self) -> &SaveNav {
        &self.save
    }

    /// The Save screen opens on its first slot, with nothing asked.
    pub fn open_save(&mut self) {
        self.enter(Screen::Save);
        self.save.open();
    }

    /// The host tells which save slots hold a game.
    pub fn set_save_used(&mut self, used: [bool; crate::screens::SAVE_SLOTS]) {
        self.save.set_used(used);
    }

    /// The original starts saving to `slot` (0 to 2).
    pub fn save_saving(&mut self, slot: u8) {
        self.save.saving(slot);
    }

    /// The original saved to `slot` (0 to 2).
    pub fn save_done(&mut self, slot: u8) {
        self.save.done(slot);
    }

    /// The Text screen opens on the game's speed `speed` (the byte of `$01D0`), its preview from the start.
    pub fn open_text(&mut self, speed: u8) {
        self.enter(Screen::Text);
        self.text.open(speed);
    }

    /// The cursor of the Data packs screen: an index of its lines (see [`ModsLine`]).
    #[must_use]
    pub const fn mods_cursor(&self) -> usize {
        self.mods.cursor
    }

    /// The question the Data packs screen shows, and the choice under its cursor.
    #[must_use]
    pub const fn mods_ask(&self) -> Option<(ModsAsk, u8)> {
        self.mods.ask
    }

    /// Whether Y is held on the Data packs screen (see [`Nav::key_mods`]).
    #[must_use]
    pub const fn mods_moving(&self) -> bool {
        self.mods.moving
    }

    /// The host tells the Data packs screen whether Y is held this frame.
    pub fn set_mods_moving(&mut self, moving: bool) {
        self.mods.moving = moving;
    }

    /// The page of the bag the cursor is on (0 on the other screens).
    #[must_use]
    pub fn page(&self) -> u8 {
        match self.screen {
            Screen::Items => u8::try_from(self.bag.cursor / PAGE_SIZE).unwrap_or(0),
            Screen::Command
            | Screen::Menu
            | Screen::Cheats
            | Screen::Options
            | Screen::Mods
            | Screen::Cards
            | Screen::Text
            | Screen::Order
            | Screen::Status
            | Screen::Save => 0,
        }
    }

    /// The item spawner, while the cheats screen shows it.
    #[must_use]
    pub fn spawner(&self) -> Option<Spawner> {
        (self.screen == Screen::Cheats && self.spawner.open).then_some(self.spawner)
    }

    /// What `key` does on the item spawner: Up and Down move through the objects, L and R by a page, Left and Right
    /// change how many (a story object stays at 1), Y puts it back to 1, A adds (a story object asks first, with the
    /// cursor on Cancel), B closes the spawner. `story` says whether an object is one the story checks.
    pub fn key_spawner(&mut self, key: Key, story: &dyn Fn(u8) -> bool) -> Action {
        let spawner = &mut self.spawner;
        if let Some(choice) = spawner.confirm {
            return match key {
                Key::Up | Key::Down | Key::Left | Key::Right | Key::Tab | Key::TabBack => {
                    spawner.confirm = Some(choice ^ 1);
                    Action::None
                }
                Key::Confirm if choice == 0 => {
                    spawner.confirm = None;
                    Action::Spawn { object: spawner.object, count: 1 }
                }
                Key::Confirm | Key::Cancel => {
                    spawner.confirm = None;
                    Action::None
                }
                Key::ResetPage | Key::ResetAll | Key::Apply => Action::None,
            };
        }
        let objects = u16::from(SPAWNER_OBJECTS);
        let page = u16::from(SPAWNER_PAGE);
        let at = u16::from(spawner.object);
        let max = |object: u8| if story(object) { 1 } else { SPAWN_MAX };
        match key {
            Key::Up => spawner.object = ((at + objects - 1) % objects) as u8,
            Key::Down => spawner.object = ((at + 1) % objects) as u8,
            Key::TabBack => spawner.object = at.saturating_sub(page) as u8,
            Key::Tab => spawner.object = (at + page).min(objects - 1) as u8,
            Key::Left => {
                let count = spawner.count.max(1);
                spawner.count = if count <= 1 { max(spawner.object) } else { count - 1 };
            }
            Key::Right => {
                let count = spawner.count.max(1);
                spawner.count = if count >= max(spawner.object) { 1 } else { count + 1 };
            }
            Key::ResetPage => spawner.count = 1,
            Key::Confirm if story(spawner.object) => spawner.confirm = Some(1),
            Key::Confirm => {
                return Action::Spawn { object: spawner.object, count: spawner.count.max(1) };
            }
            Key::Cancel => {
                spawner.open = false;
                spawner.confirm = None;
            }
            Key::ResetAll | Key::Apply => {}
        }
        spawner.count = spawner.count.max(1).min(max(spawner.object));
        Action::None
    }

    /// The row of the cheats screen under the cursor.
    #[must_use]
    pub fn cheats_row(&self) -> Row {
        Row::ALL[self.cheats.min(Row::ALL.len() - 1)]
    }

    /// The category the options screen is on.
    #[must_use]
    pub fn options_category(&self) -> Category {
        {
            let shown = Category::shown();
            shown[self.options.category.min(shown.len() - 1)]
        }
    }

    /// The row of the options screen under the cursor (an index of the category's rows; 0 when it has none).
    /// On the Controls page the cursor is a place in the grid ([`Nav::controls_position`]), and the row is the
    /// shown device's row of that place (the Touch page: one of its six rows, a list).
    #[must_use]
    pub fn options_row(&self) -> usize {
        if self.options_category() == Category::Controls {
            if let Some((first, count)) = self.list_page() {
                return match self.options.row.min(count) {
                    0 => 0,
                    at => first + at - 1,
                };
            }
            return match self.controls_action() {
                Some(action) => controls::row_of(self.controls_device(), action),
                None => 0,
            };
        }
        self.options.row.min(self.options_category().rows().len().saturating_sub(1))
    }

    /// The first row and the number of rows of the Controls page when it shows a list (the Touch or the Turbo page).
    #[must_use]
    pub fn list_page(&self) -> Option<(usize, usize)> {
        controls::list_rows(usize::from(self.options.device))
    }

    /// Whether the Controls page shows the touch controls' rows.
    #[must_use]
    pub fn touch_page(&self) -> bool {
        usize::from(self.options.device) == controls::TOUCH_DEVICE
    }

    /// The host tells the Controls page which page the stored Device value says (0 keyboard, 1 gamepad, 2 touch).
    pub fn set_controls_page(&mut self, page: usize) {
        self.options.device = u8::try_from(page).unwrap_or(0);
    }

    /// The device the Controls page shows.
    #[must_use]
    pub fn controls_device(&self) -> Device {
        Device::from_index(usize::from(self.options.device))
    }

    /// The host tells the Controls page which device the stored value says.
    pub fn set_controls_device(&mut self, device: Device) {
        self.set_controls_page(device.index());
    }

    /// The place of the Controls page's cursor: 0 is the device row, 1 to 17 the actions, two to a line.
    #[must_use]
    pub fn controls_position(&self) -> usize {
        self.options.row.min(controls::ACTIONS)
    }

    /// The action under the Controls page's cursor (0 to 16), none on the device row.
    #[must_use]
    pub fn controls_action(&self) -> Option<usize> {
        self.controls_position().checked_sub(1)
    }

    /// Whether the Controls page waits for the next key or button.
    #[must_use]
    pub const fn options_listening(&self) -> bool {
        self.options.listening
    }

    /// The Controls page stops waiting for a key (one came, or Esc cancelled).
    pub fn stop_listening(&mut self) {
        self.options.listening = false;
    }

    /// The Controls page refused a key: `by` is the action that already holds it. The row flashes for a second.
    pub fn refuse(&mut self, by: &'static str) {
        self.options.listening = false;
        self.options.refused =
            Some(Refusal { position: self.controls_position(), by, flash: REFUSAL_FLASH });
    }

    /// The key the Controls page refused, while the refusal still shows.
    #[must_use]
    pub const fn options_refusal(&self) -> Option<Refusal> {
        self.options.refused
    }

    /// One frame goes by: a refused row's flash runs down.
    pub fn tick(&mut self) {
        if self.screen == Screen::Text {
            self.text.tick();
        }
        if let Some(refusal) = self.options.refused.as_mut() {
            refusal.flash = refusal.flash.saturating_sub(1);
        }
    }

    /// While Reset all asks, the choice under the cursor (0 Yes, 1 No).
    #[must_use]
    pub const fn options_confirm(&self) -> Option<u8> {
        self.options.confirm
    }

    /// While "Apply video changes?" asks, the choice under the cursor (0 Keep, 1 Revert).
    #[must_use]
    pub const fn options_video_confirm(&self) -> Option<u8> {
        self.options.video_confirm
    }

    /// "Apply video changes?" opens, the cursor on Revert.
    pub fn open_video_confirm(&mut self) {
        self.options.video_confirm = Some(1);
    }

    /// "Apply video changes?" closes (it was answered, or timed out).
    pub fn close_video_confirm(&mut self) {
        self.options.video_confirm = None;
    }

    /// Where B on the Cheats screen goes: the options when they opened it, else the Menu list.
    #[must_use]
    pub const fn cheats_back(&self) -> Screen {
        if self.cheats_from_options { Screen::Options } else { Screen::Menu }
    }

    /// The Cheats screen opens from a button of the options.
    pub fn enter_cheats_from_options(&mut self) {
        self.enter(Screen::Cheats);
        self.cheats_from_options = true;
    }

    /// The tab of the bag.
    #[must_use]
    pub const fn tab(&self) -> Tab {
        self.bag.tab
    }

    /// The sheet over the bag's list.
    #[must_use]
    pub const fn sheet(&self) -> Sheet {
        self.bag.sheet
    }

    /// The bag's cursor: the index of the stack under it, among the stacks of the tab.
    #[must_use]
    pub const fn bag_cursor(&self) -> usize {
        self.bag.cursor
    }

    /// The choice of the open sheet (Use or Drop, Yes or No).
    #[must_use]
    pub const fn choice(&self) -> u8 {
        self.bag.choice
    }

    /// The HD shows `screen` from now on. A list starts at its first row; the command window keeps
    /// the row it had, as the original's does when it comes back from a list.
    pub fn enter(&mut self, screen: Screen) {
        self.screen = screen;
        match screen {
            // The Order screen's cursor is its own (`OrderNav`, kept by the driver).
            Screen::Command | Screen::Order => {}
            Screen::Menu => self.menu = 0,
            Screen::Items => {
                self.bag.cursor = 0;
                self.bag.sheet = Sheet::Browse;
            }
            Screen::Cheats => {
                self.cheats = 0;
                self.cheats_from_options = false;
            }
            Screen::Options => self.options = OptionsNav::default(),
            Screen::Mods => self.mods = ModsNav::default(),
            Screen::Cards => self.cards = 0,
            Screen::Status => self.status = 0,
            Screen::Text => self.text = TextNav::default(),
            Screen::Save => self.save.open(),
        }
    }

    /// The HD shows `screen` again with its cursor where it was: the Menu list after the cheats, which the
    /// original keeps open under them.
    pub fn resume(&mut self, screen: Screen) {
        self.screen = screen;
    }

    /// A new menu session: the command window at its first row, as the original opens it.
    pub fn open(&mut self) {
        self.enter(Screen::Command);
        self.command = 0;
    }

    /// Shows the bag's `tab` from its first stack.
    pub fn set_tab(&mut self, tab: Tab) {
        self.bag.tab = tab;
        self.bag.cursor = 0;
    }

    /// Back on the bag's list after an action: the sheet closes, the cursor stays where it was.
    pub fn close_sheet(&mut self) {
        self.bag.sheet = Sheet::Browse;
    }

    /// What `key` does on the HD screen: it moves the cursor or says what was chosen. `flying`
    /// and `town` pick the command rows; `items` is the number of entries of the item list.
    pub fn key(&mut self, key: Key, flying: bool, town: bool, items: usize) -> Action {
        match self.screen {
            Screen::Command => {
                let rows = command_rows(flying, town);
                self.command = self.command.min(rows.len() - 1);
                match key {
                    Key::Up => self.command = previous(self.command, rows.len()),
                    Key::Down => self.command = next(self.command, rows.len()),
                    Key::Confirm => return choose(rows[self.command]),
                    Key::Cancel => return Action::Back,
                    Key::Left
                    | Key::Right
                    | Key::Tab
                    | Key::TabBack
                    | Key::ResetPage
                    | Key::ResetAll
                    | Key::Apply => {}
                }
            }
            Screen::Menu => match key {
                Key::Up => self.menu = previous(self.menu, MENU_ROWS.len()),
                Key::Down => self.menu = next(self.menu, MENU_ROWS.len()),
                Key::Confirm => return choose(MENU_ROWS[self.menu]),
                Key::Cancel => return Action::Back,
                Key::Left
                | Key::Right
                | Key::Tab
                | Key::TabBack
                | Key::ResetPage
                | Key::ResetAll
                | Key::Apply => {}
            },
            // The bag is driven by `key_bag`, which has the stacks.
            Screen::Items => {
                let _ = items;
                if key == Key::Cancel && self.bag.sheet == Sheet::Browse {
                    return Action::Back;
                }
            }
            Screen::Cheats => match key {
                Key::Up => self.cheats = previous(self.cheats, Row::ALL.len()),
                Key::Down => self.cheats = next(self.cheats, Row::ALL.len()),
                Key::Left => return adjust(self.cheats_row(), -1),
                // A moves the choice on, and on Reset all it resets.
                Key::Confirm if self.cheats_row() == Row::ResetAll => return Action::ResetAll,
                // A or Right on the item spawner opens it, on the object it was last on, with a count of 1.
                Key::Right | Key::Confirm if self.cheats_row() == Row::ItemSpawner => {
                    self.spawner.open = true;
                    self.spawner.count = 1;
                    self.spawner.confirm = None;
                }
                Key::Right | Key::Confirm => return adjust(self.cheats_row(), 1),
                Key::Cancel => return Action::Back,
                Key::Tab | Key::TabBack | Key::ResetPage | Key::ResetAll | Key::Apply => {}
            },
            Screen::Options => return self.key_options(key),
            // The Data packs screen's keys are given by `key_mods`, with the list of its lines. The Status screen's by
            // `key_status`, which knows the party, and the Order screen's by its own cursor (`OrderNav`), which the
            // driver keeps.
            Screen::Mods | Screen::Status | Screen::Order => {}
            // The hand: Left and Right move over it (wrapping), B goes back. The original's A only closes its screen,
            // so the HD's A and the rest do nothing; Start closes (`Driver::close_menu`).
            Screen::Cards => match key {
                Key::Left => self.cards = previous(self.cards, HAND),
                Key::Right => self.cards = next(self.cards, HAND),
                Key::Cancel => return Action::Back,
                _ => {}
            },
            Screen::Text => return self.text.key(key),
            Screen::Save => return self.save.key(key),
        }
        Action::None
    }

    /// What `key` does on the Data packs screen, whose list has the lines `lines` (the packs, "Install a pack...", then "Main
    /// game" and the chapters). Up and Down move the cursor (wrapping); with Y held they move the pack under the cursor in the
    /// order. Left and Right do nothing. A switches a pack on or off (a refused pack says why it stays off), asks for the file
    /// picker on "Install a pack...", asks to start a
    /// chapter or to go back to the main game (a locked chapter says why it cannot start), and X asks to apply the set.
    /// B goes back. A question takes the keys: Left, Right, Up and Down change its choice, A chooses (the confirm button,
    /// choice 0, does the question; Cancel, choice 1, only closes it), and B closes it.
    pub fn key_mods(&mut self, key: Key, lines: &[ModsLine]) -> Action {
        if let Some((ask, choice)) = self.mods.ask {
            return match key {
                Key::Up | Key::Down | Key::Left | Key::Right | Key::Tab | Key::TabBack => {
                    self.mods.ask = Some((ask, choice ^ 1));
                    Action::None
                }
                Key::Confirm => {
                    self.mods.ask = None;
                    match (choice, ask) {
                        (0, ModsAsk::Apply) => Action::PacksApply,
                        (0, ModsAsk::Chapter(line)) => Action::ChapterStart(line),
                        (0, ModsAsk::Main) => Action::ChapterMain,
                        _ => Action::None,
                    }
                }
                Key::Cancel => {
                    self.mods.ask = None;
                    Action::None
                }
                Key::Apply | Key::ResetPage | Key::ResetAll => Action::None,
            };
        }
        let at = self.mods.cursor.min(lines.len().saturating_sub(1));
        let packs = lines.iter().take_while(|line| matches!(line, ModsLine::Pack { .. })).count();
        match key {
            Key::Up | Key::Down if self.mods.moving => {
                // Y held: the pack under the cursor moves to the place beside it, if that place is a pack too.
                let by: i8 = if key == Key::Up { -1 } else { 1 };
                let to = at.checked_add_signed(isize::from(by));
                match to {
                    Some(to) if at < packs && to < packs => {
                        self.mods.cursor = to;
                        Action::PackMove { at, by }
                    }
                    _ => Action::None,
                }
            }
            Key::Up if !lines.is_empty() => {
                self.mods.cursor = previous(at, lines.len());
                Action::None
            }
            Key::Down if !lines.is_empty() => {
                self.mods.cursor = next(at, lines.len());
                Action::None
            }
            Key::Confirm => match lines.get(at) {
                Some(ModsLine::Pack { refused: true }) => Action::Notice(REFUSED_PACK),
                Some(ModsLine::Pack { refused: false }) => Action::PackToggle(at),
                Some(ModsLine::Install) => Action::PacksInstall,
                Some(ModsLine::Main) => {
                    self.mods.ask = Some((ModsAsk::Main, 1));
                    Action::None
                }
                Some(ModsLine::Chapter { locked: true }) => Action::Notice(LOCKED_CHAPTER),
                Some(ModsLine::Chapter { locked: false }) => {
                    self.mods.ask = Some((ModsAsk::Chapter(at), 1));
                    Action::None
                }
                None => Action::None,
            },
            Key::Apply => {
                self.mods.ask = Some((ModsAsk::Apply, 1));
                Action::None
            }
            Key::Cancel => Action::Back,
            Key::Up
            | Key::Down
            | Key::Left
            | Key::Right
            | Key::Tab
            | Key::TabBack
            | Key::ResetPage
            | Key::ResetAll => Action::None,
        }
    }

    /// What `key` does on the options screen: Up and Down move the row, Left and Right (or A) change it, L and R
    /// change the page, Y resets the page, Select asks to reset all (a question with the cursor on No).
    fn key_options(&mut self, key: Key) -> Action {
        if let Some(choice) = self.options.video_confirm {
            return match key {
                Key::Up | Key::Down | Key::Left | Key::Right | Key::Tab | Key::TabBack => {
                    self.options.video_confirm = Some(choice ^ 1);
                    Action::None
                }
                Key::Confirm => Action::OptionsVideo { keep: choice == 0 },
                Key::Cancel => Action::OptionsVideo { keep: false },
                Key::ResetPage | Key::ResetAll | Key::Apply => Action::None,
            };
        }
        if let Some(choice) = self.options.confirm {
            match key {
                Key::Up | Key::Down | Key::Left | Key::Right | Key::Tab | Key::TabBack => {
                    self.options.confirm = Some(choice ^ 1);
                }
                Key::Confirm => {
                    self.options.confirm = None;
                    if choice == 0 {
                        return Action::OptionsResetAll;
                    }
                }
                Key::Cancel => self.options.confirm = None,
                Key::ResetPage | Key::ResetAll | Key::Apply => {}
            }
            return Action::None;
        }
        if self.options.listening {
            // The host gives the next key or button through the capture; a navigation key does nothing meanwhile.
            return Action::None;
        }
        self.options.refused = None;
        if self.options_category() == Category::Controls
            && !matches!(
                key,
                Key::Tab | Key::TabBack | Key::ResetPage | Key::ResetAll | Key::Cancel
            )
        {
            return self.key_controls(key);
        }
        let category = self.options_category();
        let rows = category.rows();
        let row = self.options_row();
        match key {
            Key::Up if !rows.is_empty() => self.options.row = step_row(rows, row, false),
            Key::Down if !rows.is_empty() => self.options.row = step_row(rows, row, true),
            Key::Left | Key::Right | Key::Confirm => {
                let Some(def) = rows.get(row) else { return Action::None };
                if let Kind::Button(_) = def.kind {
                    if key != Key::Left
                        && let crate::options::Source::Opens(opens) = def.source
                    {
                        return Action::OptionsOpen(opens);
                    }
                    return Action::None;
                }
                let by = if key == Key::Left { -1 } else { 1 };
                return Action::OptionChange { category, row, by };
            }
            Key::Tab => {
                self.options = OptionsNav {
                    category: category.next().index(),
                    device: self.options.device,
                    ..OptionsNav::default()
                }
            }
            Key::TabBack => {
                self.options = OptionsNav {
                    category: category.previous().index(),
                    device: self.options.device,
                    ..OptionsNav::default()
                };
            }
            Key::ResetPage => return Action::OptionsResetPage(category),
            Key::ResetAll => self.options.confirm = Some(1),
            Key::Cancel => return Action::Back,
            Key::Up | Key::Down | Key::Apply => {}
        }
        Action::None
    }

    /// What `key` does on the Controls page, a grid two to a line under the device row: Up and Down move a line
    /// (the device row between), Left and Right move across a line or, on the device row, change the device; A
    /// waits for a new key on a key box.
    fn key_controls(&mut self, key: Key) -> Action {
        if let Some((first, count)) = self.list_page() {
            return self.key_list(key, first, count);
        }
        let at = self.controls_position();
        match key {
            Key::Up => {
                self.options.row = match at {
                    0 => controls::ACTIONS,
                    1 | 2 => 0,
                    _ => at - 2,
                };
            }
            Key::Down => {
                self.options.row = match at {
                    0 => 1,
                    _ if at + 2 > controls::ACTIONS => 0,
                    _ => at + 2,
                };
            }
            Key::Left | Key::Right if at == 0 => {
                let by = if key == Key::Left { -1 } else { 1 };
                return Action::OptionChange { category: Category::Controls, row: 0, by };
            }
            Key::Left if at.is_multiple_of(2) => self.options.row = at - 1,
            Key::Right if at % 2 == 1 && at < controls::ACTIONS => self.options.row = at + 1,
            Key::Confirm if at == 0 => {
                return Action::OptionChange { category: Category::Controls, row: 0, by: 1 };
            }
            Key::Confirm => self.options.listening = true,
            _ => {}
        }
        Action::None
    }

    /// What `key` does on a list page of the Controls (the Touch page of M9, the Turbo page): the device row, then
    /// the `count` rows from table row `first`. Up and Down move (wrapping), Left and Right change a row (the device
    /// row: the page), A opens a button.
    fn key_list(&mut self, key: Key, first: usize, count: usize) -> Action {
        let at = self.options.row.min(count);
        match key {
            Key::Up => {
                self.options.row = if at == 0 { count } else { at - 1 };
            }
            Key::Down => {
                self.options.row = if at == count { 0 } else { at + 1 };
            }
            Key::Left | Key::Right | Key::Confirm if at == 0 => {
                let by = if key == Key::Left { -1 } else { 1 };
                return Action::OptionChange { category: Category::Controls, row: 0, by };
            }
            Key::Left | Key::Right | Key::Confirm => {
                let row = first + at - 1;
                let def = &controls::ROWS[row];
                if let Kind::Button(_) = def.kind {
                    if key != Key::Left
                        && let crate::options::Source::Opens(opens) = def.source
                    {
                        return Action::OptionsOpen(opens);
                    }
                    return Action::None;
                }
                let by = if key == Key::Left { -1 } else { 1 };
                return Action::OptionChange { category: Category::Controls, row, by };
            }
            _ => {}
        }
        Action::None
    }

    /// What `key` does on the bag's screen, with its stacks of the tab as `rows` and the party's
    /// `party` members. The cursors move, a sheet opens or closes, or the choice is reported.
    pub fn key_bag(&mut self, key: Key, rows: &[ItemRow]) -> Action {
        if self.screen != Screen::Items {
            return Action::None;
        }
        let last = rows.len().saturating_sub(1);
        self.bag.cursor = self.bag.cursor.min(last);
        let Some(row) = rows.get(self.bag.cursor).copied() else {
            // An empty tab (or a bag not bound yet): Back and the tabs work, and A says why nothing happens.
            return match key {
                Key::Cancel => Action::Back,
                Key::Confirm => Action::Notice(EMPTY_TAB),
                Key::Tab | Key::TabBack => self.tab_key(key),
                Key::Up
                | Key::Down
                | Key::Left
                | Key::Right
                | Key::ResetPage
                | Key::ResetAll
                | Key::Apply => Action::None,
            };
        };
        let bag = &mut self.bag;
        match bag.sheet {
            Sheet::Browse => match key {
                Key::Up => bag.cursor = bag.cursor.saturating_sub(1),
                Key::Down => bag.cursor = (bag.cursor + 1).min(last),
                Key::Left => bag.cursor = bag.cursor.saturating_sub(COLUMN),
                Key::Right => bag.cursor = (bag.cursor + COLUMN).min(last),
                Key::Tab | Key::TabBack => return self.tab_key(key),
                Key::ResetPage | Key::ResetAll | Key::Apply => {}
                Key::Cancel => return Action::Back,
                Key::Confirm => {
                    if !row.usable && !row.droppable {
                        return Action::Notice("This cannot be used or dropped here.");
                    }
                    bag.choice = u8::from(!row.usable);
                    bag.sheet = Sheet::Choose;
                }
            },
            Sheet::Choose => match key {
                Key::Up | Key::Down => {
                    if row.usable && row.droppable {
                        bag.choice ^= 1;
                    }
                }
                Key::Cancel => bag.sheet = Sheet::Browse,
                Key::Confirm => {
                    if bag.choice == 0 {
                        // One unit, as the original takes it: the original asks for nothing on the map.
                        bag.sheet = Sheet::Browse;
                        return Action::Use { stack: row.id };
                    } else if row.droppable {
                        // Drop asks Yes or No, and starts on No.
                        bag.sheet = Sheet::Confirm;
                    } else {
                        return Action::Notice("This cannot be dropped.");
                    }
                }
                Key::Left
                | Key::Right
                | Key::Tab
                | Key::TabBack
                | Key::ResetPage
                | Key::ResetAll
                | Key::Apply => {}
            },
            Sheet::Confirm => match key {
                Key::Up | Key::Down | Key::Left | Key::Right => bag.choice ^= 1,
                // Back to the choice, which is Drop: its choice (Use or Drop) is not the Yes or No toggled here.
                Key::Cancel => {
                    bag.choice = 1;
                    bag.sheet = Sheet::Choose;
                }
                Key::Confirm => {
                    if bag.choice == 0 {
                        bag.sheet = Sheet::Browse;
                        return Action::Drop { stack: row.id };
                    }
                    // No: back to the Use or Drop choice, which still shows Drop.
                    bag.sheet = Sheet::Choose;
                }
                Key::Tab | Key::TabBack | Key::ResetPage | Key::ResetAll | Key::Apply => {}
            },
        }
        Action::None
    }

    /// The Tab key on the bag: the next tab, and its first stack under the cursor.
    fn tab_key(&mut self, key: Key) -> Action {
        if matches!(key, Key::Tab | Key::TabBack) {
            let next = match self.bag.tab {
                Tab::All => Tab::Allies,
                Tab::Allies => Tab::Tools,
                Tab::Tools => Tab::All,
            };
            self.set_tab(next);
        }
        Action::None
    }
}

/// The row above `row` in a list of `rows`, wrapping to the last.
/// The row after (or before) `row` of `rows` that has a cursor, wrapping: read-only rows are skipped.
fn step_row(rows: &[crate::options::RowDef], row: usize, down: bool) -> usize {
    let mut at = row;
    for _ in 0..rows.len() {
        at = if down { next(at, rows.len()) } else { previous(at, rows.len()) };
        if !rows[at].is_info() {
            return at;
        }
    }
    row
}

fn previous(row: usize, rows: usize) -> usize {
    if row == 0 { rows - 1 } else { row - 1 }
}

/// The row below `row` in a list of `rows`, wrapping to the first.
fn next(row: usize, rows: usize) -> usize {
    (row + 1) % rows
}

/// The choice of confirming `entry`: a greyed entry chooses nothing.
fn choose(entry: Entry) -> Action {
    if entry.enabled() { Action::Choose(entry) } else { Action::None }
}

/// The change of a cheats row by `by` steps: Reset all has no choices to change.
fn adjust(row: Row, by: i8) -> Action {
    if row.choices() == 0 { Action::None } else { Action::Adjust { row, by } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(id: u32, usable: bool, droppable: bool) -> ItemRow {
        ItemRow { id, usable, droppable }
    }

    #[test]
    fn the_command_rows_depend_on_flying_and_town() {
        let labels = |rows: &[Entry]| rows.iter().map(|e| e.label()).collect::<Vec<_>>();
        assert_eq!(labels(command_rows(false, false)), ["Talk", "Look", "Fly", "Item", "Menu"]);
        assert_eq!(labels(command_rows(false, true)), ["Talk", "Look", "Item", "Menu"]);
        assert_eq!(labels(command_rows(true, false)), ["Land", "Item", "Menu"]);
        assert_eq!(labels(&MENU_ROWS[..5]), ["Cards", "Status", "Order", "Text", "Save"]);
    }

    #[test]
    fn the_command_cursor_wraps_both_ways() {
        let mut nav = Nav::default();
        nav.open();
        assert_eq!(nav.key(Key::Up, false, false, 0), Action::None);
        assert_eq!(nav.cursor(), 4, "up from the first row goes to Menu");
        assert_eq!(nav.key(Key::Down, false, false, 0), Action::None);
        assert_eq!(nav.cursor(), 0, "down from Menu goes to Talk");
        for _ in 0..2 {
            nav.key(Key::Down, true, false, 0);
        }
        assert_eq!(nav.cursor(), 2, "three rows while flying");
    }

    #[test]
    fn confirm_chooses_the_row_and_cancel_goes_back() {
        let mut nav = Nav::default();
        nav.open();
        nav.key(Key::Down, false, false, 0);
        nav.key(Key::Down, false, false, 0);
        assert_eq!(nav.key(Key::Confirm, false, false, 0), Action::Choose(Entry::Fly));
        assert_eq!(nav.key(Key::Cancel, false, false, 0), Action::Back);
        assert_eq!(nav.screen(), Screen::Command, "a choice does not move the screen by itself");
    }

    #[test]
    fn the_menu_list_opens_the_cheats_and_the_options() {
        let mut nav = Nav::default();
        nav.open();
        nav.enter(Screen::Menu);
        for _ in 0..5 {
            nav.key(Key::Down, false, false, 0);
        }
        assert_eq!(nav.cursor(), 5, "Cheats");
        assert_eq!(nav.key(Key::Confirm, false, false, 0), Action::Choose(Entry::Cheats));
        assert!(Entry::Cheats.enabled() && Entry::Save.enabled() && Entry::Options.enabled());
        assert_eq!(nav.key(Key::Down, false, false, 0), Action::None);
        assert_eq!(nav.cursor(), 6, "Options");
        assert_eq!(nav.key(Key::Confirm, false, false, 0), Action::Choose(Entry::Options));
        assert_eq!(nav.key(Key::Down, false, false, 0), Action::None);
        assert_eq!(nav.cursor(), 0, "the list wraps");
    }

    #[test]
    fn the_options_screen_moves_between_rows_and_pages_and_reports_the_changes() {
        let mut nav = Nav::default();
        nav.enter(Screen::Options);
        assert_eq!(
            (nav.options_category(), nav.options_row(), nav.cursor()),
            (Category::Difficulty, 0, 0)
        );
        let key = |nav: &mut Nav, key| nav.key(key, false, false, 0);
        // The Difficulty page: the preset changes with Left and Right (the rows that only show a value or a note are
        // skipped), and the Cheats button below opens with A and Right and does nothing on Left.
        let change = |by| Action::OptionChange { category: Category::Difficulty, row: 0, by };
        assert_eq!(key(&mut nav, Key::Right), change(1));
        assert_eq!(key(&mut nav, Key::Left), change(-1));
        assert_eq!(key(&mut nav, Key::Down), Action::None);
        assert_eq!(
            nav.options_row(),
            5,
            "Down passes the note and the three values: the Cheats button"
        );
        assert_eq!(key(&mut nav, Key::Confirm), Action::OptionsOpen(Opens::Cheats));
        assert_eq!(key(&mut nav, Key::Right), Action::OptionsOpen(Opens::Cheats));
        assert_eq!(key(&mut nav, Key::Left), Action::None);
        assert_eq!(key(&mut nav, Key::Down), Action::None);
        assert_eq!(nav.options_row(), 0, "the rows wrap to the preset");
        assert_eq!(key(&mut nav, Key::Up), Action::None);
        assert_eq!(nav.options_row(), 5, "Up wraps to the button");
        // R is the next page, with the cursor on its first row; L goes back and wraps.
        assert_eq!(key(&mut nav, Key::Tab), Action::None);
        assert_eq!(nav.options_category(), Category::Gameplay);
        assert_eq!(key(&mut nav, Key::Down), Action::None);
        assert_eq!(nav.options_row(), 1);
        assert_eq!(key(&mut nav, Key::Up), Action::None);
        assert_eq!(nav.options_row(), 0);
        assert_eq!(key(&mut nav, Key::Up), Action::None);
        assert_eq!(nav.options_row(), 5, "Up wraps to the last of the six rows");
        assert_eq!(key(&mut nav, Key::Down), Action::None);
        assert_eq!(nav.options_row(), 0, "the rows wrap");
        assert_eq!(key(&mut nav, Key::Down), Action::None);
        assert_eq!(
            key(&mut nav, Key::Left),
            Action::OptionChange { category: Category::Gameplay, row: 1, by: -1 }
        );
        assert_eq!(
            key(&mut nav, Key::Confirm),
            Action::OptionChange { category: Category::Gameplay, row: 1, by: 1 }
        );
        key(&mut nav, Key::TabBack);
        key(&mut nav, Key::TabBack);
        assert_eq!(nav.options_category(), Category::Accessibility, "L wraps to the last page");
        assert_eq!(nav.options_row(), 0);
        assert_eq!(
            key(&mut nav, Key::Right),
            Action::OptionChange { category: Category::Accessibility, row: 0, by: 1 }
        );
        assert_eq!(
            key(&mut nav, Key::ResetPage),
            Action::OptionsResetPage(Category::Accessibility)
        );
        assert_eq!(key(&mut nav, Key::Cancel), Action::Back);
    }

    #[test]
    fn the_video_question_starts_on_revert_and_the_cursor_skips_the_read_only_row() {
        let mut nav = Nav::default();
        nav.enter(Screen::Options);
        let key = |nav: &mut Nav, key| nav.key(key, false, false, 0);
        key(&mut nav, Key::Tab);
        key(&mut nav, Key::Tab);
        assert_eq!(nav.options_category(), Category::Video);
        // Display, Scale, V-Sync, Latency, Look, Aspect, [Picture size], Scanlines, CRT filter.
        for _ in 0..6 {
            key(&mut nav, Key::Down);
        }
        assert_eq!(nav.options_row(), 7, "Down from Aspect skips Picture size");
        key(&mut nav, Key::Up);
        assert_eq!(nav.options_row(), 5, "and Up from Scanlines skips it back");
        nav.open_video_confirm();
        assert_eq!(nav.options_video_confirm(), Some(1), "the cursor is on Revert");
        assert_eq!(key(&mut nav, Key::Down), Action::None, "no row moves meanwhile");
        assert_eq!(nav.options_row(), 5);
        assert_eq!(nav.options_video_confirm(), Some(0));
        assert_eq!(key(&mut nav, Key::Confirm), Action::OptionsVideo { keep: true });
        key(&mut nav, Key::Right);
        assert_eq!(key(&mut nav, Key::Confirm), Action::OptionsVideo { keep: false });
        assert_eq!(key(&mut nav, Key::Cancel), Action::OptionsVideo { keep: false }, "B reverts");
        assert_eq!(key(&mut nav, Key::ResetAll), Action::None);
        assert_eq!(nav.options_confirm(), None, "Reset all waits");
        nav.close_video_confirm();
        assert_eq!(nav.options_video_confirm(), None);
    }

    #[test]
    fn the_touch_page_is_a_list_under_the_device_row_and_its_button_opens_the_reset() {
        let mut nav = Nav::default();
        nav.enter(Screen::Options);
        let key = |nav: &mut Nav, key| nav.key(key, false, false, 0);
        for _ in 0..4 {
            key(&mut nav, Key::Tab);
        }
        nav.set_controls_page(controls::TOUCH_DEVICE);
        assert!(nav.touch_page());
        assert_eq!(nav.options_row(), 0, "the device row first");
        key(&mut nav, Key::Down);
        assert_eq!(nav.options_row(), controls::TOUCH_ROW0);
        assert_eq!(
            key(&mut nav, Key::Right),
            Action::OptionChange { category: Category::Controls, row: controls::TOUCH_ROW0, by: 1 }
        );
        // Up from the device row wraps to the last row, the reset button: A opens it, Left does nothing.
        key(&mut nav, Key::Up);
        key(&mut nav, Key::Up);
        assert_eq!(nav.options_row(), controls::TOUCH_ROW0 + controls::TOUCH_ROWS - 1);
        assert_eq!(key(&mut nav, Key::Left), Action::None);
        assert_eq!(key(&mut nav, Key::Confirm), Action::OptionsOpen(Opens::ResetTouchLayout));
        key(&mut nav, Key::Down);
        assert_eq!(nav.options_row(), 0, "the list wraps through the device row");
    }

    #[test]
    fn the_turbo_page_is_a_list_of_two_rows_under_the_device_row() {
        let mut nav = Nav::default();
        nav.enter(Screen::Options);
        let key = |nav: &mut Nav, key| nav.key(key, false, false, 0);
        for _ in 0..4 {
            key(&mut nav, Key::Tab);
        }
        nav.set_controls_page(controls::TURBO_DEVICE);
        assert_eq!(nav.list_page(), Some((controls::TURBO_ROW0, controls::TURBO_ROWS)));
        assert_eq!(nav.options_row(), 0, "the device row first");
        key(&mut nav, Key::Down);
        assert_eq!(nav.options_row(), controls::TURBO_ROW0);
        assert_eq!(
            key(&mut nav, Key::Right),
            Action::OptionChange { category: Category::Controls, row: controls::TURBO_ROW0, by: 1 }
        );
        key(&mut nav, Key::Down);
        assert_eq!(nav.options_row(), controls::TURBO_ROW0 + 1);
        assert_eq!(
            key(&mut nav, Key::Left),
            Action::OptionChange {
                category: Category::Controls,
                row: controls::TURBO_ROW0 + 1,
                by: -1
            }
        );
        key(&mut nav, Key::Down);
        assert_eq!(nav.options_row(), 0, "the list wraps through the device row");
    }

    #[test]
    fn the_controls_page_is_a_grid_two_to_a_line_and_a_waits_for_a_key() {
        let mut nav = Nav::default();
        nav.enter(Screen::Options);
        let key = |nav: &mut Nav, key| nav.key(key, false, false, 0);
        for _ in 0..4 {
            key(&mut nav, Key::Tab);
        }
        assert_eq!(nav.options_category(), Category::Controls);
        assert_eq!((nav.controls_position(), nav.options_row()), (0, 0), "the device row first");
        // Down enters the first line: Up (left) then Right to A (right), Down to Down (left).
        key(&mut nav, Key::Down);
        assert_eq!(nav.controls_action(), Some(0));
        assert_eq!(nav.options_row(), controls::row_of(Device::Keyboard, 0));
        assert_eq!(key(&mut nav, Key::Left), Action::None, "Left on the left box stays");
        key(&mut nav, Key::Right);
        assert_eq!(nav.controls_action(), Some(1), "A (confirm) is the box at the right");
        key(&mut nav, Key::Down);
        key(&mut nav, Key::Left);
        assert_eq!(nav.controls_action(), Some(2), "Down is under Up");
        key(&mut nav, Key::Up);
        key(&mut nav, Key::Up);
        assert_eq!(nav.controls_position(), 0, "Up from the first line is the device row");
        // The cursor follows the device: the gamepad's rows are the table's second block.
        nav.set_controls_device(Device::Gamepad);
        key(&mut nav, Key::Down);
        assert_eq!(nav.options_row(), controls::row_of(Device::Gamepad, 0));
        // Down to the last line and round: the shortcuts are the last two lines, then the device row.
        for _ in 0..7 {
            key(&mut nav, Key::Down);
        }
        assert_eq!(
            nav.controls_action(),
            Some(14),
            "Save state, second line of the shortcuts' three"
        );
        key(&mut nav, Key::Down);
        assert_eq!(nav.controls_action(), Some(16), "Screenshot is alone on the last line");
        assert_eq!(key(&mut nav, Key::Right), Action::None, "Right of the lone box stays");
        assert_eq!(nav.controls_action(), Some(16));
        key(&mut nav, Key::Down);
        assert_eq!(nav.controls_position(), 0, "the grid wraps through the device row");
        // Left and Right on the device row change the device; A on a key box waits.
        assert_eq!(
            key(&mut nav, Key::Right),
            Action::OptionChange { category: Category::Controls, row: 0, by: 1 }
        );
        key(&mut nav, Key::Down);
        assert_eq!(key(&mut nav, Key::Confirm), Action::None);
        assert!(nav.options_listening());
        assert_eq!(
            key(&mut nav, Key::Cancel),
            Action::None,
            "a navigation key does nothing meanwhile"
        );
        assert!(nav.options_listening());
        nav.refuse("Start");
        assert!(!nav.options_listening());
        assert_eq!(nav.options_refusal().map(|r| (r.position, r.by)), Some((1, "Start")));
        nav.tick();
        assert_eq!(nav.options_refusal().map(|r| r.flash), Some(REFUSAL_FLASH - 1));
        key(&mut nav, Key::Down);
        assert_eq!(nav.options_refusal(), None, "the next key forgets the refusal");
        assert_eq!(key(&mut nav, Key::Cancel), Action::Back);
        // L and R still change the page, and the device is kept.
        assert_eq!(key(&mut nav, Key::TabBack), Action::None);
        assert_eq!(nav.options_category(), Category::Audio);
        key(&mut nav, Key::Tab);
        assert_eq!(nav.controls_device(), Device::Gamepad);
    }

    #[test]
    fn reset_all_asks_first_with_the_cursor_on_no() {
        let mut nav = Nav::default();
        nav.enter(Screen::Options);
        let key = |nav: &mut Nav, key| nav.key(key, false, false, 0);
        assert_eq!(key(&mut nav, Key::ResetAll), Action::None);
        assert_eq!(nav.options_confirm(), Some(1), "No");
        assert_eq!(key(&mut nav, Key::Confirm), Action::None, "A on No closes the question");
        assert_eq!(nav.options_confirm(), None);
        key(&mut nav, Key::ResetAll);
        assert_eq!(key(&mut nav, Key::Cancel), Action::None, "B is No");
        assert_eq!(nav.options_confirm(), None);
        key(&mut nav, Key::ResetAll);
        assert_eq!(key(&mut nav, Key::Down), Action::None);
        assert_eq!(nav.options_confirm(), Some(0), "Down moves between the two");
        assert_eq!(nav.options_category(), Category::Difficulty, "the page does not change");
        key(&mut nav, Key::Left);
        assert_eq!(nav.options_confirm(), Some(1));
        key(&mut nav, Key::Right);
        assert_eq!(nav.options_confirm(), Some(0), "Yes");
        assert_eq!(key(&mut nav, Key::Confirm), Action::OptionsResetAll);
        assert_eq!(nav.options_confirm(), None);
    }

    #[test]
    fn the_cheats_opened_from_the_options_go_back_to_them() {
        let mut nav = Nav::default();
        nav.enter(Screen::Cheats);
        assert_eq!(nav.cheats_back(), Screen::Menu);
        nav.enter(Screen::Options);
        nav.enter_cheats_from_options();
        assert_eq!((nav.screen(), nav.cheats_back()), (Screen::Cheats, Screen::Options));
        nav.resume(Screen::Options);
        assert_eq!(nav.options_category(), Category::Difficulty);
        nav.enter(Screen::Cheats);
        assert_eq!(nav.cheats_back(), Screen::Menu, "opened from the Menu again");
    }
    #[test]
    fn the_cheats_screen_moves_its_rows_and_changes_the_row_under_the_cursor() {
        let mut nav = Nav::default();
        nav.open();
        nav.enter(Screen::Menu);
        for _ in 0..5 {
            nav.key(Key::Down, false, false, 0);
        }
        nav.key(Key::Confirm, false, false, 0);
        nav.enter(Screen::Cheats);
        assert_eq!((nav.cursor(), nav.cheats_row()), (0, Row::Damage));
        assert_eq!(
            nav.key(Key::Right, false, false, 0),
            Action::Adjust { row: Row::Damage, by: 1 }
        );
        assert_eq!(
            nav.key(Key::Left, false, false, 0),
            Action::Adjust { row: Row::Damage, by: -1 }
        );
        assert_eq!(
            nav.key(Key::Confirm, false, false, 0),
            Action::Adjust { row: Row::Damage, by: 1 },
            "A moves a choice on"
        );
        assert_eq!(nav.key(Key::Up, false, false, 0), Action::None);
        assert_eq!(nav.cheats_row(), Row::ResetAll, "up from the first row wraps to Reset all");
        assert_eq!(nav.key(Key::Confirm, false, false, 0), Action::ResetAll);
        assert_eq!(nav.key(Key::Left, false, false, 0), Action::None, "Reset all has no choices");
        assert_eq!(nav.key(Key::Down, false, false, 0), Action::None);
        assert_eq!(nav.cheats_row(), Row::Damage, "down from Reset all wraps to the first row");
        assert_eq!(nav.key(Key::Cancel, false, false, 0), Action::Back);
        nav.enter(Screen::Cheats);
        assert_eq!(nav.cursor(), 0, "a new visit starts at the first row");
    }

    #[test]
    fn the_bag_moves_by_one_and_by_a_column_within_its_stacks() {
        let rows: Vec<ItemRow> = (0..3).map(|i| stack(i, true, true)).collect();
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        assert_eq!(nav.key_bag(Key::Up, &rows), Action::None);
        assert_eq!(nav.cursor(), 0, "the first stack stays first");
        nav.key_bag(Key::Right, &rows);
        assert_eq!(nav.bag_cursor(), 2, "a column jump is clamped to the last stack");
        nav.key_bag(Key::Left, &rows);
        assert_eq!(nav.bag_cursor(), 0);
        let many: Vec<ItemRow> = (0..10).map(|i| stack(i, true, true)).collect();
        for _ in 0..9 {
            nav.key_bag(Key::Down, &many);
        }
        assert_eq!((nav.page(), nav.cursor()), (1, 1), "stack 9 is on the second page");
        assert_eq!(nav.key_bag(Key::Cancel, &many), Action::Back);
    }

    #[test]
    fn the_tab_key_cycles_and_puts_the_cursor_on_the_first_stack() {
        let rows: Vec<ItemRow> = (0..3).map(|i| stack(i, true, true)).collect();
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        nav.key_bag(Key::Down, &rows);
        nav.key_bag(Key::Tab, &rows);
        assert_eq!((nav.tab(), nav.bag_cursor()), (Tab::Allies, 0));
        nav.key_bag(Key::Tab, &rows);
        nav.key_bag(Key::Tab, &rows);
        assert_eq!(nav.tab(), Tab::All, "the tabs cycle");
    }

    #[test]
    fn a_use_is_one_unit_at_once() {
        let rows = [stack(4, true, true)];
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        nav.key_bag(Key::Confirm, &rows);
        assert_eq!(nav.sheet(), Sheet::Choose);
        assert_eq!(
            nav.key_bag(Key::Confirm, &rows),
            Action::Use { stack: 4 },
            "A use is the use itself: no fighter and no quantity is asked"
        );
        assert_eq!(nav.sheet(), Sheet::Browse);
    }

    #[test]
    fn a_drop_asks_yes_or_no_and_drops_one_unit() {
        let rows = [stack(3, false, true)];
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        nav.key_bag(Key::Confirm, &rows);
        assert_eq!(nav.choice(), 1, "an object not usable here starts on Drop");
        nav.key_bag(Key::Confirm, &rows);
        assert_eq!(nav.sheet(), Sheet::Confirm, "Drop one? asks first");
        assert_eq!(nav.choice(), 1, "No is the choice to start on");
        assert_eq!(nav.key_bag(Key::Confirm, &rows), Action::None, "No goes back");
        assert_eq!(nav.sheet(), Sheet::Choose, "to the Use or Drop choice, still on Drop");
        assert_eq!(nav.choice(), 1);
        nav.key_bag(Key::Confirm, &rows);
        nav.key_bag(Key::Up, &rows);
        assert_eq!(nav.key_bag(Key::Confirm, &rows), Action::Drop { stack: 3 });
        assert_eq!(nav.sheet(), Sheet::Browse, "the list is back");
    }

    #[test]
    fn b_on_the_drop_question_leaves_the_choice_on_drop() {
        let rows = [stack(3, false, true)];
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        nav.key_bag(Key::Confirm, &rows);
        nav.key_bag(Key::Confirm, &rows);
        nav.key_bag(Key::Up, &rows);
        assert_eq!(nav.choice(), 0, "Yes is toggled");
        assert_eq!(nav.key_bag(Key::Cancel, &rows), Action::None);
        assert_eq!(nav.sheet(), Sheet::Choose);
        assert_eq!(nav.choice(), 1, "the choice is Drop again: an unusable object is never used");
        assert_eq!(nav.key_bag(Key::Confirm, &rows), Action::None, "and A opens the question");
        assert_eq!(nav.sheet(), Sheet::Confirm);
    }

    #[test]
    fn an_empty_tab_answers_a_with_a_line_and_its_tabs_and_back_still_work() {
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        assert_eq!(nav.key_bag(Key::Confirm, &[]), Action::Notice(EMPTY_TAB));
        assert_eq!(nav.key_bag(Key::Tab, &[]), Action::None);
        assert_eq!(nav.tab(), Tab::Allies, "the tab changes on an empty list");
        assert_eq!(nav.key_bag(Key::Cancel, &[]), Action::Back);
    }

    #[test]
    fn a_pinned_stack_cannot_be_dropped_and_an_unusable_one_says_so() {
        let rows = [stack(9, false, false)];
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        assert_eq!(
            nav.key_bag(Key::Confirm, &rows),
            Action::Notice("This cannot be used or dropped here.")
        );
        let rows = [stack(9, true, false)];
        nav.key_bag(Key::Confirm, &rows);
        nav.key_bag(Key::Down, &rows);
        assert_eq!(nav.choice(), 0, "Drop is not offered: the choice stays on Use");
        assert_eq!(nav.key_bag(Key::Confirm, &rows), Action::Use { stack: 9 });
    }

    #[test]
    fn every_help_line_fits_the_help_window() {
        // The help window (560 px, 22 px of padding inside its frame) holds 31 characters of the 30 px text on one line.
        let all =
            MENU_ROWS.iter().chain(command_rows(true, false)).chain(command_rows(false, false));
        for entry in all {
            assert!(entry.help().chars().count() <= 31, "{}: {}", entry.label(), entry.help());
        }
    }

    #[test]
    fn the_item_spawner_moves_counts_and_asks_before_a_story_object() {
        let story = |object: u8| (33..=39).contains(&object);
        let mut nav = Nav::default();
        nav.enter(Screen::Cheats);
        while nav.cheats_row() != Row::ItemSpawner {
            nav.key(Key::Down, false, false, 0);
        }
        nav.key(Key::Confirm, false, false, 0);
        let at = |nav: &Nav| nav.spawner().unwrap();
        assert_eq!((at(&nav).object(), at(&nav).count()), (0, 1));
        nav.key_spawner(Key::Right, &story);
        nav.key_spawner(Key::Right, &story);
        assert_eq!(nav.key_spawner(Key::Confirm, &story), Action::Spawn { object: 0, count: 3 });
        nav.key_spawner(Key::Left, &story);
        nav.key_spawner(Key::Left, &story);
        nav.key_spawner(Key::Left, &story);
        assert_eq!(at(&nav).count(), SPAWN_MAX, "Left from 1 wraps to the most");
        nav.key_spawner(Key::ResetPage, &story);
        assert_eq!(at(&nav).count(), 1, "Y puts it back to 1");
        nav.key_spawner(Key::Up, &story);
        assert_eq!(at(&nav).object(), SPAWNER_OBJECTS - 1, "Up from the first wraps to the last");
        nav.key_spawner(Key::TabBack, &story);
        assert_eq!(at(&nav).object(), SPAWNER_OBJECTS - 1 - SPAWNER_PAGE, "L goes a page up");
        nav.key_spawner(Key::Tab, &story);
        nav.key_spawner(Key::Tab, &story);
        assert_eq!(at(&nav).object(), SPAWNER_OBJECTS - 1, "R stops at the last");
        // A Dragon Ball: one at a time, and A asks first with the cursor on Cancel.
        nav.key_spawner(Key::Right, &story);
        assert_eq!(at(&nav).count(), 1, "a story object stays at 1");
        assert_eq!(nav.key_spawner(Key::Confirm, &story), Action::None);
        assert_eq!(at(&nav).confirm(), Some(1));
        assert_eq!(nav.key_spawner(Key::Confirm, &story), Action::None, "Cancel adds nothing");
        assert_eq!(at(&nav).confirm(), None);
        nav.key_spawner(Key::Confirm, &story);
        nav.key_spawner(Key::Left, &story);
        assert_eq!(
            nav.key_spawner(Key::Confirm, &story),
            Action::Spawn { object: SPAWNER_OBJECTS - 1, count: 1 },
            "Add anyway adds one"
        );
        assert!(nav.spawner().is_some(), "the spawner stays open for the next object");
    }

    #[test]
    fn every_a_and_b_does_something_or_is_a_documented_no_op() {
        // The command window of the field, of a town and of the flying party: A chooses the row, B goes back.
        for (flying, town) in [(false, false), (false, true), (true, false)] {
            for row in 0..command_rows(flying, town).len() {
                let mut nav = Nav::default();
                nav.open();
                for _ in 0..row {
                    nav.key(Key::Down, flying, town, 0);
                }
                assert!(matches!(nav.key(Key::Confirm, flying, town, 0), Action::Choose(_)));
                assert_eq!(nav.key(Key::Cancel, flying, town, 0), Action::Back);
            }
        }
        // The Menu list: every row chooses, except Options, which is greyed out and says "Coming soon." in
        // its help line: the one documented no-op of A on the menus.
        for (row, entry) in MENU_ROWS.iter().enumerate() {
            let mut nav = Nav::default();
            nav.open();
            nav.enter(Screen::Menu);
            for _ in 0..row {
                nav.key(Key::Down, false, false, 0);
            }
            let action = nav.key(Key::Confirm, false, false, 0);
            if entry.enabled() {
                assert!(matches!(action, Action::Choose(_)), "{}", entry.label());
            } else {
                assert_eq!(action, Action::None, "Options is greyed out");
            }
            assert_eq!(nav.key(Key::Cancel, false, false, 0), Action::Back);
        }
        // The cheats screen: every row answers A (a choice, or Reset all; the item spawner opens its page, and B
        // there closes it), and B goes back to the Menu list.
        for row in 0..Row::ALL.len() {
            let mut nav = Nav::default();
            nav.enter(Screen::Cheats);
            for _ in 0..row {
                nav.key(Key::Down, false, false, 0);
            }
            if nav.cheats_row() == Row::ItemSpawner {
                assert_eq!(nav.key(Key::Confirm, false, false, 0), Action::None);
                assert!(nav.spawner().is_some(), "A opens the item spawner");
                assert_eq!(nav.key_spawner(Key::Cancel, &|_| false), Action::None);
                assert!(nav.spawner().is_none(), "B closes it");
                assert_eq!(nav.key(Key::Cancel, false, false, 0), Action::Back);
                continue;
            }
            assert_ne!(
                nav.key(Key::Confirm, false, false, 0),
                Action::None,
                "{:?}",
                nav.cheats_row()
            );
            assert_eq!(nav.key(Key::Cancel, false, false, 0), Action::Back);
        }
        // The Item screen, the list first: A on an empty tab says so, and on a stack it opens the sheet
        // or says why nothing happens; B goes back from the list and steps back one sheet at a time.
        let mut nav = Nav::default();
        nav.enter(Screen::Items);
        assert_eq!(nav.key_bag(Key::Confirm, &[]), Action::Notice(EMPTY_TAB));
        assert_eq!(nav.key_bag(Key::Cancel, &[]), Action::Back);
        let rows = [
            stack(1, true, true),
            stack(2, true, true),
            stack(3, false, true),
            stack(4, true, false),
            stack(5, false, false),
        ];
        for index in 0..rows.len() {
            let mut nav = Nav::default();
            nav.enter(Screen::Items);
            for _ in 0..index {
                nav.key_bag(Key::Down, &rows);
            }
            let row = rows[index];
            if !row.usable && !row.droppable {
                assert_eq!(
                    nav.key_bag(Key::Confirm, &rows),
                    Action::Notice("This cannot be used or dropped here.")
                );
                assert_eq!(nav.key_bag(Key::Cancel, &rows), Action::Back);
                continue;
            }
            assert_eq!(nav.key_bag(Key::Confirm, &rows), Action::None, "a stack opens its sheet");
            assert_eq!(nav.sheet(), Sheet::Choose);
            if row.usable {
                // Use: the use itself, at once.
                let mut use_nav = nav;
                assert_eq!(use_nav.key_bag(Key::Confirm, &rows), Action::Use { stack: row.id });
            }
            if row.droppable {
                // Drop: Yes or No, and No goes back to the choice, which still shows Drop.
                let mut drop_nav = nav;
                if row.usable {
                    drop_nav.key_bag(Key::Down, &rows);
                }
                assert_eq!(drop_nav.key_bag(Key::Confirm, &rows), Action::None);
                assert_eq!(drop_nav.sheet(), Sheet::Confirm);
                assert_eq!(drop_nav.key_bag(Key::Confirm, &rows), Action::None, "No goes back");
                assert_eq!(drop_nav.sheet(), Sheet::Choose);
            }
            // B on the choice goes back to the list, and B on the list goes back to the command window.
            assert_eq!(nav.key_bag(Key::Cancel, &rows), Action::None);
            assert_eq!(nav.sheet(), Sheet::Browse);
            assert_eq!(nav.key_bag(Key::Cancel, &rows), Action::Back);
        }
    }

    #[test]
    fn entering_a_list_starts_it_at_its_first_row() {
        let mut nav = Nav::default();
        nav.open();
        nav.key(Key::Down, false, false, 0);
        nav.key(Key::Down, false, false, 0);
        nav.enter(Screen::Items);
        assert_eq!(nav.cursor(), 0);
        nav.enter(Screen::Command);
        assert_eq!(nav.cursor(), 2, "the command window keeps its row");
    }

    #[test]
    fn the_data_packs_list_switches_packs_and_asks_before_a_chapter_or_the_main_game() {
        // Two packs (the second refused), a chapter session's Main game, an open chapter and a locked one.
        let lines = [
            ModsLine::Pack { refused: false },
            ModsLine::Pack { refused: true },
            ModsLine::Install,
            ModsLine::Main,
            ModsLine::Chapter { locked: false },
            ModsLine::Chapter { locked: true },
        ];
        let mut nav = Nav::default();
        nav.enter(Screen::Mods);
        assert_eq!(nav.cursor(), 0, "the first pack");
        assert_eq!(nav.key_mods(Key::Confirm, &lines), Action::PackToggle(0));
        nav.key_mods(Key::Down, &lines);
        assert_eq!(nav.key_mods(Key::Confirm, &lines), Action::Notice(REFUSED_PACK), "stays off");
        nav.key_mods(Key::Down, &lines);
        assert_eq!(nav.key_mods(Key::Confirm, &lines), Action::PacksInstall, "the file picker");
        nav.key_mods(Key::Down, &lines);
        assert_eq!(nav.mods_cursor(), 3, "the cursor crosses from the packs into the chapters");
        assert_eq!(nav.key_mods(Key::Confirm, &lines), Action::None);
        assert_eq!(
            nav.mods_ask(),
            Some((ModsAsk::Main, 1)),
            "Main game asks, the cursor on Cancel"
        );
        assert_eq!(nav.key_mods(Key::Left, &lines), Action::None);
        assert_eq!(nav.key_mods(Key::Confirm, &lines), Action::ChapterMain);
        assert_eq!(nav.mods_ask(), None, "the question is over");
        nav.key_mods(Key::Down, &lines);
        assert_eq!(nav.key_mods(Key::Confirm, &lines), Action::None);
        assert_eq!(nav.mods_ask(), Some((ModsAsk::Chapter(4), 1)), "the open chapter asks");
        assert_eq!(nav.key_mods(Key::Cancel, &lines), Action::None, "B closes the question only");
        assert_eq!(nav.mods_ask(), None);
        nav.key_mods(Key::Down, &lines);
        assert_eq!(nav.key_mods(Key::Confirm, &lines), Action::Notice(LOCKED_CHAPTER));
        nav.key_mods(Key::Down, &lines);
        assert_eq!(nav.mods_cursor(), 0, "down from the last line wraps to the first");
        assert_eq!(nav.key_mods(Key::Cancel, &lines), Action::Back);
    }

    #[test]
    fn apply_asks_first_with_the_cursor_on_cancel_and_applies_only_when_confirmed() {
        let lines = [ModsLine::Pack { refused: false }];
        let mut nav = Nav::default();
        nav.enter(Screen::Mods);
        assert_eq!(nav.key_mods(Key::Apply, &lines), Action::None);
        assert_eq!(nav.mods_ask(), Some((ModsAsk::Apply, 1)), "X asks, the cursor on Cancel");
        assert_eq!(nav.key_mods(Key::Confirm, &lines), Action::None, "Cancel closes the question");
        assert_eq!(nav.mods_ask(), None);
        nav.key_mods(Key::Apply, &lines);
        nav.key_mods(Key::Up, &lines);
        assert_eq!(nav.mods_ask(), Some((ModsAsk::Apply, 0)), "Up moves the choice to Apply");
        assert_eq!(nav.key_mods(Key::Confirm, &lines), Action::PacksApply);
        nav.key_mods(Key::Apply, &lines);
        assert_eq!(nav.key_mods(Key::Cancel, &lines), Action::None, "B closes it too");
        assert_eq!(nav.key_mods(Key::Cancel, &lines), Action::Back, "and the next B goes back");
    }

    #[test]
    fn with_y_held_up_and_down_move_a_pack_in_the_order_and_never_into_the_chapters() {
        let lines = [
            ModsLine::Pack { refused: false },
            ModsLine::Pack { refused: false },
            ModsLine::Install,
            ModsLine::Chapter { locked: false },
        ];
        let mut nav = Nav::default();
        nav.enter(Screen::Mods);
        nav.set_mods_moving(true);
        assert_eq!(nav.key_mods(Key::Down, &lines), Action::PackMove { at: 0, by: 1 });
        assert_eq!(nav.mods_cursor(), 1, "the cursor follows the pack");
        assert_eq!(nav.key_mods(Key::Down, &lines), Action::None, "the last pack stays");
        assert_eq!(nav.key_mods(Key::Up, &lines), Action::PackMove { at: 1, by: -1 });
        assert_eq!(nav.key_mods(Key::Up, &lines), Action::None, "the first pack stays");
        nav.set_mods_moving(false);
        assert_eq!(nav.key_mods(Key::Down, &lines), Action::None);
        assert_eq!(nav.mods_cursor(), 1, "without Y the cursor moves alone");
    }

    #[test]
    fn the_data_packs_button_of_the_options_opens_the_screen_and_back_keeps_the_options_row() {
        let mut nav = Nav::default();
        nav.enter(Screen::Options);
        nav.key(Key::Tab, false, false, 0);
        assert_eq!(nav.options_category(), Category::Gameplay);
        for _ in 0..5 {
            nav.key(Key::Down, false, false, 0);
        }
        assert_eq!(nav.options_row(), 5, "the Data packs button is the last row");
        assert_eq!(nav.key(Key::Confirm, false, false, 0), Action::OptionsOpen(Opens::Packs));
        nav.enter(Screen::Mods);
        assert_eq!(nav.mods_cursor(), 0, "the screen opens on its first line");
        assert_eq!(nav.key_mods(Key::Cancel, &[]), Action::Back);
        nav.resume(Screen::Options);
        assert_eq!(nav.options_row(), 5, "back on the Options, on the button");
    }

    #[test]
    fn the_cards_cursor_walks_the_hand_both_ways_and_wraps_and_only_b_does_anything_else() {
        let mut nav = Nav::default();
        nav.enter(Screen::Cards);
        assert_eq!(nav.cursor(), 0, "the hand opens on its first card");
        assert_eq!(nav.key(Key::Right, false, false, 0), Action::None);
        assert_eq!(nav.key(Key::Right, false, false, 0), Action::None);
        assert_eq!(nav.cursor(), 2);
        nav.key(Key::Left, false, false, 0);
        assert_eq!(nav.cursor(), 1);
        nav.key(Key::Left, false, false, 0);
        nav.key(Key::Left, false, false, 0);
        assert_eq!(nav.cursor(), 4, "Left from the first card is the last");
        nav.key(Key::Right, false, false, 0);
        assert_eq!(nav.cursor(), 0, "Right from the last card is the first");
        for key in [Key::Up, Key::Down, Key::Confirm, Key::Tab, Key::TabBack, Key::ResetPage] {
            assert_eq!(nav.key(key, false, false, 0), Action::None, "{key:?} does nothing here");
        }
        assert_eq!(nav.cursor(), 0);
        assert_eq!(nav.key(Key::Cancel, false, false, 0), Action::Back);
        // Opened again, the hand starts on its first card
        nav.key(Key::Right, false, false, 0);
        nav.enter(Screen::Cards);
        assert_eq!(nav.cursor(), 0);
    }
}
