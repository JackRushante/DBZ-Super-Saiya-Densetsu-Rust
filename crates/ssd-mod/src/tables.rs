//! The data tables of a pack (M10, Task 7): `tables.toml`, which holds the units with their names and command lists, the
//! attacks, the encounter groups, and the characters' level rows and caps. A pack's `[[tables]]` entry names the file.
//! Each row is checked here and laid out as one slot of a data family (`Family::Unit` to `Family::LevelCap`), in the bytes
//! the game reads (`docs/re/data/units.md`, `characters.md`, `kaioken-bp.md` section 3). The redirectors of
//! `ssd-game::mods::tables` give the game those bytes at the addresses it reads them from.
//!
//! ```toml
//! [[unit]]                          # id 0 to 104 replaces the original unit, 105 to 254 is a new one
//! id = 105
//! name = "Guard"                    # plain text, at most 15 tiles
//! class = 1                         # 0 to 6
//! level = 5                         # 0 to 255
//! hp = 300                          # 0 to 65535
//! bp = 100000                       # 0 to 16777215
//! ki = 50                           # 0 to 65535
//! defence = 30                      # 0 to 255 (the defence factor of the record)
//! tie = 7                           # 0 to 255 (the tie-break byte of the record)
//! exp = 1234                        # the EXP reward, 0 to 16777215
//! sprite_like = 4                   # an original unit (0 to 104) whose battle sprite it uses: required for a new unit
//! [[unit.command]]                  # the command list, slot 0 first: required for a new unit
//! id = 0                            # a command id, 0 to 255 (255 is the blank line)
//! attr = 16                         # 0 to 255
//! factor = 4                        # the attack factor, 0 to 255 (255 is none)
//!
//! [[attack]]                        # id 0 to 68 replaces the original attack, 69 to 254 is a new one
//! id = 69
//! name = "Guard Blow"
//! ki = 10                           # the KI cost, 0 to 255
//! animation_like = 0                # an original attack (0 to 68) whose effect and animation the attack uses
//!
//! [[group]]                         # id 0 to 111 replaces the original group, 112 to 254 is a new one
//! id = 112
//! units = [105, 105]                # 1 to 5 unit ids, the enemies of the group in order
//!
//! [[character]]                     # ids 0 to 11
//! id = 0
//! cap = 100                         # the level cap, 1 to 100
//! [character.curve]                 # either the curve, or 100 rows: [[character.row]] (bp, exp, hp, ki, tie)
//! hp = { a = 80, b = 12, c = 1.0 }  # each value is a + b * level^c, rounded
//! ki = { a = 4, b = 0, c = 1.0 }
//! bp = { a = 0, b = 100, c = 1.2 }
//! exp = { a = -10, b = 10, c = 2.5 }
//! tie = 0
//! [[character.row_override]]        # with a curve only: a level whose values change
//! level = 48
//! hp = 900
//! ```

use crate::Family;
use crate::text::{LineError, NAME_CELLS, Piece, line};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::fmt;

/// The original units: ids `0..=104` replace their rows, `105` and up are new.
pub const ORIGINAL_UNITS: i64 = 105;
/// The original attacks: ids `0..=68` replace theirs, `69` and up are new.
pub const ORIGINAL_ATTACKS: i64 = 69;
/// The original encounter groups: ids `0..=111` replace theirs, `112` and up are new.
pub const ORIGINAL_GROUPS: i64 = 112;
/// The levels a character has rows for: 1 to 100.
pub const LEVELS: usize = 100;
/// The sprite byte of a unit that borrows no sprite.
pub const NO_SPRITE: u8 = 0xFF;
/// The length of a unit's item: its 14-byte row, then the sprite byte.
pub const UNIT_ITEM_LEN: usize = 15;
/// The offset of a unit's sprite byte in its item.
pub const UNIT_SPRITE_AT: u32 = 14;
/// The offset of an attack's name in its item: its KI cost and its animation byte come first.
pub const ATTACK_NAME_AT: u32 = 2;
/// The length of a character's level row.
pub const LEVEL_ROW_LEN: usize = 11;
/// The highest id of a unit, an attack or a group: `$FF` is the empty slot of a group.
const MAX_ID: i64 = 254;
/// The highest character id the tables give rows to.
const MAX_CHARACTER: i64 = 11;
/// The highest class of a unit (`units.md`, byte 0).
const MAX_CLASS: i64 = 6;
/// The largest 24-bit value (BP, EXP).
const MAX24: i64 = 0xFF_FFFF;
/// The largest 16-bit value (HP, KI).
const MAX16: i64 = 0xFFFF;
/// The largest byte.
const MAX8: i64 = 0xFF;

/// One slot a row of the tables fills: its family, its id, and the bytes the game reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    /// The family the slot is in (`Family::Unit` to `Family::LevelCap`).
    pub family: Family,
    /// The slot's id within its family.
    pub id: u16,
    /// The bytes of the slot, in the layout the game reads.
    pub bytes: Vec<u8>,
}

/// Why a tables file is refused. Each names the table and its id, and the field or the rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TablesError {
    /// The file is not TOML of the tables form (an unknown key, a value of the wrong type). The parser's message gives
    /// the position.
    Parse(String),
    /// A value outside its range: `what` names the table, its id and the field.
    Range { what: String, value: i64, min: i64, max: i64 },
    /// A row that is not right as a whole: a missing part, a duplicate id, a name that does not encode, a curve that
    /// gives a value outside its field.
    Row(String),
}

impl fmt::Display for TablesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(message) => write!(f, "the tables file is not valid: {message}"),
            Self::Range { what, value, min, max } => {
                write!(f, "{what} is {value}, it must be {min} to {max}")
            }
            Self::Row(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for TablesError {}

/// The whole of a `tables.toml`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    unit: Vec<UnitDef>,
    #[serde(default)]
    attack: Vec<AttackDef>,
    #[serde(default)]
    group: Vec<GroupDef>,
    #[serde(default)]
    character: Vec<CharacterDef>,
}

