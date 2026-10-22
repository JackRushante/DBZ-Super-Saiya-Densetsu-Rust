//! The Data packs screen's model (M10, batch 5b): the installed packs and the set the run uses, the chapters the packs offer,
//! and the request the screen makes when the player applies the set or starts a chapter. Pure data: `ssd-app` reads the
//! packs and fills a [`Catalog`]; the menu driver keeps the list as the player changes it.

/// One installed pack, as the screen lists it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PackInfo {
    /// The pack's id, which `mods.toml` names; empty when its manifest cannot be read.
    pub id: String,
    /// The pack's name, or its file name when its manifest cannot be read.
    pub name: String,
    pub version: String,
    pub author: String,
    pub description: String,
    /// What the pack needs: "nothing", or the packs it depends on with their versions.
    pub needs: String,
    /// The pack's size in bytes.
    pub size: u64,
    /// Whether the pack is in the set the run uses.
    pub on: bool,
    /// Why the pack cannot be used (its checks refuse it): such a pack stays off.
    pub refusal: Option<String>,
    /// The chapters the pack offers, in the order of its file.
    pub chapters: Vec<ChapterInfo>,
}

/// One chapter a pack offers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ChapterInfo {
    /// The chapter's id, which `--chapter` names.
    pub id: String,
    pub name: String,
    pub description: String,
    /// Whether the chapter waits for the game to be completed (`needs_completed`).
    pub waits: bool,
}

/// What the Data packs screen lists, as the run found it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Catalog {
    /// Every installed pack: the packs of the run's set first, in the set's order, then the others.
    pub packs: Vec<PackInfo>,
    /// Whether the game was completed (the `completed` file of the saves): the chapters that wait are open.
    pub completed: bool,
    /// The name of the chapter this run plays (a chapter session), which the Chapters part answers with "Main game".
    pub playing: Option<String>,
    /// A line for the screen's foot: why the run's set is not in use, or why a change was refused.
    pub notice: Option<String>,
}

/// A pack of the list as the player leaves it: its place in [`Catalog::packs`] and whether it is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listed {
    pub pack: usize,
    pub on: bool,
}

/// What the screen asks the host to do once the player has chosen it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Apply the set: these packs' ids, in their load order. The game restarts to the title with them.
    Apply(Vec<String>),
    /// Start the chapter with this id: the game restarts into its own saves.
    Chapter(String),
    /// Back to the main game: the game restarts into the main saves.
    Main,
}
