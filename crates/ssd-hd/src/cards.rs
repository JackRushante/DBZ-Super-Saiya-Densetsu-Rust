//! The HD cards and the tile art of the UI: the battle cards (a black body in a cream frame, a Dragon Ball with its stars in
//! one corner, a coloured square with a kanji in the middle, a second ball with a kanji numeral in
//! the opposite corner) drawn again from the approved art.
//!
//! The game builds a card from tiles of its 2-bit UI set: a card is 4 x 6 tiles (frame pieces, a
//! 2 x 2 ball, a 2 x 2 kanji, a 2 x 2 ball), and the same tile pieces make every combination of
//! stars, kanji, numeral and colour. The art is therefore made tile by tile: for each tile of the
//! UI set that a card uses (found by its content, like the HD text's glyphs: `font`) the art store
//! holds the 8 x 8 game pixels redrawn at 8x (64 x 64 art pixels), in up to two layers:
//!
//! * `base`, fixed colours (the frame and the ball, painted for the game's own white card
//!   palette): it is drawn only where the game's palette colour 1 is white, scaled by the colour the
//!   white shows in (a fade of the screen darkens it);
//! * `roles`, an overlay whose channels are weights of the tile palette's colours: red the weight
//!   of colour 3, green of colour 2, blue of colour 1, alpha the coverage. The kanji, the numerals
//!   and the stars are drawn in it, so they take the colours the game's palette gives them (the
//!   blue squares, the yellow and pink one, a fade).
//!
//! The pass lives in the HD text's machinery: a card tile is a "glyph" with an index from
//! [`CARD_BASE`] up, found while the 1x frame is drawn (which pixels of the tile show, and the
//! colours of the palette through the colour math), and drawn over the picture after the look and
//! the art. The art covers the pixels the tile shows, and nothing else.

use crate::font::TextCell;
use rustc_hash::FxHashMap;
use std::path::Path;
use std::sync::OnceLock;

mod face;
pub use face::{Part as CardPart, WORDS as CARD_WORDS};

/// The first glyph index that is a card (or other tile art) tile: the text glyphs are below it (97 of them).
pub(crate) const CARD_BASE: u8 = 104;
/// The most tiles of art the index may hold.
pub const MAX_TILES: usize = 255 - CARD_BASE as usize + 1;
/// Art pixels along a game tile's edge.
pub const TILE: usize = 64;
/// Art pixels along the edge of a tooltip picture (14 game pixels at 8x).
pub const TIP_PX: usize = 112;
/// The tile of the pick cursor (a down arrow above the card).
pub const CURSOR_CODE: u16 = 0x111;
/// The cards' index in the pack: `<pack>/art/cards/cards.toml`.
pub const INDEX: &str = "art/cards/cards.toml";

/// Why the cards' art cannot be read.
#[derive(Debug, thiserror::Error)]
pub enum CardsError {
    #[error("cards: {0}")]
    Read(String),
}

/// `cards.toml`: the tiles with art.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct Index {
    pub tile: Vec<Entry>,
    /// The pictures of the tooltip (a kanji in dark ink, or an emblem in colour), by the code of the
    /// tile at the top left of the card's kanji.
    #[serde(default)]
    pub tip: Vec<TipEntry>,
}

/// A tooltip picture: [`TIP_PX`] pixels square.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub struct TipEntry {
    pub code: u16,
    pub image: String,
}

/// One tile of the index: its code in the UI set (`11:8036`, 384 tiles), or in the title set (`set` is
/// `"title"`: the stream `00:DF39`, 256 tiles), and its art files, relative to the folder of the index.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Entry {
    pub code: u16,
    /// The set the code is of: `"title"`, or absent for the UI set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<String>,
    /// The art is drawn over the tile's own colour 1 (its background), so it may carve its shape out of
    /// the tile (a separator's leaves, a rounded corner) instead of covering every pixel the tile shows.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub carve: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<String>,
}

