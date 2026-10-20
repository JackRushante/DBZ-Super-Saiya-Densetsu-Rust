//! The string the message engine is writing, read from work RAM and the ROM (`docs/re/data/text-engine.md`): which
//! string, page, line and cell, where the window's text starts, how far the page is written, and what the name and
//! number slots hold. A translation drawn over the original's window ([`crate::translation`]) is keyed and paced
//! by it. Nothing is written; nothing of the ROM's text is kept beyond the call.

use crate::names::table_name;
use crate::rom::RomView;
use crate::text::decode_tile;
use crate::translation::{page_key, render, string_key};
use ssd_game::data as d;
use std::collections::HashMap;

/// A string longer than this has lost its end.
const MAX_STRING: usize = 2048;
/// The cells a name or a phrase is taken to have where the ROM does not say (its slot is empty).
const SLOT_CELLS: usize = 6;

/// What the engine shows: the coordinates of the page being written and how far it is.
#[derive(Debug, Clone, PartialEq)]
pub struct OnScreen {
    /// `message_table` and `message_string`.
    pub table: u8,
    pub string: u16,
    /// The page of the string (0 the first) and its key in a translation (`str3:0x3E/p2`).
    pub page: usize,
    pub key: String,
    /// The low nibble of `message_state`: 1 writing, 2 and 3 waiting for A, 5 writing a name, 6 a timed wait; 0 for
    /// a message written in one frame (`message_state` bit 5 or 6).
    pub state: u8,
    /// `message_line` and `counter_071f`: where the next cell goes.
    pub line: u8,
    pub cell: u8,
    /// `message_text_base`: the tile-map offset (bytes, 32 cells of 2 bytes a row) of the text's first cell.
    pub base: u16,
    /// The page shows a portrait: its first line starts one cell to the left.
    pub portrait: bool,
    /// The cells of each line of the page in the original (spaces and slots counted).
    pub lines: Vec<usize>,
    /// The share of the page's cells written, 0 to 1 (1 when the page waits or was written at once).
    pub progress: f32,
    /// The names in the slots of `{name:1}` to `{name:3}`, when a slot holds one.
    pub names: [Option<NameSlot>; 3],
    /// The numbers in the slots of `{num:1}` to `{num:3}` (empty when a slot holds none).
    pub numbers: [String; 3],
}

/// A name in a slot: its key in a translation (`unit:0x02`, `object:7`, `attack:12`) and the original's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NameSlot {
    pub key: String,
    pub english: String,
}

impl OnScreen {
    /// The tile-map cell (column, row) of the text's first cell.
    #[must_use]
    pub fn base_cell(&self) -> (u16, u16) {
        ((self.base % 64) / 2, self.base / 64)
    }
}

/// The byte of work RAM at the bus address `address` (bank `$7E`).
fn byte(wram: &[u8], address: u32) -> Option<u8> {
    wram.get((address - 0x7E_0000) as usize).copied()
}

fn word(wram: &[u8], address: u32) -> Option<u16> {
    Some(u16::from(byte(wram, address)?) | u16::from(byte(wram, address + 1)?) << 8)
}

