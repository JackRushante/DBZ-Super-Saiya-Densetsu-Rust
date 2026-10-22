//! `mod-tileset <out dir>`: the original's 120 blocks as a Tiled tileset (`blocks.tsj` and `blocks.png`), rendered from
//! the cached ROM, so that an author can paint pack maps with them (`docs/mods/maps.md`). Tile `t` of the tileset is
//! block `t`. The picture is 12 blocks wide, 256 x 256 pixels a block; a cell of a block is 16 x 16 pixels, drawn from
//! the field's tile set (`08:8D4C`) through its metatile table (`09:EFB0`) and the block definitions (`06:F1EE`).
//!
//! The colours are those of map 0's background palettes (`06:F876`, rows 2 to 5 of the game's palette): a tile
//! palette `2` to `5` is that row; the palettes `0`, `1`, `6` and `7` are drawn with the rows they wrap to (an
//! approximation for the author's view; the game may draw them otherwise). Colour 0 of a palette is transparent.
//!
//! The files are derived from the game: they are written under `.work/` (the caller refuses a tracked path).

use anyhow::{Context, bail};
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

/// The number of original blocks (`$00` to `$77`).
const BLOCKS: usize = 120;
/// The blocks per row of the picture, and a block's side in pixels.
const COLUMNS: usize = 12;
const BLOCK_PX: usize = 256;
/// A 4-bit tile of the field: 32 bytes.
const TILE_BYTES: usize = 32;

/// The field's tile set: an LZ stream at `08:8D4C`.
const TILE_SET: (u8, u16) = (0x08, 0x8D4C);
/// The field's metatile table: `09:EFB0`.
const METATILES: (u8, u16) = (0x09, 0xEFB0);
/// The block definitions' table: a word per block (bank, then the middle byte of the definition), `06:F1EE`.
const BLOCK_TABLE: (u8, u16) = (0x06, 0xF1EE);
/// The background palette sets of the maps: a 3-byte pointer per map id to 128 bytes, `06:F876`.
const PALETTES: (u8, u16) = (0x06, 0xF876);

/// Writes `blocks.tsj` and `blocks.png` into `out` from the cached ROM `rom`.
///
/// # Errors
///
/// When the ROM does not hold the tables at the addresses above, a block's definition is not in ROM, or the files
/// cannot be written.
pub fn run(rom: &[u8], out: &Path) -> anyhow::Result<()> {
    let stream = ssd_catalogue::graphics::lz_decode(rom, offset(TILE_SET)?)
        .context("the field's tile set does not decode")?;
    let mut tiles = stream.data;
    ssd_catalogue::graphics::regroup(&mut tiles);
    let tile_count = tiles.len() / TILE_BYTES;

    let palette = palette(rom)?;
    let metatiles = offset(METATILES)?;
    let width = COLUMNS * BLOCK_PX;
    let rows = BLOCKS.div_ceil(COLUMNS);
    let height = rows * BLOCK_PX;
    let mut sheet = vec![0_u8; width * height * 4];
    for block in 0..BLOCKS {
        let definition = block_definition(rom, block)?;
        for cell in 0..256 {
            let cell_word = be_or_le_word(rom, definition + 2 * cell, false)? & 0x3FDE;
            let table = metatiles + usize::from(cell_word) * 2;
            let quadrants = [
                be_or_le_word(rom, table, true)?,
                be_or_le_word(rom, table + 2, true)?,
                be_or_le_word(rom, table + 0x40, true)?,
                be_or_le_word(rom, table + 0x42, true)?,
            ];
            for (quadrant, word) in quadrants.into_iter().enumerate() {
                let tile = usize::from(word & 0x3FF);
                let row = usize::from((word >> 10) & 7);
                let (flip_x, flip_y) = (word & 0x4000 != 0, word & 0x8000 != 0);
                for py in 0..8 {
                    for px in 0..8 {
                        let sx = if flip_x { 7 - px } else { px };
                        let sy = if flip_y { 7 - py } else { py };
                        let index = if tile < tile_count { pixel(&tiles, tile, sx, sy) } else { 0 };
                        let x = (block % COLUMNS) * BLOCK_PX
                            + (cell % 16) * 16
                            + (quadrant % 2) * 8
                            + px;
                        let y = (block / COLUMNS) * BLOCK_PX
                            + (cell / 16) * 16
                            + (quadrant / 2) * 8
                            + py;
                        let at = (y * width + x) * 4;
                        if index != 0 {
                            let colour =
                                palette[((row.wrapping_sub(2) & 3) * 16) + usize::from(index)];
                            sheet[at..at + 4].copy_from_slice(&rgba(colour));
                        }
                    }
                }
            }
        }
    }

    std::fs::create_dir_all(out).with_context(|| format!("cannot create {}", out.display()))?;
    let png_path = out.join("blocks.png");
    let mut encoder = png::Encoder::new(
        BufWriter::new(
            File::create(&png_path)
                .with_context(|| format!("cannot write {}", png_path.display()))?,
        ),
        width as u32,
        height as u32,
    );
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header()?.write_image_data(&sheet)?;
    let tileset = serde_json::json!({
        "name": "blocks",
        "type": "tileset",
        "version": "1.10",
        "tiledversion": "1.10",
        "image": "blocks.png",
        "imagewidth": width,
        "imageheight": height,
        "tilewidth": BLOCK_PX,
        "tileheight": BLOCK_PX,
        "tilecount": BLOCKS,
        "columns": COLUMNS,
        "margin": 0,
        "spacing": 0,
    });
    let tsj_path = out.join("blocks.tsj");
    std::fs::write(&tsj_path, serde_json::to_string_pretty(&tileset)?)
        .with_context(|| format!("cannot write {}", tsj_path.display()))?;
    println!(
        "mod-tileset: {BLOCKS} blocks, {tile_count} tiles, map 0's background palettes: wrote {} and {}",
        tsj_path.display(),
        png_path.display()
    );
    Ok(())
}

