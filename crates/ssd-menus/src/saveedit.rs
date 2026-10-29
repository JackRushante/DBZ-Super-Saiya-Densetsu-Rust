//! The save editor (title screen, Select): the party's levels and battle power and the bag of a save slot, edited in the
//! cartridge's battery save while no game runs. Pure Rust: the model and its rules; [`draw`] paints it and `ssd-app` wires it
//! to the machine's cartridge save and the sidecar files.
//!
//! # Battle power as a delta over the game's table
//!
//! The game's level-up *adds* the row of the new level to the member's BP (`03:851B`: the row's BP gain is added to the
//! record), so the BP a member has is the table's sum for its level plus whatever was added to it by other means. The editor
//! shows that difference as the member's delta over the game's table (`BP - table[level]`). A level change in the editor sets
//! the BP to `table[new level] + delta`, so a delta of +3000 follows the level, and since the original's level-up adds the same
//! rows, the delta stays on every later level-up of the game without any hook or extra file. A delta of 0 is the game's own.
//!
//! # Writing a slot
//!
//! [`Editor`] holds a copy of the battery save. A write builds the slot as the game's own save routine would
//! ([`ssd_save::SaveFile::set_slot`]: mark, checksum and backup copy) with the edited records and the bag projected into the
//! item list (the entries, their state words, the count and the row count); the result is a [`Commit`], which the host writes to
//! the machine's cartridge save and whose sidecar entries it re-keys to the new checksum word ([`Commit::rekey`]).
//!
//! # Levels past 99 (the level cap of `free_progression`)
//!
//! With a pack whose `level_cap` is set ([`Editor::set_progression`]) the editor sets the levels the game reaches with it, up to
//! the cap, on the same curves the game uses ([`crate::levels`], through [`Growth::stats_at`]). The record keeps its byte (at
//! most 99); the real level, the EXP's high byte and the BP's high part go to the slot's entry of `ssd.mods` ([`Wide`],
//! [`Commit::rekey`]). Without such a pack the editor is what it was: the record's byte, the game's caps and a BP of at most
//! [`BP_MAX`].

mod draw;

pub use draw::{scene, title_strip};

use crate::bag::{Bag, Class, Rules, Sidecar, Stack, story_object};
use crate::cheats::{CheatSidecar, Cheats};
use crate::levels::RECORD_LEVEL_MAX;
use crate::mods_state::{HIGH_BYTES, LEVEL_BYTES, ModState, ModsSidecar};
use crate::options::SlotOptions;
use crate::screens::{Summary, place_name};
use crate::view::{Growth, Labels, Tab};
use ssd_save::{CHECKSUM_OFFSET, Item, Record, SLOT_BASES, SLOT_COUNT, SaveFile, Slot, SlotStatus};

/// The most BP the editor sets: the seven digits the game's screens show.
pub const BP_MAX: u32 = 9_999_999;
/// The most BP the editor sets with the level cap (a wide BP, `docs/mods/state.md`): ten digits, the width of the BP at level
/// 999 (about 1.9 billion).
pub const BP_MAX_WIDE: u64 = 9_999_999_999;
/// The most a held key multiplies a level step by, with the level cap (the levels run to 999).
const LEVEL_FAST_MAX: i64 = 100;
/// The characters whose levels the level cap extends: those with a level table of the game's (Goku to Vegeta).
const EXTENDED_IDS: u8 = 8;
/// The most units a stack holds.
pub const STACK_MAX: u16 = 99;
/// The objects of the game (ids 0 to 39), as the picker shows them.
pub const OBJECTS: usize = 40;
/// Pictures in a row of the picker.
pub const GRID_COLUMNS: usize = 8;
/// The frames a refusal stays in the foot line: about two seconds of the game's sixty.
const NOTICE_FRAMES: u32 = 120;
/// Frames a direction is held before it repeats, and between two repeats.
const REPEAT_DELAY: u32 = 20;
const REPEAT_EVERY: u32 = 4;
/// Frames of holding after which BP steps by 10, 100 and 1000.
const BP_FAST: [u32; 3] = [60, 120, 180];

/// A key of the editor: the buttons of the SNES pad.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    X,
    Y,
    L,
    R,
    Start,
}

/// The four pages of the editor, walked with L and R.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Party,
    All,
    Allies,
    Tools,
}

impl Page {
    /// The label on the tab.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Party => "Party",
            Self::All => "All",
            Self::Allies => "Allies",
            Self::Tools => "Tools",
        }
    }

    /// The bag's tab of a bag page.
    #[must_use]
    pub const fn tab(self) -> Option<Tab> {
        match self {
            Self::Party => None,
            Self::All => Some(Tab::All),
            Self::Allies => Some(Tab::Allies),
            Self::Tools => Some(Tab::Tools),
        }
    }
}

/// The two rows of a member's edit panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row {
    Level,
    Bp,
}

/// What a slot of the picker holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Card {
    Empty,
    /// Marked but neither copy has a good checksum.
    Damaged,
    Game {
        summary: Summary,
        cheats: bool,
    },
}

/// What a member has beyond its record (the mod state, `docs/mods/state.md`): the real level when it is above 99 (0 otherwise),
/// the EXP's bits 24 to 31, and the BP's bits 24 to 39.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Wide {
    pub level: u16,
    pub exp_high: u8,
    pub bp_high: u16,
}

/// The level of a record with its wide level: the wide one while the record's byte agrees with it (`min(wide, 99) = byte`),
/// else the byte, as the game's own reader decides it.
#[must_use]
pub fn level_of(record: &Record, wide: &Wide) -> u16 {
    let cap = u16::from(RECORD_LEVEL_MAX);
    if wide.level > cap && wide.level.min(cap) == u16::from(record.level()) {
        wide.level
    } else {
        u16::from(record.level())
    }
}