/// What the engine is writing, from the work RAM `wram` at the end of a frame and the ROM image `rom`; `None` when
/// no message is up, the engine is between two strings (state 0, 4: its bytes still name the last string), or the
/// string cannot be read. `hint` is the page shown before, for a string longer than the 256 bytes its offset
/// counts.
#[must_use]
pub fn on_screen(rom: &[u8], wram: &[u8], hint: Option<usize>) -> Option<OnScreen> {
    let rom = RomView::new(rom).ok()?;
    let status = byte(wram, d::MESSAGE_STATE)?;
    let instant = status & 0x60 != 0;
    let state = status & 0x0F;
    if status & 0x80 == 0 || !(instant || matches!(state, 1 | 2 | 3 | 5 | 6)) {
        return None;
    }
    let table = byte(wram, d::MESSAGE_TABLE)?;
    let string = word(wram, d::MESSAGE_STRING)?;
    let names: [Option<NameSlot>; 3] = std::array::from_fn(|n| {
        let slot = word(wram, d::MESSAGE_NAME_SLOTS + 2 * n as u32)?;
        name_slot(rom, slot)
    });
    let numbers: [String; 3] = std::array::from_fn(|n| {
        let at = (d::MESSAGE_NUMBER_SLOTS - 0x7E_0000) as usize + 16 * n;
        wram.get(at..at + 16).map(number).unwrap_or_default()
    });
    let start = string_address(rom, table, string)?;
    let slot_cells = |code: u8| -> usize {
        match code {
            0xF9..=0xFB => names[usize::from(0xFB - code)]
                .as_ref()
                .map_or(SLOT_CELLS, |name| name.english.chars().count()),
            0xF6..=0xF8 => numbers[usize::from(0xF8 - code)].chars().count(),
            _ => SLOT_CELLS,
        }
    };
    let pages = pages(rom, start, if table == 1 { 2 } else { 1 }, &slot_cells)?;
    let offset = usize::from(byte(wram, d::COUNTER_071D)?);
    // In state 2 the offset is already past the page's `FE FF`
    let at = if state == 2 { offset.wrapping_sub(2) & 0xFF } else { offset };
    let page = page_of(&pages, at, hint)?;
    let (line, cell) = (byte(wram, d::MESSAGE_LINE)?, byte(wram, d::COUNTER_071F)?);
    let lines = pages[page].lines.clone();
    let progress = if instant || !matches!(state, 1 | 5) {
        1.0
    } else {
        let total: usize = lines.iter().sum();
        // While a name is written (state 5) the cell does not move: the slot's code is still the next byte, and
        // its name counts as written
        let naming = match rom.u8(start + offset as u32) {
            Ok(code @ (0xF6..=0xFB | 0xE0..=0xE7)) if state == 5 => slot_cells(code),
            _ => 0,
        };
        let written: usize =
            lines.iter().take(usize::from(line)).sum::<usize>() + usize::from(cell) + naming;
        if total == 0 { 1.0 } else { (written as f32 / total as f32).min(1.0) }
    };
    Some(OnScreen {
        table,
        string,
        page,
        key: page_key(&string_key(table, u32::from(string)), page),
        state: if instant { 0 } else { state },
        line,
        cell,
        base: word(wram, d::MESSAGE_TEXT_BASE)?,
        portrait: pages[page].portrait,
        lines,
        progress,
        names,
        numbers,
    })
}

/// A table of bank `$02` names: its pointers, their size in bytes, and the translation key of an index.
type NameTable = (u32, u32, fn(u32) -> String);

