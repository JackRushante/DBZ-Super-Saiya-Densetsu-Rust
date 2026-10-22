//! The chapters of a data pack (M10, Task 8): a chapter is a start the title can offer, with a party and its levels, the place,
//! the story flags, the money and the items the start has. A pack's `[[chapters]]` entry names a file, which holds the
//! `[[chapter]]` tables of this module's form:
//!
//! ```toml
//! [[chapter]]
//! id = "androids"                 # 1 to 32 of a-z, 0-9 and _; unique in the set (the chapter's save folder is named by it)
//! name = "The Androids"           # the name the title shows
//! description = "Goku's family."  # what the chapter is, a few lines; default empty
//! needs_completed = true          # waits for the game to be completed (see `unlocked`); default false
//! map = 64                        # 0 to 47 (an original map) or 64 to 127 (a pack map of the set); never 48 to 63 (towns)
//! leader = [1024, 768]            # the leader's position, in pixels of the map
//! follower = [896, 648]           # the follower's position
//! party = [{ character = 0, level = 30 }, { character = 1, level = 20 }]   # 1 to 9 members, each character once; level 1 to 100
//! flags = [3, 29]                 # story flags set at the start, 0 to 127; default none
//! flagx = [1]                     # extended flags set at the start, 0 to 2047; default none
//! items = [{ object = 5, count = 3 }]   # objects of the item list, each entry given `count` times; 24 entries at most; default none
//! zeni = 200                      # 0 to 999; default 0
//! ```
//!
//! [`synthesize`] builds the save slot a chapter session starts from: slot 1, written the way the game's save routine writes it
//! (the mark, the checksum and the backup, by `ssd_save::SaveFile::set_slot`). The party's records come from [`Tables`], which the
//! game's own tables give (the builder of `save-edit`, in `ssd_catalogue`), so a chapter's records are the ones `save-edit` makes for
//! the same party. The game's Continue loads that slot; the extended flags go to the `ssd.mods` sidecar of the slot.

use serde::Deserialize;
use ssd_save::{
    FLAG_BYTES, ITEM_SLOTS, Item, PARTY_MAX, Position, ROSTER_LEN, Record, SaveFile, Slot, ZENI_MAX,
};
use std::fmt;

/// The original maps a chapter may start on: `0` to `47`. `48` to `63` are the town screens, which are not rooms.
pub const ORIGINAL_MAPS: u8 = 48;
/// The last town screen (`48` to `63`).
const LAST_TOWN: u8 = 63;
/// The highest map id a chapter may name (the pack maps are `64` to `127`, see [`crate::map`]).
const LAST_MAP: u8 = 127;
/// The longest chapter id.
const MAX_ID_LEN: usize = 32;
/// The highest level a party member of a chapter starts at (the level cap the plan gives).
pub const MAX_LEVEL: u8 = 100;
/// The extended flags: 2048 of them, in [`FLAGX_BYTES`] bytes (flag `n` is bit `n % 8` of byte `n / 8`).
pub const FLAGX_COUNT: u16 = 2048;
/// The bytes of the extended flags of a slot's sidecar.
pub const FLAGX_BYTES: usize = FLAGX_COUNT as usize / 8;
/// The page offsets the synthesis writes beyond what [`ssd_save::Slot`] names: the map (`$01A0`) and the countdown byte
/// (`$01E4`, title-chapters.md, section 12).
const MAP_AT: usize = 0xA0;
const COUNTDOWN_AT: usize = 0xE4;

/// One chapter of a `chapters.toml`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chapter {
    /// The id: unique in the set, `[a-z0-9_]`, 1 to 32 characters.
    pub id: String,
    /// The name the title shows.
    pub name: String,
    /// What the chapter is, in a few lines (the loader screen shows it); empty when the file gives none.
    pub description: String,
    /// Whether the chapter waits for the game to be completed (see [`unlocked`]).
    pub needs_completed: bool,
    /// The map the chapter starts on.
    pub map: u8,
    /// The leader's position on the map, in pixels.
    pub leader: Position,
    /// The follower's position on the map, in pixels.
    pub follower: Position,
    /// The party, the leader first.
    pub party: Vec<Member>,
    /// The story flags set at the start (0 to 127).
    pub flags: Vec<u8>,
    /// The extended flags set at the start (0 to 2047).
    pub flagx: Vec<u16>,
    /// The entries of the item list at the start, in order.
    pub items: Vec<ItemCount>,
    /// The money at the start.
    pub zeni: u16,
}

