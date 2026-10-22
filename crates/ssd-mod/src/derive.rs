//! The SNES art of a picture, derived from its HD picture (M10 Task 12): `xtask art-derive`, and the faces of a pack
//! (`[[face]]`, see [`crate::image`]).
//!
//! An HD picture is drawn at [`SCALE`] pixels to a game pixel. A game pixel is the average colour of its block, weighted
//! by alpha; a block whose alpha averages under 50 % is transparent. Colours are SNES colours (`0BBBBBGGGGGRRRRR`, 5 bits
//! a channel). A palette is 16 words: word 0 is transparent, words 1 to 15 are colours. The tool chooses a palette (median
//! cut, deterministic) or takes the one it is given; a face is always derived with the palette row the game draws it in.
//!
//! The tiles are the game's 4-bit tiles (`docs/re/data/graphics.md`): 32 bytes each in the SNES layout (`tiles.bin`). The
//! stream the game's LZ decoder reads holds each tile as two 16-byte blocks (planes 0 and 1, then planes 2 and 3), each
//! plane as its eight rows; the game regroups it to the SNES layout (`00:C68C`). `tiles.lz` is that stream, LZ-encoded.
//!
//! A face record is the stream of 32 tiles that the game's portrait routine decodes (`01:CA0B`, `docs/re/data/menus.md`,
//! "Corner faces"): two pictures of 16 tiles in a 4 x 4 grid, the first from tiles 0 to 7 and 16 to 23, the second from
//! tiles 8 to 15 and 24 to 31. The derived 16 tiles are put in both, so whichever picture the game shows is the face.

use crate::lz;
use std::collections::{BTreeMap, HashMap};
use std::fmt;

/// HD pixels along a game pixel's edge.
pub const SCALE: usize = 4;
/// Game pixels along a tile's edge.
pub const TILE: usize = 8;
/// A face is 32 x 32 game pixels (4 x 4 characters); its HD picture is 128 x 128.
pub const FACE_SIDE: usize = 32;
/// A tile of a pack map's HD art: 256 x 256 game pixels (one block) at 4x, 1024 x 1024 pixels.
pub const MAP_ART_TILE: usize = 256 * SCALE;
/// The tiles of a face's picture, and of its record (two pictures).
pub const FACE_TILES: usize = 16;
pub const RECORD_TILES: usize = 32;
/// The most tiles a map sheet may hold: the field's tile set (`docs/re/data/maps.md`, section 4).
pub const MAX_TILES: usize = 432;
/// The table of object palette pointers (`object_portrait_palettes`): a word per object, in bank `$02`.
pub const OBJECT_PALETTES: (u8, u16) = (0x02, 0xA356);
/// The object whose palette is the field menu's colour row 6, where the corner faces are drawn (`menus.md`).
pub const FACE_PALETTE_OBJECT: usize = 28;

/// The tile of each cell of a face record's first picture, row by row.
pub const FIRST_PICTURE: [usize; 16] = [0, 1, 2, 3, 16, 17, 18, 19, 4, 5, 6, 7, 20, 21, 22, 23];
/// The tile of each cell of the second picture, row by row.
pub const SECOND_PICTURE: [usize; 16] =
    [8, 9, 10, 11, 24, 25, 26, 27, 12, 13, 14, 15, 28, 29, 30, 31];

/// A picture of RGBA pixels, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

/// Why an HD picture cannot be derived. Each names what is wrong, in plain words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeriveError {
    /// The file is not a PNG of 8-bit samples, or it is damaged.
    Png(String),
    /// The picture's size does not fit its kind.
    Size(String),
    /// The sheet needs more tiles than the field's tile set holds: `count` is one past the budget (the count stops there).
    Tiles { count: usize },
    /// The palette is not 16 words of 15-bit colours.
    Palette(String),
}

impl fmt::Display for DeriveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Png(text) => write!(f, "the picture is not a readable PNG: {text}"),
            Self::Size(text) => write!(f, "the picture's size is wrong: {text}"),
            Self::Tiles { .. } => write!(
                f,
                "the picture needs more than {MAX_TILES} distinct tiles, the field's tile set holds {MAX_TILES}: merge some cells"
            ),
            Self::Palette(text) => write!(f, "the palette is not valid: {text}"),
        }
    }
}

impl std::error::Error for DeriveError {}