/// The art of one tile: 64 x 64 RGBA, straight alpha.
struct Art {
    code: u16,
    /// The tile is of the title set.
    title: bool,
    carve: bool,
    base: Option<Vec<u8>>,
    roles: Option<Vec<u8>>,
}

/// A tile's art resized to the picture's scale: `8 * scale` pixels square.
struct Scaled {
    carve: bool,
    base: Option<Vec<u8>>,
    roles: Option<Vec<u8>>,
}

/// The card art of a pack.
pub struct CardArt {
    tiles: Vec<Art>,
    scaled: [OnceLock<Vec<Scaled>>; 8],
    /// The tooltip pictures: the code of the kanji tile they go with, and the pixels.
    pub(crate) tips: Vec<(u16, Vec<u8>)>,
    tip_scaled: [OnceLock<Vec<Vec<u8>>>; 8],
    /// The tooltip last shown (the cursor blinks, the tooltip stays).
    pub(crate) seen: std::sync::Mutex<Option<crate::tooltip::Seen>>,
}

impl std::fmt::Debug for CardArt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CardArt").field("tiles", &self.tiles.len()).finish_non_exhaustive()
    }
}

impl CardArt {
    /// The art listed by `<pack>/art/cards/cards.toml`; `None` when the pack has none.
    ///
    /// # Errors
    ///
    /// The index or an art file cannot be read, or an art file is not 64 x 64.
    pub fn load(pack: &Path) -> Result<Option<Self>, CardsError> {
        let index_path = pack.join(INDEX);
        let Ok(text) = std::fs::read_to_string(&index_path) else { return Ok(None) };
        let index: Index = toml::from_str(&text).map_err(|e| CardsError::Read(e.to_string()))?;
        let folder = index_path.parent().unwrap_or(pack);
        let read_sized = |name: &str, side: usize| -> Result<Vec<u8>, CardsError> {
            let bytes = std::fs::read(folder.join(name))
                .map_err(|e| CardsError::Read(format!("{name}: {e}")))?;
            let (w, h, rgba) = crate::png::read_rgba(&bytes)
                .map_err(|e| CardsError::Read(format!("{name}: {e}")))?;
            if (w as usize, h as usize) != (side, side) {
                return Err(CardsError::Read(format!("{name}: {w}x{h}, expected {side}x{side}")));
            }
            Ok(rgba)
        };
        let read = |name: &str| read_sized(name, TILE);
        // More tiles than the glyph space holds: the first ones are kept and the rest skipped, so the
        // HD text is never lost for the sake of the art
        if index.tile.len() > MAX_TILES {
            log::warn!(
                "the card art lists {} tiles, more than the {MAX_TILES} it can hold: the last {} are skipped",
                index.tile.len(),
                index.tile.len() - MAX_TILES
            );
        }
        let mut tiles = Vec::new();
        for entry in index.tile.iter().take(MAX_TILES) {
            tiles.push(Art {
                code: entry.code,
                title: entry.set.as_deref() == Some("title"),
                carve: entry.carve,
                base: entry.base.as_deref().map(read).transpose()?,
                roles: entry.roles.as_deref().map(read).transpose()?,
            });
        }
        let mut tips = Vec::new();
        for tip in &index.tip {
            tips.push((tip.code, read_sized(&tip.image, TIP_PX)?));
        }
        Ok(Some(Self::new(tiles, tips)))
    }

    fn new(tiles: Vec<Art>, tips: Vec<(u16, Vec<u8>)>) -> Self {
        Self {
            tiles,
            scaled: std::array::from_fn(|_| OnceLock::new()),
            tips,
            tip_scaled: std::array::from_fn(|_| OnceLock::new()),
            seen: std::sync::Mutex::new(None),
        }
    }

