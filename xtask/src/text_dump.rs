//! `text-dump`: every string of the game's text tables, decoded, as one TSV row per page, keyed by
//! where it sits in the ROM (the base of a translation that replaces the text on screen). The
//! file is derived from the ROM: it is written outside the repository and never tracked.
//!
//! The tables and the codes are in `docs/re/data/text-engine.md`. A row is `key`, `rom_address`
//! (bank:address of the page's first byte), `max_line_cells` (the width of the window in 8-pixel
//! cells where it was measured, else the longest line of the table, a name's padded length or a
//! block's width), `lines` and `english`: the lines joined by a literal `\n`, the tiles of
//! `ssd_catalogue::decode_tile` as their character and the control codes as tokens:
//!
//! | Code | Token | What the engine does |
//! |---|---|---|
//! | `F4 nn` (`F4 nn mm` in table 1) | `{portrait:nn}` (`{portrait:nn:mm}`) | shows a portrait; the line starts one cell to the left |
//! | `FB`, `FA`, `F9` | `{name:1}`, `{name:2}`, `{name:3}` | a unit, object or attack name from the slot at `$1640`, `$1642`, `$1644` |
//! | `F8`, `F7`, `F6` | `{num:1}`, `{num:2}`, `{num:3}` | the tiles at `$1610`, `$1620`, `$1630` (numbers) |
//! | `FD` | `{auto}` | ends the string; the window closes by itself |
//! | `FC` | `{FC}` | the tile `$132` |
//! | `C8`-`CC` | `{tile:10C}`... | a tile of the second set |
//! | `E0`-`E7` | `{phrase:n}` | the string `inline_phrase_pointers[n]` in place |
//! | `FE FF` | (a new row `/pN`) | a page break: waits for A |
//!
//! A tile without a known character is `<XX>`. A row whose string has the address of an earlier
//! row says `{same:KEY}` instead of repeating it. Next to the TSV, `messages.tsv` lists each
//! message of `message_pointers` as the keys of its strings (`{table:n}` for a `D0nn` word,
//! `{E0:nn}` for a handler word).

use anyhow::Context as _;
use ssd_catalogue::{RomView, decode_tile};
use ssd_game::data as d;
use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Where the TSV goes without an argument: the scratch folder, which git ignores (the text is ROM-derived and
/// never tracked).
pub const DEFAULT_OUT: &str = ".work/text/strings-en.tsv";

/// The parsed command line: the TSV to write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub out: PathBuf,
}

/// The arguments: none, or the path of the TSV.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    match args {
        [] => Ok(Args { out: PathBuf::from(DEFAULT_OUT) }),
        [path] if !path.starts_with("--") => Ok(Args { out: PathBuf::from(path) }),
        [other, ..] => Err(format!("takes one argument, the TSV to write (`{other}`)")),
    }
}

/// A page of a string, or a name or a block: one row of the TSV.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub key: String,
    /// The physical address (bank:address) of the page's first byte.
    pub address: u32,
    pub max_line_cells: usize,
    /// The decoded lines; a row of a duplicate string holds one line, `{same:KEY}`.
    pub lines: Vec<String>,
    /// The cells of each line up to its last tile that is not a space (a token counts none).
    pub cells: Vec<usize>,
}

/// What a table gave: its rows and how many of its entries could not be read.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Table {
    pub name: &'static str,
    /// Entries of the table (strings, names, blocks), duplicates included.
    pub entries: usize,
    pub rows: Vec<Row>,
    /// Entries whose pointer leads outside the ROM, or whose string has no end.
    pub failures: usize,
    /// Rows with a tile the font table does not spell (`<XX>`).
    pub unknown: usize,
    /// Lines longer than `max_line_cells`.
    pub over: usize,
}

/// How wide a table's window is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Width {
    /// Measured on the original's screen (`docs/re/data/text-engine.md`).
    Cells(usize),
    /// Not measured: the longest line of the table, which the original fits.
    Longest,
}