/// A `[[unit]]`. Its stats and name are all given, or (for an original unit only) `commands_only = true` gives its command list
/// alone, and the original's stats, name and sprite stay.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct UnitDef {
    id: i64,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    class: Option<i64>,
    #[serde(default)]
    level: Option<i64>,
    #[serde(default)]
    hp: Option<i64>,
    #[serde(default)]
    bp: Option<i64>,
    #[serde(default)]
    ki: Option<i64>,
    #[serde(default)]
    defence: Option<i64>,
    #[serde(default)]
    tie: Option<i64>,
    #[serde(default)]
    exp: Option<i64>,
    #[serde(default)]
    sprite_like: Option<i64>,
    #[serde(default)]
    commands_only: bool,
    #[serde(default)]
    commands_append: bool,
    #[serde(default)]
    command: Vec<CommandDef>,
}

/// A `[[unit.command]]`: one slot of the command list.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommandDef {
    id: i64,
    attr: i64,
    factor: i64,
}

/// A `[[attack]]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttackDef {
    id: i64,
    name: String,
    ki: i64,
    animation_like: i64,
}

/// A `[[group]]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GroupDef {
    id: i64,
    units: Vec<i64>,
}

/// A `[[character]]`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CharacterDef {
    id: i64,
    cap: i64,
    #[serde(default)]
    row: Vec<RowDef>,
    #[serde(default)]
    curve: Option<CurveDef>,
    #[serde(default)]
    row_override: Vec<OverrideDef>,
}

/// A `[[character.row]]`: one level's row, in the fields of `characters.md`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RowDef {
    bp: i64,
    exp: i64,
    hp: i64,
    ki: i64,
    tie: i64,
}

/// A `[character.curve]`: four formulas and the tie-break byte the rows of the curve share.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CurveDef {
    hp: Formula,
    ki: Formula,
    bp: Formula,
    exp: Formula,
    #[serde(default)]
    tie: i64,
}

/// A formula `a + b * level^c`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
struct Formula {
    a: f64,
    b: f64,
    c: f64,
}

/// A `[[character.row_override]]`: the fields it names change for its level, the others keep the curve's value.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OverrideDef {
    level: i64,
    #[serde(default)]
    bp: Option<i64>,
    #[serde(default)]
    exp: Option<i64>,
    #[serde(default)]
    hp: Option<i64>,
    #[serde(default)]
    ki: Option<i64>,
    #[serde(default)]
    tie: Option<i64>,
}

/// A unit's stats row as the game reads it: the 14 bytes of `units.md`, then the sprite byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UnitRow {
    class: u8,
    level: u8,
    hp: u16,
    bp: u32,
    ki: u16,
    defence: u8,
    tie: u8,
    exp: u32,
    sprite: u8,
}

impl UnitRow {
    /// The 15 bytes: class, level, HP (low first), BP (24 bits, low first), KI, defence, tie, EXP (24 bits), sprite.
    fn bytes(self) -> [u8; 15] {
        let [hp0, hp1] = self.hp.to_le_bytes();
        let [bp0, bp1, bp2, _] = self.bp.to_le_bytes();
        let [ki0, ki1] = self.ki.to_le_bytes();
        let [exp0, exp1, exp2, _] = self.exp.to_le_bytes();
        [
            self.class,
            self.level,
            hp0,
            hp1,
            bp0,
            bp1,
            bp2,
            ki0,
            ki1,
            self.defence,
            self.tie,
            exp0,
            exp1,
            exp2,
            self.sprite,
        ]
    }
}

/// A character's row for one level, in the 11 bytes of `characters.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LevelRow {
    bp: u32,
    exp: u32,
    hp: u16,
    ki: u16,
    tie: u8,
}

impl LevelRow {
    /// The 11 bytes: BP (24 bits, low first), EXP (24 bits), HP, KI, tie.
    fn bytes(self) -> [u8; 11] {
        let [bp0, bp1, bp2, _] = self.bp.to_le_bytes();
        let [exp0, exp1, exp2, _] = self.exp.to_le_bytes();
        let [hp0, hp1] = self.hp.to_le_bytes();
        let [ki0, ki1] = self.ki.to_le_bytes();
        [bp0, bp1, bp2, exp0, exp1, exp2, hp0, hp1, ki0, ki1, self.tie]
    }
}

/// The slots of the tables file `text`, checked and laid out. Every row is refused with its table, its id and the field
/// that is wrong.
///
/// # Errors
///
/// [`TablesError::Parse`] for a file that is not the tables form, [`TablesError::Range`] for a value out of its range,
/// [`TablesError::Row`] for a row that is not right as a whole.
pub fn compile(text: &str) -> Result<Vec<Compiled>, TablesError> {
    let file: File = toml::from_str(text).map_err(|error| TablesError::Parse(error.to_string()))?;
    let mut items = Vec::new();
    items.extend(each("unit", &file.unit, |def| def.id, unit)?);
    items.extend(each(
        "attack",
        &file.attack,
        |def| def.id,
        |def| attack(def).map(|item| vec![item]),
    )?);
    items.extend(each(
        "group",
        &file.group,
        |def| def.id,
        |def| group(def).map(|item| vec![item]),
    )?);
    items.extend(each("character", &file.character, |def| def.id, character)?);
    Ok(items)
}

/// Each row of `defs`, in order, with its slots; an id given twice is refused.
fn each<T>(
    table: &str,
    defs: &[T],
    id: impl Fn(&T) -> i64,
    row: impl Fn(&T) -> Result<Vec<Compiled>, TablesError>,
) -> Result<Vec<Compiled>, TablesError> {
    let mut seen = BTreeSet::new();
    let mut items = Vec::new();
    for def in defs {
        if !seen.insert(id(def)) {
            return Err(TablesError::Row(format!("{table} {} is given twice", id(def))));
        }
        items.extend(row(def)?);
    }
    Ok(items)
}

/// The value `value` if it is within `min` to `max`, else the refusal that names `what`.
fn bounded(what: String, value: i64, min: i64, max: i64) -> Result<i64, TablesError> {
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(TablesError::Range { what, value, min, max })
    }
}

