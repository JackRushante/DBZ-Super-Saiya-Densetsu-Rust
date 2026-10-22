//! The game's tables as the battery-save tools use them, and the party records the game builds from them: the record of a
//! character at a level (the battle power and KI of every row up to the level added, the rest from the row of the level), and the
//! record of a character whose stats come from a unit row. `save-edit` (xtask) and the chapters of a data pack (`ssd_mod::chapter`)
//! both build their records here, so the two give the same bytes. Docs: `docs/re/data/characters.md`, `save-format.md`.

use crate::{Catalogue, Character, Enemy, RomView};
use anyhow::{Context, bail};
use ssd_mod::chapter::Tables;
use ssd_save::{Item, Record};

/// The game's tables the named fields need.
pub struct Game {
    pub catalogue: Catalogue,
    /// The attribute byte of each object id (`item_attributes`).
    item_attributes: Vec<u8>,
}

impl Game {
    /// The tables from a catalogue and the item attribute bytes.
    #[must_use]
    pub fn new(catalogue: Catalogue, item_attributes: Vec<u8>) -> Self {
        Self { catalogue, item_attributes }
    }

    /// Reads the tables from the ROM image.
    ///
    /// # Errors
    ///
    /// The ROM is not a valid image.
    pub fn from_rom(rom: &[u8]) -> anyhow::Result<Self> {
        let catalogue = Catalogue::from_rom(rom).context("reading the catalogue")?;
        let view = RomView::new(rom).context("the ROM image")?;
        let item_attributes = view
            .slice(ssd_game::data::ITEM_ATTRIBUTES, ssd_game::data::ITEM_ATTRIBUTES_SIZE as usize)
            .context("the item attribute table")?
            .to_vec();
        Ok(Self::new(catalogue, item_attributes))
    }

    /// A character by name (any case) or by id.
    /// # Errors
    ///
    /// `spec` is neither a character id nor the name of a character.
    pub fn character(&self, spec: &str) -> anyhow::Result<&Character> {
        let found = match spec.parse::<usize>() {
            Ok(id) => self.catalogue.characters.get(id),
            Err(_) => self
                .catalogue
                .characters
                .iter()
                .find(|c| c.name.trimmed().eq_ignore_ascii_case(spec)),
        };
        found.with_context(|| {
            let names: Vec<&str> =
                self.catalogue.characters.iter().map(|c| c.name.trimmed()).collect();
            format!("no character `{spec}` (ids 0 to 11 or {})", names.join(", "))
        })
    }

    /// A record for a character with no level table, from its unit row.
    /// # Errors
    ///
    /// The character's unit row is not in the unit table.
    pub fn unit_record(&self, character: &Character, join_order: u8) -> anyhow::Result<Record> {
        let unit = character.fixed_stats_unit.unwrap_or(character.unit_id);
        let enemy = self
            .catalogue
            .enemies
            .get(usize::from(unit))
            .with_context(|| format!("unit {unit} is not in the unit table"))?;
        let mut record = Record::blank();
        record.set_flags(0xC0);
        record.set_unit_id(character.unit_id);
        record.set_character_id(character.id);
        record.0[0x17] = enemy.unknown_09;
        record.0[0x1B] = join_order;
        set_from_unit(&mut record, enemy);
        Ok(record)
    }

    /// An item by object id, or by name (any case, without the quotes some names have): the
    /// first object of that name, or the Nth with a suffix `#N` (the table has two `Turtle`s).
    /// The attribute byte is the one the game keeps for the object.
    /// # Errors
    ///
    /// `spec` names no object, or the object has no attribute byte.
    pub fn item(&self, spec: &str) -> anyhow::Result<Item> {
        let id = match spec.parse::<u8>() {
            Ok(id) => id,
            Err(_) => {
                let (text, nth) = match spec.split_once('#') {
                    Some((text, n)) => (
                        text,
                        n.trim()
                            .parse::<usize>()
                            .map_err(|_| anyhow::anyhow!("`{spec}`: `{n}` is not a number"))?
                            .max(1),
                    ),
                    None => (spec, 1),
                };
                self.catalogue
                    .items
                    .iter()
                    .filter(|i| i.name.trimmed().trim_matches('"').eq_ignore_ascii_case(text))
                    .nth(nth - 1)
                    .with_context(|| format!("no object called `{spec}`"))?
                    .id
            }
        };
        let attr = *self
            .item_attributes
            .get(usize::from(id))
            .with_context(|| format!("object id {id} is not in the item attribute table"))?;
        Ok(Item { id, attr })
    }
}