/// A table of word pointers to engine strings (a value of `message_table`).
struct StringTable {
    name: &'static str,
    /// The `message_table` value.
    table: u8,
    pointers: u32,
    count: u32,
    /// The index of the first entry (table 0 is cut in two: help, then the battle's table 1).
    first: u32,
    width: Width,
}

/// The string tables, by `message_table`. Table 0 is `item_help_pointers` up to index 41 (40 and
/// 41 are two menu lines); from index 42 the same words are table 1, the battle's messages.
const STRING_TABLES: [StringTable; 9] = [
    StringTable {
        name: "help",
        table: 0,
        pointers: d::ITEM_HELP_POINTERS,
        count: HELP_ENTRIES,
        first: 0,
        width: Width::Longest,
    },
    StringTable {
        name: "battle",
        table: 1,
        pointers: d::BATTLE_MESSAGE_POINTERS,
        count: d::BATTLE_MESSAGE_POINTERS_SIZE / 2,
        first: 0,
        // The battle's window: tile columns 6 to 25, the text from column 8.
        width: Width::Cells(17),
    },
    StringTable {
        name: "str2",
        table: 2,
        pointers: d::TALK_STRING_POINTERS,
        count: d::TALK_STRING_POINTERS_SIZE / 2,
        first: 0,
        // The message box of columns 2 to 29, the text from column 9 (the portrait is left of
        // it); the first line after a portrait starts at column 8.
        width: Width::Cells(20),
    },
    StringTable {
        name: "str3",
        table: 3,
        pointers: d::STORY_STRING_POINTERS,
        count: d::STORY_STRING_POINTERS_SIZE / 2,
        first: 0,
        // The same box, the text from column 4 to column 28.
        width: Width::Cells(25),
    },
    StringTable {
        name: "str4",
        table: 4,
        pointers: d::WISH_STRING_POINTERS,
        count: d::WISH_STRING_POINTERS_SIZE / 2,
        first: 0,
        width: Width::Longest,
    },
    StringTable {
        name: "str5",
        table: 5,
        pointers: d::WISH_STRING_POINTERS_5,
        count: d::WISH_STRING_POINTERS_5_SIZE / 2,
        first: 0,
        width: Width::Longest,
    },
    StringTable {
        name: "str6",
        table: 6,
        pointers: d::WISH_STRING_POINTERS_6,
        count: d::WISH_STRING_POINTERS_6_SIZE / 2,
        first: 0,
        width: Width::Longest,
    },
    StringTable {
        name: "str7",
        table: 7,
        pointers: d::SYSTEM_STRING_POINTERS,
        count: d::SYSTEM_STRING_POINTERS_SIZE / 2,
        first: 0,
        width: Width::Longest,
    },
    StringTable {
        name: "phrase",
        table: PHRASE_TABLE,
        pointers: d::INLINE_PHRASE_POINTERS,
        count: d::INLINE_PHRASE_POINTERS_SIZE / 2,
        first: 0,
        width: Width::Longest,
    },
];

/// The space tile (the engine moves on a cell without writing it).
const SPACE: u8 = 0x01;
/// The entries of table 0 before table 1 starts on the same words.
const HELP_ENTRIES: u32 = 42;
/// Not a `message_table` value: the phrases of the codes `E0`-`E7`.
const PHRASE_TABLE: u8 = 0xFF;
/// A string longer than this has lost its end.
const MAX_STRING: u32 = 2048;
/// The entries of `bank2_pointer_table` that lead to blocks; the words after them point into code.
const BLOCKS: u32 = 80;
/// The texts of the ending (mode `$11`), bank 0: the immediates of the calls to 00:B2B1 at
/// 00:B071, 00:B25C, 00:B3DD, 00:B45F and 00:B4BE.
const ENDING_TEXTS: [u32; 5] = [0x00_BD87, 0x00_BEC7, 0x00_BFB4, 0x00_BFF0, 0x00_C052];