/// The names the game writes straight into its tile maps (the battle's lists of fighters, attacks and objects, the
/// characters' names), as the screen spells them, and their translations in `table`: the strings of the four name
/// tables of bank `$02` whose keys (`string:N`, `unit:0xNN`, `attack:N`, `object:N`) the translation has. A name
/// shorter than two letters, or one the tables spell two ways, is left out (a single letter on screen is not a
/// name). Nothing of the ROM is kept but the map.
#[must_use]
pub fn screen_names(
    rom: &[u8],
    table: &crate::translation::Translation,
) -> HashMap<String, String> {
    let Ok(rom) = RomView::new(rom) else { return HashMap::new() };
    let tables: [NameTable; 4] = [
        (d::STRING_POINTERS, d::STRING_POINTERS_SIZE, |i| format!("string:{i}")),
        (d::UNIT_NAME_POINTERS, d::UNIT_NAME_POINTERS_SIZE, |i| format!("unit:0x{i:02X}")),
        (d::ATTACK_NAME_POINTERS, d::ATTACK_NAME_POINTERS_SIZE, |i| format!("attack:{i}")),
        (d::OBJECT_NAME_POINTERS, d::OBJECT_NAME_POINTERS_SIZE, |i| format!("object:{i}")),
    ];
    let mut names: HashMap<String, Option<String>> = HashMap::new();
    for (pointers, size, key) in tables {
        for index in 0..size / 2 {
            let Ok(name) = table_name(rom, pointers, index) else { continue };
            let english = name.trimmed();
            let Some(text) = table.text(&key(index)) else { continue };
            let translated = render(text, |_| String::new()).replace('\n', "");
            if english.chars().count() < 2 || !name.is_text() || translated.trim().is_empty() {
                continue;
            }
            names
                .entry(english.to_owned())
                .and_modify(|seen| {
                    if seen.as_deref() != Some(translated.as_str()) {
                        *seen = None;
                    }
                })
                .or_insert(Some(translated));
        }
    }
    // The menu blocks of bank `$02` (`block:0xNN`): a row of a block is a word of its own (`Atk.`, `Pick`)
    for index in 0..MENU_BLOCKS {
        let Some(text) = table.text(&format!("block:0x{index:02X}")) else { continue };
        let translated = render(text, |_| String::new());
        let Some(rows) = block_rows(rom, index) else { continue };
        for (english, translated) in rows.iter().zip(translated.split('\n')) {
            let (english, translated) = (english.trim(), translated.trim());
            if english.chars().count() < 2 || translated.is_empty() || english.contains('\u{fffd}')
            {
                continue;
            }
            names
                .entry(english.to_owned())
                .and_modify(|seen| {
                    if seen.as_deref() != Some(translated) {
                        *seen = None;
                    }
                })
                .or_insert(Some(translated.to_owned()));
        }
    }
    // (a name the translation keeps as it is, `Lv` or `HP`, is left to the HD letters)
    names
        .into_iter()
        .filter_map(|(english, translated)| Some((english, translated?)))
        .filter(|(english, translated)| english != translated)
        .collect()
}

/// The blocks of `bank2_pointer_table` that can be text (`docs/re/data/menus.md`, `xtask text-dump`).
const MENU_BLOCKS: u32 = 80;

/// The rows of menu block `index` as the screen spells them (a tile the HD font has no letter for is U+FFFD); `None`
/// for a block that is not text (no place in the tile map, or less than half letters).
fn block_rows(rom: RomView<'_>, index: u32) -> Option<Vec<String>> {
    let address = rom.pointer(d::BANK2_POINTER_TABLE + 2 * index, 0x02).ok()?;
    let head = rom.slice(address, 4).ok()?;
    let (width, height) = (usize::from(head[0]) + 1, usize::from(head[1]) + 1);
    let offset = u16::from_le_bytes([head[2], head[3]]);
    let tiles = rom.slice(address + 4, width * height).ok()?;
    let letters = tiles.iter().filter(|&&tile| decode_tile(tile).is_some()).count();
    if offset == 0 || letters * 2 < tiles.len() {
        return None;
    }
    let spell =
        |row: &[u8]| row.iter().map(|&tile| decode_tile(tile).unwrap_or('\u{fffd}')).collect();
    Some(tiles.chunks(width).map(spell).collect())
}

/// The bus address of string `index` of the `message_table` value `table`: `string_table_pointers` gives the
/// table (a word, then the bank), whose words point into its bank.
fn string_address(rom: RomView<'_>, table: u8, index: u16) -> Option<u32> {
    if table > 7 {
        return None;
    }
    let entry = d::STRING_TABLE_POINTERS + 3 * u32::from(table);
    let pointers = u32::from(rom.u16(entry).ok()?) | u32::from(rom.u8(entry + 2).ok()?) << 16;
    rom.pointer(pointers + 2 * u32::from(index), (pointers >> 16) as u8).ok()
}