/// A member of the party under edit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Member {
    pub id: u8,
    pub name: String,
    pub face: Option<u8>,
    /// The record as it is now, and as the slot was opened.
    pub record: Record,
    pub opened: Record,
    /// What the mod state adds to the record, as it is now and as the slot was opened (all zero without the level cap).
    pub wide: Wide,
    pub opened_wide: Wide,
    /// The position in the party (the wide BP's high part is kept by it).
    pub position: usize,
}

impl Member {
    /// Whether the level, the BP or the stats changed since the slot was opened.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.record != self.opened || self.wide != self.opened_wide
    }

    /// The real level, and the one the slot was opened with.
    #[must_use]
    pub fn level(&self) -> u16 {
        level_of(&self.record, &self.wide)
    }

    #[must_use]
    pub fn opened_level(&self) -> u16 {
        level_of(&self.opened, &self.opened_wide)
    }

    /// The BP with its high part, and the one the slot was opened with.
    #[must_use]
    pub fn bp(&self) -> u64 {
        u64::from(self.wide.bp_high) << 24 | u64::from(self.record.bp())
    }

    #[must_use]
    pub fn opened_bp(&self) -> u64 {
        u64::from(self.opened_wide.bp_high) << 24 | u64::from(self.opened.bp())
    }

    /// The EXP with its high byte, and the one the slot was opened with.
    #[must_use]
    pub fn exp(&self) -> u64 {
        u64::from(self.wide.exp_high) << 24 | u64::from(self.record.exp())
    }

    #[must_use]
    pub fn opened_exp(&self) -> u64 {
        u64::from(self.opened_wide.exp_high) << 24 | u64::from(self.opened.exp())
    }

    /// Puts a BP of up to 40 bits in the record (its low 24 bits) and in [`Wide::bp_high`].
    fn set_bp(&mut self, bp: u64) {
        self.record.set_bp((bp & 0xFF_FFFF) as u32);
        self.wide.bp_high = u16::try_from(bp >> 24).unwrap_or(u16::MAX);
    }
}

/// The question that asks before a story object is added or removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    Add { object: u8, count: u8 },
    Remove { stack: u32 },
}

/// Where the editor is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// The three slots.
    Slots,
    /// A slot's pages.
    Edit,
    /// The picker of objects.
    Add,
    /// The red question, with the choice (0 go on, 1 cancel).
    Ask(Ask, u8),
    /// "Write the changes?": the choice (0 Write, 1 Discard, 2 Keep editing).
    Write(u8),
}

/// The slot under edit.
#[derive(Debug, Clone)]
pub struct Session {
    pub slot: usize,
    /// The checksum word of the slot when it was opened.
    pub checksum: u16,
    data: Slot,
    pub members: Vec<Member>,
    pub bag: Bag,
    pub opened_bag: Bag,
    pub page: Page,
    pub member: usize,
    pub row: Row,
    /// The cursor of a bag page: an index of the stacks the page lists.
    pub cursor: usize,
    /// The picker's cursor (an object id) and how many it adds.
    pub pick: usize,
    pub how_many: u8,
}

/// The write the host makes: the new cartridge save and what the sidecar files must know about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The whole battery save after the write.
    pub sram: Vec<u8>,
    pub slot: u8,
    /// The slot's checksum word before and after.
    pub old_checksum: u16,
    pub new_checksum: u16,
    /// The bag the slot has now (`ssd.bag`).
    pub bag: Bag,
    /// The members whose BP or level was edited, as (position in the party, character id): the wide BP's high part of each is
    /// dropped, since the editor sets a BP of at most [`BP_MAX`].
    pub edited: Vec<(usize, u8)>,
    /// With the level cap, what the mod state keeps for each edited member (see [`Wide`]); empty without it.
    pub wide: Vec<WideEdit>,
}

/// The mod state of an edited member, written by [`Commit::rekey`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WideEdit {
    /// The position in the party and the character id.
    pub position: usize,
    pub id: u8,
    pub wide: Wide,
}

/// The four sidecar files of the save slots, as the host keeps them in memory.
pub struct Sidecars<'a> {
    pub bags: &'a mut Sidecar,
    pub cheats: &'a mut CheatSidecar,
    pub mods: &'a mut ModsSidecar,
    pub options: &'a mut SlotOptions,
}

impl Commit {
    /// Re-keys the slot's entries of the sidecar files from its old checksum word to the new one: the bag is the edited
    /// one, the cheats get the CHEATS badge (as the item spawner marks a save), the options and the mod state follow the slot
    /// (the mod state without the wide BP of the members that were edited). Without this the game would rebuild the bag from
    /// the list and start the slot with the cheats and options off.
    pub fn rekey(&self, sidecars: &mut Sidecars<'_>) {
        let (slot, old, new) = (self.slot, self.old_checksum, self.new_checksum);
        sidecars.bags.store(slot, new, &self.bag);
        let mut cheats: Cheats =
            sidecars.cheats.entry_for(slot, old).map_or_else(Cheats::default, |saved| saved.cheats);
        cheats.set_spawned(true);
        sidecars.cheats.store(slot, new, cheats);
        if let Some(values) = sidecars.options.entry_for(slot, old).cloned() {
            sidecars.options.store(slot, new, values);
        }
        let existing = sidecars.mods.entry_for(slot, old).cloned();
        let had_entry = existing.is_some();
        let mut state = existing.unwrap_or_default();
        self.rekey_mods(&mut state);
        if had_entry || !state.is_empty() {
            sidecars.mods.store(slot, new, state);
        }
    }