    /// Card art of tiles that draw nothing but a flat colour, for the tests of the passes built on it.
    #[cfg(test)]
    pub(crate) fn flat(codes: &[u16], tips: Vec<(u16, Vec<u8>)>) -> Self {
        let tiles = codes
            .iter()
            .map(|&code| Art {
                code,
                title: false,
                carve: false,
                base: None,
                roles: Some([255, 0, 0, 255].repeat(TILE * TILE)),
            })
            .collect();
        Self::new(tiles, tips)
    }

    /// Card art of tiles of the title set that draw nothing but a flat colour, for the tests of the binding.
    #[cfg(test)]
    pub(crate) fn flat_title(codes: &[u16]) -> Self {
        let tiles = codes
            .iter()
            .map(|&code| Art {
                code,
                title: true,
                carve: true,
                base: None,
                roles: Some([255, 0, 0, 255].repeat(TILE * TILE)),
            })
            .collect();
        Self::new(tiles, Vec::new())
    }

    /// The glyph index of the tile of the UI set with code `code`, when it has art.
    pub(crate) fn glyph_of(&self, code: u16) -> Option<u8> {
        self.tiles
            .iter()
            .position(|tile| tile.code == code && !tile.title)
            .map(|n| CARD_BASE + n as u8)
    }

    /// The code of the tile of the UI set of glyph `glyph`.
    pub(crate) fn code_of(&self, glyph: u8) -> Option<u16> {
        let tile = self.tiles.get(usize::from(glyph.checked_sub(CARD_BASE)?))?;
        (!tile.title).then_some(tile.code)
    }

    /// The tooltip picture that goes with the kanji tile `code`, 14 game pixels square at `scale`.
    pub(crate) fn tip_picture(&self, scale: usize, code: u16) -> Option<&[u8]> {
        let all = self.tip_scaled[scale - 1].get_or_init(|| {
            self.tips
                .iter()
                .map(|(_, art)| resize_rgba(art, TIP_PX, TIP_PX, 14 * scale, 14 * scale))
                .collect()
        });
        let at = self.tips.iter().position(|(tip, _)| *tip == code)?;
        all.get(at).map(Vec::as_slice)
    }

    /// The art of the card symbol `symbol` (0 to 6, a fighter's class too): its tooltip picture, the side in pixels and the
    /// RGBA pixels, for the menus that show a class. `None` for a number that is no symbol, or when the art has no picture of it.
    #[must_use]
    pub fn symbol(&self, symbol: u8) -> Option<(usize, &[u8])> {
        let code = crate::tooltip::TIPS.get(usize::from(symbol))?.code;
        let (_, art) = self.tips.iter().find(|(tip, _)| *tip == code)?;
        Some((TIP_PX, art.as_slice()))
    }