/// A `[[unit]]`: its stats row and name, and its command list if it has one. A new unit needs a sprite and a command list.
/// With `commands_only` it is [`commands_only`].
fn unit(def: &UnitDef) -> Result<Vec<Compiled>, TablesError> {
    let id = bounded("unit id".to_owned(), def.id, 0, MAX_ID)?;
    if def.commands_only && def.commands_append {
        return Err(TablesError::Row(format!(
            "unit {id}: `commands_only` and `commands_append` cannot both be given"
        )));
    }
    if def.commands_only || def.commands_append {
        return unit_commands_only(id, def);
    }
    let what = |field: &str| format!("unit {id}: {field}");
    let given = |field: &str, value: Option<i64>| {
        value.ok_or_else(|| {
            TablesError::Row(format!(
                "unit {id}: its {field} is missing (a unit gives all its stats and its name, or says `commands_only = true` to replace its command list alone)"
            ))
        })
    };
    let class = bounded(what("class"), given("class", def.class)?, 0, MAX_CLASS)?;
    let level = bounded(what("level"), given("level", def.level)?, 0, MAX8)?;
    let hp = bounded(what("hp"), given("hp", def.hp)?, 0, MAX16)?;
    let bp = bounded(what("bp"), given("bp", def.bp)?, 0, MAX24)?;
    let ki = bounded(what("ki"), given("ki", def.ki)?, 0, MAX16)?;
    let defence = bounded(what("defence"), given("defence", def.defence)?, 0, MAX8)?;
    let tie = bounded(what("tie"), given("tie", def.tie)?, 0, MAX8)?;
    let exp = bounded(what("exp"), given("exp", def.exp)?, 0, MAX24)?;
    let name = def.name.as_deref().ok_or_else(|| {
        TablesError::Row(format!(
            "unit {id}: its name is missing (a unit gives all its stats and its name, or says `commands_only = true`)"
        ))
    })?;
    let new = id >= ORIGINAL_UNITS;
    let sprite = match def.sprite_like {
        Some(sprite) => bounded(what("sprite_like"), sprite, 0, ORIGINAL_UNITS - 1)? as u8,
        None if new => {
            return Err(TablesError::Row(format!(
                "unit {id} is a new unit: it names the original unit whose sprite it uses (`sprite_like`, 0 to 104)"
            )));
        }
        None => NO_SPRITE,
    };
    let commands = commands(&format!("unit {id}"), &def.command)?;
    if new && commands.is_empty() {
        return Err(TablesError::Row(format!(
            "unit {id} is a new unit: it needs its command list (`[[unit.command]]`)"
        )));
    }
    let row = UnitRow {
        class: class as u8,
        level: level as u8,
        hp: hp as u16,
        bp: bp as u32,
        ki: ki as u16,
        defence: defence as u8,
        tie: tie as u8,
        exp: exp as u32,
        sprite,
    };
    let id = id as u16;
    let mut items = vec![
        Compiled { family: Family::Unit, id, bytes: row.bytes().to_vec() },
        Compiled { family: Family::UnitName, id, bytes: name_bytes(&format!("unit {id}"), name)? },
    ];
    if !commands.is_empty() {
        items.push(Compiled { family: Family::CommandList, id, bytes: command_list(&commands) });
    }
    Ok(items)
}

/// A `[[unit]]` with `commands_only = true`: the command list of an original unit (0 to 104), and nothing else. Its stats, name
/// and sprite are the original's, so it names none of them; a new unit has no original list to replace and is refused.
fn unit_commands_only(id: i64, def: &UnitDef) -> Result<Vec<Compiled>, TablesError> {
    let what = format!("unit {id}");
    if id >= ORIGINAL_UNITS {
        return Err(TablesError::Row(format!(
            "{what} is a new unit: `commands_only` replaces the command list of an original unit (0 to {})",
            ORIGINAL_UNITS - 1
        )));
    }
    let given: Vec<&str> = [
        ("name", def.name.is_some()),
        ("class", def.class.is_some()),
        ("level", def.level.is_some()),
        ("hp", def.hp.is_some()),
        ("bp", def.bp.is_some()),
        ("ki", def.ki.is_some()),
        ("defence", def.defence.is_some()),
        ("tie", def.tie.is_some()),
        ("exp", def.exp.is_some()),
        ("sprite_like", def.sprite_like.is_some()),
    ]
    .into_iter()
    .filter_map(|(field, is_given)| is_given.then_some(field))
    .collect();
    if !given.is_empty() {
        return Err(TablesError::Row(format!(
            "{what}: `commands_only` keeps the original's stats, name and sprite, so `{}` is not allowed",
            given.join("`, `")
        )));
    }
    let commands = commands(&what, &def.command)?;
    if commands.is_empty() {
        return Err(TablesError::Row(format!(
            "{what}: `commands_only` needs its command list (`[[unit.command]]`)"
        )));
    }
    // `commands_append` adds the slots after the original's: the image build joins them to the list it reads from the ROM.
    let family = if def.commands_append { Family::CommandAppend } else { Family::CommandList };
    Ok(vec![Compiled { family, id: id as u16, bytes: command_list(&commands) }])
}

/// The commands of a list in the layout of [`command_list`], as `(id, attr, factor)`; `None` when the bytes are not such a list.
#[must_use]
pub fn command_list_commands(bytes: &[u8]) -> Option<Vec<[u8; 3]>> {
    let count = usize::from(*bytes.first()?);
    if bytes.len() < 1 + 4 * count {
        return None;
    }
    Some(
        (0..count)
            .map(|slot| [bytes[1 + slot], bytes[1 + count + slot], bytes[1 + 2 * count + 2 * slot]])
            .collect(),
    )
}

/// The original command list of unit `id` in `rom` (1 MiB `LoROM`): the word at `02:B7D2 + 2 * id` points at the list in
/// bank 2 (`kaioken-bp.md`, section 3). `None` for a unit past the table or a list that runs out of the ROM.
#[must_use]
pub fn original_command_list(rom: &[u8], id: u16) -> Option<Vec<[u8; 3]>> {
    let offset = |bank: usize, address: usize| {
        Some(bank * 0x8000 + address.checked_sub(0x8000)?.min(0x7FFF))
    };
    let table = offset(2, 0xB7D2)? + 2 * usize::from(id);
    let pointer = usize::from(u16::from_le_bytes([*rom.get(table)?, *rom.get(table + 1)?]));
    let start = offset(2, pointer)?;
    let count = usize::from(*rom.get(start)?);
    command_list_commands(rom.get(start..start + 1 + 4 * count)?)
}