    /// The mod state of the slot after the write: the high parts of the edited members dropped (the snapshot's always, the party
    /// slot's replaced by the edit's own with the level cap), and with the level cap their wide level and EXP's high byte.
    fn rekey_mods(&self, state: &mut ModState) {
        if let Some(high) = state.high.as_mut().filter(|high| high.len() == HIGH_BYTES) {
            for &(position, id) in &self.edited {
                // The party slot's high part at +2i, the snapshot's at +20 + 2 * id.
                for at in [2 * position, 0x20 + 2 * usize::from(id)] {
                    if let Some(part) = high.get_mut(at..at + 2) {
                        part.fill(0);
                    }
                }
            }
        }
        for edit in &self.wide {
            let high = state.high.get_or_insert_with(|| vec![0; HIGH_BYTES]);
            if let Some(part) = high.get_mut(2 * edit.position..2 * edit.position + 2) {
                part.copy_from_slice(&edit.wide.bp_high.to_le_bytes());
            }
            let levels = state.levels.get_or_insert_with(|| vec![0; LEVEL_BYTES]);
            let (at, exp_at) = (2 * usize::from(edit.id), 2 * 12 + usize::from(edit.id));
            if let Some(part) = levels.get_mut(at..at + 2) {
                part.copy_from_slice(&edit.wide.level.to_le_bytes());
            }
            if let Some(byte) = levels.get_mut(exp_at) {
                *byte = edit.wide.exp_high;
            }
        }
        if state.high.as_ref().is_some_and(|high| high.iter().all(|&byte| byte == 0)) {
            state.high = None;
        }
        if state.levels.as_ref().is_some_and(|levels| levels.iter().all(|&byte| byte == 0)) {
            state.levels = None;
        }
    }
}

/// The save editor.
#[derive(Debug, Clone)]
pub struct Editor {
    labels: Labels,
    rules: Rules,
    bags: Sidecar,
    cheated: [bool; SLOT_COUNT],
    file: SaveFile,
    cards: [Card; SLOT_COUNT],
    stage: Stage,
    /// The slot under the picker's cursor.
    cursor: usize,
    session: Option<Session>,
    notice: Option<(String, u32)>,
    /// The line of the picker's help when a write just happened.
    written: Option<usize>,
    commit: Option<Commit>,
    leave: bool,
    /// Frames the current direction was held, and the direction (-1, 0, 1).
    held: (i8, u32),
    /// The pack's `level_cap` when the enabled packs have one (above the game's 48), and the mod state of the slots, which
    /// holds the wide levels the editor reads and writes with it.
    level_cap: Option<u16>,
    mods: ModsSidecar,
}

/// The checksum word of a slot of the cartridge save.
fn checksum_word(sram: &[u8], slot: usize) -> u16 {
    let at = SLOT_BASES[slot] + CHECKSUM_OFFSET;
    u16::from_le_bytes([sram[at], sram[at + 1]])
}

/// The name of the object, from the labels.
fn object_name(labels: &Labels, object: u8) -> String {
    labels
        .objects
        .get(usize::from(object))
        .map_or_else(|| format!("Object {object}"), |label| label.name.trim().to_owned())
}

/// The class of an object: its story bit comes from the labels.
fn class_of(labels: &Labels, object: u8) -> Class {
    Class::of(object, labels.objects.get(usize::from(object)).is_some_and(|label| label.story))
}

/// Whether the story checks the object (the red frame and the warning).
#[must_use]
pub fn story(labels: &Labels, object: u8) -> bool {
    story_object(labels.objects.get(usize::from(object)).is_some_and(|label| label.story), object)
}

impl Editor {
    /// Opens the editor on the battery save `sram` (the machine's), with the bags of `bags`, the cheats' badges of `cheated`
    /// (the slots whose cheats were saved with one on) and the names and tables of `labels`.
    ///
    /// # Errors
    ///
    /// `sram` is not a battery save.
    pub fn open(
        sram: &[u8],
        labels: &Labels,
        bags: &Sidecar,
        cheated: [bool; SLOT_COUNT],
    ) -> Result<Self, ssd_save::SaveError> {
        let file = SaveFile::from_bytes(sram)?;
        let mut editor = Self {
            labels: labels.clone(),
            rules: Rules::default(),
            bags: bags.clone(),
            cheated,
            cards: [Card::Empty, Card::Empty, Card::Empty],
            file,
            stage: Stage::Slots,
            cursor: 0,
            session: None,
            notice: None,
            written: None,
            commit: None,
            leave: false,
            held: (0, 0),
            level_cap: None,
            mods: ModsSidecar::empty(),
        };
        editor.refresh_cards();
        Ok(editor)
    }

    /// Tells the editor what the enabled packs give: `level_cap`, the cap of the pack with `free_progression`'s `level_cap` (read
    /// from the image the game runs, as the game reads it; `None` for no such pack: the editor is then the game's own), and
    /// `mods`, the mod state of the slots (`ssd.mods`), which holds the wide levels.
    pub fn set_progression(&mut self, level_cap: Option<u16>, mods: &ModsSidecar) {
        self.level_cap =
            level_cap.filter(|&cap| cap > 48).map(|cap| cap.min(crate::levels::LEVEL_MAX));
        self.mods = mods.clone();
        self.refresh_cards();
    }

    /// The level cap in force: the pack's.
    #[must_use]
    pub fn level_cap(&self) -> Option<u16> {
        self.level_cap
    }

    /// The most BP the editor sets: a wide BP with the level cap.
    #[must_use]
    pub fn bp_max(&self) -> u64 {
        if self.level_cap.is_some() { BP_MAX_WIDE } else { u64::from(BP_MAX) }
    }

    /// Whether the level cap extends the levels of character `id` past its game cap.
    #[must_use]
    pub fn extended(&self, id: u8) -> bool {
        self.level_cap.is_some()
            && id < EXTENDED_IDS
            && self.labels.growth.get(usize::from(id)).is_some_and(Growth::has_levels)
    }