/// The key of string `index` of `message_table` value `table` (the coordinates the engine has
/// while it writes it: `message_table`, `message_string`).
#[must_use]
pub fn string_key(table: u8, index: u32) -> String {
    match table {
        0 if index < HELP_ENTRIES => format!("help:{index}"),
        0 => format!("battle:{}", index - HELP_ENTRIES),
        1 => format!("battle:{index}"),
        2 => format!("str2:0x{index:03X}"),
        3..=7 => format!("str{table}:0x{index:02X}"),
        PHRASE_TABLE => format!("phrase:{index}"),
        _ => format!("table{table}:{index}"),
    }
}

/// The key of a page: the first page has the string's key, the next ones `/p2`, `/p3`...
fn page_key(key: &str, page: usize) -> String {
    if page == 0 { key.to_owned() } else { format!("{key}/p{}", page + 1) }
}

/// The next byte of a string, which never leaves its bank.
fn byte(rom: RomView<'_>, address: u32) -> Option<u8> {
    rom.u8(address).ok()
}

/// One page of an engine string being decoded.
struct PageText {
    address: u32,
    lines: Vec<String>,
    /// The cells of each line up to its last tile that is not a space.
    cells: Vec<usize>,
    /// The spaces after the last other tile of the line.
    spaces: usize,
}

impl PageText {
    fn new(address: u32) -> Self {
        Self { address, lines: vec![String::new()], cells: vec![0], spaces: 0 }
    }

    fn push_tile(&mut self, tile: u8) {
        let line = self.lines.last_mut().expect("a page has a line");
        match decode_tile(tile) {
            Some(c) => line.push(c),
            None => {
                let _ = write!(line, "<{tile:02X}>");
            }
        }
        if tile == SPACE {
            self.spaces += 1;
        } else {
            *self.cells.last_mut().expect("a page has a line") += self.spaces + 1;
            self.spaces = 0;
        }
    }

    fn push_token(&mut self, token: &str) {
        self.lines.last_mut().expect("a page has a line").push_str(token);
    }

    fn new_line(&mut self) {
        self.lines.push(String::new());
        self.cells.push(0);
        self.spaces = 0;
    }
}

/// The pages of the engine string at `start`, as 01:CFB4 reads it; `None` when it leaves its
/// bank or has no end. `portrait_operands` is 2 in table 1 (01:D2D9), else 1.
fn engine_pages(rom: RomView<'_>, start: u32, portrait_operands: u32) -> Option<Vec<PageText>> {
    let mut pages = vec![PageText::new(start)];
    let mut address = start;
    while address - start < MAX_STRING {
        let code = byte(rom, address)?;
        address += 1;
        let page = pages.last_mut().expect("a string has a page");
        match code {
            0xFF if byte(rom, address)? == 0xFF => return Some(pages),
            0xFF => page.new_line(),
            // `FE FF`: the page ends and the next starts after the `FF`.
            0xFE => {
                address += 1;
                pages.push(PageText::new(address));
            }
            0xFD => {
                page.push_token("{auto}");
                return Some(pages);
            }
            0xFC => page.push_token("{FC}"),
            0xF4 => {
                let mut token = String::from("{portrait");
                for _ in 0..portrait_operands {
                    let _ = write!(token, ":{:02X}", byte(rom, address)?);
                    address += 1;
                }
                token.push('}');
                page.push_token(&token);
            }
            0xF6..=0xF8 => page.push_token(&format!("{{num:{}}}", 0xF9 - code)),
            0xF9..=0xFB => page.push_token(&format!("{{name:{}}}", 0xFC - code)),
            0xE0..=0xE7 => page.push_token(&format!("{{phrase:{}}}", code & 0x0F)),
            0xC8..=0xCC => {
                let tile = [0x10C, 0x10D, 0x10E, 0x150, 0x151][usize::from(code - 0xC8)];
                page.push_token(&format!("{{tile:{tile:03X}}}"));
            }
            tile => page.push_tile(tile),
        }
    }
    None
}