/// The list of `commands` joined after the original list of unit `id`, in the layout the game reads.
#[must_use]
pub fn appended_command_list(rom: &[u8], id: u16, appended: &[u8]) -> Option<Vec<u8>> {
    let mut commands = original_command_list(rom, id)?;
    commands.extend(command_list_commands(appended)?);
    (commands.len() <= 255).then(|| command_list(&commands))
}

/// The command list of a unit: `[[unit.command]]` in slot order, as `(id, attr, factor)`. An empty list is none; the
/// caller decides whether that is allowed.
fn commands(what: &str, list: &[CommandDef]) -> Result<Vec<[u8; 3]>, TablesError> {
    bounded(format!("{what}: its number of commands"), list.len() as i64, 0, 255)?;
    list.iter()
        .enumerate()
        .map(|(slot, command)| -> Result<[u8; 3], TablesError> {
            let id = bounded(format!("{what}: command of slot {slot}, id"), command.id, 0, 255)?;
            let attr =
                bounded(format!("{what}: command of slot {slot}, attr"), command.attr, 0, 255)?;
            let factor =
                bounded(format!("{what}: command of slot {slot}, factor"), command.factor, 0, 255)?;
            Ok([id as u8, attr as u8, factor as u8])
        })
        .collect()
}

/// The command list's bytes (`kaioken-bp.md`, section 3): the count, the command ids, the attribute bytes, then one word
/// per slot whose low byte is the attack factor (the high byte is 0).
fn command_list(commands: &[[u8; 3]]) -> Vec<u8> {
    let count = commands.len() as u8;
    let mut bytes = vec![count];
    bytes.extend(commands.iter().map(|command| command[0]));
    bytes.extend(commands.iter().map(|command| command[1]));
    for command in commands {
        bytes.extend([command[2], 0]);
    }
    bytes
}

/// A name as the game reads it: a length byte, then that many tiles (`names.md`). Plain text only, at least one tile and
/// at most [`NAME_CELLS`].
fn name_bytes(what: &str, text: &str) -> Result<Vec<u8>, TablesError> {
    let pieces = line(text).map_err(|error| {
        TablesError::Row(format!("{what}: the name `{text}` does not encode: {}", describe(&error)))
    })?;
    let mut tiles = Vec::with_capacity(pieces.len());
    for piece in pieces {
        let Piece::Tile(code) = piece else {
            return Err(TablesError::Row(format!(
                "{what}: the name `{text}` holds a slot or a portrait; a name is plain text"
            )));
        };
        tiles.push(code);
    }
    if tiles.is_empty() {
        return Err(TablesError::Row(format!("{what}: the name is empty")));
    }
    bounded(
        format!("{what}: the length of the name in tiles"),
        tiles.len() as i64,
        1,
        NAME_CELLS as i64,
    )?;
    let mut bytes = vec![tiles.len() as u8];
    bytes.extend(tiles);
    Ok(bytes)
}

/// Why a name does not encode, in words.
fn describe(error: &LineError) -> String {
    match error {
        LineError::Character { ch, column } => {
            format!("the character `{ch}` at column {column} has no tile")
        }
        LineError::Unclosed => "a `{` has no `}`".to_owned(),
        LineError::Token(token) => format!("{token} is not a token"),
    }
}

/// A `[[attack]]`: its item is its KI cost, the original attack whose effect and animation it uses, and its name. The
/// name's pointer is the item's address plus two (see the redirectors).
fn attack(def: &AttackDef) -> Result<Compiled, TablesError> {
    let id = bounded("attack id".to_owned(), def.id, 0, MAX_ID)?;
    let what = format!("attack {id}");
    let ki = bounded(format!("{what}: ki"), def.ki, 0, MAX8)? as u8;
    let like =
        bounded(format!("{what}: animation_like"), def.animation_like, 0, ORIGINAL_ATTACKS - 1)?;
    let mut bytes = vec![ki, like as u8];
    bytes.extend(name_bytes(&what, &def.name)?);
    Ok(Compiled { family: Family::Attack, id: id as u16, bytes })
}

/// A `[[group]]`: its units, in order, at the screen positions the game gives a group of their number (see
/// [`positions`]); the other slots are `$FF`.
fn group(def: &GroupDef) -> Result<Compiled, TablesError> {
    let id = bounded("group id".to_owned(), def.id, 0, MAX_ID)?;
    let what = format!("group {id}");
    bounded(format!("{what}: its number of units"), def.units.len() as i64, 1, 5)?;
    let slots = positions(def.units.len());
    let mut bytes = vec![0xFF_u8; 5];
    for (&slot, &unit) in slots.iter().zip(&def.units) {
        bytes[slot] = bounded(format!("{what}: unit of position {slot}"), unit, 0, MAX_ID)? as u8;
    }
    Ok(Compiled { family: Family::Group, id: id as u16, bytes })
}

/// The screen positions (the slots of the five-byte group) of the `count` enemies of a group, in order: 1 enemy in slot 0;
/// 2 in slots 1 and 2; 3 in slots 0, 3 and 4; 4 in slots 1 to 4; 5 in all (`units.md`, encounter groups). The battle
/// reads the slots for the count it counts, so the units of a group sit there.
fn positions(count: usize) -> &'static [usize] {
    match count {
        1 => &[0],
        2 => &[1, 2],
        3 => &[0, 3, 4],
        4 => &[1, 2, 3, 4],
        _ => &[0, 1, 2, 3, 4],
    }
}