/// A party member of a chapter: the character (0 to 11) and the level the member starts at (1 to 100).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Member {
    pub character: u8,
    pub level: u8,
}

/// An object of the item list and the number of entries of it the start gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemCount {
    pub object: u8,
    pub count: u8,
}

/// The game's own tables a chapter's slot is built from. The party's records are the game's builder's (the code `save-edit`
/// uses, in `ssd_catalogue`); the attribute byte of an object is the game's table. This crate reads no ROM itself.
pub trait Tables {
    /// The record of `character` at `level`, for a member who joins the party as `join_order` (1 for the first member).
    ///
    /// # Errors
    ///
    /// A text that names what the tables do not give: no such character, or a level outside its table.
    fn record(&self, character: u8, level: u8, join_order: u8) -> Result<Record, String>;

    /// The attribute byte the game keeps for the object `object` of the item list.
    ///
    /// # Errors
    ///
    /// A text that names the object, when the table has no such one.
    fn item_attribute(&self, object: u8) -> Result<u8, String>;
}

/// The slot a chapter starts from, and the chapter's extended flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Synthesized {
    /// The save, with the chapter's slot as slot 1 (index 0): the mark, the checksum and the backup are written.
    pub save: SaveFile,
    /// The extended flags of the chapter ([`FLAGX_BYTES`] bytes, flag `n` at bit `n % 8` of byte `n / 8`), for the `ssd.mods`
    /// sidecar of that slot; `None` when the chapter sets none.
    pub flagx: Option<Vec<u8>>,
}

/// Why a chapters file, or a chapter of a set, is refused. Each names the chapter by its number in the file or by its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChapterError {
    /// The file is not a chapters file: its TOML does not parse, or it has a key other than `chapter`.
    File(String),
    /// Chapter `index` (counted from 1) is refused for `problem`.
    Entry { index: usize, problem: String },
    /// The id `id` is given by two chapters of the set, in the packs `first` and `second`.
    Duplicate { id: String, first: String, second: String },
    /// Chapter `id` starts on `map`, which is neither an original map nor a pack map of the set.
    MissingMap { id: String, map: u8 },
    /// Chapter `id` cannot be built: `problem` is the tables' text or the slot's refusal.
    Synthesis { id: String, problem: String },
}

impl fmt::Display for ChapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(message) => write!(f, "the file is not a chapters file: {message}"),
            Self::Entry { index, problem } => write!(f, "chapter {index}: {problem}"),
            Self::Duplicate { id, first, second } => write!(
                f,
                "the chapter `{id}` is given by the packs `{first}` and `{second}`: the ids of a set are unique"
            ),
            Self::MissingMap { id, map } => write!(
                f,
                "chapter `{id}`: map {map} is neither an original map (0 to 47) nor a pack map of the set"
            ),
            Self::Synthesis { id, problem } => write!(f, "chapter `{id}`: {problem}"),
        }
    }
}

impl std::error::Error for ChapterError {}