    /// The highest level of character `id`: the game's cap, or the pack's when it extends it.
    #[must_use]
    pub fn cap_of(&self, id: u8) -> u16 {
        let game =
            self.labels.growth.get(usize::from(id)).map_or(0, |growth| u16::from(growth.cap));
        match self.level_cap {
            Some(cap) if self.extended(id) => cap.max(game),
            _ => game,
        }
    }

    /// What the mod state of `slot` adds to the record of character `id` at party position `position`, as the slot was written;
    /// all zero without the level cap.
    fn wide_of(&self, slot: usize, checksum: u16, position: usize, id: u8) -> Wide {
        if self.level_cap.is_none() {
            return Wide::default();
        }
        let Some(state) =
            u8::try_from(slot).ok().and_then(|slot| self.mods.entry_for(slot, checksum))
        else {
            return Wide::default();
        };
        let pair = |bytes: &[u8], at: usize| {
            bytes.get(at..at + 2).map_or(0, |part| u16::from_le_bytes([part[0], part[1]]))
        };
        let (level, exp_high) = match state.levels.as_deref() {
            Some(levels) if id < 12 => (
                pair(levels, 2 * usize::from(id)),
                levels.get(24 + usize::from(id)).copied().unwrap_or(0),
            ),
            _ => (0, 0),
        };
        let bp_high = state.high.as_deref().map_or(0, |high| pair(high, 2 * position));
        Wide { level, exp_high, bp_high }
    }

    /// The picker's three slots as the editor's copy of the save has them.
    fn refresh_cards(&mut self) {
        for index in 0..SLOT_COUNT {
            self.cards[index] = match self.file.status(index) {
                Ok(SlotStatus::Empty) | Err(_) => Card::Empty,
                Ok(SlotStatus::Corrupt) => Card::Damaged,
                Ok(SlotStatus::Good | SlotStatus::FromBackup) => match self.file.slot(index) {
                    Ok(slot) => Card::Game {
                        summary: self.summary_of(index, &slot),
                        cheats: self.cheated[index],
                    },
                    Err(_) => Card::Damaged,
                },
            };
        }
    }

    fn summary_of(&self, index: usize, slot: &Slot) -> Summary {
        let party = slot.party();
        let faces = party
            .iter()
            .map(|&id| self.labels.characters.get(usize::from(id)).and_then(|label| label.face))
            .collect();
        let level = party
            .first()
            .and_then(|&leader| slot.record(leader).ok().map(|record| (leader, record)))
            .map_or(0, |(leader, record)| {
                let checksum = checksum_word(self.file.as_bytes(), index);
                level_of(&record, &self.wide_of(index, checksum, 0, leader))
            });
        let units = self
            .bags
            .bag_for(index as u8, checksum_word(self.file.as_bytes(), index))
            .map(Bag::units);
        let items = units.map_or_else(|| slot.items().len(), |units| units as usize);
        Summary {
            faces,
            place: place_name(slot.map_id()),
            level,
            zeni: slot.zeni(),
            items: u16::try_from(items).unwrap_or(u16::MAX),
        }
    }

    /// Where the editor is.
    #[must_use]
    pub fn stage(&self) -> Stage {
        self.stage
    }

    /// The picker's slots.
    #[must_use]
    pub fn cards(&self) -> &[Card; SLOT_COUNT] {
        &self.cards
    }

    /// The slot under the picker's cursor.
    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The slot under edit.
    #[must_use]
    pub fn session(&self) -> Option<&Session> {
        self.session.as_ref()
    }

    /// The slot written last, for the picker's help line.
    #[must_use]
    pub fn written(&self) -> Option<usize> {
        self.written
    }