/// A `[[character]]`: its level cap, and its 100 level rows (from the rows or from the curve).
fn character(def: &CharacterDef) -> Result<Vec<Compiled>, TablesError> {
    let id = bounded("character id".to_owned(), def.id, 0, MAX_CHARACTER)?;
    let what = format!("character {id}");
    let cap = bounded(format!("{what}: cap"), def.cap, 1, LEVELS as i64)? as u8;
    let rows = match (&def.curve, def.row.is_empty()) {
        (Some(curve), true) => expand(&what, curve, &def.row_override)?,
        (None, false) => {
            if !def.row_override.is_empty() {
                return Err(TablesError::Row(format!(
                    "{what}: `row_override` goes with `[character.curve]`; with rows, give every row"
                )));
            }
            from_rows(&what, &def.row)?
        }
        (Some(_), false) => {
            return Err(TablesError::Row(format!(
                "{what}: give either the rows (`[[character.row]]`) or the curve (`[character.curve]`), not both"
            )));
        }
        (None, true) => {
            return Err(TablesError::Row(format!(
                "{what}: give its level rows (`[[character.row]]`, {LEVELS} of them) or its `[character.curve]`"
            )));
        }
    };
    let id = id as u16;
    Ok(vec![
        Compiled {
            family: Family::LevelRows,
            id,
            bytes: rows.iter().flat_map(|row| row.bytes()).collect(),
        },
        Compiled { family: Family::LevelCap, id, bytes: vec![cap] },
    ])
}

/// The rows of a character given row by row: exactly [`LEVELS`] of them, level 1 first.
fn from_rows(what: &str, defs: &[RowDef]) -> Result<Vec<LevelRow>, TablesError> {
    if defs.len() != LEVELS {
        return Err(TablesError::Row(format!(
            "{what}: the level rows are {LEVELS} (levels 1 to {LEVELS}), the file has {}",
            defs.len()
        )));
    }
    let rows = defs
        .iter()
        .enumerate()
        .map(|(index, def)| -> Result<LevelRow, TablesError> {
            let level = index + 1;
            let at = |field: &str| format!("{what}, level {level}: {field}");
            Ok(LevelRow {
                bp: bounded(at("bp"), def.bp, 0, MAX24)? as u32,
                exp: bounded(at("exp"), def.exp, 0, MAX24)? as u32,
                hp: bounded(at("hp"), def.hp, 0, MAX16)? as u16,
                ki: bounded(at("ki"), def.ki, 0, MAX16)? as u16,
                tie: bounded(at("tie"), def.tie, 0, MAX8)? as u8,
            })
        })
        .collect::<Result<Vec<_>, TablesError>>()?;
    first_level_starts_at_zero(what, &rows)?;
    Ok(rows)
}

/// The rows of a character from its curve, then its overrides: one row per level 1 to [`LEVELS`].
fn expand(
    what: &str,
    curve: &CurveDef,
    overrides: &[OverrideDef],
) -> Result<Vec<LevelRow>, TablesError> {
    let mut rows = Vec::with_capacity(LEVELS);
    for level in 1..=LEVELS {
        rows.push(LevelRow {
            bp: curve_value(what, "bp", curve.bp, level, MAX24)? as u32,
            exp: curve_value(what, "exp", curve.exp, level, MAX24)? as u32,
            hp: curve_value(what, "hp", curve.hp, level, MAX16)? as u16,
            ki: curve_value(what, "ki", curve.ki, level, MAX16)? as u16,
            tie: bounded(format!("{what}: tie"), curve.tie, 0, MAX8)? as u8,
        });
    }
    let mut overridden = BTreeSet::new();
    for over in overrides {
        let level =
            bounded(format!("{what}: the level of a row_override"), over.level, 1, LEVELS as i64)?
                as usize;
        if !overridden.insert(level) {
            return Err(TablesError::Row(format!("{what}: level {level} is overridden twice")));
        }
        let at = |field: &str| format!("{what}, level {level}: {field}");
        let row = &mut rows[level - 1];
        if let Some(bp) = over.bp {
            row.bp = bounded(at("bp"), bp, 0, MAX24)? as u32;
        }
        if let Some(exp) = over.exp {
            row.exp = bounded(at("exp"), exp, 0, MAX24)? as u32;
        }
        if let Some(hp) = over.hp {
            row.hp = bounded(at("hp"), hp, 0, MAX16)? as u16;
        }
        if let Some(ki) = over.ki {
            row.ki = bounded(at("ki"), ki, 0, MAX16)? as u16;
        }
        if let Some(tie) = over.tie {
            row.tie = bounded(at("tie"), tie, 0, MAX8)? as u8;
        }
    }
    first_level_starts_at_zero(what, &rows)?;
    Ok(rows)
}

/// `formula` at `level`, rounded half away from zero, refused when it is not a number or is outside 0 to `max`.
fn curve_value(
    what: &str,
    field: &str,
    formula: Formula,
    level: usize,
    max: i64,
) -> Result<i64, TablesError> {
    let x = formula.a + formula.b * (level as f64).powf(formula.c);
    let value = x.round();
    if !x.is_finite() || value < 0.0 || value > max as f64 {
        return Err(TablesError::Row(format!(
            "{what}: the curve of `{field}` gives {x} at level {level}, it must be 0 to {max}"
        )));
    }
    Ok(value as i64)
}

