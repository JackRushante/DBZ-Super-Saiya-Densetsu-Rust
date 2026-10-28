//! The playable characters: a name, and per level the points they gain.
//!
//! A character's record is built in a few steps: the rows of its level (and of the levels
//! below, for the cumulative values) are read from `character_level_rows`, and the characters
//! whose id is 8 or more (except the one with its own table, id 11) take their stats from a row
//! of the unit table instead (`fixed_stats_unit`), overriding the level rows. Docs:
//! `docs/re/data/characters.md`.

use crate::names::table_name;
use crate::rom::le;
use crate::text::Name;
use crate::{CatalogueError, RomView};
use ssd_game::data as d;

/// Bytes of one level row.
pub const LEVEL_ROW: usize = 11;
/// The character id of the one character from id 8 up that has a level table of its own.
const OWN_TABLE_ID: u8 = 11;
/// The characters from this id on take their stats from the unit table, from the row
/// `id + UNIT_ROW_SKEW` (the exception: [`OWN_TABLE_ID`]).
const FIRST_FIXED_ID: u8 = 8;
/// Added to a character id (8 and up) to give its row of the unit table.
const UNIT_ROW_SKEW: u8 = 26;

/// The table of level caps (bank 3): one byte per character id, the highest level the character reaches. It is not a named
/// symbol: the original reads it at two places (`docs/re/data/tables.md`, row 16).
const LEVEL_CAPS: u32 = 0x03_850E;

/// What a character gains on reaching a level (one row of the level table).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LevelRow {
    /// The level (1 to 48; 1 to 3 for the character with the short table).
    pub level: u8,
    /// Battle power gained on reaching the level; the row of level 1 is the starting value.
    pub bp_gain: u32,
    /// The EXP total at which the level begins: the game writes it into the record's EXP field
    /// when it builds a record at that level, and compares EXP with the next row to level up.
    pub exp_start: u32,
    /// Maximum HP at the level (the value itself, not a gain).
    pub max_hp: u16,
    /// Maximum KI gained on reaching the level; the row of level 1 is the starting value.
    pub ki_gain: u16,
    /// Byte 10 of the row, stored in byte $0E of the record (1 at the start, 9 to 12 at the
    /// top; its use is not known).
    pub unknown_0a: u8,
}

impl LevelRow {
    fn parse(level: u8, row: &[u8; LEVEL_ROW]) -> Self {
        Self {
            level,
            bp_gain: le(&row[0..3]),
            exp_start: le(&row[3..6]),
            max_hp: u16::try_from(le(&row[6..8])).expect("two bytes"),
            ki_gain: u16::try_from(le(&row[8..10])).expect("two bytes"),
            unknown_0a: row[10],
        }
    }
}

/// A playable character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Character {
    /// The character id (byte `$1C` of its record).
    pub id: u8,
    /// The name, from the string table.
    pub name: Name,
    /// The unit id of the character (`party_unit_ids`): the id the unit tables and the unit
    /// name table use for it.
    pub unit_id: u8,
    /// The levels, 48 rows (3 for id 11, whose table is short); empty for the characters whose
    /// stats come from the unit table.
    pub levels: Vec<LevelRow>,
    /// The row of the unit table that gives the stats of this character instead of the level
    /// rows (ids 8, 9 and 10).
    pub fixed_stats_unit: Option<u8>,
    /// The highest level the character reaches (47 or 48; 3 for the character with the short table): the level-up check
    /// stops there, and the last row of the table is no level to reach.
    pub level_cap: u8,
}

impl Character {
    /// The starting values (level 1) of a character with level rows: BP, max HP, max KI.
    #[must_use]
    pub fn start(&self) -> Option<(u32, u16, u16)> {
        self.levels.first().map(|row| (row.bp_gain, row.max_hp, row.ki_gain))
    }
}