/// Reads one string table into rows, the duplicates of an address already seen (in any table)
/// as `{same:KEY}`.
fn read_string_table(
    rom: RomView<'_>,
    spec: &StringTable,
    seen: &mut HashMap<u32, String>,
) -> Table {
    let mut table = Table { name: spec.name, ..Table::default() };
    let bank = (spec.pointers >> 16) as u8;
    let operands = if spec.table == 1 { 2 } else { 1 };
    for i in 0..spec.count {
        let index = spec.first + i;
        table.entries += 1;
        let key = string_key(spec.table, index);
        let Ok(address) = rom.pointer(spec.pointers + 2 * i, bank) else {
            table.failures += 1;
            continue;
        };
        if let Some(first) = seen.get(&address) {
            table.rows.push(Row {
                key,
                address,
                max_line_cells: 0,
                lines: vec![format!("{{same:{first}}}")],
                cells: vec![0],
            });
            continue;
        }
        let Some(pages) = engine_pages(rom, address, operands) else {
            table.failures += 1;
            continue;
        };
        seen.insert(address, key.clone());
        for (n, page) in pages.into_iter().enumerate() {
            table.rows.push(Row {
                key: page_key(&key, n),
                address: page.address,
                max_line_cells: 0,
                lines: page.lines,
                cells: page.cells,
            });
        }
    }
    set_widths(&mut table, spec.width);
    table
}

/// Fills `max_line_cells` of every row from the table's width, and counts the long lines.
fn set_widths(table: &mut Table, width: Width) {
    let longest = table.rows.iter().flat_map(|row| row.cells.iter().copied()).max().unwrap_or(0);
    let cells = match width {
        Width::Cells(cells) => cells,
        Width::Longest => longest,
    };
    for row in &mut table.rows {
        row.max_line_cells = cells;
        table.over += row.cells.iter().filter(|&&c| c > cells).count();
    }
    table.unknown =
        table.rows.iter().filter(|row| row.lines.iter().any(|l| has_unknown(l))).count();
}

/// Whether a line holds a `<XX>` tile.
fn has_unknown(line: &str) -> bool {
    line.find('<').is_some_and(|i| line[i..].find('>') == Some(3))
}

/// The length-prefixed names of a table of bank-2 word pointers (`names.md`): one row each, the
/// padded length as the width.
fn read_names(
    rom: RomView<'_>,
    name: &'static str,
    pointers: u32,
    size: u32,
    key: impl Fn(u32) -> String,
) -> Table {
    let mut table = Table { name, ..Table::default() };
    let mut seen: HashMap<u32, String> = HashMap::new();
    for i in 0..size / 2 {
        table.entries += 1;
        let tiles = rom.pointer(pointers + 2 * i, 0x02).ok().and_then(|address| {
            let len = rom.u8(address).ok()?;
            Some((address, rom.slice(address + 1, usize::from(len)).ok()?))
        });
        let Some((address, tiles)) = tiles else {
            table.failures += 1;
            continue;
        };
        let mut page = PageText::new(address);
        if let Some(first) = seen.get(&address) {
            page.push_token(&format!("{{same:{first}}}"));
        } else {
            seen.insert(address, key(i));
            for &tile in tiles {
                page.push_tile(tile);
            }
        }
        table.rows.push(Row {
            key: key(i),
            address,
            max_line_cells: tiles.len(),
            lines: page.lines,
            cells: page.cells,
        });
    }
    table.unknown =
        table.rows.iter().filter(|row| row.lines.iter().any(|l| has_unknown(l))).count();
    table
}