/// Level 1 begins at 0 EXP (`characters.md`): a row set that starts above 0 is refused.
fn first_level_starts_at_zero(what: &str, rows: &[LevelRow]) -> Result<(), TablesError> {
    match rows.first() {
        Some(row) if row.exp != 0 => Err(TablesError::Row(format!(
            "{what}: the EXP of level 1 is {}, it must be 0 (level 1 begins at 0)",
            row.exp
        ))),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tile codes of `text`, as a name is written (the encoder's own, not a copy of the font).
    fn tiles(text: &str) -> Vec<u8> {
        line(text)
            .unwrap()
            .into_iter()
            .map(|piece| match piece {
                Piece::Tile(code) => code,
                other => panic!("{other:?}"),
            })
            .collect()
    }

    /// The item of `family` and `id` in `items`.
    fn item(items: &[Compiled], family: Family, id: u16) -> Vec<u8> {
        items
            .iter()
            .find(|item| item.family == family && item.id == id)
            .unwrap_or_else(|| panic!("no {family:?} {id}"))
            .bytes
            .clone()
    }

    /// A character's rows of a curve with the given formulas and no overrides.
    fn curve_file(id: u8, hp: &str, exp: &str) -> String {
        format!(
            "[[character]]\nid = {id}\ncap = 100\n[character.curve]\n\
             hp = {hp}\nki = {{ a = 4, b = 0, c = 1 }}\nbp = {{ a = 0, b = 100, c = 1 }}\nexp = {exp}\n"
        )
    }

    /// Every `[[unit]]` of the tests: a new unit with two commands, the rest of a replaced unit.
    const GUARD: &str = r#"
[[unit]]
id = 105
name = "Guard"
class = 1
level = 5
hp = 300
bp = 100000
ki = 50
defence = 30
tie = 7
exp = 1234
sprite_like = 4
[[unit.command]]
id = 0
attr = 16
factor = 4
[[unit.command]]
id = 1
attr = 15
factor = 6
"#;

    #[test]
    fn a_unit_row_is_its_fourteen_bytes_then_its_sprite_and_its_command_list_is_written_out() {
        let items = compile(GUARD).unwrap();
        assert_eq!(
            item(&items, Family::Unit, 105),
            [
                1, 5, // class, level
                0x2C, 0x01, // HP 300
                0xA0, 0x86, 0x01, // BP 100000, 24 bits low first
                0x32, 0x00, // KI 50
                30, 7, // defence, tie
                0xD2, 0x04, 0x00, // EXP 1234, 24 bits low first
                4,    // the sprite: unit 4
            ]
        );
        assert_eq!(
            item(&items, Family::CommandList, 105),
            [
                2, // two slots
                0x00, 0x01, // the command ids
                0x10, 0x0F, // the attribute bytes
                0x04, 0x00, 0x06, 0x00, // the factors, one word each
            ]
        );
        let mut name = vec![5];
        name.extend(tiles("Guard"));
        assert_eq!(item(&items, Family::UnitName, 105), name);
    }

    #[test]
    fn a_replaced_unit_without_a_sprite_or_commands_keeps_the_originals_list_and_sprite() {
        let text = r#"
[[unit]]
id = 4
name = "Saibaman"
class = 0
level = 1
hp = 1
bp = 1
ki = 0
defence = 0
tie = 0
exp = 0
"#;
        let items = compile(text).unwrap();
        assert_eq!(items.len(), 2, "the row and the name, no command list");
        assert_eq!(item(&items, Family::Unit, 4)[14], NO_SPRITE);
    }

    #[test]
    fn a_group_puts_its_units_in_the_positions_its_count_names() {
        let group = |units: &str| {
            let items = compile(&format!(
                "[[group]]
id = 112
units = [{units}]
"
            ))
            .unwrap();
            item(&items, Family::Group, 112)
        };
        assert_eq!(group("105"), [105, 0xFF, 0xFF, 0xFF, 0xFF], "one enemy: slot 0");
        assert_eq!(group("105, 106"), [0xFF, 105, 106, 0xFF, 0xFF], "two: slots 1 and 2");
        assert_eq!(group("1, 2, 3"), [1, 0xFF, 0xFF, 2, 3], "three: slots 0, 3 and 4");
        assert_eq!(group("1, 2, 3, 4"), [0xFF, 1, 2, 3, 4], "four: slots 1 to 4");
        assert_eq!(group("1, 2, 3, 4, 5"), [1, 2, 3, 4, 5], "five: all");
    }

    #[test]
    fn an_attack_is_its_ki_its_animation_then_its_name() {
        let text = "[[attack]]\nid = 69\nname = \"Guard Blow\"\nki = 10\nanimation_like = 14\n";
        let items = compile(text).unwrap();
        let mut bytes = vec![10, 14, 10];
        bytes.extend(tiles("Guard Blow"));
        assert_eq!(item(&items, Family::Attack, 69), bytes);
    }

    #[test]
    fn a_name_is_plain_text_of_one_to_fifteen_tiles() {
        let fifteen = "A".repeat(NAME_CELLS);
        let text =
            format!("[[attack]]\nid = 70\nname = \"{fifteen}\"\nki = 1\nanimation_like = 0\n");
        assert!(compile(&text).is_ok(), "fifteen tiles");
        let sixteen = "A".repeat(NAME_CELLS + 1);
        let text =
            format!("[[attack]]\nid = 70\nname = \"{sixteen}\"\nki = 1\nanimation_like = 0\n");
        assert!(matches!(compile(&text), Err(TablesError::Range { value: 16, max: 15, .. })));
        let slot = "[[attack]]\nid = 70\nname = \"a{number:1}\"\nki = 1\nanimation_like = 0\n";
        assert!(
            matches!(compile(slot), Err(TablesError::Row(message)) if message.contains("plain text"))
        );
        let empty = "[[attack]]\nid = 70\nname = \"\"\nki = 1\nanimation_like = 0\n";
        assert!(
            matches!(compile(empty), Err(TablesError::Row(message)) if message.contains("empty"))
        );
        let symbol = "[[attack]]\nid = 70\nname = \"a:b\"\nki = 1\nanimation_like = 0\n";
        assert!(
            matches!(compile(symbol), Err(TablesError::Row(message)) if message.contains("column 2"))
        );
    }

    #[test]
    fn a_curve_is_expanded_for_levels_1_48_and_100_and_rounded() {
        let text = curve_file(0, "{ a = 80, b = 12, c = 1 }", "{ a = -10, b = 10, c = 2 }");
        let items = compile(&text).unwrap();
        let rows = item(&items, Family::LevelRows, 0);
        assert_eq!(rows.len(), LEVELS * 11, "100 rows of 11 bytes");
        let row = |level: usize| &rows[(level - 1) * 11..level * 11];
        // Level 1: BP 100, EXP 0, HP 92, KI 4, tie 0.
        assert_eq!(row(1), [100, 0, 0, 0, 0, 0, 92, 0, 4, 0, 0]);
        // Level 48: BP 4800, EXP 10 * (48^2 - 1) = 23030 (0x59F6), HP 80 + 12 * 48 = 656.
        assert_eq!(row(48), [0xC0, 0x12, 0, 0xF6, 0x59, 0, 0x90, 0x02, 4, 0, 0]);
        // Level 100: BP 10000, EXP 10 * (100^2 - 1) = 99990 (0x018696), HP 1280.
        assert_eq!(row(100), [0x10, 0x27, 0, 0x96, 0x86, 0x01, 0x00, 0x05, 4, 0, 0]);
        assert_eq!(item(&items, Family::LevelCap, 0), [100]);
    }

    #[test]
    fn a_fraction_rounds_half_away_from_zero_and_the_curve_may_not_pass_the_24_bit_exp() {
        let text = curve_file(1, "{ a = 0.5, b = 0, c = 1 }", "{ a = -2000, b = 2000, c = 1 }");
        let items = compile(&text).unwrap();
        assert_eq!(item(&items, Family::LevelRows, 1)[6], 1, "0.5 rounds to 1");
        let too_big = curve_file(0, "{ a = 80, b = 12, c = 1 }", "{ a = -2000, b = 2000, c = 2 }");
        // 2000 * (91^2 - 1) = 16_560_000 fits; 2000 * (92^2 - 1) = 16_926_000 passes 16 777 215 at level 92.
        match compile(&too_big) {
            Err(TablesError::Row(message)) => {
                assert!(message.contains("exp") && message.contains("level 92"), "{message}");
                assert!(message.contains("16777215"), "{message}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_exp_of_level_one_must_be_zero() {
        let text = curve_file(0, "{ a = 80, b = 12, c = 1 }", "{ a = 5, b = 10, c = 2 }");
        assert!(
            matches!(compile(&text), Err(TablesError::Row(message)) if message.contains("level 1"))
        );
    }

    #[test]
    fn a_row_override_changes_only_its_level_and_only_with_a_curve() {
        let text = format!(
            "{}\n[[character.row_override]]\nlevel = 48\nhp = 900\n",
            curve_file(0, "{ a = 80, b = 12, c = 1 }", "{ a = -10, b = 10, c = 2 }")
        );
        let items = compile(&text).unwrap();
        let rows = item(&items, Family::LevelRows, 0);
        assert_eq!(&rows[47 * 11 + 6..47 * 11 + 8], [0x84, 0x03], "900 at level 48");
        assert_eq!(&rows[46 * 11 + 6..46 * 11 + 8], [0x84, 0x02], "644 at level 47 is unchanged");
        let twice = format!("{text}[[character.row_override]]\nlevel = 48\nki = 9\n");
        assert!(
            matches!(compile(&twice), Err(TablesError::Row(message)) if message.contains("twice"))
        );
        let with_rows = format!(
            "[[character]]\nid = 0\ncap = 1\n[[character.row_override]]\nlevel = 1\nhp = 1\n{}",
            rows_text(100)
        );
        assert!(
            matches!(compile(&with_rows), Err(TablesError::Row(message)) if message.contains("row_override"))
        );
    }

    /// `count` level rows in the `[[character.row]]` form, all zero.
    fn rows_text(count: usize) -> String {
        "[[character.row]]\nbp = 0\nexp = 0\nhp = 1\nki = 0\ntie = 0\n".repeat(count)
    }

    #[test]
    fn rows_given_row_by_row_are_exactly_one_hundred_and_start_at_zero_exp() {
        let text = format!("[[character]]\nid = 0\ncap = 100\n{}", rows_text(100));
        let items = compile(&text).unwrap();
        assert_eq!(item(&items, Family::LevelRows, 0).len(), 1100);
        let short = format!("[[character]]\nid = 0\ncap = 100\n{}", rows_text(99));
        assert!(
            matches!(compile(&short), Err(TablesError::Row(message)) if message.contains("99"))
        );
        let neither = "[[character]]\nid = 0\ncap = 100\n";
        assert!(
            matches!(compile(neither), Err(TablesError::Row(message)) if message.contains("level rows"))
        );
        let both = format!(
            "[[character]]\nid = 0\ncap = 100\n{}[character.curve]\nhp = {{ a = 1, b = 0, c = 1 }}\n\
             ki = {{ a = 1, b = 0, c = 1 }}\nbp = {{ a = 1, b = 0, c = 1 }}\nexp = {{ a = 0, b = 0, c = 1 }}\n",
            rows_text(100)
        );
        assert!(
            matches!(compile(&both), Err(TablesError::Row(message)) if message.contains("not both"))
        );
        let exp_at_one = format!(
            "[[character]]\nid = 0\ncap = 100\n{}",
            rows_text(100).replacen("exp = 0", "exp = 3", 1)
        );
        assert!(
            matches!(compile(&exp_at_one), Err(TablesError::Row(message)) if message.contains("level 1"))
        );
    }

    /// The Guard sample with the line `field = ...` set to `field = value`.
    fn guard_with(field: &str, value: &str) -> String {
        GUARD
            .lines()
            .map(|line| {
                if line.starts_with(&format!("{field} = ")) {
                    format!("{field} = {value}")
                } else {
                    line.to_owned()
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_range_error_names_its_table_id_and_field() {
        let cases: Vec<(String, &str)> = vec![
            (guard_with("hp", "70000"), "unit 105: hp is 70000"),
            (guard_with("bp", "16777216"), "unit 105: bp is 16777216"),
            (guard_with("class", "7"), "unit 105: class is 7"),
            (guard_with("level", "256"), "unit 105: level is 256"),
            (guard_with("ki", "-1"), "unit 105: ki is -1"),
            (guard_with("defence", "300"), "unit 105: defence is 300"),
            (guard_with("tie", "300"), "unit 105: tie is 300"),
            (guard_with("exp", "16777216"), "unit 105: exp is 16777216"),
            (guard_with("sprite_like", "105"), "unit 105: sprite_like is 105"),
            (GUARD.replace("id = 105", "id = 255"), "unit id is 255"),
            (
                GUARD.replace("factor = 6", "factor = 256"),
                "unit 105: command of slot 1, factor is 256",
            ),
            (GUARD.replace("attr = 15", "attr = -1"), "unit 105: command of slot 1, attr is -1"),
            (GUARD.replace("id = 1\n", "id = 300\n"), "unit 105: command of slot 1, id is 300"),
        ];
        for (text, expected) in cases {
            match compile(&text) {
                Err(error) => {
                    assert!(error.to_string().contains(expected), "{error} (want {expected})");
                }
                Ok(_) => panic!("accepted, want {expected}"),
            }
        }
    }

    #[test]
    fn a_new_unit_needs_a_sprite_and_commands_and_a_repeated_id_is_refused() {
        let no_sprite = GUARD.replace("sprite_like = 4\n", "");
        assert!(
            matches!(compile(&no_sprite), Err(TablesError::Row(message)) if message.contains("sprite_like"))
        );
        let no_commands = GUARD.split("[[unit.command]]").next().unwrap().to_owned();
        assert!(
            matches!(compile(&no_commands), Err(TablesError::Row(message)) if message.contains("command list"))
        );
        let twice = format!("{GUARD}{GUARD}");
        assert!(
            matches!(compile(&twice), Err(TablesError::Row(message)) if message.contains("105 is given twice"))
        );
    }

    #[test]
    fn attacks_groups_and_characters_have_their_own_ranges() {
        let cases = [
            (
                "[[attack]]\nid = 68\nname = \"A\"\nki = 1\nanimation_like = 69\n",
                "animation_like is 69",
            ),
            (
                "[[attack]]\nid = 255\nname = \"A\"\nki = 1\nanimation_like = 0\n",
                "attack id is 255",
            ),
            (
                "[[attack]]\nid = 70\nname = \"A\"\nki = 256\nanimation_like = 0\n",
                "attack 70: ki is 256",
            ),
            ("[[group]]\nid = 255\nunits = [1]\n", "group id is 255"),
            ("[[group]]\nid = 112\nunits = [1, 2, 3, 4, 5, 6]\n", "its number of units is 6"),
            ("[[group]]\nid = 112\nunits = []\n", "its number of units is 0"),
            ("[[group]]\nid = 112\nunits = [255]\n", "unit of position 0 is 255"),
            ("[[character]]\nid = 12\ncap = 1\n", "character id is 12"),
            ("[[character]]\nid = 0\ncap = 0\n", "character 0: cap is 0"),
            ("[[character]]\nid = 0\ncap = 101\n", "character 0: cap is 101"),
        ];
        for (text, expected) in cases {
            match compile(text) {
                Err(error) => {
                    assert!(error.to_string().contains(expected), "{error} (want {expected})");
                }
                Ok(_) => panic!("accepted, want {expected}"),
            }
        }
    }

    #[test]
    fn commands_append_joins_the_new_slots_after_the_original_list_read_from_the_rom() {
        // A made-up 1 MiB ROM: unit 2's pointer at 02:B7D2 + 4 leads to a two-slot list at 02:C000.
        let mut rom = vec![0_u8; 0x10_0000];
        let at = |address: usize| 2 * 0x8000 + address - 0x8000;
        rom[at(0xB7D2 + 4)..at(0xB7D2 + 6)].copy_from_slice(&0xC000_u16.to_le_bytes());
        rom[at(0xC000)..at(0xC000) + 9].copy_from_slice(&[2, 0x00, 0x05, 0x10, 0x11, 4, 0, 24, 0]);
        let text = "[[unit]]
id = 2
commands_append = true
[[unit.command]]
id = 0x33
attr = 0xFF
factor = 255
";
        let compiled = compile(text).unwrap();
        assert_eq!(compiled[0].family, Family::CommandAppend);
        let joined = appended_command_list(&rom, 2, &compiled[0].bytes).unwrap();
        assert_eq!(joined, [3, 0x00, 0x05, 0x33, 0x10, 0x11, 0xFF, 4, 0, 24, 0, 255, 0]);
        let both = "[[unit]]
id = 2
commands_only = true
commands_append = true
[[unit.command]]
id = 1
attr = 1
factor = 1
";
        assert!(compile(both).is_err());
    }

    #[test]
    fn a_commands_only_unit_is_the_command_list_of_an_original_unit_and_nothing_else() {
        let text = "[[unit]]\nid = 59\ncommands_only = true\n\
            [[unit.command]]\nid = 0\nattr = 16\nfactor = 8\n\
            [[unit.command]]\nid = 255\nattr = 255\nfactor = 255\n";
        let items = compile(text).unwrap();
        assert_eq!(
            items,
            [Compiled {
                family: Family::CommandList,
                id: 59,
                bytes: vec![2, 0x00, 0xFF, 0x10, 0xFF, 0x08, 0x00, 0xFF, 0x00],
            }],
            "the count, the ids, the attributes, then the factor words; no row, no name"
        );
    }

    #[test]
    fn a_commands_only_unit_refuses_a_stat_a_new_id_and_an_empty_list() {
        let base = "[[unit.command]]\nid = 0\nattr = 16\nfactor = 8\n";
        let with_hp = format!("[[unit]]\nid = 59\ncommands_only = true\nhp = 5\n{base}");
        assert!(
            matches!(compile(&with_hp), Err(TablesError::Row(message)) if message.contains("unit 59: `commands_only` keeps the original's stats, name and sprite, so `hp` is not allowed")),
            "a stat is refused"
        );
        let new = format!("[[unit]]\nid = 105\ncommands_only = true\n{base}");
        assert!(
            matches!(compile(&new), Err(TablesError::Row(message)) if message.contains("unit 105 is a new unit")),
            "a new unit has no list to replace"
        );
        let empty = "[[unit]]\nid = 59\ncommands_only = true\n";
        assert!(
            matches!(compile(empty), Err(TablesError::Row(message)) if message.contains("needs its command list")),
            "the list is required"
        );
    }

    #[test]
    fn a_unit_that_gives_some_of_its_stats_is_refused_and_told_how_to_replace_its_list_alone() {
        let text = GUARD.replace("hp = 300\n", "");
        assert!(
            matches!(compile(&text), Err(TablesError::Row(message)) if message.contains("unit 105: its hp is missing") && message.contains("commands_only = true")),
        );
        let no_name = GUARD.replace("name = \"Guard\"\n", "");
        assert!(
            matches!(compile(&no_name), Err(TablesError::Row(message)) if message.contains("unit 105: its name is missing")),
        );
    }

    #[test]
    fn an_unknown_key_and_a_wrong_type_are_refused_by_the_parser() {
        assert!(matches!(compile("[[unit]]\nid = 105\nfoo = 1\n"), Err(TablesError::Parse(_))));
        assert!(matches!(compile("[[unit]]\nid = \"105\"\n"), Err(TablesError::Parse(_))));
        assert!(matches!(compile("[[banana]]\nid = 1\n"), Err(TablesError::Parse(_))));
        assert!(compile("").unwrap().is_empty(), "an empty file has no slots");
    }
}
