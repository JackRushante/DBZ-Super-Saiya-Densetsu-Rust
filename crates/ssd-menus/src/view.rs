//! What an HD screen shows, as plain values: the menu state read from the game (`ssd-game`'s
//! `MenuProbe`) joined with the names, prices and help texts of the catalogue. The screens of
//! [`crate::screens`] draw it; nothing here reads the game or the ROM.

use crate::cheats::{Cheats, Row};
use crate::nav::ModsAsk;
use crate::options::Category;

/// The names and texts the view needs, by id: the characters (by character id) and the objects
/// (by object id, the item list's ids). Built from the catalogue by the application.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Labels {
    /// One entry per character id.
    pub characters: Vec<CharacterLabel>,
    /// One entry per object id.
    pub objects: Vec<ObjectLabel>,
}

/// A playable character.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CharacterLabel {
    pub name: String,
    /// The face index of the character's corner picture (the field menu's `face_portrait_pointers`), when it
    /// has one.
    pub face: Option<u8>,
}

/// The picture a portrait box shows: an object's (the item screen's picture box) or a corner face of the field
/// menu, by its face index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Picture {
    /// The picture of an object, by its id.
    Object(u8),
    /// The corner face of a face index.
    Face(u8),
}

/// A named object of the item list.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ObjectLabel {
    pub name: String,
    /// The price in Pt; `None` for an object that is not sold (its box shows no figure).
    pub price: Option<u8>,
    /// The first page of the help text, one string per line.
    pub help: Vec<String>,
    /// The attribute byte of the object (`item_attributes`): the flags below are its bits.
    pub attribute: u8,
    /// Usable in a battle (bit 7 of the attribute byte).
    pub battle: bool,
    /// Usable on the map (bit 6 of the attribute byte).
    pub field: bool,
    /// A story helper, never used from the menu (bit 5 of the attribute byte).
    pub story: bool,
    /// Cannot be dropped (bit 2 of the attribute byte).
    pub fixed: bool,
    /// Asks for a party member to use it on (bit 3 of the attribute byte).
    pub member: bool,
    /// Asks for a card of the hand (bit 4 of the attribute byte): the original's card screen does it.
    pub card: bool,
    /// The object is a person of the story (its name is a character's): listed under Allies.
    pub ally: bool,
}

/// Which HD screen the view is of.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Screen {
    /// The command window of the field (or of a town, or while flying).
    #[default]
    Command,
    /// The Menu list: Cards, Status, Order, Text, Save, Cheats, and Options greyed out.
    Menu,
    /// The bag: the HD Item screen.
    Items,
    /// The cheats: the choices of the run (M5).
    Cheats,
    /// The options: the categories and their rows (M7).
    Options,
    /// The data packs: the installed packs, the set the game runs with, and the chapters (M10).
    Mods,
}

/// The Data packs screen as it shows: the packs, "Install a pack...", then the chapters of the packs that are on (with "Main
/// game" first in a chapter session), the cursor, the question under way and the notice of the foot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModsView {
    pub packs: Vec<PackLine>,
    pub chapters: Vec<ChapterLine>,
    /// The line under the cursor: an index of the packs, then [`ModsView::install_line`] ("Install a pack..."), then the
    /// chapters.
    pub cursor: usize,
    /// Y is held: the pack under the cursor moves in the order.
    pub moving: bool,
    /// The question under way and its choice (0 the confirm button, 1 Cancel).
    pub ask: Option<(ModsAsk, u8)>,
    /// A line for the foot: why a change was refused, or why the set is not in use.
    pub notice: Option<String>,
}

impl ModsView {
    /// The line of "Install a pack...", after the packs.
    #[must_use]
    pub fn install_line(&self) -> usize {
        self.packs.len()
    }

    /// The chapter under the cursor, if the cursor is on one.
    #[must_use]
    pub fn chapter_at_cursor(&self) -> Option<&ChapterLine> {
        self.cursor.checked_sub(self.install_line() + 1).and_then(|at| self.chapters.get(at))
    }
}

/// A pack of the Data packs screen: its name and identity, its details and its state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackLine {
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    pub needs: String,
    /// The size in bytes.
    pub size: u64,
    pub state: PackState,
    /// Why the pack is refused (the reason shown in its details).
    pub reason: Option<String>,
}

/// Whether a pack of the list is on, off, or refused by its checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackState {
    On,
    Off,
    Refused,
}

/// A chapter, or "Main game" (`main`), of the Data packs screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChapterLine {
    pub name: String,
    pub description: String,
    /// Locked: its condition does not hold yet.
    pub locked: bool,
    pub main: bool,
}

/// The Cheats screen as it shows: the choices of the run, the row under the cursor, and the names of the
/// characters the "For" row offers (by character id).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheatsView {
    pub cheats: Cheats,
    /// The row under the cursor.
    pub row: Row,
    pub names: Vec<String>,
    /// The item spawner, while it is open.
    pub spawner: Option<SpawnerView>,
}

/// The item spawner as it shows: the page of objects, the one under the cursor with its help, how many to add, and the
/// story warning while it asks.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpawnerView {
    /// The rows of the page the cursor is on.
    pub rows: Vec<SpawnerRow>,
    /// The object under the cursor.
    pub object: u8,
    /// Its name and the first page of its help text.
    pub name: String,
    pub help: Vec<String>,
    /// Whether the story checks it (the warning, and one unit at a time).
    pub story: bool,
    /// How many units the next add puts in the bag.
    pub count: u8,
    /// The story warning's choice while it asks: 0 Add anyway, 1 Cancel.
    pub confirm: Option<u8>,
}