    /// The line of the foot, while a refusal or a notice shows.
    #[must_use]
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_ref().map(|(text, _)| text.as_str())
    }

    /// The labels the editor shows.
    #[must_use]
    pub fn labels(&self) -> &Labels {
        &self.labels
    }

    /// Whether the player left the editor (B on the picker): the host closes it.
    #[must_use]
    pub fn finished(&self) -> bool {
        self.leave
    }

    /// The write the last A on "Write" made, once: the host applies it.
    pub fn take_commit(&mut self) -> Option<Commit> {
        self.commit.take()
    }

    /// Whether a write waits for the host (the editor's copy of the save is ahead of the machine's).
    #[must_use]
    pub fn pending(&self) -> bool {
        self.commit.is_some()
    }

    /// The host's cartridge save at this frame: a slot under edit that is not what it was when it was opened (written by
    /// another way) cannot be written over.
    pub fn observe(&mut self, sram: &[u8]) {
        if self.commit.is_some() || sram.len() != self.file.as_bytes().len() {
            return;
        }
        let changed = (0..SLOT_COUNT).any(|slot| {
            let at = SLOT_BASES[slot]..SLOT_BASES[slot] + ssd_save::SLOT_LEN;
            sram[at.clone()] != self.file.as_bytes()[at]
        });
        if !changed {
            return;
        }
        let Ok(file) = SaveFile::from_bytes(sram) else { return };
        let slot = self.session.as_ref().map(|session| session.slot);
        self.file = file;
        self.refresh_cards();
        if let Some(slot) = slot {
            self.session = None;
            self.stage = Stage::Slots;
            self.say(format!("Slot {:02} changed. Open it again.", slot + 1));
        }
    }

    fn say(&mut self, text: impl Into<String>) {
        self.notice = Some((text.into(), NOTICE_FRAMES));
    }

    /// One frame: the refusal's time runs out.
    pub fn tick(&mut self) {
        if let Some((_, frames)) = self.notice.as_mut() {
            *frames = frames.saturating_sub(1);
            if *frames == 0 {
                self.notice = None;
            }
        }
    }

    /// The direction held this frame (-1 Left, 1 Right, 0 none): a held Left or Right repeats after a moment, and BP steps by
    /// 10, 100 and 1000 the longer it is held. The press itself came as a key.
    pub fn held(&mut self, direction: i8) {
        if direction == 0 || direction != self.held.0 {
            self.held = (direction, u32::from(direction != 0));
            return;
        }
        self.held.1 += 1;
        let frames = self.held.1;
        if frames >= REPEAT_DELAY && (frames - REPEAT_DELAY).is_multiple_of(REPEAT_EVERY) {
            let fast = BP_FAST.iter().filter(|&&at| frames >= at).count();
            let scale = 10_i64.pow(fast as u32);
            if self.stage == Stage::Add {
                self.how_many(direction);
            } else {
                self.step(i64::from(direction), scale);
            }
        }
    }

    /// A press of `key`.
    pub fn key(&mut self, key: Key) {
        match self.stage {
            Stage::Slots => self.key_slots(key),
            Stage::Edit => self.key_edit(key),
            Stage::Add => self.key_add(key),
            Stage::Ask(ask, choice) => self.key_ask(key, ask, choice),
            Stage::Write(choice) => self.key_write(key, choice),
        }
    }

    fn key_slots(&mut self, key: Key) {
        match key {
            Key::Up => self.cursor = (self.cursor + SLOT_COUNT - 1) % SLOT_COUNT,
            Key::Down => self.cursor = (self.cursor + 1) % SLOT_COUNT,
            Key::B | Key::Start => self.leave = true,
            Key::A => match self.cards[self.cursor] {
                Card::Empty => {}
                Card::Damaged => {
                    self.say(format!(
                        "Slot {:02} is damaged and cannot be edited.",
                        self.cursor + 1
                    ));
                }
                Card::Game { .. } => self.begin(self.cursor),
            },
            _ => {}
        }
    }

    /// Opens slot `index` for editing.
    fn begin(&mut self, index: usize) {
        let Ok(data) = self.file.slot(index) else {
            self.say(format!("Slot {:02} is damaged and cannot be edited.", index + 1));
            return;
        };
        let checksum = checksum_word(self.file.as_bytes(), index);
        let members: Vec<Member> = data
            .party()
            .into_iter()
            .enumerate()
            .filter_map(|(position, id)| {
                let record = data.record(id).ok()?;
                let label = self.labels.characters.get(usize::from(id));
                let wide = self.wide_of(index, checksum, position, id);
                Some(Member {
                    id,
                    name: label.map_or_else(|| format!("#{id}"), |label| label.name.clone()),
                    face: label.and_then(|label| label.face),
                    record,
                    opened: record,
                    wide,
                    opened_wide: wide,
                    position,
                })
            })
            .collect();
        let bag = self.bag_of(index, checksum, &data);
        self.session = Some(Session {
            slot: index,
            checksum,
            data,
            members,
            opened_bag: bag.clone(),
            bag,
            page: Page::Party,
            member: 0,
            row: Row::Level,
            cursor: 0,
            pick: 0,
            how_many: 1,
        });
        self.written = None;
        self.stage = Stage::Edit;
    }

    /// The bag of a slot: the sidecar's for its checksum word, else the stacks the item list makes (one unit an entry, as the
    /// game binds a save that has no bag file).
    fn bag_of(&self, index: usize, checksum: u16, data: &Slot) -> Bag {
        if let Some(mut bag) = self.bags.bag_for(index as u8, checksum).cloned() {
            bag.set_rules(self.rules);
            bag.coalesce();
            return bag;
        }
        let mut bag = Bag::new(self.rules);
        let words = data.item_words();
        for (entry, item) in data.items().iter().enumerate() {
            let _ = bag.add(item.id, words[entry], class_of(&self.labels, item.id));
        }
        bag
    }

    /// The stacks a page lists, as the Item screen orders them: the key items first, then the others in the order they were
    /// acquired.
    #[must_use]
    pub fn stacks(&self, page: Page) -> Vec<Stack> {
        let Some(session) = self.session.as_ref() else { return Vec::new() };
        let Some(tab) = page.tab() else { return Vec::new() };
        let shown = |stack: &&Stack| {
            tab.shows(self.labels.objects.get(usize::from(stack.object)).is_some_and(|l| l.ally))
        };
        let all = session.bag.stacks();
        all.iter()
            .filter(|stack| stack.class.key)
            .filter(shown)
            .chain(all.iter().filter(|stack| !stack.class.key).filter(shown))
            .copied()
            .collect()
    }

    /// Whether anything differs from the slot as it was opened.
    #[must_use]
    pub fn changed(&self) -> bool {
        self.session.as_ref().is_some_and(|session| {
            session.members.iter().any(Member::changed) || self.bag_changes(session) != (0, 0, 0)
        })
    }

    /// The stacks of the bag that changed, were added and were removed since the slot was opened.
    #[must_use]
    pub fn bag_changes(&self, session: &Session) -> (usize, usize, usize) {
        let now = session.bag.stacks();
        let before = session.opened_bag.stacks();
        let added = now.iter().filter(|s| session.opened_bag.stack(s.id).is_none()).count();
        let removed = before.iter().filter(|s| session.bag.stack(s.id).is_none()).count();
        let changed = now
            .iter()
            .filter(|s| session.opened_bag.stack(s.id).is_some_and(|old| old.count != s.count))
            .count();
        (changed, added, removed)
    }

    /// The count of changes the foot line shows: members changed and the stacks changed, added or removed.
    #[must_use]
    pub fn change_count(&self) -> usize {
        self.session.as_ref().map_or(0, |session| {
            let (changed, added, removed) = self.bag_changes(session);
            session.members.iter().filter(|m| m.changed()).count() + changed + added + removed
        })
    }

    fn key_edit(&mut self, key: Key) {
        match key {
            Key::Start => self.finish(),
            Key::B => {
                if self.changed() {
                    self.stage = Stage::Write(2);
                } else {
                    self.session = None;
                    self.stage = Stage::Slots;
                }
            }
            Key::L => self.walk(-1),
            Key::R => self.walk(1),
            Key::Left => self.step(-1, 1),
            Key::Right => self.step(1, 1),
            Key::Up | Key::Down => self.move_cursor(key == Key::Down),
            Key::Y => self.reset_or_remove(),
            Key::X => {
                if self.session.as_ref().is_some_and(|s| s.page != Page::Party) {
                    self.stage = Stage::Add;
                }
            }
            Key::A => {}
        }
    }

    /// Start: the write sheet when something changed, else back to the slots.
    fn finish(&mut self) {
        if self.changed() {
            self.stage = Stage::Write(2);
        } else {
            self.session = None;
            self.stage = Stage::Slots;
            self.say("Nothing changed.");
        }
    }

    /// L and R walk the four pages: the party's members, then All, Allies and Tools, and round again.
    fn walk(&mut self, direction: i32) {
        let Some(session) = self.session.as_mut() else { return };
        let members = session.members.len();
        let order = [Page::Party, Page::All, Page::Allies, Page::Tools];
        match (session.page, direction) {
            (Page::Party, 1) if session.member + 1 < members => session.member += 1,
            (Page::Party, -1) if session.member > 0 => session.member -= 1,
            (page, _) => {
                let at = order.iter().position(|&p| p == page).unwrap_or(0) as i32;
                let next = order[(at + direction).rem_euclid(4) as usize];
                session.page = next;
                session.cursor = 0;
                if next == Page::Party {
                    session.member = if direction > 0 { 0 } else { members.saturating_sub(1) };
                }
            }
        }
    }

    fn move_cursor(&mut self, down: bool) {
        let count = self.stacks(self.session.as_ref().map_or(Page::Party, |s| s.page)).len();
        let Some(session) = self.session.as_mut() else { return };
        if session.page == Page::Party {
            // The rows of the right panel.
            session.row = if down { Row::Bp } else { Row::Level };
        } else if count > 0 {
            session.cursor = if down {
                (session.cursor + 1) % count
            } else {
                (session.cursor + count - 1) % count
            };
        }
    }

    /// Left and Right: the level or the BP of the member, or the count of the stack under the cursor. `scale` is the
    /// multiplier a held key brings to the BP.
    fn step(&mut self, direction: i64, scale: i64) {
        if self.stage != Stage::Edit {
            return;
        }
        let Some(session) = self.session.as_ref() else { return };
        match (session.page, session.row) {
            // With the level cap a held key also strides the levels (the levels run to 999).
            (Page::Party, Row::Level) => {
                let stride = if self.level_cap.is_some() { scale.min(LEVEL_FAST_MAX) } else { 1 };
                self.change_level((direction * stride) as i32);
            }
            (Page::Party, Row::Bp) => self.change_bp(direction * scale),
            _ => self.step_count(direction),
        }
    }

    fn growth(&self, id: u8) -> Growth {
        self.labels.growth.get(usize::from(id)).cloned().unwrap_or_default()
    }

    /// Sets the level of the member under the cursor by `by` levels (clamped to the member's rows, with a refusal), setting
    /// the max HP and KI and the EXP from the level's row, filling HP and KI, and the BP to the table's value plus the delta.
    /// With the level cap the rows past the game's own are the extension's ([`Growth::stats_at`]).
    fn change_level(&mut self, by: i32) {
        let Some(session) = self.session.as_ref() else { return };
        let Some(member) = session.members.get(session.member) else { return };
        let growth = self.growth(member.id);
        let name = member.name.clone();
        if !growth.has_levels() {
            self.say(format!("{name} has no level table."));
            return;
        }
        let extended = self.extended(member.id);
        let (cap, min) = (i32::from(self.cap_of(member.id)), i32::from(growth.min));
        let level = i32::from(member.level());
        let target = level + by;
        if target > cap || target < min {
            let (limit, text) = if target > cap {
                (cap, format!("{name} stops at LV {cap}."))
            } else if min > 1 {
                (min, format!("{name} starts at LV {min}."))
            } else {
                (min, format!("{name} starts at LV 1."))
            };
            self.say(text);
            if level != limit {
                self.set_member_level(&growth, extended, limit);
            }
            return;
        }
        self.set_member_level(&growth, extended, target);
    }

    /// Puts the member under the cursor at `level`, keeping its delta over the table.
    fn set_member_level(&mut self, growth: &Growth, extended: bool, level: i32) {
        let wide_mode = self.level_cap.is_some();
        let bp_max = self.bp_max();
        let Some(session) = self.session.as_mut() else { return };
        let member = &mut session.members[session.member];
        let delta = delta_wide(growth, extended, member.level(), member.bp());
        if let Ok(level) = u16::try_from(level) {
            set_level(member, growth, extended, wide_mode, level, delta, bp_max);
        }
    }

    /// Moves the BP by `by` (never past 0 and [`Editor::bp_max`]).
    fn change_bp(&mut self, by: i64) {
        let max = self.bp_max();
        let Some(session) = self.session.as_mut() else { return };
        let Some(member) = session.members.get_mut(session.member) else { return };
        let now = i64::try_from(member.bp()).unwrap_or(i64::MAX);
        let wanted = now.saturating_add(by);
        let new = u64::try_from(wanted).unwrap_or(0).min(max);
        let text = if u64::try_from(wanted).is_ok_and(|wanted| wanted > max) {
            Some(format!("BP stops at {max}."))
        } else if wanted < 0 {
            Some("BP stops at 0.".to_owned())
        } else {
            None
        };
        member.set_bp(new);
        if let Some(text) = text {
            self.say(text);
        }
    }

    fn step_count(&mut self, direction: i64) {
        let page = self.session.as_ref().map_or(Page::Party, |s| s.page);
        let stacks = self.stacks(page);
        let cursor = self.session.as_ref().map_or(0, |s| s.cursor);
        let Some(stack) = stacks.get(cursor).copied() else { return };
        let name = object_name(&self.labels, stack.object);
        if direction > 0 {
            if !stack.class.stackable {
                self.say(format!("\"{name}\": one at a time."));
            } else if stack.count >= STACK_MAX {
                self.say(format!("{name}: {STACK_MAX} is the most a stack holds."));
            } else if let Some(session) = self.session.as_mut() {
                session.bag.set_count(stack.id, stack.count + 1);
            }
        } else if stack.count <= 1 {
            self.remove(stack);
        } else if let Some(session) = self.session.as_mut() {
            session.bag.set_count(stack.id, stack.count - 1);
        }
    }

    /// Takes a whole stack out: a story stack asks first.
    fn remove(&mut self, stack: Stack) {
        if story(&self.labels, stack.object) {
            self.stage = Stage::Ask(Ask::Remove { stack: stack.id }, 1);
            return;
        }
        self.drop_stack(stack.id);
    }

    fn drop_stack(&mut self, id: u32) {
        let count = self.stacks(self.session.as_ref().map_or(Page::Party, |s| s.page)).len();
        if let Some(session) = self.session.as_mut() {
            session.bag.set_count(id, 0);
            session.cursor = session.cursor.min(count.saturating_sub(2));
        }
    }

    /// Y: on the party page the BP goes back to the level's table value (delta 0); on a bag page the stack under the cursor is
    /// removed.
    fn reset_or_remove(&mut self) {
        let Some(session) = self.session.as_ref() else { return };
        if session.page == Page::Party {
            let Some(member) = session.members.get(session.member) else { return };
            let growth = self.growth(member.id);
            let Some(table) = growth.stats_at(member.level(), self.extended(member.id)) else {
                self.say("No table value for this level.");
                return;
            };
            let max = self.bp_max();
            if let Some(session) = self.session.as_mut() {
                let at = session.member;
                session.members[at].set_bp(table.bp.min(max));
            }
        } else {
            let stacks = self.stacks(session.page);
            if let Some(stack) = stacks.get(session.cursor).copied() {
                self.remove(stack);
            }
        }
    }

    fn key_add(&mut self, key: Key) {
        let Some(pick) = self.session.as_ref().map(|session| session.pick) else { return };
        let moved = |to: usize| Some(to % OBJECTS);
        match key {
            Key::B | Key::Start => self.stage = Stage::Edit,
            Key::Up => self.set_pick(moved(pick + OBJECTS - GRID_COLUMNS)),
            Key::Down => self.set_pick(moved(pick + GRID_COLUMNS)),
            Key::L => self.set_pick(moved(pick + OBJECTS - 1)),
            Key::R => self.set_pick(moved(pick + 1)),
            Key::Left => self.how_many(-1),
            Key::Right => self.how_many(1),
            Key::Y => {
                if let Some(session) = self.session.as_mut() {
                    session.how_many = 1;
                }
            }
            Key::A => self.add_picked(),
            Key::X => {}
        }
    }

    /// The picker's cursor goes to `to`; a key item adds one unit whatever the number said.
    fn set_pick(&mut self, to: Option<usize>) {
        let Some(to) = to else { return };
        let single = !self.labels_class(to as u8).stackable;
        if let Some(session) = self.session.as_mut() {
            session.pick = to;
            if single {
                session.how_many = 1;
            }
        }
    }

    /// Left and Right of the picker: how many units the next add puts in the bag (1 to 99; a key item one at a time).
    fn how_many(&mut self, direction: i8) {
        let Some(object) = self.session.as_ref().map(|session| session.pick as u8) else { return };
        if direction > 0 && !self.labels_class(object).stackable {
            let name = object_name(&self.labels, object);
            self.say(format!("\"{name}\": one at a time."));
            return;
        }
        if let Some(session) = self.session.as_mut() {
            session.how_many = if direction > 0 {
                (session.how_many + 1).min(STACK_MAX as u8)
            } else {
                session.how_many.saturating_sub(1).max(1)
            };
        }
    }

    fn labels_class(&self, object: u8) -> Class {
        class_of(&self.labels, object)
    }

    /// A on the picker: the object goes into the bag (a story object asks first).
    fn add_picked(&mut self) {
        let Some(session) = self.session.as_ref() else { return };
        let object = session.pick as u8;
        let count = session.how_many;
        let name = object_name(&self.labels, object);
        let class = self.labels_class(object);
        if !class.stackable && session.bag.units_of(object) > 0 {
            self.say(format!("\"{name}\": one at a time."));
            return;
        }
        if session.bag.is_full()
            && !session
                .bag
                .stacks()
                .iter()
                .any(|s| s.object == object && s.count < STACK_MAX && class.stackable)
        {
            self.say("Bag full: no slot left for a new object.");
            return;
        }
        if story(&self.labels, object) {
            self.stage = Stage::Ask(Ask::Add { object, count }, 1);
            return;
        }
        self.do_add(object, count);
    }

    fn do_add(&mut self, object: u8, count: u8) {
        let class = self.labels_class(object);
        let name = object_name(&self.labels, object);
        let Some(session) = self.session.as_mut() else { return };
        let mut added = 0;
        while added < count && session.bag.add(object, 0, class).is_ok() {
            added += 1;
        }
        if added < count {
            self.say(if added == 0 {
                "Bag full: no slot left for a new object.".to_owned()
            } else {
                format!("Bag full: {added} of {count} {name} added.")
            });
        }
    }

    fn key_ask(&mut self, key: Key, ask: Ask, choice: u8) {
        match key {
            Key::Left | Key::Right | Key::Up | Key::Down | Key::L | Key::R => {
                self.stage = Stage::Ask(ask, choice ^ 1);
            }
            Key::B => self.leave_ask(ask),
            Key::A => {
                if choice == 0 {
                    match ask {
                        Ask::Add { object, count } => self.do_add(object, count),
                        Ask::Remove { stack } => self.drop_stack(stack),
                    }
                }
                self.leave_ask(ask);
            }
            _ => {}
        }
    }

    /// Back from the red question: to the picker for an add, to the page for a removal.
    fn leave_ask(&mut self, ask: Ask) {
        self.stage = match ask {
            Ask::Add { .. } => Stage::Add,
            Ask::Remove { .. } => Stage::Edit,
        };
    }

    fn key_write(&mut self, key: Key, choice: u8) {
        match key {
            Key::Up | Key::L => self.stage = Stage::Write(choice.saturating_sub(1)),
            Key::Down | Key::R => self.stage = Stage::Write((choice + 1).min(2)),
            Key::B | Key::Start => self.stage = Stage::Edit,
            Key::A => match choice {
                0 => self.write(),
                1 => {
                    self.session = None;
                    self.stage = Stage::Slots;
                    self.say("Nothing was written.");
                }
                _ => self.stage = Stage::Edit,
            },
            _ => {}
        }
    }

    /// Builds the slot as the game's save routine would (see the module's notes) and makes the [`Commit`].
    fn write(&mut self) {
        let Some(session) = self.session.take() else { return };
        let index = session.slot;
        let mut slot = session.data.clone();
        let mut edited = Vec::new();
        let mut wide = Vec::new();
        for member in session.members.iter().filter(|m| m.changed()) {
            if slot.set_record(member.id, &member.record).is_err() {
                continue;
            }
            edited.push((member.position, member.id));
            if self.level_cap.is_some() {
                wide.push(WideEdit { position: member.position, id: member.id, wide: member.wide });
            }
        }
        let order = session.bag.project(None);
        let list: Vec<(Item, u16)> = order
            .into_iter()
            .filter_map(|id| session.bag.stack(id))
            .map(|stack| {
                let attribute =
                    self.labels.objects.get(usize::from(stack.object)).map_or(0, |l| l.attribute);
                (Item { id: stack.object, attr: attribute & !0x01 }, stack.extra)
            })
            .collect();
        if slot.set_item_list(&list).is_err() || self.file.set_slot(index, &slot).is_err() {
            self.say("The slot could not be written.");
            self.stage = Stage::Slots;
            return;
        }
        let new_checksum = checksum_word(self.file.as_bytes(), index);
        self.commit = Some(Commit {
            sram: self.file.as_bytes().to_vec(),
            slot: index as u8,
            old_checksum: session.checksum,
            new_checksum,
            bag: session.bag.clone(),
            edited,
            wide,
        });
        self.bags.store(index as u8, new_checksum, &session.bag);
        self.cheated[index] = true;
        self.refresh_cards();
        self.written = Some(index);
        self.stage = Stage::Slots;
    }
}