/// A `[[chapter]]` table as it is written. Every key is known; each number is checked by [`parse`] afterwards, so the
/// range is named in the error.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawChapter {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    needs_completed: bool,
    map: i64,
    leader: Vec<i64>,
    follower: Vec<i64>,
    party: Vec<RawMember>,
    #[serde(default)]
    flags: Vec<i64>,
    #[serde(default)]
    flagx: Vec<i64>,
    #[serde(default)]
    items: Vec<RawItem>,
    #[serde(default)]
    zeni: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMember {
    character: i64,
    level: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawItem {
    object: i64,
    count: i64,
}

/// The chapters of a chapters file, in the order the file gives them.
///
/// # Errors
///
/// [`ChapterError::File`] if the text is not TOML or holds a key other than `chapter`, and [`ChapterError::Entry`] for the
/// first chapter that is refused: a number out of its range, an id that is not valid or given twice in the file, a town map,
/// a party that is empty or too large or repeats a character, and so on.
pub fn parse(text: &str) -> Result<Vec<Chapter>, ChapterError> {
    let table: toml::Table =
        toml::from_str(text).map_err(|error| ChapterError::File(error.to_string()))?;
    if let Some(key) = table.keys().find(|key| key.as_str() != "chapter") {
        return Err(ChapterError::File(format!(
            "the key `{key}` is not `chapter`: a chapters file holds `[[chapter]]` tables"
        )));
    }
    let Some(value) = table.get("chapter") else { return Ok(Vec::new()) };
    let toml::Value::Array(items) = value else {
        return Err(ChapterError::File("`chapter` is not a list of `[[chapter]]` tables".into()));
    };
    let chapters = items
        .iter()
        .enumerate()
        .map(|(at, item)| parse_chapter(at + 1, item))
        .collect::<Result<Vec<_>, _>>()?;
    for (at, chapter) in chapters.iter().enumerate() {
        if chapters[..at].iter().any(|earlier| earlier.id == chapter.id) {
            return Err(ChapterError::Entry {
                index: at + 1,
                problem: format!("the id `{}` is given twice in this file", chapter.id),
            });
        }
    }
    Ok(chapters)
}

/// Chapter `index` (counted from 1), from its table.
fn parse_chapter(index: usize, value: &toml::Value) -> Result<Chapter, ChapterError> {
    let refused = |problem: String| ChapterError::Entry { index, problem };
    let raw: RawChapter =
        value.clone().try_into().map_err(|error: toml::de::Error| refused(error.to_string()))?;
    if !valid_id(&raw.id) {
        return Err(refused(format!(
            "the id `{}` is not 1 to {MAX_ID_LEN} of a-z, 0-9 and _",
            raw.id
        )));
    }
    let name_of_chapter = raw.id.clone();
    let fail = |problem: String| ChapterError::Entry {
        index,
        problem: format!("chapter `{name_of_chapter}`: {problem}"),
    };
    if raw.name.trim().is_empty() {
        return Err(fail("the name is empty".into()));
    }
    if (i64::from(ORIGINAL_MAPS)..=i64::from(LAST_TOWN)).contains(&raw.map) {
        return Err(fail(format!(
            "map {} is a town screen: a chapter starts on an original map (0 to 47) or a pack map (64 to {LAST_MAP})",
            raw.map
        )));
    }
    let map = bounded("map", raw.map, 0, i64::from(LAST_MAP)).map_err(fail)? as u8;
    let leader = position("leader", &raw.leader).map_err(fail)?;
    let follower = position("follower", &raw.follower).map_err(fail)?;
    let mut party: Vec<Member> = Vec::with_capacity(raw.party.len());
    if raw.party.is_empty() || raw.party.len() > PARTY_MAX {
        return Err(fail(format!(
            "the party has {} members: it takes 1 to {PARTY_MAX}",
            raw.party.len()
        )));
    }
    for member in &raw.party {
        let character =
            bounded("character", member.character, 0, ROSTER_LEN as i64 - 1).map_err(fail)? as u8;
        let level = bounded("level", member.level, 1, i64::from(MAX_LEVEL)).map_err(fail)? as u8;
        if party.iter().any(|earlier| earlier.character == character) {
            return Err(fail(format!("character {character} is in the party twice")));
        }
        party.push(Member { character, level });
    }
    let flags = raw
        .flags
        .iter()
        .map(|&flag| bounded("flag", flag, 0, FLAG_BYTES as i64 * 8 - 1).map(|flag| flag as u8))
        .collect::<Result<Vec<_>, _>>()
        .map_err(fail)?;
    let flagx = raw
        .flagx
        .iter()
        .map(|&flag| {
            bounded("extended flag", flag, 0, i64::from(FLAGX_COUNT) - 1).map(|flag| flag as u16)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(fail)?;
    let mut items = Vec::with_capacity(raw.items.len());
    for item in &raw.items {
        let object = bounded("object", item.object, 0, 255).map_err(fail)? as u8;
        let count = bounded("count", item.count, 1, ITEM_SLOTS as i64).map_err(fail)? as u8;
        items.push(ItemCount { object, count });
    }
    let entries: usize = items.iter().map(|item| usize::from(item.count)).sum();
    if entries > ITEM_SLOTS {
        return Err(fail(format!("the items are {entries} entries: the list holds {ITEM_SLOTS}")));
    }
    let zeni = bounded("zeni", raw.zeni, 0, i64::from(ZENI_MAX)).map_err(fail)? as u16;
    Ok(Chapter {
        id: raw.id,
        name: raw.name,
        description: raw.description,
        needs_completed: raw.needs_completed,
        map,
        leader,
        follower,
        party,
        flags,
        flagx,
        items,
        zeni,
    })
}

/// Whether `id` is a chapter id: 1 to 32 characters, each a lowercase letter, a digit or `_`.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    (1..=MAX_ID_LEN).contains(&id.len())
        && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// `value` when it is from `low` to `high`, else a text that names `what` and the range.
fn bounded(what: &str, value: i64, low: i64, high: i64) -> Result<i64, String> {
    if (low..=high).contains(&value) {
        Ok(value)
    } else {
        Err(format!("{what} is {value}, it must be {low} to {high}"))
    }
}

/// A position: exactly two numbers, each from 0 to 65535.
fn position(what: &str, values: &[i64]) -> Result<Position, String> {
    match values {
        &[x, y] => Ok(Position {
            x: bounded(what, x, 0, 65535)? as u16,
            y: bounded(what, y, 0, 65535)? as u16,
        }),
        _ => Err(format!("{what} takes x and y, two numbers from 0 to 65535")),
    }
}

/// The refusals of the chapters of a set: an id that two chapters of the set give, and a map that is neither an original map
/// nor a pack map of the set. `chapters` are the chapters with the id of their pack and the file they are in, `maps` the ids of
/// the set's `[[map]]` entries. Each refusal names the pack and the file of the chapter it is about.
#[must_use]
pub fn set_refusals(
    chapters: &[(String, String, Chapter)],
    maps: &[u8],
) -> Vec<(String, String, ChapterError)> {
    let mut refusals = Vec::new();
    for (at, (pack, file, chapter)) in chapters.iter().enumerate() {
        if let Some((first, _, _)) =
            chapters[..at].iter().find(|(_, _, earlier)| earlier.id == chapter.id)
        {
            refusals.push((
                pack.clone(),
                file.clone(),
                ChapterError::Duplicate {
                    id: chapter.id.clone(),
                    first: first.clone(),
                    second: pack.clone(),
                },
            ));
        }
        if chapter.map >= ORIGINAL_MAPS && !maps.contains(&chapter.map) {
            refusals.push((
                pack.clone(),
                file.clone(),
                ChapterError::MissingMap { id: chapter.id.clone(), map: chapter.map },
            ));
        }
    }
    refusals
}

/// Whether a chapter is offered: it does not wait for the game's completion, or the game was completed (`completed`: the
/// `completed` file of the saves exists), or a cheat unlocks it (`cheat`; the cheat row is the title's, Task 11).
#[must_use]
pub fn unlocked(chapter: &Chapter, completed: bool, cheat: bool) -> bool {
    !chapter.needs_completed || completed || cheat
}

/// The byte at `$01E4` the set-up of a map gives (title-chapters.md, section 6, item 4): a countdown for a few original maps,
/// and 0 for every other map and for a pack map. The values are the research's, not re-read in this batch.
fn countdown(map: u8) -> u8 {
    match map {
        0 | 5 | 11 => 0x0A,
        9 => 0x09,
        16 | 22 => 0x0C,
        24 => 0x0F,
        _ => 0,
    }
}

/// The slot a chapter starts from (see [`Synthesized`]): the party's records from `tables` (the game's builder), the party list
/// and its size, the map and the positions, the story flags, the money, the items and the countdown byte.
///
/// # Errors
///
/// [`ChapterError::Synthesis`], naming the chapter: a record or an attribute the tables do not give, or a value the slot refuses.
pub fn synthesize(tables: &dyn Tables, chapter: &Chapter) -> Result<Synthesized, ChapterError> {
    let fail = |problem: String| ChapterError::Synthesis { id: chapter.id.clone(), problem };
    let mut slot = Slot::empty();
    let mut ids = Vec::with_capacity(chapter.party.len());
    for (index, member) in chapter.party.iter().enumerate() {
        let join_order = u8::try_from(index + 1).unwrap_or(u8::MAX);
        let record = tables.record(member.character, member.level, join_order).map_err(fail)?;
        slot.set_record(member.character, &record).map_err(|error| fail(error.to_string()))?;
        ids.push(member.character);
    }
    slot.set_party(&ids).map_err(|error| fail(error.to_string()))?;
    slot.set_page_byte(MAP_AT, chapter.map);
    slot.set_position(chapter.leader, chapter.follower);
    for &flag in &chapter.flags {
        slot.set_flag(usize::from(flag), true).map_err(|error| fail(error.to_string()))?;
    }
    slot.set_zeni(chapter.zeni).map_err(|error| fail(error.to_string()))?;
    let mut items = Vec::new();
    for entry in &chapter.items {
        let attr = tables.item_attribute(entry.object).map_err(fail)?;
        items
            .extend(std::iter::repeat_n(Item { id: entry.object, attr }, usize::from(entry.count)));
    }
    slot.set_items(&items).map_err(|error| fail(error.to_string()))?;
    slot.set_page_byte(COUNTDOWN_AT, countdown(chapter.map));
    let mut save = SaveFile::blank();
    save.set_slot(0, &slot).map_err(|error| fail(error.to_string()))?;
    let flagx = (!chapter.flagx.is_empty()).then(|| {
        let mut bytes = vec![0_u8; FLAGX_BYTES];
        for &flag in &chapter.flagx {
            bytes[usize::from(flag) / 8] |= 1 << (flag % 8);
        }
        bytes
    });
    Ok(Synthesized { save, flagx })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_save::{MARK, MARK_OFFSET, SlotStatus};

    /// The example of the module documentation, with a second chapter that has the optional keys left out.
    const EXAMPLE: &str = r#"
[[chapter]]
id = "androids"
name = "The Androids"
needs_completed = true
map = 64
leader = [1024, 768]
follower = [896, 648]
party = [{ character = 0, level = 30 }, { character = 1, level = 20 }]
flags = [3, 29]
flagx = [1]
items = [{ object = 5, count = 3 }]
zeni = 200

[[chapter]]
id = "plain_start"
name = "Plain"
map = 0
leader = [1024, 768]
follower = [896, 648]
party = [{ character = 2, level = 1 }]
"#;

    /// Records and attribute bytes of the tests: a record at a level carries the character and the level, the attribute is the
    /// object plus `$40`, and a character or an object the table lacks is an error (the game's tables are not in the tests).
    struct FakeTables;

    impl Tables for FakeTables {
        fn record(&self, character: u8, level: u8, join_order: u8) -> Result<Record, String> {
            if character > 2 || level > 48 {
                return Err(format!("character {character} at level {level} is not in the table"));
            }
            let mut record = Record::blank();
            record.set_flags(0xC0);
            record.set_character_id(character);
            record.set_level(level);
            record.set_hp(u16::from(level) * 10);
            record.set_max_hp(u16::from(level) * 10);
            record.set_byte_0e(join_order);
            Ok(record)
        }

        fn item_attribute(&self, object: u8) -> Result<u8, String> {
            if object == 0xFF {
                return Err("object 255 is not in the attribute table".into());
            }
            Ok(object.wrapping_add(0x40))
        }
    }

    fn refusal(text: &str) -> String {
        match parse(text) {
            Err(error) => error.to_string(),
            Ok(chapters) => panic!("accepted {chapters:?}"),
        }
    }

    /// One `[[chapter]]` table: the id, the name, the map, the leader, the party, and `extra` lines.
    fn table(id: &str, name: &str, map: &str, leader: &str, party: &str, extra: &str) -> String {
        format!(
            "[[chapter]]\nid = \"{id}\"\nname = \"{name}\"\nmap = {map}\nleader = {leader}\nfollower = [1, 2]\nparty = {party}\n{extra}\n"
        )
    }

    #[test]
    fn the_example_parses_with_its_values_and_its_defaults() {
        let chapters = parse(EXAMPLE).unwrap();
        assert_eq!(chapters.len(), 2);
        let androids = &chapters[0];
        assert_eq!(androids.id, "androids");
        assert!(androids.needs_completed);
        assert_eq!(androids.map, 64);
        assert_eq!(androids.leader, Position { x: 1024, y: 768 });
        assert_eq!(androids.follower, Position { x: 896, y: 648 });
        assert_eq!(
            androids.party,
            [Member { character: 0, level: 30 }, Member { character: 1, level: 20 }]
        );
        assert_eq!(androids.flags, [3, 29]);
        assert_eq!(androids.flagx, [1]);
        assert_eq!(androids.items, [ItemCount { object: 5, count: 3 }]);
        assert_eq!(androids.zeni, 200);
        let plain = &chapters[1];
        assert!(!plain.needs_completed, "the default is false");
        assert!(plain.flags.is_empty() && plain.flagx.is_empty() && plain.items.is_empty());
        assert_eq!(plain.zeni, 0);
        assert!(parse("").unwrap().is_empty(), "a file with no chapter holds none");
    }

    #[test]
    fn every_range_and_rule_is_checked_and_the_refusal_names_the_chapter() {
        let one = [(0, 1)];
        let party = |members: &[(u8, u8)]| {
            let items: Vec<String> = members
                .iter()
                .map(|(c, l)| format!("{{ character = {c}, level = {l} }}"))
                .collect();
            format!("[{}]", items.join(", "))
        };
        let good_party = party(&one);
        let cases = [
            (
                table("Bad Id", "C", "64", "[1, 2]", &good_party, ""),
                "the id `Bad Id` is not 1 to 32",
            ),
            (
                table("c", "C", "64", "[1, 2]", &good_party, "zeni = 1000"),
                "zeni is 1000, it must be 0 to 999",
            ),
            (table("c", "C", "64", "[1, 2]", "[]", ""), "the party has 0 members: it takes 1 to 9"),
            (
                table("c", "C", "64", "[1, 2]", &party(&[(12, 1)]), ""),
                "character is 12, it must be 0 to 11",
            ),
            (
                table("c", "C", "64", "[1, 2]", &party(&[(0, 0)]), ""),
                "level is 0, it must be 1 to 100",
            ),
            (
                table("c", "C", "64", "[1, 2]", &party(&[(0, 1), (0, 2)]), ""),
                "character 0 is in the party twice",
            ),
            (
                table("c", "C", "64", "[1, 2]", &good_party, "flags = [128]"),
                "flag is 128, it must be 0 to 127",
            ),
            (
                table("c", "C", "64", "[1, 2]", &good_party, "flagx = [2048]"),
                "extended flag is 2048, it must be 0 to 2047",
            ),
            (
                table(
                    "c",
                    "C",
                    "64",
                    "[1, 2]",
                    &good_party,
                    "items = [{ object = 256, count = 1 }]",
                ),
                "object is 256, it must be 0 to 255",
            ),
            (
                table("c", "C", "64", "[1, 2]", &good_party, "items = [{ object = 1, count = 0 }]"),
                "count is 0, it must be 1 to 24",
            ),
            (
                table(
                    "c",
                    "C",
                    "64",
                    "[1, 2]",
                    &good_party,
                    "items = [{ object = 1, count = 24 }, { object = 2, count = 1 }]",
                ),
                "the items are 25 entries: the list holds 24",
            ),
            (table("c", "C", "64", "[1, 2, 3]", &good_party, ""), "leader takes x and y"),
            (
                table("c", "C", "64", "[70000, 2]", &good_party, ""),
                "leader is 70000, it must be 0 to 65535",
            ),
            (table("c", "C", "50", "[1, 2]", &good_party, ""), "map 50 is a town screen"),
            (table("c", "C", "128", "[1, 2]", &good_party, ""), "map is 128, it must be 0 to 127"),
            (table("c", "  ", "64", "[1, 2]", &good_party, ""), "the name is empty"),
        ];
        for (text, expected) in cases {
            let message = refusal(&text);
            assert!(message.contains(expected), "{expected}: {message}");
            assert!(message.starts_with("chapter 1"), "the chapter is named: {message}");
        }
        assert!(refusal("[[chapter]]\nid = \"a\"\nname = \"a\"\nmap = 0\nleader = [1,1]\nfollower = [1,1]\nparty = [{ character = 0, level = 1 }]\nextra = 1\n")
            .contains("unknown field"), "an unknown key is refused");
        assert!(refusal("[other]\nx = 1\n").contains("is not `chapter`"));
        assert!(refusal("chapter = 3\n").contains("is not a list"));
        assert!(
            refusal("[[chapter]]\nid = \"a\"\n").contains("chapter 1"),
            "a missing key names the chapter"
        );
        let twice = format!(
            "{}{}",
            table("c", "C", "64", "[1, 2]", &good_party, ""),
            table("c", "D", "64", "[1, 2]", &good_party, "")
        );
        assert!(
            refusal(&twice).contains("given twice in this file"),
            "the id is unique in the file"
        );
    }

    #[test]
    fn ids_are_one_to_thirty_two_of_the_allowed_characters() {
        assert!(valid_id("androids"));
        assert!(valid_id("a_2"));
        assert!(valid_id(&"a".repeat(32)));
        assert!(!valid_id(""));
        assert!(!valid_id(&"a".repeat(33)));
        assert!(!valid_id("Androids"));
        assert!(!valid_id("a-b"), "a dash is not allowed: the id names a folder");
        assert!(!valid_id("a b"));
    }

    fn set_of(entries: &[(&str, &str, u8)]) -> Vec<(String, String, Chapter)> {
        entries
            .iter()
            .map(|&(pack, id, map)| {
                let text = format!(
                    "[[chapter]]\nid = \"{id}\"\nname = \"x\"\nmap = {map}\nleader = [1, 1]\nfollower = [1, 1]\nparty = [{{ character = 0, level = 1 }}]\n"
                );
                (pack.to_owned(), "chapters.toml".to_owned(), parse(&text).unwrap().remove(0))
            })
            .collect()
    }

    #[test]
    fn a_set_refuses_an_id_given_twice_and_a_map_that_is_no_room_of_the_set() {
        let set = set_of(&[
            ("a.pack", "androids", 64),
            ("b.pack", "androids", 0),
            ("b.pack", "other", 65),
        ]);
        let refusals = set_refusals(&set, &[64]);
        assert_eq!(refusals.len(), 2, "{refusals:?}");
        assert_eq!(
            refusals[0],
            (
                "b.pack".to_owned(),
                "chapters.toml".to_owned(),
                ChapterError::Duplicate {
                    id: "androids".into(),
                    first: "a.pack".into(),
                    second: "b.pack".into()
                }
            )
        );
        assert_eq!(
            refusals[1],
            (
                "b.pack".to_owned(),
                "chapters.toml".to_owned(),
                ChapterError::MissingMap { id: "other".into(), map: 65 }
            )
        );
        assert_eq!(set_refusals(&set_of(&[("a.pack", "x", 64)]), &[64]), Vec::new());
        assert_eq!(
            set_refusals(&set_of(&[("a.pack", "x", 47)]), &[]),
            Vec::new(),
            "an original map needs no pack"
        );
    }

    #[test]
    fn a_chapter_is_unlocked_by_the_completion_or_by_a_cheat_only_when_it_waits() {
        let chapters = parse(EXAMPLE).unwrap();
        let (waits, free) = (&chapters[0], &chapters[1]);
        assert!(!unlocked(waits, false, false), "a chapter that waits is locked at first");
        assert!(unlocked(waits, true, false), "the completion unlocks it");
        assert!(unlocked(waits, false, true), "the cheat unlocks it");
        assert!(unlocked(free, false, false), "a chapter that does not wait is always offered");
    }

    #[test]
    fn a_description_is_optional_and_kept_as_written() {
        let text = "[[chapter]]\nid = \"one\"\nname = \"One\"\ndescription = \"Goku's start.\"\nmap = 64\n\
                    leader = [1, 2]\nfollower = [1, 2]\nparty = [{ character = 0, level = 1 }]\n";
        assert_eq!(parse(text).unwrap()[0].description, "Goku's start.");
        assert_eq!(parse(EXAMPLE).unwrap()[0].description, "", "a file without one gives none");
    }

    #[test]
    fn the_slot_holds_the_party_map_positions_flags_money_and_items() {
        let chapters = parse(EXAMPLE).unwrap();
        let synthesized = synthesize(&FakeTables, &chapters[0]).unwrap();
        let save = &synthesized.save;
        assert_eq!(save.status(0), Ok(SlotStatus::Good), "the checksum and the mark are valid");
        assert_eq!(save.status(1), Ok(SlotStatus::Empty));
        let slot = save.slot(0).unwrap();
        assert_eq!(slot.map_id(), 64);
        assert_eq!(slot.party(), [0, 1]);
        assert_eq!(slot.record(0).unwrap().level(), 30);
        assert_eq!(slot.record(1).unwrap().level(), 20);
        assert_eq!(slot.record(1).unwrap().byte_0e(), 2, "the join order is the record's");
        assert_eq!(slot.leader(), Position { x: 1024, y: 768 });
        assert_eq!(slot.follower(), Position { x: 896, y: 648 });
        assert!(slot.flag(3) && slot.flag(29) && !slot.flag(4));
        assert_eq!(slot.zeni(), 200);
        assert_eq!(
            slot.items(),
            [Item { id: 5, attr: 0x45 }, Item { id: 5, attr: 0x45 }, Item { id: 5, attr: 0x45 }]
        );
        assert_eq!(slot.page()[COUNTDOWN_AT], 0, "map 64 is a pack map: no countdown");
        let flagx = synthesized.flagx.expect("the chapter sets an extended flag");
        assert_eq!(flagx.len(), FLAGX_BYTES);
        assert_eq!(flagx[0], 0x02, "extended flag 1 is bit 1 of byte 0");
        assert_eq!(&save.as_bytes()[0x100 + MARK_OFFSET..0x100 + MARK_OFFSET + 8], &MARK);
    }

    #[test]
    fn a_chapter_without_extended_flags_has_no_sidecar_bytes_and_a_countdown_map_has_its_byte() {
        let chapters = parse(EXAMPLE).unwrap();
        let plain = synthesize(&FakeTables, &chapters[1]).unwrap();
        assert_eq!(plain.flagx, None);
        let slot = plain.save.slot(0).unwrap();
        assert_eq!(slot.map_id(), 0);
        assert_eq!(slot.page()[COUNTDOWN_AT], 0x0A, "map 0 has the research's countdown");
        let original =
            parse(&table("c", "C", "9", "[1, 2]", "[{ character = 0, level = 1 }]", "")).unwrap();
        let nine = synthesize(&FakeTables, &original[0]).unwrap();
        assert_eq!(
            nine.save.slot(0).unwrap().page()[COUNTDOWN_AT],
            0x09,
            "map 9 has the research's countdown"
        );
    }

    #[test]
    fn a_record_the_tables_refuse_names_the_chapter() {
        let mut chapter = parse(EXAMPLE).unwrap().remove(1);
        chapter.party = vec![Member { character: 2, level: 49 }];
        let error = synthesize(&FakeTables, &chapter).unwrap_err();
        assert_eq!(
            error,
            ChapterError::Synthesis {
                id: "plain_start".into(),
                problem: "character 2 at level 49 is not in the table".into()
            }
        );
    }
}