/// The text blocks of `bank2_pointer_table` (`menus.md`): a width - 1, a height - 1, the tile-map
/// offset and a tile byte per cell, rows two tile rows apart (00:9C91). Only the blocks drawn
/// at a place (offset not 0) and at least half Latin are text; the others are frames and
/// pictures.
fn read_blocks(rom: RomView<'_>) -> Table {
    let mut table = Table { name: "block", ..Table::default() };
    for i in 0..BLOCKS {
        table.entries += 1;
        let block = rom.pointer(d::BANK2_POINTER_TABLE + 2 * i, 0x02).ok().and_then(|address| {
            let head = rom.slice(address, 4).ok()?;
            let (width, height) = (usize::from(head[0]) + 1, usize::from(head[1]) + 1);
            let offset = u16::from_le_bytes([head[2], head[3]]);
            Some((address, width, height, offset, rom.slice(address + 4, width * height).ok()?))
        });
        let Some((address, width, height, offset, tiles)) = block else {
            table.failures += 1;
            continue;
        };
        let latin = tiles.iter().filter(|&&t| decode_tile(t).is_some()).count();
        if offset == 0 || latin * 2 < tiles.len() {
            continue;
        }
        let mut page = PageText::new(address);
        for (r, row) in tiles.chunks(width).enumerate() {
            if r > 0 {
                page.new_line();
            }
            for &tile in row {
                page.push_tile(tile);
            }
        }
        debug_assert_eq!(page.lines.len(), height);
        table.rows.push(Row {
            key: format!("block:0x{i:02X}"),
            address,
            max_line_cells: width,
            lines: page.lines,
            cells: page.cells,
        });
    }
    table.unknown =
        table.rows.iter().filter(|row| row.lines.iter().any(|l| has_unknown(l))).count();
    table
}

/// The ending's texts (00:BA73): `FF` ends a line, `FF FF` a screen, `FF 00` the text.
fn read_ending(rom: RomView<'_>) -> Table {
    let mut table = Table { name: "ending", ..Table::default() };
    for (i, &start) in ENDING_TEXTS.iter().enumerate() {
        table.entries += 1;
        let pages = (|| {
            let mut pages = vec![PageText::new(start)];
            let mut address = start;
            while address - start < MAX_STRING {
                let code = byte(rom, address)?;
                address += 1;
                let page = pages.last_mut().expect("a text has a page");
                if code != 0xFF {
                    page.push_tile(code);
                    continue;
                }
                match byte(rom, address)? {
                    0x00 => return Some(pages),
                    0xFF => {
                        address += 1;
                        pages.push(PageText::new(address));
                    }
                    _ => page.new_line(),
                }
            }
            None
        })();
        let Some(pages) = pages else {
            table.failures += 1;
            continue;
        };
        for (n, page) in pages.into_iter().enumerate() {
            table.rows.push(Row {
                key: page_key(&format!("ending:{i}"), n),
                address: page.address,
                max_line_cells: 0,
                lines: page.lines,
                cells: page.cells,
            });
        }
    }
    set_widths(&mut table, Width::Longest);
    table
}

/// Every table of the ROM, in the order of the TSV.
///
/// # Errors
///
/// The image is not a ROM image of whole banks.
pub fn read_tables(rom: &[u8]) -> anyhow::Result<Vec<Table>> {
    let rom = RomView::new(rom).context("reading the ROM")?;
    let mut seen = HashMap::new();
    let mut tables = Vec::new();
    for spec in &STRING_TABLES {
        tables.push(read_string_table(rom, spec, &mut seen));
    }
    tables.push(read_ending(rom));
    tables.push(read_names(rom, "string", d::STRING_POINTERS, d::STRING_POINTERS_SIZE, |i| {
        format!("string:{i}")
    }));
    tables.push(read_names(rom, "unit", d::UNIT_NAME_POINTERS, d::UNIT_NAME_POINTERS_SIZE, |i| {
        format!("unit:0x{i:02X}")
    }));
    tables.push(read_names(
        rom,
        "attack",
        d::ATTACK_NAME_POINTERS,
        d::ATTACK_NAME_POINTERS_SIZE,
        |i| format!("attack:{i}"),
    ));
    tables.push(read_names(
        rom,
        "object",
        d::OBJECT_NAME_POINTERS,
        d::OBJECT_NAME_POINTERS_SIZE,
        |i| format!("object:{i}"),
    ));
    tables.push(read_blocks(rom));
    Ok(tables)
}