/// The BP a member has over the table's value for its level (0 when the table has no row for the level).
#[must_use]
pub fn delta_of(growth: &Growth, record: &Record) -> i64 {
    delta_wide(growth, false, u16::from(record.level()), u64::from(record.bp()))
}

/// The same for a level and a BP that may be past the record's fields: `extended` takes the rows past the game's cap from the
/// extension ([`Growth::stats_at`]).
#[must_use]
pub fn delta_wide(growth: &Growth, extended: bool, level: u16, bp: u64) -> i64 {
    growth.stats_at(level, extended).map_or(0, |table| {
        i64::try_from(bp).unwrap_or(i64::MAX).saturating_sub(i64::try_from(table.bp).unwrap_or(0))
    })
}

/// Puts the stats of `level` in the member as the game builds a record: the level (the record's byte saturates at 99 and, with
/// the level cap, the real level is in [`Wide::level`]), the max HP and KI from the row, HP and KI full, the EXP at the start
/// of the level (its bits 24 to 31 in [`Wide::exp_high`]) and byte `$0E` of the row; the BP is the table's value plus `delta`,
/// between 0 and `bp_max`. A level the table has no row for leaves the member as it is.
pub fn set_level(
    member: &mut Member,
    growth: &Growth,
    extended: bool,
    wide_mode: bool,
    level: u16,
    delta: i64,
    bp_max: u64,
) {
    let Some(row) = growth.stats_at(level, extended) else { return };
    let record = &mut member.record;
    record.set_level(u8::try_from(level.min(u16::from(RECORD_LEVEL_MAX))).unwrap_or(u8::MAX));
    if wide_mode {
        member.wide.level = if level > u16::from(RECORD_LEVEL_MAX) { level } else { 0 };
        member.wide.exp_high = u8::try_from(row.exp >> 24).unwrap_or(u8::MAX);
    }
    record.set_max_hp(row.max_hp);
    record.set_hp(row.max_hp);
    record.set_max_ki(row.max_ki);
    record.set_ki(row.max_ki);
    record.set_exp((row.exp & 0xFF_FFFF) as u32);
    record.set_byte_0e(row.byte_0e);
    let bp = i64::try_from(row.bp).unwrap_or(i64::MAX).saturating_add(delta);
    member.set_bp(u64::try_from(bp).unwrap_or(0).min(bp_max));
}

#[cfg(test)]
mod tests;