/// The name a slot word holds (the first byte the index, the second the type: bit 7 a unit, 6 an object, 5 an
/// attack); `None` for an empty slot or another type.
fn name_slot(rom: RomView<'_>, slot: u16) -> Option<NameSlot> {
    let (index, kind) = (slot & 0xFF, slot >> 8);
    let (key, table) = if kind & 0x80 != 0 {
        (format!("unit:0x{index:02X}"), d::UNIT_NAME_POINTERS)
    } else if kind & 0x40 != 0 {
        (format!("object:{index}"), d::OBJECT_NAME_POINTERS)
    } else if kind & 0x20 != 0 {
        (format!("attack:{index}"), d::ATTACK_NAME_POINTERS)
    } else {
        return None;
    };
    let english = table_name(rom, table, u32::from(index)).ok()?.trimmed().to_owned();
    Some(NameSlot { key, english })
}

/// The text of a number slot: its tiles up to `FF`, spaces trimmed.
fn number(tiles: &[u8]) -> String {
    let text: String = tiles
        .iter()
        .take_while(|&&tile| tile != 0xFF)
        .filter_map(|&tile| decode_tile(tile))
        .collect();
    text.trim().to_owned()
}

/// One page of a string: the byte offset (from the string's start) of its first byte, its lines' cells, and whether
/// it shows a portrait.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Page {
    start: usize,
    lines: Vec<usize>,
    portrait: bool,
}

/// The pages of the string at `start`, as `01:CFB4` reads it (`portrait_operands` is 2 in table 1); `slot_cells`
/// gives the cells of a name, number or phrase code. `None` when the string leaves the ROM or has no end.
fn pages(
    rom: RomView<'_>,
    start: u32,
    portrait_operands: usize,
    slot_cells: &dyn Fn(u8) -> usize,
) -> Option<Vec<Page>> {
    let mut pages = vec![Page { start: 0, lines: vec![0], portrait: false }];
    let mut at = 0;
    while at < MAX_STRING {
        let code = rom.u8(start + at as u32).ok()?;
        at += 1;
        let page = pages.last_mut()?;
        let line = page.lines.last_mut()?;
        match code {
            0xFF if rom.u8(start + at as u32).ok()? == 0xFF => return Some(pages),
            0xFF => page.lines.push(0),
            // `FE FF`: the next page starts after the `FF`
            0xFE => {
                at += 1;
                pages.push(Page { start: at, lines: vec![0], portrait: false });
            }
            0xFD => return Some(pages),
            0xF4 => {
                page.portrait = true;
                at += portrait_operands;
            }
            0xF6..=0xFB | 0xE0..=0xE7 => *line += slot_cells(code),
            _ => *line += 1,
        }
    }
    None
}