pub(crate) fn read(rom: RomView<'_>) -> Result<Vec<Character>, CatalogueError> {
    let malformed =
        |message: String| CatalogueError::Malformed { table: "character_level_pointers", message };
    let rows_end = d::CHARACTER_LEVEL_ROWS + d::CHARACTER_LEVEL_ROWS_SIZE;
    let count = d::PARTY_UNIT_IDS_SIZE;
    let pointers: Vec<u32> = (0..d::CHARACTER_LEVEL_POINTERS_SIZE / 2)
        .map(|id| rom.pointer(d::CHARACTER_LEVEL_POINTERS + 2 * id, 0x02))
        .collect::<Result<_, _>>()?;
    let mut characters = Vec::new();
    for id in 0..count {
        let id8 = u8::try_from(id).expect("a dozen characters");
        let pointer = *pointers
            .get(id as usize)
            .ok_or_else(|| malformed(format!("no level pointer for character {id}")))?;
        let fixed = id8 >= FIRST_FIXED_ID && id8 != OWN_TABLE_ID;
        let levels = if fixed {
            Vec::new()
        } else {
            // A table runs to the start of the next one in the ROM, or to the end of the rows.
            let end = pointers.iter().copied().filter(|&p| p > pointer).min().unwrap_or(rows_end);
            let len = end.saturating_sub(pointer) as usize;
            if pointer < d::CHARACTER_LEVEL_ROWS
                || pointer >= rows_end
                || len == 0
                || !len.is_multiple_of(LEVEL_ROW)
            {
                return Err(malformed(format!(
                    "character {id}: ${pointer:06X} is not the start of a level table"
                )));
            }
            rom.slice(pointer, len)?
                .as_chunks::<LEVEL_ROW>()
                .0
                .iter()
                .zip(1_u8..)
                .map(|(row, level)| LevelRow::parse(level, row))
                .collect()
        };
        characters.push(Character {
            id: id8,
            name: table_name(rom, d::STRING_POINTERS, id)?,
            unit_id: rom.u8(d::PARTY_UNIT_IDS + id)?,
            levels,
            fixed_stats_unit: fixed.then(|| id8 + UNIT_ROW_SKEW),
            level_cap: rom.u8(LEVEL_CAPS + id)?,
        });
    }
    Ok(characters)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::rom_with;

    const FULL: usize = 48;
    const SHORT: usize = 3;

    /// Eight tables of 48 levels, then one of 3, as the real ROM lays them out; the ids from 8
    /// but 11 share the first table; and the names "Goku" and "Pa" for ids 0 and 1.
    fn rom() -> Vec<u8> {
        let table = |levels: usize| -> Vec<u8> {
            (0..levels)
                .flat_map(|level| {
                    let level = u8::try_from(level).unwrap();
                    // BP gain 100 + level, EXP level, HP 50 + level, KI 4, byte 1.
                    [100 + level, 0, 0, level, 0, 0, 50 + level, 0, 4, 0, 1]
                })
                .collect()
        };
        let mut rows = Vec::new();
        for _ in 0..8 {
            rows.extend(table(FULL));
        }
        rows.extend(table(SHORT));
        assert_eq!(rows.len(), d::CHARACTER_LEVEL_ROWS_SIZE as usize);
        let at = |table: usize| {
            u16::try_from(d::CHARACTER_LEVEL_ROWS & 0xFFFF).unwrap()
                + u16::try_from(table * FULL * LEVEL_ROW).unwrap()
        };
        let tables = [0, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 8, 0];
        let pointers: Vec<u8> = tables.iter().flat_map(|&t| at(t).to_le_bytes()).collect();
        rom_with(&[
            (d::CHARACTER_LEVEL_ROWS, &rows),
            (d::CHARACTER_LEVEL_POINTERS, &pointers),
            (d::PARTY_UNIT_IDS, &[0, 9, 1, 10, 11, 12, 13, 0x50, 0x22, 0x23, 0x24, 0x61]),
            (LEVEL_CAPS, &[47, 47, 47, 47, 47, 47, 47, 48, 48, 48, 48, 3]),
            (d::STRING_POINTERS, &[0x00, 0xAC, 0x05, 0xAC]),
            (0x02_AC00, &[4, 0xED, 0x1B, 0x17, 0x55, 0x01]),
            (0x02_AC05, &[2, 0x8B, 0x05]),
        ])
    }

    #[test]
    fn a_character_has_its_levels_read_from_its_table() {
        let rom = rom();
        let characters = read(RomView::new(&rom).unwrap()).unwrap();
        assert_eq!(characters.len(), 12);
        let goku = &characters[0];
        assert_eq!(goku.name.text, "Goku");
        assert_eq!(goku.unit_id, 0);
        assert_eq!(goku.levels.len(), FULL);
        assert_eq!(
            goku.levels[2],
            LevelRow {
                level: 3,
                bp_gain: 102,
                exp_start: 2,
                max_hp: 52,
                ki_gain: 4,
                unknown_0a: 1
            }
        );
        assert_eq!(goku.start(), Some((100, 50, 4)));
        assert_eq!(characters[1].name.text, "Pa");
        assert_eq!(characters[1].unit_id, 9);
        assert_eq!(characters[7].levels.len(), FULL, "a table ends where the next begins");
        assert_eq!(
            (goku.level_cap, characters[7].level_cap, characters[11].level_cap),
            (47, 48, 3)
        );
    }

    #[test]
    fn the_last_table_ends_with_the_rows_and_is_short() {
        let rom = rom();
        let characters = read(RomView::new(&rom).unwrap()).unwrap();
        assert_eq!(characters[11].levels.len(), SHORT);
        assert_eq!(characters[11].fixed_stats_unit, None);
    }

    #[test]
    fn the_characters_from_id_8_but_11_take_a_row_of_the_unit_table() {
        let rom = rom();
        let characters = read(RomView::new(&rom).unwrap()).unwrap();
        for (id, character) in characters.iter().enumerate().skip(8).filter(|&(id, _)| id != 11) {
            assert_eq!(character.levels.len(), 0);
            assert_eq!(usize::from(character.fixed_stats_unit.unwrap()), id + 26);
            assert_eq!(character.start(), None);
        }
    }

    #[test]
    fn a_level_pointer_outside_the_table_is_malformed() {
        let mut rom = rom();
        let view = RomView::new(&rom).unwrap();
        let at = view.offset(d::CHARACTER_LEVEL_POINTERS).unwrap();
        // Id 0 now points one byte into its table: its length is no whole number of rows.
        rom[at] += 1;
        let error = read(RomView::new(&rom).unwrap()).unwrap_err();
        assert!(error.to_string().contains("character 0"), "{error}");
    }
}