    /// The tiles with art, in glyph order (glyph `CARD_BASE + n` is the `n`th): their code, and whether it
    /// is of the title set.
    pub(crate) fn tiles(&self) -> impl Iterator<Item = (u16, bool)> + '_ {
        self.tiles.iter().map(|tile| (tile.code, tile.title))
    }

    fn scaled(&self, scale: usize) -> &[Scaled] {
        self.scaled[scale - 1].get_or_init(|| {
            let side = 8 * scale;
            self.tiles
                .iter()
                .map(|tile| Scaled {
                    carve: tile.carve,
                    base: tile.base.as_deref().map(|art| resize(art, side)),
                    roles: tile.roles.as_deref().map(|art| resize(art, side)),
                })
                .collect()
        })
    }

    /// Draws the card tile `cell` into the picture `rgba` (`width` pixels wide, `scale` times the
    /// 1x frame): the pixels of the tile that show.
    pub(crate) fn draw(&self, rgba: &mut [u8], width: usize, scale: usize, cell: &TextCell) {
        if !(2..=crate::MAX_SCALE as usize).contains(&scale) {
            return;
        }
        let Some(art) = self.scaled(scale).get(usize::from(cell.glyph - CARD_BASE)) else {
            return;
        };
        let height = rgba.len() / 4 / width;
        let side = 8 * scale;
        for bit in 0..64_usize {
            if cell.mask >> bit & 1 == 0 {
                continue;
            }
            let (column, row) = (bit % 8, bit / 8);
            for sy in 0..scale {
                let y = (cell.y + row as i32) * scale as i32 + sy as i32;
                if !(0..height as i32).contains(&y) {
                    continue;
                }
                for sx in 0..scale {
                    let x = (cell.x + column as i32) * scale as i32 + sx as i32;
                    if !(0..width as i32).contains(&x) {
                        continue;
                    }
                    let at = (row * scale + sy) * side + column * scale + sx;
                    let out = (y as usize * width + x as usize) * 4;
                    if art.carve {
                        rgba[out..out + 3].copy_from_slice(&cell.bg);
                    }
                    if let (Some(base), Some(white)) = (&art.base, cell.white) {
                        let p = &base[at * 4..at * 4 + 4];
                        let colour = [0, 1, 2]
                            .map(|c| ((u32::from(p[c]) * u32::from(white[c]) + 127) / 255) as u8);
                        blend(&mut rgba[out..out + 3], colour, p[3]);
                    }
                    if let Some(roles) = &art.roles {
                        let p = &roles[at * 4..at * 4 + 4];
                        let weight = [u32::from(p[0]), u32::from(p[1]), u32::from(p[2])];
                        let total = (weight[0] + weight[1] + weight[2]).max(1);
                        let colour = [0, 1, 2].map(|c| {
                            let mixed = weight[0] * u32::from(cell.fg[c])
                                + weight[1] * u32::from(cell.mid[c])
                                + weight[2] * u32::from(cell.bg[c]);
                            ((mixed + total / 2) / total) as u8
                        });
                        blend(&mut rgba[out..out + 3], colour, p[3]);
                    }
                }
            }
        }
    }
}

/// The four tiles of a 2 x 2 block of a card, row by row.
type Quad = [u16; 4];
/// The attack ball with four dots, and with the Z.
const ATTACK_DOTS: Quad = [0x96, 0x97, 0xA6, 0xA7];
const ATTACK_Z: Quad = [0x9E, 0x9F, 0xAE, 0xAF];
/// The defence ball with the kanji seven, and with the Z.
const DEFENCE_SEVEN: Quad = [0xBC, 0xBD, 0xCC, 0xCD];
const DEFENCE_Z: Quad = [0xBE, 0xBF, 0xCE, 0xCF];
/// The symbol in the middle: the kanji 必.
const SYMBOL_MUST: Quad = [0xF0, 0xF1, 0x12D, 0x12E];
/// The plain black tile that fills the card's body.
const BLACK: u16 = 0x03;
/// The colours the cards' tiles take in the game's palettes (colour 3, 2, 1): the frame and the balls
/// (black, orange, white), and the symbol's square (blue ink, orange ring, cream).
const BODY_COLOURS: [[u8; 3]; 3] = [[0, 0, 0], [0xFF, 0x8B, 0x00], [0xFF, 0xFF, 0xFF]];
const SYMBOL_COLOURS: [[u8; 3]; 3] = [[0x52, 0x4A, 0xFF], [0xFF, 0x52, 0x00], [0xFF, 0xEE, 0xCD]];
/// The art pixels along a game pixel's edge of [`CardArt::preview`].
const PREVIEW_SCALE: usize = 8;
/// The tiles across and down a card.
pub const PREVIEW_TILES: (usize, usize) = (4, 6);

/// A card drawn for a screen of the HD menus: RGBA, straight alpha.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preview {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}