/// The values of a record at `level`, the way the game builds one: the battle power and KI of
/// every row up to the level added, the rest taken from the row of the level.
pub struct Stats {
    pub bp: u32,
    pub max_hp: u16,
    pub max_ki: u16,
    pub exp: u32,
    pub byte_0e: u8,
}

/// The values of a record at `level` for `character` (see [`Stats`]).
///
/// # Errors
///
/// The character has no level table, or `level` is not one of its rows.
pub fn stats_at(character: &Character, level: u8) -> anyhow::Result<Stats> {
    if character.levels.is_empty() {
        bail!(
            "{} has no level table (its stats come from a unit row): set its fields one by one",
            character.name.trimmed()
        );
    }
    let rows = &character.levels[..usize::from(level).min(character.levels.len())];
    let Some(last) =
        rows.last().filter(|_| level >= 1 && usize::from(level) <= character.levels.len())
    else {
        bail!(
            "{}: level {level} is not between 1 and {}",
            character.name.trimmed(),
            character.levels.len()
        );
    };
    Ok(Stats {
        bp: rows.iter().map(|r| r.bp_gain).sum(),
        max_hp: last.max_hp,
        max_ki: rows.iter().map(|r| r.ki_gain).sum(),
        exp: last.exp_start,
        byte_0e: last.unknown_0a,
    })
}

/// Puts the stats of the unit row `enemy` in `record` (HP and KI full): the way the game builds the
/// records of the characters that have no level table (Nail, Tsumuri and Maima).
fn set_from_unit(record: &mut Record, enemy: &Enemy) {
    record.set_level(enemy.level);
    record.set_bp(enemy.bp);
    record.set_max_hp(enemy.hp);
    record.set_hp(enemy.hp);
    record.set_max_ki(enemy.ki);
    record.set_ki(enemy.ki);
    record.set_byte_0e(enemy.unknown_0a);
}

/// Puts the stats of `level` in `record` (HP and KI full).
/// # Errors
///
/// `level` is not one of the character's rows (see [`stats_at`]).
pub fn set_level(record: &mut Record, character: &Character, level: u8) -> anyhow::Result<()> {
    let stats = stats_at(character, level)?;
    record.set_level(level);
    record.set_bp(stats.bp);
    record.set_max_hp(stats.max_hp);
    record.set_hp(stats.max_hp);
    record.set_max_ki(stats.max_ki);
    record.set_ki(stats.max_ki);
    record.set_exp(stats.exp);
    record.set_byte_0e(stats.byte_0e);
    Ok(())
}

/// A record for a character who joins at `level`. Bytes the game sets that this does not
/// decode: `+17` is 30 in every record seen, `+1B` is 1, 2 and 3 for Goku, Piccolo and Vegeta
/// (the order they join in: `join_order` is used), and `+14` to `+16` start at zero.
/// # Errors
///
/// `level` is not one of the character's rows, or the character has no level table.
pub fn new_record(character: &Character, level: u8, join_order: u8) -> anyhow::Result<Record> {
    let mut record = Record::blank();
    record.set_flags(0xC0);
    record.set_unit_id(character.unit_id);
    record.set_character_id(character.id);
    record.0[0x17] = 0x1E;
    record.0[0x1B] = join_order;
    set_level(&mut record, character, level)?;
    Ok(record)
}

impl Tables for Game {
    fn record(&self, character: u8, level: u8, join_order: u8) -> Result<Record, String> {
        let character =
            self.character(&character.to_string()).map_err(|error| format!("{error:#}"))?;
        if character.levels.is_empty() {
            // The stats of a unit row: the level is the unit's, not a choice.
            let record =
                self.unit_record(character, join_order).map_err(|error| format!("{error:#}"))?;
            if record.level() != level {
                return Err(format!(
                    "{} takes its stats from its unit row (level {}): the level cannot be {level}",
                    character.name.trimmed(),
                    record.level()
                ));
            }
            return Ok(record);
        }
        new_record(character, level, join_order).map_err(|error| format!("{error:#}"))
    }

    fn item_attribute(&self, object: u8) -> Result<u8, String> {
        self.item_attributes
            .get(usize::from(object))
            .copied()
            .ok_or_else(|| format!("object {object} is not in the item attribute table"))
    }
}