/// The file offset of a `LoROM` address (`bank:address`, the address at least `$8000`).
fn offset((bank, address): (u8, u16)) -> anyhow::Result<usize> {
    if address < 0x8000 {
        bail!("{bank:02X}:{address:04X} is not a ROM address");
    }
    Ok(usize::from(bank) * 0x8000 + usize::from(address - 0x8000))
}

/// The file offset of the block definition of `block` (its table word gives the bank and the middle byte).
fn block_definition(rom: &[u8], block: usize) -> anyhow::Result<usize> {
    let table = offset(BLOCK_TABLE)? + 2 * block;
    let word = le_word(rom, table)?;
    let (bank, middle) = ((word >> 8) as u8, word & 0xFF);
    if middle < 0x80 {
        bail!("block {block} is defined outside the ROM (middle byte {middle:02X})");
    }
    offset((bank, middle << 8))
}

/// The background palette of map 0: 64 colours (rows 2 to 5 of the game's palette), BGR15.
fn palette(rom: &[u8]) -> anyhow::Result<Vec<u16>> {
    let pointer = offset(PALETTES)?;
    let address = le_word(rom, pointer)?;
    let bank = rom.get(pointer + 2).copied().context("the palette pointer is past the ROM")?;
    let data = offset((bank, address))?;
    (0..64).map(|k| le_word(rom, data + 2 * k)).collect()
}

/// The colour index of pixel `(x, y)` of tile `tile`: 4 bitplanes in the regrouped layout (planes 0 and 1 interleaved
/// row by row in the first 16 bytes, planes 2 and 3 in the next 16).
fn pixel(tiles: &[u8], tile: usize, x: usize, y: usize) -> u8 {
    let base = tile * TILE_BYTES;
    let shift = 7 - x;
    let bit = |byte: usize| (tiles[base + byte] >> shift) & 1;
    bit(2 * y) | bit(2 * y + 1) << 1 | bit(16 + 2 * y) << 2 | bit(16 + 2 * y + 1) << 3
}

/// A BGR15 colour as RGBA (opaque).
fn rgba(colour: u16) -> [u8; 4] {
    let scale = |channel: u16| (channel * 255 / 31) as u8;
    [scale(colour & 0x1F), scale((colour >> 5) & 0x1F), scale((colour >> 10) & 0x1F), 0xFF]
}

/// The little-endian word at file offset `at`.
fn le_word(rom: &[u8], at: usize) -> anyhow::Result<u16> {
    match (rom.get(at), rom.get(at + 1)) {
        (Some(&low), Some(&high)) => Ok(u16::from(low) | u16::from(high) << 8),
        _ => bail!("the ROM is shorter than file offset {at:#X}"),
    }
}

/// The word at file offset `at`, high byte first (`big`) or low byte first.
fn be_or_le_word(rom: &[u8], at: usize, big: bool) -> anyhow::Result<u16> {
    let word = le_word(rom, at)?;
    Ok(if big { word.swap_bytes() } else { word })
}