impl CardArt {
    /// A whole card as the battle's hand shows it, drawn from the art of its tiles: the attack ball with
    /// four dots or the Z, the symbol 必 and the defence ball with the numeral seven or the Z, on the black
    /// body in its cream frame (`PREVIEW_TILES` tiles at 8x). `None` when the art lacks a tile of it.
    #[must_use]
    pub fn preview(&self, attack_z: bool, defence_z: bool) -> Option<Preview> {
        let attack = if attack_z { ATTACK_Z } else { ATTACK_DOTS };
        let defence = if defence_z { DEFENCE_Z } else { DEFENCE_SEVEN };
        let symbol = SYMBOL_MUST;
        let grid: [[u16; 4]; 6] = [
            [attack[0], attack[1], 0x94, 0x95],
            [attack[2], attack[3], BLACK, 0xA5],
            [0x92, symbol[0], symbol[1], 0xA5],
            [0x92, symbol[2], symbol[3], 0xA5],
            [0x92, BLACK, defence[0], defence[1]],
            [0xA2, 0xA4, defence[2], defence[3]],
        ];
        let side = 8 * PREVIEW_SCALE;
        let (width, height) = (PREVIEW_TILES.0 * side, PREVIEW_TILES.1 * side);
        let mut rgba = vec![0_u8; width * height * 4];
        let mut put = |x: usize, y: usize, colour: [u8; 3], alpha: u8| {
            let at = (y * width + x) * 4;
            let under = u32::from(rgba[at + 3]);
            let a = u32::from(alpha);
            // Straight alpha, the colour over what is there
            let out = a * 255 + under * (255 - a);
            if out == 0 {
                return;
            }
            for c in 0..3 {
                let mixed =
                    u32::from(colour[c]) * a * 255 + u32::from(rgba[at + c]) * under * (255 - a);
                rgba[at + c] = ((mixed + out / 2) / out) as u8;
            }
            rgba[at + 3] = ((out + 127) / 255) as u8;
        };
        for (row, codes) in grid.iter().enumerate() {
            for (column, &code) in codes.iter().enumerate() {
                let (x0, y0) = (column * side, row * side);
                if code == BLACK {
                    for y in y0..y0 + side {
                        for x in x0..x0 + side {
                            put(x, y, [0, 0, 0], 255);
                        }
                    }
                    continue;
                }
                let art = self.tiles.iter().find(|tile| tile.code == code)?;
                let in_symbol = (2..4).contains(&row) && (1..3).contains(&column);
                let colours = if in_symbol { SYMBOL_COLOURS } else { BODY_COLOURS };
                let (base, roles) = (&art.base, &art.roles);
                for y in 0..side {
                    for x in 0..side {
                        // The art's pixels are 64 across a tile: the preview is drawn at 8x, as they are
                        let at = (y * TILE / side) * TILE + x * TILE / side;
                        if let Some(base) = base {
                            let p = &base[at * 4..at * 4 + 4];
                            put(x0 + x, y0 + y, [p[0], p[1], p[2]], p[3]);
                        }
                        if let Some(roles) = roles {
                            let p = &roles[at * 4..at * 4 + 4];
                            let weight = [u32::from(p[0]), u32::from(p[1]), u32::from(p[2])];
                            let total = (weight[0] + weight[1] + weight[2]).max(1);
                            let colour = [0, 1, 2].map(|c| {
                                let mixed = weight[0] * u32::from(colours[0][c])
                                    + weight[1] * u32::from(colours[1][c])
                                    + weight[2] * u32::from(colours[2][c]);
                                ((mixed + total / 2) / total) as u8
                            });
                            put(x0 + x, y0 + y, colour, p[3]);
                        }
                    }
                }
            }
        }
        Some(Preview { width, height, rgba })
    }
}

/// `under` mixed with `colour` by `alpha`.
fn blend(under: &mut [u8], colour: [u8; 3], alpha: u8) {
    if alpha == 0 {
        return;
    }
    for c in 0..3 {
        under[c] = ((u32::from(under[c]) * (255 - u32::from(alpha))
            + u32::from(colour[c]) * u32::from(alpha)
            + 127)
            / 255) as u8;
    }
}

/// A 64 x 64 RGBA tile resized to `side` pixels square (see [`resize_rgba`]).
fn resize(art: &[u8], side: usize) -> Vec<u8> {
    if side == TILE { art.to_vec() } else { resize_rgba(art, TILE, TILE, side, side) }
}