/// The page whose bytes hold the offset `at` (the engine counts it in a byte: a string longer than 256 bytes has
/// several candidates, and the one at or after `hint` is taken).
fn page_of(pages: &[Page], at: usize, hint: Option<usize>) -> Option<usize> {
    let ends = pages.iter().skip(1).map(|page| page.start).chain([usize::MAX]);
    let holds: Vec<usize> = pages
        .iter()
        .zip(ends)
        .enumerate()
        .filter(|(_, (page, end))| (page.start..*end).take(256).any(|offset| offset & 0xFF == at))
        .map(|(index, _)| index)
        .take(8)
        .collect();
    holds.iter().copied().find(|&index| Some(index) >= hint).or_else(|| holds.first().copied())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::rom_with;

    /// A work RAM with a message up in `status`, of table 3 string 0, at `offset`, line and cell.
    fn wram(status: u8, offset: u8, line: u8, cell: u8) -> Vec<u8> {
        let mut wram = vec![0_u8; 0x20000];
        let at = |address: u32| (address - 0x7E_0000) as usize;
        wram[at(d::MESSAGE_STATE)] = status;
        wram[at(d::MESSAGE_TABLE)] = 3;
        wram[at(d::COUNTER_071D)] = offset;
        wram[at(d::MESSAGE_LINE)] = line;
        wram[at(d::COUNTER_071F)] = cell;
        wram[at(d::MESSAGE_TEXT_BASE)..][..2].copy_from_slice(&0x05C8_u16.to_le_bytes());
        wram
    }

    /// A ROM whose table 3 is at 07:9000 and whose string 0 is "ab" / "c" FE FF "dddd" FF FF at 07:9100.
    fn rom() -> Vec<u8> {
        rom_with(&[
            (d::STRING_TABLE_POINTERS + 9, &[0x00, 0x90, 0x07]),
            (0x07_9000, &[0x00, 0x91]),
            (0x07_9100, &[0x05, 0x07, 0xFF, 0x09, 0xFE, 0xFF, 0x0A, 0x0A, 0x0A, 0x0A, 0xFF, 0xFF]),
        ])
    }

    #[test]
    fn the_page_line_and_progress_follow_the_engines_bytes() {
        let rom = rom();
        // Writing the first page: "ab" done, the cursor at the start of line 1
        let shown = on_screen(&rom, &wram(0x81, 3, 1, 0), None).unwrap();
        assert_eq!((shown.key.as_str(), shown.page, shown.state), ("str3:0x00", 0, 1));
        assert_eq!(shown.lines, [2, 1]);
        assert!((shown.progress - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(shown.base_cell(), (4, 23));
        // The page waits for A: the offset is past its FE FF, and the page is whole
        let waiting = on_screen(&rom, &wram(0x82, 6, 0, 0), None).unwrap();
        assert_eq!((waiting.page, waiting.progress), (0, 1.0));
        // The second page, half written
        let second = on_screen(&rom, &wram(0x81, 8, 0, 2), None).unwrap();
        assert_eq!((second.key.as_str(), second.lines.as_slice()), ("str3:0x00/p2", &[4][..]));
        assert!((second.progress - 0.5).abs() < 1e-6);
    }

    #[test]
    fn a_name_being_written_counts_as_written() {
        // String 1 of table 3: "a" FB FF FF, at 07:9200; the slot holds no name, so it counts six cells
        let mut rom = rom();
        let more = rom_with(&[(0x07_9002, &[0x00, 0x92]), (0x07_9200, &[0x05, 0xFB, 0xFF, 0xFF])]);
        let view = RomView::new(&more).unwrap();
        for address in [0x07_9002_u32, 0x07_9200] {
            let at = view.offset(address).unwrap();
            rom[at..at + 4].copy_from_slice(&more[at..at + 4]);
        }
        let mut ram = wram(0x85, 1, 0, 1);
        ram[(d::MESSAGE_STRING - 0x7E_0000) as usize] = 1;
        let shown = on_screen(&rom, &ram, None).unwrap();
        assert_eq!((shown.key.as_str(), shown.lines.as_slice()), ("str3:0x01", &[7][..]));
        assert!((shown.progress - 1.0).abs() < 1e-6, "{}", shown.progress);
    }

    #[test]
    fn no_message_or_a_state_between_strings_shows_nothing() {
        let rom = rom();
        assert_eq!(on_screen(&rom, &wram(0x00, 3, 0, 0), None), None);
        assert_eq!(
            on_screen(&rom, &wram(0x80, 3, 0, 0), None),
            None,
            "state 0: the last string's bytes"
        );
        assert!(on_screen(&rom, &wram(0xC0, 0, 0, 0), None).is_some(), "written in one frame");
    }

    #[test]
    fn a_long_string_takes_the_page_at_or_after_the_hint() {
        let pages = [
            Page { start: 0, lines: vec![1], portrait: false },
            Page { start: 200, lines: vec![1], portrait: false },
            Page { start: 300, lines: vec![1], portrait: false },
        ];
        assert_eq!(page_of(&pages, 10, None), Some(0));
        assert_eq!(page_of(&pages, 10, Some(2)), Some(2), "offset 266 is page 2's");
        assert_eq!(page_of(&pages, 210, None), Some(1));
    }

    #[test]
    fn numbers_are_their_digits() {
        assert_eq!(number(&[0x01, 0x81, 0x82, 0x80, 0xFF, 0x85]), "120");
        assert_eq!(number(&[0xFF]), "");
    }
}