/// One object of the item spawner's list.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SpawnerRow {
    pub object: u8,
    pub name: String,
    pub story: bool,
    /// The units the bag holds.
    pub in_bag: u32,
}

/// The Options screen as it shows: the category and the row under the cursor, the value of each row of the
/// category (an index of its choices), and Reset all's question while it is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionsView {
    pub category: Category,
    /// The row under the cursor: an index of the category's rows.
    pub row: usize,
    /// The value of each row of the category.
    pub values: Vec<usize>,
    /// Reset all's question: the choice under the cursor, 0 Yes and 1 No.
    pub confirm: Option<u8>,
    /// "Apply video changes?" while it asks.
    pub video: Option<VideoAsk>,
    /// The Controls page's state while it waits for a key or shows a refusal (none on the other pages).
    pub controls: Option<ControlsView>,
}

/// The question "Apply video changes?" of the Video page: the choice under the cursor (0 Keep, 1 Revert), the
/// tenths of the ten seconds still left (100 down to 0) and what the video would be if kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoAsk {
    pub choice: u8,
    pub tenths: u32,
    pub summary: String,
}

/// The Controls page's moment: whether it waits for the next key or button, and the key it just refused.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ControlsView {
    /// The key box under the cursor waits for the next key or button.
    pub listening: bool,
    /// The last key was refused.
    pub refused: Option<RefusedKey>,
}

/// A key the Controls page refused: the place of its row in the grid (0 is the device row, 1 to 16 the actions),
/// the line of the help strip, and whether the flash is on this frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefusedKey {
    pub position: usize,
    pub message: String,
    pub blink: bool,
}

/// The tabs of the bag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    /// Every stack.
    #[default]
    All,
    /// The people of the story (Bulma, Roshi, Vegeta...).
    Allies,
    /// Everything else: the Dragon Balls, the Scouter, the Radar...
    Tools,
}

impl Tab {
    /// The label of the tab.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Allies => "Allies",
            Self::Tools => "Tools",
        }
    }

    /// Whether a stack of `ally` (a person or not) is listed under this tab.
    #[must_use]
    pub const fn shows(self, ally: bool) -> bool {
        match self {
            Self::All => true,
            Self::Allies => ally,
            Self::Tools => !ally,
        }
    }
}

/// Where the Item screen is: the bag's list, or one of the sheets that act on the stack under the
/// cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Sheet {
    /// The list of stacks.
    #[default]
    Browse,
    /// Use or Drop for the stack under the cursor.
    Choose,
    /// Drop one unit? Yes or No.
    Confirm,
}

/// One stack of the Item screen, as the screen shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackView {
    /// The stack's id in the bag.
    pub id: u32,
    pub object: u8,
    pub name: String,
    pub count: u16,
    /// Usable here (on the map) and not a story helper: grey otherwise.
    pub usable: bool,
    /// Can be dropped: not a key item and not fixed.
    pub droppable: bool,
    /// Listed under Allies.
    pub ally: bool,
}

/// The bag as the Item screen shows it: the stacks of the tab, the cursor and the sheet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BagView {
    pub tab: Tab,
    /// The stacks the tab lists, in bag order (key items first).
    pub stacks: Vec<StackView>,
    /// Stacks in use and the slots of the bag.
    pub used: usize,
    pub slots: usize,
    /// The bag has no free slot: a new object is refused.
    pub full: bool,
    /// The stack under the cursor can be used here, and can be dropped (the Choose sheet's rows).
    pub usable_here: bool,
    pub droppable_here: bool,
    /// The sheet over the list.
    pub sheet: Sheet,
    /// On the Choose sheet: 0 Use, 1 Drop. On Confirm: 0 Yes, 1 No.
    pub choice: u8,
    /// A line the screen shows at its foot: a refusal, a notice.
    pub notice: Option<&'static str>,
}

/// The menu as the HD screens show it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuView {
    pub screen: Screen,
    /// The bag, on the Item screen (the list of the original is not what the screen shows).
    pub bag: Option<BagView>,
    /// The town's commands: no Fly.
    pub town: bool,
    /// The flying commands: Land instead of Fly.
    pub flying: bool,
    /// The cursor: the command under it, or the row of the item list on the page (0 to 7).
    pub cursor: u8,
    /// The page of the item list, from 0.
    pub page: u8,
    /// The money.
    pub zeni: u16,
    /// The party, in party order.
    pub party: Vec<Member>,
    /// The item list, in order (at most 24 entries).
    pub items: Vec<Slot>,
    /// The object under the cursor on the item list, if there is one.
    pub selected: Option<Selected>,
}

/// A party member: a corner of the field menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub name: String,
    pub level: u8,
    pub hp: u16,
    pub max_hp: u16,
    pub ki: u16,
    pub max_ki: u16,
    /// The battle power (24 bits in the record).
    pub bp: u32,
    /// The object whose picture is the member's face.
    pub face: Option<u8>,
}

/// One entry of the item list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slot {
    /// The object id.
    pub object: u8,
    /// The attribute byte of the entry (bit 7 battle, bit 6 field).
    pub attribute: u8,
    pub name: String,
}

/// The object under the cursor of the item list: its picture, price and help.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selected {
    pub object: u8,
    pub name: String,
    pub price: Option<u8>,
    pub help: Vec<String>,
    pub battle: bool,
    pub field: bool,
}