/// The game picture: each game pixel's colour (5 bits a channel), or `None` when it is transparent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Option<[u8; 3]>>,
}

/// One cell of a sheet: its tile, and the flips the cell shows it with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub tile: u16,
    pub h: bool,
    pub v: bool,
}

/// A map sheet: its palette, its distinct tiles (SNES layout), and its cells row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    pub palette: [u16; 16],
    pub tiles: Vec<[u8; 32]>,
    pub columns: usize,
    pub rows: usize,
    pub cells: Vec<Cell>,
}

/// A face: its palette, and its 16 tiles in the cells of its picture, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Face {
    pub palette: [u16; 16],
    pub tiles: [[u8; 32]; FACE_TILES],
}

/// Reads a PNG as RGBA.
///
/// # Errors
///
/// [`DeriveError::Png`] if the bytes are not a PNG of 8-bit samples.
pub fn read_png(bytes: &[u8]) -> Result<Rgba, DeriveError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().map_err(|e| DeriveError::Png(e.to_string()))?;
    let mut buffer =
        vec![0; reader.output_buffer_size().ok_or_else(|| DeriveError::Png("too large".into()))?];
    let info = reader.next_frame(&mut buffer).map_err(|e| DeriveError::Png(e.to_string()))?;
    if info.bit_depth != png::BitDepth::Eight {
        return Err(DeriveError::Png(format!("{:?}-bit samples", info.bit_depth)));
    }
    let data = &buffer[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => data.to_vec(),
        png::ColorType::Rgb => {
            data.as_chunks::<3>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect()
        }
        png::ColorType::GrayscaleAlpha => {
            data.as_chunks::<2>().0.iter().flat_map(|p| [p[0], p[0], p[0], p[1]]).collect()
        }
        png::ColorType::Grayscale => data.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err(DeriveError::Png("an indexed image".into())),
    };
    Ok(Rgba { width: info.width as usize, height: info.height as usize, rgba })
}

/// The size of a PNG, read from its header.
///
/// # Errors
///
/// [`DeriveError::Png`] if the bytes are not a PNG.
pub fn png_size(bytes: &[u8]) -> Result<(usize, usize), DeriveError> {
    let reader = png::Decoder::new(std::io::Cursor::new(bytes))
        .read_info()
        .map_err(|e| DeriveError::Png(e.to_string()))?;
    let info = reader.info();
    Ok((info.width as usize, info.height as usize))
}

/// The game picture of an HD picture: each game pixel is the alpha-weighted average of its `SCALE` x `SCALE` block, in
/// 5-bit channels; a block whose alpha averages under 50 % is transparent. The HD size must be a multiple of `SCALE`.
#[must_use]
pub fn downsample(hd: &Rgba) -> Game {
    let (width, height) = (hd.width / SCALE, hd.height / SCALE);
    let mut pixels = Vec::with_capacity(width * height);
    for gy in 0..height {
        for gx in 0..width {
            let mut alpha = 0_u32;
            let mut sums = [0_u32; 3];
            for y in 0..SCALE {
                for x in 0..SCALE {
                    let at = ((gy * SCALE + y) * hd.width + gx * SCALE + x) * 4;
                    let pixel = &hd.rgba[at..at + 4];
                    let a = u32::from(pixel[3]);
                    alpha += a;
                    for (sum, &channel) in sums.iter_mut().zip(pixel) {
                        *sum += a * u32::from(channel);
                    }
                }
            }
            // The mean alpha is at least 50 % when twice the sum is at least the full alpha of the block
            let opaque = alpha * 2 >= (SCALE * SCALE * 255) as u32;
            pixels.push(opaque.then(|| {
                sums.map(|sum| u8::try_from(((sum + alpha / 2) / alpha) >> 3).unwrap_or(31))
            }));
        }
    }
    Game { width, height, pixels }
}

/// The SNES colour of 5-bit channels `[r, g, b]`.
#[must_use]
pub fn pack_colour(channels: [u8; 3]) -> u16 {
    u16::from(channels[0] & 31)
        | u16::from(channels[1] & 31) << 5
        | u16::from(channels[2] & 31) << 10
}

/// The 5-bit channels of a SNES colour.
#[must_use]
pub fn unpack_colour(word: u16) -> [u8; 3] {
    [(word & 31) as u8, (word >> 5 & 31) as u8, (word >> 10 & 31) as u8]
}