/// An RGBA image of `sw` x `sh` pixels resized to `ow` x `oh`: the area average of the source
/// pixels each output pixel covers, weighted by alpha (integer arithmetic, so the output does not
/// depend on the machine).
pub(crate) fn resize_rgba(art: &[u8], sw: usize, sh: usize, ow: usize, oh: usize) -> Vec<u8> {
    let mut out = vec![0_u8; ow * oh * 4];
    // Each output pixel covers a range of source pixels, in sixteenths
    let span = |i: usize, o: usize, s: usize| ((i * s * 16) / o, ((i + 1) * s * 16) / o);
    for y in 0..oh {
        let (y0, y1) = span(y, oh, sh);
        for x in 0..ow {
            let (x0, x1) = span(x, ow, sw);
            let mut sums = [0_u64; 4];
            let mut area = 0_u64;
            for sy in y0 / 16..y1.div_ceil(16) {
                let wy = (y1.min((sy + 1) * 16) - y0.max(sy * 16)) as u64;
                for sx in x0 / 16..x1.div_ceil(16) {
                    let wx = (x1.min((sx + 1) * 16) - x0.max(sx * 16)) as u64;
                    let weight = wx * wy;
                    let p = &art[(sy * sw + sx) * 4..][..4];
                    let alpha = u64::from(p[3]);
                    for c in 0..3 {
                        sums[c] += u64::from(p[c]) * alpha * weight;
                    }
                    sums[3] += alpha * weight;
                    area += weight;
                }
            }
            let at = (y * ow + x) * 4;
            if sums[3] == 0 {
                continue;
            }
            for c in 0..3 {
                out[at + c] = ((sums[c] + sums[3] / 2) / sums[3]) as u8;
            }
            out[at + 3] = ((sums[3] + area / 2) / area) as u8;
        }
    }
    out
}