/// The messages of `message_pointers`: per message its key, address and the keys of its words.
///
/// # Errors
///
/// The image is not a ROM image of whole banks.
pub fn read_messages(rom: &[u8]) -> anyhow::Result<Vec<(String, u32, Vec<String>)>> {
    let rom = RomView::new(rom).context("reading the ROM")?;
    let mut messages = Vec::new();
    for m in 0..d::MESSAGE_POINTERS_SIZE / 2 {
        let key = format!("msg:0x{m:03X}");
        let Ok(start) = rom.pointer(d::MESSAGE_POINTERS + 2 * m, 0x07) else {
            messages.push((key, 0, vec!["{unreadable}".to_owned()]));
            continue;
        };
        let mut words = Vec::new();
        let mut table = 0_u8;
        let mut address = start;
        // 01:D37A: a D0nn word sets the table, an E0nn word runs a handler, F0nn ends.
        while let Ok(word) = rom.u16(address) {
            address += 2;
            match word & 0xF000 {
                0xD000 => {
                    table = (word & 0xFF) as u8;
                    words.push(format!("{{table:{table}}}"));
                }
                0xE000 => words.push(format!("{{E0:{:02X}}}", word & 0xFF)),
                0xF000 => break,
                _ => words.push(string_key(table, u32::from(word))),
            }
            if words.len() > 64 {
                words.push("{unended}".to_owned());
                break;
            }
        }
        messages.push((key, start, words));
    }
    Ok(messages)
}

/// The bank:address of a physical address.
fn address(address: u32) -> String {
    format!("{:02X}:{:04X}", address >> 16, address & 0xFFFF)
}

/// The TSV of the tables.
#[must_use]
pub fn render(tables: &[Table]) -> String {
    let mut out = String::from("key\trom_address\tmax_line_cells\tlines\tenglish\n");
    for row in tables.iter().flat_map(|t| &t.rows) {
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}",
            row.key,
            address(row.address),
            row.max_line_cells,
            row.lines.len(),
            row.lines.join("\\n")
        );
    }
    out
}

/// The TSV of the messages.
#[must_use]
pub fn render_messages(messages: &[(String, u32, Vec<String>)]) -> String {
    let mut out = String::from("key\trom_address\twords\n");
    for (key, start, words) in messages {
        let _ = writeln!(out, "{key}\t{}\t{}", address(*start), words.join(" "));
    }
    out
}