/// The palette of the colours of `game`: its colours themselves when there are at most 15, else 15 colours chosen by median
/// cut (the box of the widest channel is split at the median of its pixels, until there are 15 boxes). Word 0 is transparent;
/// the colours follow in ascending order, and any word left over is 0.
#[must_use]
pub fn choose_palette(game: &Game) -> [u16; 16] {
    let mut counts: BTreeMap<[u8; 3], u32> = BTreeMap::new();
    for colour in game.pixels.iter().flatten() {
        *counts.entry(*colour).or_insert(0) += 1;
    }
    let mut boxes: Vec<Vec<([u8; 3], u32)>> = vec![counts.into_iter().collect()];
    while boxes.len() < 15 {
        // The box and the channel with the widest range; the first of them when they tie.
        let mut widest: Option<(usize, usize, u8)> = None;
        for (index, colours) in boxes.iter().enumerate() {
            if colours.len() < 2 {
                continue;
            }
            for channel in 0..3 {
                let low = colours.iter().map(|(c, _)| c[channel]).min().unwrap_or(0);
                let high = colours.iter().map(|(c, _)| c[channel]).max().unwrap_or(0);
                let extent = high - low;
                if widest.is_none_or(|(_, _, best)| extent > best) {
                    widest = Some((index, channel, extent));
                }
            }
        }
        let Some((index, channel, extent)) = widest else { break };
        if extent == 0 {
            break;
        }
        let mut colours = boxes.remove(index);
        colours.sort_by_key(|(c, _)| (c[channel], *c));
        let total: u32 = colours.iter().map(|(_, n)| n).sum();
        let mut before = 0;
        let mut split = 1;
        for (at, (_, n)) in colours.iter().enumerate() {
            before += n;
            if before * 2 >= total {
                split = (at + 1).clamp(1, colours.len() - 1);
                break;
            }
        }
        let upper = colours.split_off(split);
        boxes.push(colours);
        boxes.push(upper);
    }
    let mut words: Vec<u16> = boxes
        .iter()
        .filter(|colours| !colours.is_empty())
        .map(|colours| {
            let total: u32 = colours.iter().map(|(_, n)| n).sum();
            let mean = std::array::from_fn(|channel| {
                let sum: u32 = colours.iter().map(|(c, n)| u32::from(c[channel]) * n).sum();
                ((sum + total / 2) / total) as u8
            });
            pack_colour(mean)
        })
        .collect();
    words.sort_unstable();
    let mut palette = [0_u16; 16];
    for (slot, word) in palette[1..].iter_mut().zip(words) {
        *slot = word;
    }
    palette
}

/// The palette index of each game pixel: 0 when transparent, else the nearest colour of the palette (words 1 to 15, by the
/// squared distance of the 5-bit channels; the lower index when two are as near).
#[must_use]
pub fn map_pixels(game: &Game, palette: &[u16; 16]) -> Vec<u8> {
    let colours: Vec<[i32; 3]> =
        palette[1..].iter().map(|&word| unpack_colour(word).map(i32::from)).collect();
    game.pixels
        .iter()
        .map(|pixel| {
            let Some(colour) = pixel else { return 0 };
            let colour = colour.map(i32::from);
            let mut best = (i32::MAX, 0_u8);
            for (at, candidate) in colours.iter().enumerate() {
                let distance: i32 = (0..3).map(|k| (colour[k] - candidate[k]).pow(2)).sum();
                if distance < best.0 {
                    best = (distance, at as u8 + 1);
                }
            }
            best.1
        })
        .collect()
}

/// The 4-bit tile of the 8 x 8 palette indexes `pixels` (row by row), in the SNES layout: rows `2y` and `2y + 1` of planes 0
/// and 1, then rows `16 + 2y` and `17 + 2y` of planes 2 and 3.
#[must_use]
pub fn encode_tile(pixels: &[u8; 64]) -> [u8; 32] {
    let mut tile = [0_u8; 32];
    for y in 0..TILE {
        for x in 0..TILE {
            let value = pixels[y * TILE + x];
            for plane in 0..4 {
                if value >> plane & 1 != 0 {
                    tile[plane / 2 * 16 + 2 * y + plane % 2] |= 0x80 >> x;
                }
            }
        }
    }
    tile
}

