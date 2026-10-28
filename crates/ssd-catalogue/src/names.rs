//! The strings of bank `$02` and the tables of pointers to them: names of characters, units,
//! attacks and objects. A string is a length byte followed by that many tile codes.

use crate::text::Name;
use crate::{CatalogueError, RomView};
use ssd_game::data as d;

/// The bank of every name pointer table.
pub(crate) const TEXT_BANK: u8 = 0x02;

/// The string at `pointer`: empty when the pointer does not lead into the ROM (a few entries of
/// the tables point at nothing) or the string would leave its bank.
pub(crate) fn name_at(rom: RomView<'_>, pointer: u32) -> Name {
    let Ok(len) = rom.u8(pointer) else { return Name::default() };
    match rom.slice(pointer + 1, usize::from(len)) {
        Ok(tiles) => Name::from_tiles(tiles),
        Err(_) => Name::default(),
    }
}

/// The `index`th name of the table of word pointers at `table`.
pub(crate) fn table_name(rom: RomView<'_>, table: u32, index: u32) -> Result<Name, CatalogueError> {
    Ok(name_at(rom, rom.pointer(table + 2 * index, TEXT_BANK)?))
}

/// A special attack: its name and its KI cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attack {
    /// Index in the name table (the id the text engine takes).
    pub id: u8,
    /// The name; empty for the two entries that point at nothing, and partly or wholly kana for
    /// the attacks the translation left in Japanese (see [`Name::is_text`]).
    pub name: Name,
    /// The KI the attack costs (the command cost table, `03:884B`, one byte per command id; the
    /// menu's own KI-USE figure). The table is not a named symbol: it is read at one place by the
    /// original, see `docs/re/data/kaioken-bp.md`.
    pub ki_cost: u8,
}

/// The command cost table (bank 3): one byte per command id, as many as the attack names.
const COMMAND_COSTS: u32 = 0x03_884B;

/// A named person or object of the story: the table the text engine reads for the second kind
/// of name. It holds the key items (Scouter, Radar, the seven dragon balls...) next to the
/// characters of the story, so it is the closest the game has to an item list. Its attribute,
/// price, effect and help are the item catalogue of `docs/re/data/menus.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// Index in the name table, which is also the object id of the item list.
    pub id: u8,
    /// The name.
    pub name: Name,
    /// The attribute byte: bit 7 usable in a battle (INT), bit 6 on the map (EXT), bit 5 a story
    /// helper that is never used from the menu (AID).
    pub attribute: u8,
    /// The price in Pt; `None` for an object without one (`$FF` in `item_prices`).
    pub price: Option<u8>,
    /// The effect routine of the object (a word of bank `$01`), for the ids that have one.
    pub effect: Option<u16>,
    /// The first page of the help text: its lines, at most three.
    pub help: Vec<Name>,
}

impl Item {
    /// Usable in a battle (bit 7 of the attribute).
    #[must_use]
    pub fn usable_in_battle(&self) -> bool {
        self.attribute & 0x80 != 0
    }

    /// Usable on the map (bit 6 of the attribute).
    #[must_use]
    pub fn usable_on_map(&self) -> bool {
        self.attribute & 0x40 != 0
    }
}

pub(crate) fn read_attacks(rom: RomView<'_>) -> Result<Vec<Attack>, CatalogueError> {
    (0..d::ATTACK_NAME_POINTERS_SIZE / 2)
        .map(|i| {
            Ok(Attack {
                id: u8::try_from(i).expect("fewer than 256 attacks"),
                name: table_name(rom, d::ATTACK_NAME_POINTERS, i)?,
                ki_cost: rom.u8(COMMAND_COSTS + i)?,
            })
        })
        .collect()
}

/// The lines of one page of help text at most.
const HELP_LINES: usize = 3;
/// The tile that ends a line of help text.
const LINE_END: u8 = 0xFF;
/// The bank of the help texts (the pointers of `item_help_pointers` are its words).
const HELP_BANK: u8 = 0x07;
/// The price of an object without one.
const NO_PRICE: u8 = 0xFF;
/// The objects of the effect table: the words of `item_effect_table`, one per id from 0.
const EFFECT_COUNT: u32 = d::ITEM_EFFECT_TABLE_SIZE / 2;

pub(crate) fn read_items(rom: RomView<'_>) -> Result<Vec<Item>, CatalogueError> {
    (0..d::OBJECT_NAME_POINTERS_SIZE / 2)
        .map(|i| {
            let price = rom.u8(d::ITEM_PRICES + i)?;
            Ok(Item {
                id: u8::try_from(i).expect("fewer than 256 objects"),
                name: table_name(rom, d::OBJECT_NAME_POINTERS, i)?,
                attribute: rom.u8(d::ITEM_ATTRIBUTES + i)?,
                price: (price != NO_PRICE).then_some(price),
                effect: (i < EFFECT_COUNT)
                    .then(|| rom.u16(d::ITEM_EFFECT_TABLE + 2 * i))
                    .transpose()?,
                help: help_page(rom, i).unwrap_or_default(),
            })
        })
        .collect()
}

/// The first page of the help text of object `i`: its lines, up to [`HELP_LINES`], each ended by
/// `$FF`, the page by an empty line. A pointer that leads nowhere gives no help.
fn help_page(rom: RomView<'_>, i: u32) -> Result<Vec<Name>, CatalogueError> {
    let mut address = rom.pointer(d::ITEM_HELP_POINTERS + 2 * i, HELP_BANK)?;
    let mut lines = Vec::new();
    while lines.len() < HELP_LINES {
        let mut line = Vec::new();
        loop {
            let tile = rom.u8(address)?;
            address = next_address(address);
            if tile == LINE_END {
                break;
            }
            line.push(tile);
        }
        if line.is_empty() {
            break;
        }
        lines.push(Name::from_tiles(&line));
    }
    Ok(lines)
}

/// The next byte of the ROM after `address`, crossing into the next bank as the CPU would
/// (an address past `$xxFFFF` continues at `$(xx+1)8000`).
pub(crate) fn next_address(address: u32) -> u32 {
    let next = address + 1;
    if next & 0xFFFF == 0 { (next & !0xFFFF) | 0x8000 } else { next }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::rom_with;

    #[test]
    fn a_string_is_a_length_and_that_many_tiles() {
        // "Kami" then a pointer to it.
        let rom = rom_with(&[
            (0x02_B129, &[4, 0x8D, 0x05, 0x19, 0x15]),
            (d::OBJECT_NAME_POINTERS, &[0x29, 0xB1]),
        ]);
        let view = RomView::new(&rom).unwrap();
        let items = read_items(view).unwrap();
        assert_eq!(items.len(), 40);
        assert_eq!(items[0].name.text, "Kami");
        assert_eq!(items[0].id, 0);
    }

    #[test]
    fn a_pointer_into_nothing_gives_an_empty_name() {
        let rom = rom_with(&[(d::ATTACK_NAME_POINTERS, &[0x70, 0x5F])]);
        let view = RomView::new(&rom).unwrap();
        let attacks = read_attacks(view).unwrap();
        assert_eq!(attacks.len(), 69);
        assert!(attacks[0].name.tiles.is_empty(), "$5F70 is below the ROM window");
        assert_eq!(attacks[0].name, Name::default());
    }

    #[test]
    fn a_string_that_would_leave_its_bank_is_empty() {
        let rom = rom_with(&[(0x02_FFFF, &[200])]);
        let view = RomView::new(&rom).unwrap();
        assert_eq!(name_at(view, 0x02_FFFF), Name::default());
    }
}