/// Maps the tile contents of the codes of `art` (read from the cartridge image's UI set, or its title
/// set) to their glyph indexes (`CARD_BASE + n`), without overriding the glyphs already there. A title
/// tile is bound in both forms VRAM may hold it in: as the stream has it, and with colour 1 clear.
pub(crate) fn register(
    rom: &[u8],
    art: &CardArt,
    tiles: &mut FxHashMap<[u16; 8], u8>,
) -> Result<(), crate::font::FontError> {
    let title =
        art.tiles().any(|(_, title)| title).then(|| crate::font::title_tiles(rom)).flatten();
    for (n, (code, is_title)) in art.tiles().enumerate() {
        let glyph = CARD_BASE + n as u8;
        if is_title {
            // A title tile the image does not have is skipped (a short image has no title stream)
            let Some(words) = title.as_deref().and_then(|set| crate::font::words_in(set, code))
            else {
                continue;
            };
            tiles.entry(words).or_insert(glyph);
            tiles.entry(crate::font::clear_colour_one(words)).or_insert(glyph);
        } else {
            let words =
                crate::font::tile_words(rom, code).ok_or(crate::font::FontError::ShortRom)?;
            tiles.entry(words).or_insert(glyph);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn art(code: u16, base: Option<[u8; 4]>, roles: Option<[u8; 4]>) -> Art {
        let fill = |p: [u8; 4]| p.iter().copied().cycle().take(TILE * TILE * 4).collect();
        Art { code, title: false, carve: false, base: base.map(fill), roles: roles.map(fill) }
    }

    fn cards(tiles: Vec<Art>) -> CardArt {
        CardArt::new(tiles, Vec::new())
    }

    fn cell(glyph: u8, mask: u64, white: Option<[u8; 3]>) -> TextCell {
        TextCell {
            x: 1,
            y: 1,
            glyph,
            half: 0,
            clear: false,
            bold: false,
            layer: 0,
            fg: [10, 20, 30],
            mid: [100, 110, 120],
            bg: [200, 210, 220],
            white,
            mask,
        }
    }

    #[test]
    fn a_resize_keeps_a_flat_tile_flat_and_weights_colours_by_alpha() {
        let flat = art(1, Some([10, 20, 30, 255]), None).base.unwrap();
        for side in [16, 24, 40, 48, 56] {
            let small = resize(&flat, side);
            assert_eq!(small.len(), side * side * 4);
            assert!(small.chunks(4).all(|p| p == [10, 20, 30, 255]), "side {side}");
        }
        // A half transparent red left half and an opaque blue right half
        let mut tile = vec![0_u8; TILE * TILE * 4];
        for (at, p) in tile.chunks_mut(4).enumerate() {
            p.copy_from_slice(if at % TILE < TILE / 2 {
                &[255, 0, 0, 0]
            } else {
                &[0, 0, 255, 255]
            });
        }
        let small = resize(&tile, 16);
        assert_eq!(&small[..4], [0, 0, 0, 0], "a transparent pixel stays so");
        assert_eq!(&small[15 * 4..16 * 4], [0, 0, 255, 255]);
    }

    #[test]
    fn the_base_takes_the_white_it_shows_in_and_the_roles_the_palette_colours() {
        let both = art(1, Some([255, 255, 255, 255]), Some([0, 255, 0, 255]));
        let cards = cards(vec![both]);
        let (width, scale) = (16 * 4, 4);
        let mut rgba = vec![7_u8; width * width * 4];
        // Only pixel (0, 0) of the tile shows
        let shown = cell(CARD_BASE, 1, Some([128, 128, 128]));
        cards.draw(&mut rgba, width, scale, &shown);
        let at = |x: usize, y: usize| &rgba[(y * width + x) * 4..][..3];
        // The roles layer is over the base: colour 2 (green weight)
        assert_eq!(at(scale, scale), [100, 110, 120]);
        assert_eq!(at(scale + scale - 1, scale + scale - 1), [100, 110, 120]);
        assert_eq!(at(2 * scale, scale), [7, 7, 7], "the next pixel is not shown");
        // Without a white, the base is not drawn
        let only_base = cards_with_base_only();
        let mut rgba = vec![7_u8; width * width * 4];
        only_base.draw(&mut rgba, width, scale, &cell(CARD_BASE, 1, None));
        assert_eq!(&rgba[(scale * width + scale) * 4..][..3], [7, 7, 7]);
        let mut rgba = vec![7_u8; width * width * 4];
        only_base.draw(&mut rgba, width, scale, &cell(CARD_BASE, 1, Some([128, 64, 0])));
        assert_eq!(&rgba[(scale * width + scale) * 4..][..3], [128, 64, 0]);
    }

    fn cards_with_base_only() -> CardArt {
        cards(vec![art(1, Some([255, 255, 255, 255]), None)])
    }

    #[test]
    fn the_preview_card_is_made_of_the_battle_cards_tiles() {
        let mut codes: Vec<u16> =
            vec![0x96, 0x97, 0xA6, 0xA7, 0xBC, 0xBD, 0xCC, 0xCD, 0xF0, 0xF1, 0x12D, 0x12E];
        codes.extend([0x94, 0x95, 0xA5, 0x92, 0xA2, 0xA4]);
        let all = CardArt::flat(&codes, Vec::new());
        let card = all.preview(false, false).expect("every tile has art");
        assert_eq!((card.width, card.height), (4 * 64, 6 * 64));
        let at = |x: usize, y: usize| &card.rgba[(y * card.width + x) * 4..][..4];
        // The flat art is colour 3 of its tile's palette: the symbol's blue in the middle, black on the
        // frame; the plain black tile is opaque black
        assert_eq!(at(64 + 3, 2 * 64 + 3), [0x52, 0x4A, 0xFF, 255]);
        assert_eq!(at(3, 2 * 64 + 3), [0, 0, 0, 255]);
        assert_eq!(at(2 * 64 + 3, 64 + 3), [0, 0, 0, 255]);
        // The Z balls are other tiles: without their art there is no preview
        assert!(all.preview(true, false).is_none());
        assert!(all.preview(false, true).is_none());
        codes.extend([0x9E, 0x9F, 0xAE, 0xAF, 0xBE, 0xBF, 0xCE, 0xCF]);
        assert!(CardArt::flat(&codes, Vec::new()).preview(true, true).is_some());
    }

    #[test]
    fn more_tiles_than_the_glyph_space_holds_keep_the_first_and_skip_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("art/cards");
        std::fs::create_dir_all(&folder).unwrap();
        crate::png::write_rgba(&folder.join("t.png"), 64, 64, &[7_u8; 64 * 64 * 4]).unwrap();
        let count = MAX_TILES + 20;
        assert!(count > 127, "more than the old limit of 127");
        let tile: Vec<Entry> = (0..count)
            .map(|n| Entry { code: n as u16, roles: Some("t.png".to_owned()), ..Entry::default() })
            .collect();
        let toml = toml::to_string(&Index { tile, tip: Vec::new() }).unwrap();
        std::fs::write(dir.path().join(INDEX), toml).unwrap();
        let cards = CardArt::load(dir.path()).unwrap().expect("the art loads");
        assert_eq!(cards.tiles().count(), MAX_TILES);
        assert_eq!(cards.glyph_of(MAX_TILES as u16 - 1), Some(255));
        assert_eq!(cards.glyph_of(MAX_TILES as u16), None, "the tiles past the space are skipped");
    }

    #[test]
    fn a_carving_tile_is_drawn_over_its_own_colour_one_and_may_leave_it_showing() {
        // Roles art that is empty (alpha 0) on the left half of the first pixel of the tile and
        // colour 3 on the right half (a pixel of the tile is 8 art pixels across)
        let mut half = vec![0_u8; TILE * TILE * 4];
        for (at, p) in half.chunks_mut(4).enumerate() {
            if at % TILE >= 4 {
                p.copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let tile = |carve: bool| Art {
            code: 1,
            title: false,
            carve,
            base: None,
            roles: Some(half.clone()),
        };
        let (width, scale) = (16 * 4, 4);
        let pixel = |rgba: &[u8], x: usize| rgba[(scale * width + x) * 4..][..3].to_vec();
        // The pixel (0, 0) of the cell shows, the picture under it is grey
        let draw = |carve: bool| {
            let mut rgba = vec![77_u8; width * width * 4];
            let cell = TextCell { glyph: CARD_BASE, mask: 1, ..cell(CARD_BASE, 1, None) };
            cards(vec![tile(carve)]).draw(&mut rgba, width, scale, &cell);
            rgba
        };
        // Without carving, the empty half shows the look (grey); with it, the cell's colour 1
        // (the background of the tile), and the drawn half is colour 3 either way
        assert_eq!(pixel(&draw(false), scale), [77, 77, 77]);
        assert_eq!(pixel(&draw(true), scale), [200, 210, 220]);
        assert_eq!(pixel(&draw(true), scale + scale - 1), [10, 20, 30]);
        assert_eq!(pixel(&draw(false), scale + scale - 1), [10, 20, 30]);
    }

    #[test]
    fn a_tile_is_found_by_its_content() {
        let mut rom = vec![0_u8; crate::font::FONT_OFFSET + 384 * 16];
        let at = crate::font::FONT_OFFSET + 5 * 16;
        rom[at] = 0xAA;
        rom[at + 1] = 0x55;
        let cards = cards(vec![art(5, Some([1, 2, 3, 255]), None)]);
        let mut tiles = FxHashMap::default();
        register(&rom, &cards, &mut tiles).unwrap();
        let mut words = [0_u16; 8];
        words[0] = 0x55AA;
        assert_eq!(tiles.get(&words), Some(&CARD_BASE));
        assert!(register(&[0; 10], &cards, &mut tiles).is_err());
    }
}