/// The 8 x 8 indexes flipped horizontally and/or vertically.
fn flipped(pixels: &[u8; 64], h: bool, v: bool) -> [u8; 64] {
    std::array::from_fn(|at| {
        let (x, y) = (at % TILE, at / TILE);
        let (x, y) = (if h { TILE - 1 - x } else { x }, if v { TILE - 1 - y } else { y });
        pixels[y * TILE + x]
    })
}

/// The 8 x 8 cells of a `columns` x `rows` grid of game pixels `indexes` (`width` pixels wide), row by row.
fn cells_of(indexes: &[u8], width: usize, columns: usize, rows: usize) -> Vec<[u8; 64]> {
    let mut cells = Vec::with_capacity(columns * rows);
    for cy in 0..rows {
        for cx in 0..columns {
            cells.push(std::array::from_fn(|at| {
                let (x, y) = (at % TILE, at / TILE);
                indexes[(cy * TILE + y) * width + cx * TILE + x]
            }));
        }
    }
    cells
}

/// The sheet of an HD picture (`xtask art-derive ... tiles`): cut into 8 x 8 tiles, each distinct tile once, a cell that is
/// another tile flipped shows it with the flip. `palette` is the one given, else the chosen one.
///
/// # Errors
///
/// [`DeriveError::Size`] if the picture is not a multiple of `SCALE * TILE` (32 pixels) each way, [`DeriveError::Tiles`] if
/// it needs more than [`MAX_TILES`] tiles.
pub fn derive_sheet(hd: &Rgba, palette: Option<[u16; 16]>) -> Result<Sheet, DeriveError> {
    let unit = SCALE * TILE;
    if hd.width == 0
        || hd.height == 0
        || !hd.width.is_multiple_of(unit)
        || !hd.height.is_multiple_of(unit)
    {
        return Err(DeriveError::Size(format!(
            "{} x {} pixels: a map sheet is a multiple of {unit} pixels each way (8 game pixels at 4x)",
            hd.width, hd.height
        )));
    }
    let game = downsample(hd);
    let palette = palette.unwrap_or_else(|| choose_palette(&game));
    let indexes = map_pixels(&game, &palette);
    let (columns, rows) = (game.width / TILE, game.height / TILE);
    let mut tiles: Vec<[u8; 32]> = Vec::new();
    let mut seen: HashMap<[u8; 32], u16> = HashMap::new();
    let mut cells = Vec::with_capacity(columns * rows);
    let flips = [(false, false), (true, false), (false, true), (true, true)];
    for cell in cells_of(&indexes, game.width, columns, rows) {
        let found = flips.iter().find_map(|&(h, v)| {
            seen.get(&encode_tile(&flipped(&cell, h, v))).map(|&tile| Cell { tile, h, v })
        });
        if let Some(found) = found {
            cells.push(found);
            continue;
        }
        // Stops at the first tile past the budget (so the count is at least this one)
        if tiles.len() == MAX_TILES {
            return Err(DeriveError::Tiles { count: MAX_TILES + 1 });
        }
        let tile = u16::try_from(tiles.len()).unwrap_or(u16::MAX);
        let bytes = encode_tile(&cell);
        seen.insert(bytes, tile);
        tiles.push(bytes);
        cells.push(Cell { tile, h: false, v: false });
    }
    Ok(Sheet { palette, tiles, columns, rows, cells })
}

/// The face of an HD picture (`xtask art-derive ... face`, and the faces of a pack): 128 x 128 pixels, 4 x 4 cells of 8 x 8
/// tiles, in the given palette (the row the game draws the face in).
///
/// # Errors
///
/// [`DeriveError::Size`] if the picture is not 128 x 128.
pub fn derive_face(hd: &Rgba, palette: [u16; 16]) -> Result<Face, DeriveError> {
    let side = FACE_SIDE * SCALE;
    if hd.width != side || hd.height != side {
        return Err(DeriveError::Size(format!(
            "{} x {} pixels: a face is {side} x {side} (32 x 32 game pixels at 4x)",
            hd.width, hd.height
        )));
    }
    let game = downsample(hd);
    let indexes = map_pixels(&game, &palette);
    let cells = cells_of(&indexes, game.width, 4, 4);
    let tiles = std::array::from_fn(|at| encode_tile(&cells[at]));
    Ok(Face { palette, tiles })
}