/// Writes the TSV (and `messages.tsv` beside it) and prints a summary line per table.
///
/// # Errors
///
/// The ROM is not one, or a file cannot be written.
pub fn run(rom: &[u8], args: &Args, out: &mut dyn std::io::Write) -> anyhow::Result<()> {
    let tables = read_tables(rom)?;
    let messages = read_messages(rom)?;
    let dir = args.out.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    std::fs::write(&args.out, render(&tables))
        .with_context(|| format!("writing {}", args.out.display()))?;
    let messages_path = dir.join("messages.tsv");
    std::fs::write(&messages_path, render_messages(&messages))
        .with_context(|| format!("writing {}", messages_path.display()))?;
    writeln!(
        out,
        "{:<8} {:>7} {:>5} {:>8} {:>7} {:>5}",
        "table", "entries", "rows", "failures", "unknown", "over"
    )?;
    for t in &tables {
        writeln!(
            out,
            "{:<8} {:>7} {:>5} {:>8} {:>7} {:>5}",
            t.name,
            t.entries,
            t.rows.len(),
            t.failures,
            t.unknown,
            t.over
        )?;
    }
    let rows: usize = tables.iter().map(|t| t.rows.len()).sum();
    writeln!(out, "text-dump: wrote {rows} rows to {}", args.out.display())?;
    writeln!(out, "text-dump: wrote {} messages to {}", messages.len(), messages_path.display())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A blank 1 MiB image with `bytes` at their physical addresses.
    fn rom_with(bytes: &[(u32, &[u8])]) -> Vec<u8> {
        let mut rom = vec![0_u8; 1 << 20];
        let view_offset =
            |address: u32| usize::try_from((address >> 16) * 0x8000 + (address & 0x7FFF)).unwrap();
        for (address, data) in bytes {
            let at = view_offset(*address);
            rom[at..at + data.len()].copy_from_slice(data);
        }
        rom
    }

    #[test]
    fn the_arguments_are_an_optional_path() {
        assert_eq!(parse_args(&[]).unwrap().out, PathBuf::from(DEFAULT_OUT));
        assert_eq!(parse_args(&["x.tsv"]).unwrap().out, PathBuf::from("x.tsv"));
        assert!(parse_args(&["--out"]).is_err());
        assert!(parse_args(&["a", "b"]).is_err());
    }

    #[test]
    fn keys_follow_the_engine_tables() {
        assert_eq!(string_key(0, 12), "help:12");
        assert_eq!(string_key(0, 42 + 78), "battle:78", "table 0 goes on as table 1");
        assert_eq!(string_key(1, 78), "battle:78");
        assert_eq!(string_key(2, 0x41), "str2:0x041");
        assert_eq!(string_key(3, 0x3E), "str3:0x3E");
        assert_eq!(page_key("str3:0x3E", 1), "str3:0x3E/p2");
    }

    #[test]
    fn pages_lines_and_codes_decode() {
        // "Hi" / "a" FE FF "Go" FB "!" FF FF, at 07:9000.
        let text = [0x8A, 0x15, 0xFF, 0x05, 0xFE, 0xFF, 0xED, 0x1B, 0xFB, 0x3F, 0xFF, 0xFF];
        let rom = rom_with(&[(0x07_9000, &text)]);
        let view = RomView::new(&rom).unwrap();
        let pages = engine_pages(view, 0x07_9000, 1).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(pages[0].lines, ["Hi", "a"]);
        assert_eq!(pages[1].address, 0x07_9006);
        assert_eq!(pages[1].lines, ["Go{name:1}!"]);
        assert_eq!(pages[1].cells, [3]);
    }

    #[test]
    fn a_portrait_takes_two_operands_in_the_battle_table() {
        let text = [0xF4, 0x01, 0x6A, 0x05, 0xF8, 0xFD];
        let rom = rom_with(&[(0x07_9000, &text)]);
        let view = RomView::new(&rom).unwrap();
        assert_eq!(
            engine_pages(view, 0x07_9000, 2).unwrap()[0].lines,
            ["{portrait:01:6A}a{num:1}{auto}"]
        );
        assert_eq!(
            engine_pages(view, 0x07_9000, 1).unwrap()[0].lines,
            ["{portrait:01}<6A>a{num:1}{auto}"]
        );
    }

    #[test]
    fn a_string_without_an_end_fails() {
        let rom = rom_with(&[(0x07_FFFE, &[0x05, 0x05])]);
        let view = RomView::new(&rom).unwrap();
        assert!(engine_pages(view, 0x07_FFFE, 1).is_none());
    }

    #[test]
    fn a_repeated_pointer_names_the_first_key() {
        let rom = rom_with(&[
            (0x07_9000, &[0x05, 0xFF, 0xFF]),
            (d::STORY_STRING_POINTERS, &[0x00, 0x90, 0x00, 0x90]),
        ]);
        let view = RomView::new(&rom).unwrap();
        let spec = &STRING_TABLES[3];
        let table = read_string_table(view, spec, &mut HashMap::new());
        assert_eq!(table.rows[0].lines, ["a"]);
        assert_eq!(table.rows[1].lines, ["{same:str3:0x00}"]);
        assert_eq!(table.rows[0].max_line_cells, 25);
        // The other pointers of the blank image are $0000: outside the ROM.
        assert_eq!(table.failures, table.entries - 2);
        let tsv = render(&[table]);
        assert!(
            tsv.starts_with(
                "key\trom_address\tmax_line_cells\tlines\tenglish\nstr3:0x00\t07:9000\t25\t1\ta\n"
            ),
            "{tsv}"
        );
    }

    #[test]
    fn unknown_tiles_are_counted() {
        assert!(has_unknown("a<4D>b"));
        assert!(!has_unknown("{name:1}"));
    }
}