/// The 32 tiles of a face record: the 16 tiles of the face in the cells of both pictures (see the module documentation).
#[must_use]
pub fn record_tiles(face: &[[u8; 32]; FACE_TILES]) -> Vec<[u8; 32]> {
    let mut tiles = vec![[0_u8; 32]; RECORD_TILES];
    for (cell, tile) in face.iter().enumerate() {
        tiles[FIRST_PICTURE[cell]] = *tile;
        tiles[SECOND_PICTURE[cell]] = *tile;
    }
    tiles
}

/// The stream of 4-bit tiles in the game's layout (see the module documentation): the inverse of the regroup `00:C68C`.
#[must_use]
pub fn stream_of(tiles: &[[u8; 32]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(tiles.len() * 32);
    for tile in tiles {
        for block in tile.as_chunks::<16>().0 {
            let mut stream = [0_u8; 16];
            for row in 0..8 {
                stream[row] = block[2 * row];
                stream[8 + row] = block[2 * row + 1];
            }
            out.extend_from_slice(&stream);
        }
    }
    out
}

/// The LZ stream of a sheet's tiles (`tiles.lz`), or `None` when it is empty.
#[must_use]
pub fn sheet_stream(tiles: &[[u8; 32]]) -> Option<Vec<u8>> {
    lz::encode(&stream_of(tiles))
}

/// The face record the game reads: the LZ stream of the 32 tiles of [`record_tiles`].
#[must_use]
pub fn face_record(face: &[[u8; 32]; FACE_TILES]) -> Option<Vec<u8>> {
    lz::encode(&stream_of(&record_tiles(face)))
}

/// The row the game draws the corner faces in (CGRAM row 6 of the field menu): the palette of object
/// [`FACE_PALETTE_OBJECT`], read from the ROM through [`OBJECT_PALETTES`] (bank `$09` is forced by the game's loader).
#[must_use]
pub fn face_palette(rom: &[u8]) -> Option<[u16; 16]> {
    let (bank, address) = OBJECT_PALETTES;
    let entry = u16::try_from(FACE_PALETTE_OBJECT * 2).ok()?;
    let pointer = rom_word(rom, bank, address.checked_add(entry)?)?;
    let base = rom_offset(0x09, pointer)?;
    let row = rom.get(base..base + 32)?;
    Some(std::array::from_fn(|at| u16::from_le_bytes([row[2 * at], row[2 * at + 1]])))
}

/// The file offset of a `LoROM` address (`$8000` and up) in a headerless image.
fn rom_offset(bank: u8, address: u16) -> Option<usize> {
    (address >= 0x8000).then(|| usize::from(bank & 0x7F) * 0x8000 + usize::from(address - 0x8000))
}

fn rom_word(rom: &[u8], bank: u8, address: u16) -> Option<u16> {
    let at = rom_offset(bank, address)?;
    Some(u16::from_le_bytes([*rom.get(at)?, *rom.get(at + 1)?]))
}

/// The palette file (`palette.toml`): `words`, sixteen 15-bit colours, word 0 transparent.
#[derive(serde::Serialize, serde::Deserialize)]
struct PaletteFile {
    words: Vec<u16>,
}

/// The text of a palette file.
#[must_use]
pub fn palette_text(palette: &[u16; 16]) -> String {
    let text = toml::to_string(&PaletteFile { words: palette.to_vec() }).unwrap_or_default();
    format!("# SNES colours (0BBBBBGGGGGRRRRR, 5 bits a channel); word 0 is transparent.\n{text}")
}

/// The palette of a palette file (see [`palette_text`]).
///
/// # Errors
///
/// [`DeriveError::Palette`] if the text is not a palette file of 16 words.
pub fn palette_from_text(text: &str) -> Result<[u16; 16], DeriveError> {
    let file: PaletteFile =
        toml::from_str(text).map_err(|e| DeriveError::Palette(e.message().to_owned()))?;
    <[u16; 16]>::try_from(file.words.as_slice()).map_err(|_| {
        DeriveError::Palette(format!("{} words, the palette holds 16", file.words.len()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An opaque picture of `width` x `height`, every pixel `colour(x, y)` (RGB, opaque).
    fn picture(width: usize, height: usize, colour: impl Fn(usize, usize) -> [u8; 3]) -> Rgba {
        let mut rgba = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = colour(x, y);
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        Rgba { width, height, rgba }
    }

    const BLACK: [u8; 3] = [0, 0, 0];
    const WHITE: [u8; 3] = [255, 255, 255];

    #[test]
    fn a_two_colour_picture_gives_its_palette_tiles_and_map() {
        // 64 x 32 HD: two game tiles side by side, the left one black, the right one white
        let hd = picture(64, 32, |x, _| if x < 32 { BLACK } else { WHITE });
        let sheet = derive_sheet(&hd, None).unwrap();
        // Black is 0 and white 0x7FFF: sorted, black is index 1 and white index 2
        assert_eq!(sheet.palette[1], pack_colour([0, 0, 0]));
        assert_eq!(sheet.palette[2], pack_colour([31, 31, 31]));
        assert!(sheet.palette[3..].iter().all(|&word| word == 0));
        assert_eq!(sheet.tiles.len(), 2, "one tile of index 1, one of index 2");
        // Index 1 is plane 0 set everywhere; index 2 is plane 1 set everywhere
        let ones = (0..8).flat_map(|_| [0xFF, 0x00]).chain([0; 16]).collect::<Vec<u8>>();
        assert_eq!(sheet.tiles[0].to_vec(), ones);
        assert_eq!(sheet.tiles[1][1], 0xFF, "row 0, plane 1");
        assert_eq!((sheet.columns, sheet.rows), (2, 1));
        assert_eq!(
            sheet.cells,
            [Cell { tile: 0, h: false, v: false }, Cell { tile: 1, h: false, v: false }]
        );
    }

    #[test]
    fn a_tile_that_is_another_flipped_is_one_tile_with_the_flip() {
        // Left tile: column 0 white, the rest black. Right tile: the same mirrored, so it is the left one flipped.
        let hd = picture(64, 32, |x, _| {
            let game_x = (x / SCALE) % 16;
            let column = if game_x < 8 { game_x } else { 15 - game_x };
            if column == 0 { WHITE } else { BLACK }
        });
        let sheet = derive_sheet(&hd, None).unwrap();
        assert_eq!(sheet.tiles.len(), 1, "the mirror is not a new tile");
        assert_eq!(sheet.cells[0], Cell { tile: 0, h: false, v: false });
        assert_eq!(
            sheet.cells[1],
            Cell { tile: 0, h: true, v: false },
            "the right cell is flipped"
        );
    }

    #[test]
    fn the_same_picture_gives_the_same_bytes() {
        let hd = picture(64, 64, |x, y| {
            [(x * 7 % 256) as u8, (y * 3 % 256) as u8, ((x ^ y) % 256) as u8]
        });
        let (a, b) = (derive_sheet(&hd, None).unwrap(), derive_sheet(&hd, None).unwrap());
        assert_eq!(a, b);
        assert_eq!(sheet_stream(&a.tiles), sheet_stream(&b.tiles));
    }

    #[test]
    fn a_palette_of_more_than_fifteen_colours_is_chosen_by_median_cut() {
        // Twenty-four different greys: the palette keeps 15 colours (words 1 to 15), all inside the picture's range
        let hd = picture(96, 32, |x, _| {
            let grey = (x / SCALE * 255 / 23) as u8;
            [grey, grey, grey]
        });
        let game = downsample(&hd);
        let palette = choose_palette(&game);
        assert_eq!(palette[0], 0);
        assert!(palette[1..].windows(2).all(|pair| pair[0] <= pair[1]), "in ascending order");
        assert!(palette[15] > palette[1], "the colours spread over the picture's range");
        let used: std::collections::BTreeSet<u8> =
            map_pixels(&game, &palette).into_iter().collect();
        assert!(used.iter().all(|&index| (1..16).contains(&index)), "{used:?}");
    }

    #[test]
    fn a_transparent_block_is_colour_zero_and_an_opaque_one_is_not() {
        let mut hd = picture(8, 4, |_, _| WHITE);
        // The left block (4 x 4) is alpha 0; the right block is alpha 255
        for y in 0..4 {
            for x in 0..4 {
                hd.rgba[(y * 8 + x) * 4 + 3] = 0;
            }
        }
        let game = downsample(&hd);
        assert_eq!(game.pixels[0], None);
        assert_eq!(game.pixels[1], Some([31, 31, 31]));
        let palette = [0, pack_colour([31, 31, 31]), 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        assert_eq!(map_pixels(&game, &palette), [0, 1]);
    }

    #[test]
    fn a_sheet_needing_more_than_432_tiles_is_refused() {
        // 433 cells of 8 x 8, all different and none equal to another flipped: a white corner at (0, 0) (never at the
        // other corners, so no flip of one cell is another cell), and 12 bits of the number in columns 1 to 6 of rows 0 and 1
        let cells = MAX_TILES + 1;
        let hd = picture(TILE * SCALE * cells, TILE * SCALE, |x, y| {
            let cell = x / (TILE * SCALE);
            let (px, py) = ((x / SCALE) % TILE, (y / SCALE) % TILE);
            let bit = py * 6 + px.saturating_sub(1);
            let number = py < 2 && (1..=6).contains(&px) && cell >> bit & 1 != 0;
            if (px, py) == (0, 0) || number { WHITE } else { BLACK }
        });
        assert_eq!(derive_sheet(&hd, None), Err(DeriveError::Tiles { count: cells }));
    }

    #[test]
    fn a_picture_of_the_wrong_size_names_its_size() {
        let not_a_sheet = picture(40, 32, |_, _| BLACK);
        assert!(
            matches!(derive_sheet(&not_a_sheet, None), Err(DeriveError::Size(text)) if text.contains("40 x 32"))
        );
        let not_a_face = picture(64, 128, |_, _| BLACK);
        assert!(matches!(derive_face(&not_a_face, [0; 16]), Err(DeriveError::Size(_))));
    }

    #[test]
    fn a_face_is_sixteen_tiles_and_its_record_holds_both_pictures() {
        let face_hd =
            picture(128, 128, |x, y| if (x / 16 + y / 16) % 2 == 0 { WHITE } else { BLACK });
        let palette =
            [0, pack_colour([0; 3]), pack_colour([31; 3]), 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];
        let face = derive_face(&face_hd, palette).unwrap();
        assert_eq!(face.tiles.len(), FACE_TILES);
        let tiles = record_tiles(&face.tiles);
        assert_eq!(tiles.len(), RECORD_TILES);
        for cell in 0..FACE_TILES {
            assert_eq!(tiles[FIRST_PICTURE[cell]], face.tiles[cell]);
            assert_eq!(tiles[SECOND_PICTURE[cell]], face.tiles[cell]);
        }
        // The stream is the 32 tiles in the game's layout: the regroup of 00:C68C gives the SNES tiles back
        let stream = stream_of(&tiles);
        assert_eq!(stream.len(), 1024);
        let regrouped: Vec<u8> = stream
            .as_chunks::<16>()
            .0
            .iter()
            .flat_map(|block| {
                let mut out = [0_u8; 16];
                for row in 0..8 {
                    out[2 * row] = block[row];
                    out[2 * row + 1] = block[8 + row];
                }
                out
            })
            .collect();
        assert_eq!(regrouped, tiles.concat());
        assert!(face_record(&face.tiles).is_some());
    }

    #[test]
    fn the_palette_file_reads_back_and_a_short_one_is_refused() {
        let palette: [u16; 16] = std::array::from_fn(|at| (at * 1000) as u16 & 0x7FFF);
        assert_eq!(palette_from_text(&palette_text(&palette)), Ok(palette));
        assert!(matches!(palette_from_text("words = [1, 2]"), Err(DeriveError::Palette(_))));
    }

    #[test]
    fn the_face_palette_is_read_through_the_object_table_in_bank_nine() {
        // A ROM with the table at $02:A356 and object 28's pointer at bank 9, $8000 (its words are zero)
        let mut rom = vec![0_u8; 0x10_0000];
        let table = rom_offset(0x02, OBJECT_PALETTES.1 + 56).unwrap();
        rom[table..table + 2].copy_from_slice(&0x8000_u16.to_le_bytes());
        let base = rom_offset(0x09, 0x8000).unwrap();
        rom[base + 2..base + 4].copy_from_slice(&0x1234_u16.to_le_bytes());
        let row = face_palette(&rom).unwrap();
        assert_eq!(row[1], 0x1234);
        assert_eq!(row[0], 0);
    }
}
