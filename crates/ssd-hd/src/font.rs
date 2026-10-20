//! The HD text: the letters, digits and marks of the game's text tile sets, drawn again from Share
//! Tech Mono (SIL OFL, `assets/fonts/`) at the picture's scale, in the colours the game draws each
//! glyph in.
//!
//! The game draws its text as 8 x 8 tiles of 2-bit fonts, from two sets:
//!
//! * the UI set, the English tile set the patch writes over `08:8000` (`11:8036`): the dialogue,
//!   the menus, the names and numbers, the captions of the endings;
//! * the title set, the stream `00:DF39` that the title, the intro and the credits draw with. Its
//!   white letters, digits and marks are one tile each; its bold letters are two tiles stacked, 16
//!   pixels tall, the top one in colour 2 and the one under it in colour 3. The game writes the
//!   white letters with colour 1 clear (where the stream has colour 1, VRAM reads colour 0), so they
//!   are bound through that form, and the picture beside a clear glyph shows where its colour 1 is.
//!
//! A tile is a glyph of the text when its eight words are those of a glyph tile of the ROM's sets
//! (found by content, whichever BG layer and palette show it). While the renderer draws the 1x frame
//! it notes, for each such tile cell, where the cell is, which of its 64 pixels show, and the colours
//! the pixels showed (through the colour math and brightness). [`Font::draw_with`] then fills those
//! pixels of the picture with the cell's background colour (or, for a clear glyph, the picture
//! beneath it) and draws the glyph over them, anti-aliased. The
//! metrics are those chosen for the UI set (cap height 7 and x-height 5 pixels of the
//! original, one glyph per 8-pixel cell, the font stretched to 1.3 times its width, a stroke of 1/16
//! of an original pixel added to match the original's weight). A bold letter takes the same metrics
//! at twice the em, so it is 16 pixels tall, and each of its two tiles draws its own half in its own
//! colour. The font and the tiles are only read; nothing of the ROM's pixels is kept: the glyph
//! shapes come from the vendored font.
//!
//! The pass is off (the picture is the look alone) unless a renderer is given a [`Font`] and the
//! scale is 2 or more, so the 1x picture stays the PPU's frame.

use crate::Pixel;
use crate::cards::{CARD_BASE, CardArt};
use crate::screen::{Layer, Plan, Rendered, convert, resolve_color};
use ab_glyph::{Font as _, FontRef, PxScale, point};
use rustc_hash::FxHashMap;
use ssd_catalogue::{RomView, graphics};
use ssd_oracle::{Color, FrameLog, LineRegs};
use std::sync::{Arc, OnceLock};

/// Share Tech Mono (SIL OFL 1.1, Carrois Type Design; the licence is beside it): the game's HD text
/// font. It is loaded once here, and the HD menus of `ssd-app` draw with the same bytes.
pub const SHARE_TECH_MONO: &[u8] =
    include_bytes!("../../../assets/fonts/ShareTechMono-Regular.ttf");

/// Where the English UI tile set is in the cartridge image: `11:8036` (`LoROM`), 384 tiles of 2 bits.
pub(crate) const FONT_OFFSET: usize = 0x11 * 0x8000 + 0x36;
const FONT_TILES: usize = 384;

/// The title set: the LZ stream at `00:DF39`, 256 tiles of 2 bits once decoded.
const TITLE_STREAM: u32 = 0x00_DF39;
const TITLE_TILES: usize = 256;
/// The tile under a bold letter's top tile (the sheet's tiles are 16 to a row).
const UNDER: u8 = 16;

/// The em of a one-tile glyph in pixels of the original: a cap height of 7 (0.7 em).
const EM: f32 = 10.0;
/// The horizontal stretch that gives the font the width of the original's letters.
const STRETCH: f32 = 1.3;
/// Where the glyph starts in its cell (original pixels) and where its baseline is.
const ORIGIN_X: f32 = -0.5;
const BASELINE: f32 = 7.0;
/// A bold letter: twice the em and baseline (a cap height of 14 in its 16 pixels), and the stretch
/// that fits its 8-pixel width.
const TALL_EM: f32 = 20.0;
const TALL_STRETCH: f32 = 0.9;
const TALL_BASELINE: f32 = 14.0;
/// Original pixels around the cell that a glyph may reach (descenders, and the overhang).
pub(crate) const MARGIN: usize = 2;
/// Samples per picture pixel the glyphs are rasterised at, about (the picture scale times this is
/// at least 16 samples per original pixel).
const SAMPLES: usize = 16;

/// The characters of the game's UI font: the tile code of the UI set and what it shows. The codes are
/// those of `ssd_catalogue::decode_tile` (our own table, read off the font on screen).
const TILE_CHARS: &[(u8, char)] = &[
    (0x01, ' '),
    (0x05, 'a'),
    (0x07, 'b'),
    (0x09, 'c'),
    (0x0A, 'd'),
    (0x0C, 'e'),
    (0x0D, 'f'),
    (0x0E, 'g'),
    (0x11, 'h'),
    (0x15, 'i'),
    (0x16, 'j'),
    (0x17, 'k'),
    (0x18, 'l'),
    (0x19, 'm'),
    (0x1A, 'n'),
    (0x1B, 'o'),
    (0x1D, 'p'),
    (0x51, 'q'),
    (0x52, 'r'),
    (0x53, 's'),
    (0x54, 't'),
    (0x55, 'u'),
    (0x56, 'v'),
    (0x57, 'w'),
    (0x58, 'x'),
    (0x5B, 'y'),
    (0x5C, 'z'),
    (0xE8, 'A'),
    (0x8C, 'B'),
    (0xE9, 'C'),
    (0xEA, 'D'),
    (0xEB, 'E'),
    (0xEC, 'F'),
    (0xED, 'G'),
    (0x8A, 'H'),
    (0x8E, 'I'),
    (0x04, 'J'),
    (0x8D, 'K'),
    (0xEE, 'L'),
    (0x0F, 'M'),
    (0x50, 'N'),
    (0x5A, 'O'),
    (0x8B, 'P'),
    (0x12, 'Q'),
    (0x06, 'R'),
    (0x14, 'S'),
    (0x47, 'T'),
    (0x0B, 'U'),
    (0xEF, 'V'),
    (0x10, 'W'),
    (0x08, 'X'),
    (0x45, 'Y'),
    (0x13, 'Z'),
    (0x80, '0'),
    (0x81, '1'),
    (0x82, '2'),
    (0x83, '3'),
    (0x84, '4'),
    (0x85, '5'),
    (0x86, '6'),
    (0x87, '7'),
    (0x88, '8'),
    (0x89, '9'),
    (0x1C, '-'),
    (0x3E, '?'),
    (0x3F, '!'),
    (0x42, '\''),
    (0x7B, ','),
    (0x7C, '.'),
    (0x7E, '"'),
    (0x7F, '"'),
];

/// The title set's white glyphs: the tile of `00:DF39` (its index in the decoded stream) and what it
/// shows, read off the title, the intro and the credits on screen.
const TITLE_CHARS: &[(u8, char)] = &[
    (0x80, '0'),
    (0x81, '1'),
    (0x82, '2'),
    (0x83, '3'),
    (0x84, '4'),
    (0x85, '5'),
    (0x86, '6'),
    (0x87, '7'),
    (0x88, '8'),
    (0x89, '9'),
    (0x42, '\''),
    (0x8A, '-'),
    (0x8B, '.'),
    (0x8C, ','),
    (0xFE, '©'),
    (0xFF, '/'),
    (0xDA, 'A'),
    (0xDB, 'B'),
    (0xDC, 'C'),
    (0xDD, 'D'),
    (0xDE, 'E'),
    (0xDF, 'F'),
    (0xEA, 'G'),
    (0xEB, 'H'),
    (0xEC, 'I'),
    (0xED, 'J'),
    (0xEE, 'K'),
    (0xEF, 'L'),
    (0xF0, 'M'),
    (0xF1, 'N'),
    (0xF2, 'O'),
    (0xF3, 'P'),
    (0xF4, 'Q'),
    (0xF5, 'R'),
    (0xF6, 'S'),
    (0xF7, 'T'),
    (0xF8, 'U'),
    (0xF9, 'V'),
    (0xFA, 'W'),
    (0xFB, 'X'),
    (0xFC, 'Y'),
    (0xFD, 'Z'),
];

/// The title set's bold letters: the top tile of each (its index in `00:DF39`); the tile under it is
/// [`UNDER`] tiles on. Read off the sheet of the tiles and checked against the names of the credits.
const TITLE_TALL: &[(u8, char)] = &[
    (0x90, 'A'),
    (0x91, 'B'),
    (0x92, 'C'),
    (0x93, 'D'),
    (0x94, 'E'),
    (0x95, 'F'),
    (0x96, 'G'),
    (0x97, 'H'),
    (0x98, 'I'),
    (0x99, 'J'),
    (0x9A, 'K'),
    (0x9B, 'L'),
    (0x9C, 'M'),
    (0x9D, 'N'),
    (0x9E, 'O'),
    (0x9F, 'P'),
    (0xB1, 'R'),
    (0xB2, 'S'),
    (0xB3, 'T'),
    (0xB4, 'U'),
    (0xB5, 'V'),
    (0xB6, 'W'),
    (0xB7, 'X'),
    (0xB8, 'Y'),
    (0xB9, 'Z'),
    (0xBD, '.'),
];

/// The blank glyph: a cell of background only.
const BLANK: u8 = 0;

/// A glyph's shape: its character, and how many tiles tall it is (1, or 2 for a bold letter).
type Shape = (char, u8);

/// A glyph's alpha mask at a scale: `w` by `h` pixels, its cell at `MARGIN * scale` from the top left.
struct Mask {
    w: usize,
    h: usize,
    alpha: Vec<u8>,
    /// The box of the ink (the pixels of at least a half), in original pixels from the cell's top
    /// left (x0, y0, x1, y1).
    ink: (f32, f32, f32, f32),
}

/// The HD text font: which tiles are glyphs, and the glyphs' shapes at each scale.
pub struct Font {
    face: FontRef<'static>,
    /// The tile contents (eight words, as VRAM holds them) of the glyph tiles of the text sets, to
    /// what they show.
    tiles: FxHashMap<[u16; 8], Hit>,
    /// The bold letters by their two halves (the upper tile's contents, and the tile 16 characters
    /// under it), to the glyph. A half alone does not always tell its letter: the lower halves of O,
    /// S, C, J and U are one tile, so a bold half is bound with the half beside it.
    pairs: FxHashMap<([u16; 8], [u16; 8]), u8>,
    /// The card art's tiles by their contents (glyphs from `CARD_BASE` up), when the pack has it.
    card_tiles: FxHashMap<[u16; 8], u8>,
    /// The glyphs, by their index in a [`TextCell`]: a character and its height in tiles. Index 0 is
    /// the space.
    shapes: Vec<Shape>,
    masks: [OnceLock<Vec<Mask>>; 8],
    /// The masks of the bold glyphs (the same shapes, a wider stroke), by scale.
    bold_masks: [OnceLock<Vec<Mask>>; 8],
    /// The art of the card tiles (glyphs from `CARD_BASE` up), when the pack has it.
    cards: Option<Arc<CardArt>>,
}

impl std::fmt::Debug for Font {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Font").field("glyphs", &self.shapes.len()).finish_non_exhaustive()
    }
}

/// Why the font could not be made.
#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("the cartridge image is too short to hold the UI font")]
    ShortRom,
    #[error("the font file cannot be read")]
    Face,
    #[error(transparent)]
    Cards(#[from] crate::cards::CardsError),
}

/// The eight words of a 2-bit tile (16 bytes in the SNES layout: a row's plane 0 then its plane 1,
/// a word a row, the low byte the first bitplane), as the tile sits in VRAM.
fn words_of(tile: &[u8]) -> [u16; 8] {
    let mut words = [0_u16; 8];
    for (row, word) in words.iter_mut().enumerate() {
        *word = u16::from(tile[2 * row]) | u16::from(tile[2 * row + 1]) << 8;
    }
    words
}

/// The eight words of tile `code` of the UI set in the cartridge image, as the tile sits in VRAM.
pub(crate) fn tile_words(rom: &[u8], code: u16) -> Option<[u16; 8]> {
    let at = FONT_OFFSET + usize::from(code) * 16;
    rom.get(at..at + 16).map(words_of)
}

/// The eight words of tile `code` of a decoded tile set (the title set of [`title_tiles`]).
pub(crate) fn words_in(set: &[u8], code: u16) -> Option<[u16; 8]> {
    let at = usize::from(code) * 16;
    set.get(at..at + 16).map(words_of)
}

/// The words of a tile with colour 1 clear: where a row's plane 0 is set and its plane 1 is not, the
/// pixel reads colour 0. That is the form the game's VRAM holds the title set's white letters in.
pub(crate) fn clear_colour_one(words: [u16; 8]) -> [u16; 8] {
    words.map(|word| {
        let (plane0, plane1) = (word & 0xFF, word >> 8);
        (plane0 & plane1) | plane1 << 8
    })
}

/// The tiles of the title set: the stream at `00:DF39` decoded from the ROM, in the SNES layout
/// (`None` when the image has no such stream).
pub(crate) fn title_tiles(rom: &[u8]) -> Option<Vec<u8>> {
    let offset = RomView::new(rom).ok()?.offset(TITLE_STREAM)?;
    let mut stream = graphics::lz_decode(rom, offset)?;
    graphics::regroup(&mut stream.data);
    (stream.data.len() >= TITLE_TILES * 16).then_some(stream.data)
}

impl Font {
    /// The game's UI font of the cartridge image `rom`, drawn from the vendored Share Tech Mono.
    ///
    /// # Errors
    ///
    /// The image is too short to hold the font tiles.
    pub fn standard(rom: &[u8]) -> Result<Self, FontError> {
        Self::new(rom, SHARE_TECH_MONO)
    }

    /// The game's text of the cartridge image `rom` (the UI set, and the title set when the image
    /// has its stream) drawn from the TrueType font `face`.
    ///
    /// # Errors
    ///
    /// The image is too short, or `face` is not a font.
    pub fn new(rom: &[u8], face: &'static [u8]) -> Result<Self, FontError> {
        let data =
            rom.get(FONT_OFFSET..FONT_OFFSET + FONT_TILES * 16).ok_or(FontError::ShortRom)?;
        let face = FontRef::try_from_slice(face).map_err(|_| FontError::Face)?;
        let mut font = Self {
            face,
            tiles: FxHashMap::default(),
            pairs: FxHashMap::default(),
            card_tiles: FxHashMap::default(),
            shapes: vec![(' ', 1)],
            masks: std::array::from_fn(|_| OnceLock::new()),
            bold_masks: std::array::from_fn(|_| OnceLock::new()),
            cards: None,
        };
        for &(code, character) in TILE_CHARS {
            let glyph = font.shape(character, 1);
            let words = words_of(&data[usize::from(code) * 16..][..16]);
            font.tiles.insert(words, Hit { glyph, half: 0, ink: 3, clear: false, bold: false });
        }
        // The title set comes after the UI set: a tile whose contents are a UI glyph's is that glyph
        if let Some(title) = title_tiles(rom) {
            for &(code, character) in TITLE_CHARS {
                let glyph = font.shape(character, 1);
                // The white letters come in two forms in VRAM: as the stream holds them (colour 1
                // drawn), and with colour 1 clear (the intro's)
                let words = words_of(&title[usize::from(code) * 16..][..16]);
                let clear_words = clear_colour_one(words);
                // A mark with no colour 1 in it (a comma, a dot) is the same in both forms: its
                // transparent pixels are colour 0, the picture beside it
                let as_held =
                    Hit { glyph, half: 0, ink: 3, clear: clear_words == words, bold: true };
                let clear = Hit { glyph, half: 0, ink: 3, clear: true, bold: true };
                font.tiles.entry(words).or_insert(as_held);
                font.tiles.entry(clear_words).or_insert(clear);
            }
            for &(top, character) in TITLE_TALL {
                let glyph = font.shape(character, 2);
                let upper = words_of(&title[usize::from(top) * 16..][..16]);
                let lower = usize::from(top) + usize::from(UNDER);
                let lower = words_of(&title[lower * 16..][..16]);
                font.pairs.entry((upper, lower)).or_insert(glyph);
            }
        }
        Ok(font)
    }

    /// The character a glyph index of a [`TextCell`] draws (' ' for a blank cell); `None` for a card's tile or an
    /// index the font never gave.
    #[must_use]
    pub fn character(&self, glyph: u8) -> Option<char> {
        self.shapes.get(usize::from(glyph)).map(|&(character, _)| character)
    }

    /// The index of the glyph of `character` at `tall` tiles high, made when it is new.
    fn shape(&mut self, character: char, tall: u8) -> u8 {
        if let Some(at) = self.shapes.iter().position(|&shape| shape == (character, tall)) {
            return at as u8;
        }
        if self.shapes.len() >= usize::from(CARD_BASE) {
            return BLANK;
        }
        self.shapes.push((character, tall));
        (self.shapes.len() - 1) as u8
    }

    /// Draws the card tiles of `cards` too (see [`crate::cards`]): the tiles are found by their content
    /// in the cartridge image `rom`.
    ///
    /// # Errors
    ///
    /// The image is too short to hold the UI tile set.
    pub fn with_cards(mut self, rom: &[u8], cards: Arc<CardArt>) -> Result<Self, FontError> {
        crate::cards::register(rom, &cards, &mut self.card_tiles)?;
        self.cards = Some(cards);
        Ok(self)
    }

    /// The UI font of `rom` with the card art of the pack in `pack` when it has any.
    ///
    /// # Errors
    ///
    /// The image is too short, or the pack's card art cannot be read.
    pub fn standard_with_pack(
        rom: &[u8],
        pack: Option<&std::path::Path>,
    ) -> Result<Self, FontError> {
        let font = Self::standard(rom)?;
        let Some(dir) = pack else { return Ok(font) };
        match CardArt::load(dir) {
            Ok(Some(cards)) => font.with_cards(rom, Arc::new(cards)),
            Ok(None) => Ok(font),
            // An unreadable card art costs the cards their art, never the text its font
            Err(error) => {
                log::warn!("the HD card art is not used: {error}");
                Ok(font)
            }
        }
    }

    /// What the tile at VRAM word `address` shows, when it is a glyph tile of a text set (or a card).
    pub(crate) fn glyph_at(&self, vram: &[u16], address: u16) -> Option<Hit> {
        let words: &[u16] = vram.get(usize::from(address)..usize::from(address) + 8)?;
        let key: [u16; 8] = words.try_into().ok()?;
        if let Some(hit) = self.tiles.get(&key) {
            return Some(*hit);
        }
        if let Some(hit) = self.bold_half(vram, address, key) {
            return Some(hit);
        }
        self.card_tiles.get(&key).map(|&glyph| Hit {
            glyph,
            half: 0,
            ink: 3,
            clear: false,
            bold: false,
        })
    }

    /// The half of a bold letter that the tile at VRAM word `address` (of contents `key`) is: the
    /// letter whose two halves are this tile and the tile 16 characters under it, or, when this is
    /// the lower half, the tile 16 characters above.
    fn bold_half(&self, vram: &[u16], address: u16, key: [u16; 8]) -> Option<Hit> {
        // 16 characters of 8 words: the tile under a top half is this far on in VRAM
        const UNDER: usize = 16 * 8;
        let at = usize::from(address);
        let word_tile = |start: usize| -> Option<[u16; 8]> {
            let words = vram.get(start..start + 8)?;
            words.try_into().ok()
        };
        let upper = word_tile(at + UNDER).and_then(|lower| self.pairs.get(&(key, lower)).copied());
        if let Some(glyph) = upper {
            return Some(Hit { glyph, half: 0, ink: 2, clear: false, bold: true });
        }
        let lower = at
            .checked_sub(UNDER)
            .and_then(word_tile)
            .and_then(|upper| self.pairs.get(&(upper, key)).copied());
        lower.map(|glyph| Hit { glyph, half: 1, ink: 3, clear: false, bold: true })
    }

    /// The card art, when the font has it.
    pub(crate) fn cards(&self) -> Option<&CardArt> {
        self.cards.as_deref()
    }

    /// The alpha mask of the character `ch` at `scale` (see [`Font::mask`]), when the font has it.
    pub(crate) fn char_mask(&self, scale: usize, ch: char) -> Option<&[u8]> {
        let glyph = self.shapes.iter().position(|&shape| shape == (ch, 1))?;
        Some(&self.mask(scale, glyph as u8, false).alpha)
    }

    /// The mask of `glyph` at `scale` (2 to 8): `(8 + 2 * MARGIN) * scale` pixels wide and, for a
    /// bold letter, `(16 + 2 * MARGIN) * scale` high, the cell at `MARGIN * scale` from the top left.
    /// A bold mask is the same glyph with its stroke widened by half an original pixel across.
    fn mask(&self, scale: usize, glyph: u8, bold: bool) -> &Mask {
        let cache = if bold { &self.bold_masks } else { &self.masks };
        let masks = cache[scale - 1].get_or_init(|| {
            self.shapes
                .iter()
                .map(|&(character, tall)| self.rasterise(scale, character, tall, bold))
                .collect()
        });
        &masks[usize::from(glyph)]
    }

    /// One glyph at `scale`: the outline of a character `tall` tiles high rasterised at about 16
    /// samples an original pixel, thickened by one sample (or by half an original pixel across, for
    /// a bold one), and averaged down.
    fn rasterise(&self, scale: usize, character: char, tall: u8, bold: bool) -> Mask {
        let (cell_w, cell_h) = (8, 8 * usize::from(tall));
        let (w, h) = ((cell_w + 2 * MARGIN) * scale, (cell_h + 2 * MARGIN) * scale);
        if character == ' ' {
            return Mask { w, h, alpha: vec![0; w * h], ink: (0.0, 0.0, 0.0, 0.0) };
        }
        let per_pixel = SAMPLES.div_ceil(scale);
        let samples = scale * per_pixel;
        let (canvas_w, canvas_h) = (w * per_pixel, h * per_pixel);
        let mut canvas = vec![0_f32; canvas_w * canvas_h];
        // `PxScale` is the font's height (ascent to descent) in pixels, not its em
        let Some(units) = self.face.units_per_em() else {
            return Mask { w, h, alpha: vec![0; w * h], ink: (0.0, 0.0, 0.0, 0.0) };
        };
        let (em, stretch, baseline) = if tall == 1 {
            (EM, STRETCH, BASELINE)
        } else {
            (TALL_EM, TALL_STRETCH, TALL_BASELINE)
        };
        let em = em * samples as f32;
        let height = em * self.face.height_unscaled() / units;
        let px = PxScale { x: height * stretch, y: height };
        let glyph = self.face.glyph_id(character).with_scale_and_position(
            px,
            point(
                (MARGIN as f32 + ORIGIN_X) * samples as f32,
                (MARGIN as f32 + baseline) * samples as f32,
            ),
        );
        if let Some(outlined) = self.face.outline_glyph(glyph) {
            let bounds = outlined.px_bounds();
            outlined.draw(|x, y, coverage| {
                let (x, y) = (x as i32 + bounds.min.x as i32, y as i32 + bounds.min.y as i32);
                if (0..canvas_w as i32).contains(&x) && (0..canvas_h as i32).contains(&y) {
                    canvas[y as usize * canvas_w + x as usize] = coverage;
                }
            });
        }
        let (reach_x, reach_y) = if bold { (samples / 2, 0) } else { (1, 1) };
        let thick = dilate(&canvas, canvas_w, canvas_h, reach_x, reach_y);
        let mut alpha = vec![0_u8; w * h];
        let area = (per_pixel * per_pixel) as f32;
        for y in 0..h {
            for x in 0..w {
                let mut sum = 0.0;
                for sy in 0..per_pixel {
                    for sx in 0..per_pixel {
                        sum += thick[(y * per_pixel + sy) * canvas_w + x * per_pixel + sx];
                    }
                }
                alpha[y * w + x] = (sum / area * 255.0 + 0.5).min(255.0) as u8;
            }
        }
        let ink = ink_box(&alpha, w, h, scale);
        Mask { w, h, alpha, ink }
    }

    /// Draws the text `cells` of a frame into its picture: `rgba` is `width` pixels of `scale`
    /// times the 1x frame `base` (whose rows are `base_width` pixels). `beneath` holds, per BG
    /// layer, the frame drawn with only that layer off (the size of `base`), and a clear glyph's
    /// pixels take the picture of its own layer.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_with(
        &self,
        rgba: &mut [u8],
        width: usize,
        scale: usize,
        cells: &[TextCell],
        base: &[Color],
        base_width: usize,
        beneath: [Option<&[Color]>; 4],
    ) {
        if !(2..=crate::MAX_SCALE as usize).contains(&scale) {
            return;
        }
        let height = rgba.len() / 4 / width;
        let base_height = base.len() / base_width;
        // The pixels the cells show take their background colour (a card tile keeps what the look
        // drew: its art covers it); a clear glyph's take the picture beside them
        for cell in cells.iter().filter(|cell| cell.glyph < CARD_BASE) {
            for bit in 0..64_usize {
                if cell.mask >> bit & 1 == 0 {
                    continue;
                }
                let (column, line) = (cell.x + (bit % 8) as i32, cell.y + (bit / 8) as i32);
                let colour = shown_background(cell, column, line, base, base_width, beneath);
                let x0 = column * scale as i32;
                let y0 = line * scale as i32;
                for y in y0.max(0)..(y0 + scale as i32).min(height as i32) {
                    for x in x0.max(0)..(x0 + scale as i32).min(width as i32) {
                        let at = (y as usize * width + x as usize) * 4;
                        rgba[at..at + 3].copy_from_slice(&colour);
                    }
                }
            }
        }
        // The art of the tiles over what they stand on
        if let Some(cards) = &self.cards {
            for cell in cells.iter().filter(|cell| cell.glyph >= CARD_BASE) {
                cards.draw(rgba, width, scale, cell);
            }
        }
        // The glyphs over them: inside the cell where its pixels show, outside it where the 1x
        // frame is the cell's background (a descender over the blank line below); a clear glyph
        // stays inside its cell
        for cell in cells.iter().filter(|cell| cell.glyph != BLANK && cell.glyph < CARD_BASE) {
            let mask = self.mask(scale, cell.glyph, cell.bold);
            let tall = i32::from(self.shapes[usize::from(cell.glyph)].1);
            // A bold letter's lower tile starts 8 pixels under its top tile
            let top = cell.y - 8 * i32::from(cell.half);
            let (mut left, mut first) =
                ((cell.x - MARGIN as i32) * scale as i32, (top - MARGIN as i32) * scale as i32);
            // A title mark sits where the original's does: its centre across and its bottom are
            // those of the original's ink
            if cell.clear
                && matches!(self.shapes[usize::from(cell.glyph)].0, ',' | '.' | '\'' | '-')
            {
                let (dx, dy) = mark_shift(cell.mask, mask.ink);
                left += (dx * scale as f32).round() as i32;
                first += (dy * scale as f32).round() as i32;
            }
            for my in 0..mask.h {
                let y = first + my as i32;
                if !(0..height as i32).contains(&y) {
                    continue;
                }
                for mx in 0..mask.w {
                    let alpha = u32::from(mask.alpha[my * mask.w + mx]);
                    let x = left + mx as i32;
                    if alpha == 0 || !(0..width as i32).contains(&x) {
                        continue;
                    }
                    let (px, py) = (x / scale as i32, y / scale as i32);
                    let (column, row) = (px - cell.x, py - cell.y);
                    let inside = (0..8).contains(&column) && (0..8).contains(&row);
                    let other_half = !inside
                        && (top..top + 8 * tall).contains(&py)
                        && (cell.x..cell.x + 8).contains(&px);
                    let allowed = if inside {
                        // A title glyph is drawn over its whole cell: where the original was
                        // transparent, the picture shows through the glyph's ink only
                        cell.bold || cell.mask >> (row * 8 + column) & 1 != 0
                    } else if other_half {
                        // The other tile of a bold letter draws its own half
                        false
                    } else if cell.bold {
                        // A title glyph stays in its cell: beside it, a neighbour's glyph would
                        // be drawn over
                        false
                    } else {
                        picture_at(base, base_width, base_height, px, py)
                            .is_some_and(|rgb| rgb == cell.bg)
                    };
                    if !allowed {
                        continue;
                    }
                    let at = (y as usize * width + x as usize) * 4;
                    for channel in 0..3 {
                        let under = u32::from(rgba[at + channel]);
                        let ink = u32::from(cell.fg[channel]);
                        rgba[at + channel] =
                            ((under * (255 - alpha) + ink * alpha + 127) / 255) as u8;
                    }
                }
            }
        }
        crate::tooltip::draw(self, rgba, width, scale, cells);
    }
}

/// The colour a pixel of a text cell shows once its letter is taken away: the picture beside or
/// beneath it for a clear glyph (its own layer's, else the nearest pixel that is not the ink), the
/// cell's background colour otherwise. `base` is the 1x frame with the letters in it.
fn shown_background(
    cell: &TextCell,
    column: i32,
    line: i32,
    base: &[Color],
    base_width: usize,
    beneath: [Option<&[Color]>; 4],
) -> [u8; 3] {
    if !cell.clear {
        return cell.bg;
    }
    let base_height = base.len() / base_width;
    beneath[usize::from(cell.layer & 3)]
        .and_then(|beneath| picture_at(beneath, base_width, base_height, column, line))
        .filter(|&rgb| !is_ink(rgb, cell.fg))
        .or_else(|| under(base, base_width, base_height, column, line, cell.fg))
        .unwrap_or(cell.bg)
}

/// The 1x frame `base` with the pixels the text cells show (their letters) painted with the
/// background they stand on (see [`shown_background`]): the frame the filter that enlarges the
/// picture should see, since a pixel beside a letter takes sub-pixels of the letter's colour from
/// it, and those would be left as specks around the redrawn glyph.
pub(crate) fn without_letters(
    base: &[Color],
    base_width: usize,
    cells: &[TextCell],
    beneath: [Option<&[Color]>; 4],
) -> Vec<Color> {
    let mut clean = base.to_vec();
    let base_height = base.len() / base_width;
    for cell in cells.iter().filter(|cell| cell.glyph < CARD_BASE) {
        for bit in 0..64_usize {
            if cell.mask >> bit & 1 == 0 {
                continue;
            }
            let (column, line) = (cell.x + (bit % 8) as i32, cell.y + (bit / 8) as i32);
            if picture_at(base, base_width, base_height, column, line).is_none() {
                continue;
            }
            let [r, g, b] = shown_background(cell, column, line, base, base_width, beneath);
            let at = line as usize * base_width + column as usize;
            clean[at] = Color { r, g, b, a: clean[at].a };
        }
    }
    clean
}

/// How far a picture colour may be from a glyph's ink and still be the ink (the colour math moves
/// it a little from line to line).
const INK_REACH: u8 = 24;

/// The colour of the 1x picture `base` at (`x`, `y`), when it is in the frame.
fn picture_at(base: &[Color], width: usize, height: usize, x: i32, y: i32) -> Option<[u8; 3]> {
    if !(0..width as i32).contains(&x) || !(0..height as i32).contains(&y) {
        return None;
    }
    let colour = base[y as usize * width + x as usize];
    Some([colour.r, colour.g, colour.b])
}

/// Whether a picture colour is a glyph's ink (`ink`, within [`INK_REACH`] on every channel).
fn is_ink(rgb: [u8; 3], ink: [u8; 3]) -> bool {
    rgb.iter().zip(ink).all(|(&c, i)| c.abs_diff(i) <= INK_REACH)
}

/// The colour the picture shows beside the clear glyph's pixel at (`x`, `y`): the nearest pixel that
/// is not the glyph's ink, looking up and down first (the backgrounds of the title are vertical bands,
/// so the colour above or below is the band's own), then left and right, or `None`.
fn under(
    base: &[Color],
    width: usize,
    height: usize,
    x: i32,
    y: i32,
    ink: [u8; 3],
) -> Option<[u8; 3]> {
    let beside =
        |x: i32, y: i32| picture_at(base, width, height, x, y).filter(|&rgb| !is_ink(rgb, ink));
    (1..=16)
        .find_map(|d| beside(x, y - d))
        .or_else(|| (1..=16).find_map(|d| beside(x, y + d)))
        .or_else(|| (1..=8).find_map(|d| beside(x - d, y)))
        .or_else(|| (1..=8).find_map(|d| beside(x + d, y)))
}

/// The frame logs of the picture under the title's clear glyphs, one per BG layer that holds one
/// (`None` for the others): the frame with only that layer switched off, so the layers beneath it
/// show through its transparent pixels. Switching off every text layer at once would also drop the
/// picture of the other layers, which the original shows where the glyph is clear.
pub(crate) fn beneath_logs(log: &FrameLog, cells: &[TextCell]) -> [Option<FrameLog>; 4] {
    let mut layers = [false; 4];
    for cell in cells.iter().filter(|cell| cell.clear) {
        layers[usize::from(cell.layer & 3)] = true;
    }
    std::array::from_fn(|bg| {
        layers[bg].then(|| {
            let mut beneath = log.clone();
            for entry in &mut beneath.entries {
                entry.regs.main_bg[bg] = false;
            }
            beneath
        })
    })
}

/// The box of the ink of a mask (its pixels of at least a half), in original pixels from the cell's
/// top left (x0, y0, x1, y1); all zero when there is no ink.
fn ink_box(alpha: &[u8], w: usize, h: usize, scale: usize) -> (f32, f32, f32, f32) {
    let s = scale as f32;
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for y in 0..h {
        for x in 0..w {
            if alpha[y * w + x] >= 128 {
                let (fx, fy) = (x as f32 / s - MARGIN as f32, y as f32 / s - MARGIN as f32);
                x0 = x0.min(fx);
                y0 = y0.min(fy);
                x1 = x1.max(fx + 1.0 / s);
                y1 = y1.max(fy + 1.0 / s);
            }
        }
    }
    if x1 < x0 { (0.0, 0.0, 0.0, 0.0) } else { (x0, y0, x1, y1) }
}

/// How far a title mark's ink must move (in original pixels) so that its centre across and its bottom
/// are those of the original's ink: the shown pixels `shown` of its cell, and the mark's own ink box
/// `ink`.
fn mark_shift(shown: u64, ink: (f32, f32, f32, f32)) -> (f32, f32) {
    let (mut x0, mut y0, mut x1, mut y1) = (8_i32, 8_i32, 0_i32, 0_i32);
    for bit in 0..64 {
        if shown >> bit & 1 != 0 {
            let (column, row) = (bit % 8, bit / 8);
            x0 = x0.min(column);
            y0 = y0.min(row);
            x1 = x1.max(column + 1);
            y1 = y1.max(row + 1);
        }
    }
    if x1 == 0 {
        return (0.0, 0.0);
    }
    let original_centre = (x0 + x1) as f32 / 2.0;
    let glyph_centre = f32::midpoint(ink.0, ink.2);
    (original_centre - glyph_centre, y1 as f32 - ink.3)
}

/// The largest sample within `reach_x` samples across and `reach_y` down (the stroke of a glyph's
/// outline: with a reach of one each way, the largest of a sample and its eight neighbours).
fn dilate(canvas: &[f32], width: usize, height: usize, reach_x: usize, reach_y: usize) -> Vec<f32> {
    let mut across = vec![0_f32; width * height];
    for y in 0..height {
        for x in 0..width {
            let (from, to) = (x.saturating_sub(reach_x), (x + reach_x).min(width - 1));
            across[y * width + x] =
                canvas[y * width + from..=y * width + to].iter().copied().fold(0.0, f32::max);
        }
    }
    let mut thick = vec![0_f32; width * height];
    for y in 0..height {
        let (from, to) = (y.saturating_sub(reach_y), (y + reach_y).min(height - 1));
        for x in 0..width {
            thick[y * width + x] =
                (from..=to).map(|row| across[row * width + x]).fold(0.0, f32::max);
        }
    }
    thick
}

/// What a glyph tile shows (see [`Font::glyph_at`]): the glyph (a shape of the text, or a card tile
/// from `CARD_BASE`), which tile of a bold letter it is, the colour its ink takes, and whether colour
/// 1 is clear in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Hit {
    pub glyph: u8,
    pub half: u8,
    pub ink: u8,
    pub clear: bool,
    pub bold: bool,
}

/// A cell of the game's text in a frame: where it is, its glyph, the colours it shows in, and which
/// of its 64 pixels (bit `row * 8 + column`) show it. A card tile (glyph `CARD_BASE` and up) also
/// has the colour 2 it shows in, and the white of its colour 1 when that is white.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct TextCell {
    /// The cell's top left corner in the 1x frame.
    pub x: i32,
    pub y: i32,
    pub glyph: u8,
    /// Which tile of a bold letter the cell is: 0 the top one, 1 the one under it.
    pub half: u8,
    /// Whether colour 1 is clear (or absent) in the tile: the picture beside the glyph shows where it
    /// would be.
    pub clear: bool,
    /// Whether the glyph is a title set glyph: drawn with a wider stroke, and over its whole cell
    /// (where the original was transparent, the picture shows through except where the ink is).
    pub bold: bool,
    /// The BG layer (0 to 3) the cell's pixels show from.
    pub layer: u8,
    pub fg: [u8; 3],
    pub bg: [u8; 3],
    pub mid: [u8; 3],
    pub white: Option<[u8; 3]>,
    pub mask: u64,
}

/// The text cells of a frame, noted while it is drawn.
#[derive(Default)]
pub(crate) struct TextCapture {
    cells: Vec<TextCell>,
    index: FxHashMap<(i32, i32, u8), usize>,
    /// The last character looked up in this line's entry, and what it shows.
    last: Option<(u16, Option<Hit>)>,
}

impl TextCapture {
    pub(crate) fn clear(&mut self) {
        self.cells.clear();
        self.index.clear();
        self.last = None;
    }

    pub(crate) fn cells(&self) -> &[TextCell] {
        &self.cells
    }

    /// The VRAM may differ from the entry before: look the characters up again.
    pub(crate) fn new_entry(&mut self) {
        self.last = None;
    }

    /// Notes the pixel `x` of row `row` (both 0-based) of the frame, that the renderer has
    /// resolved as `rendered`, when it is a pixel of a glyph tile of a 2-bit BG.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn note(
        &mut self,
        font: &Font,
        row: usize,
        x: i32,
        rendered: &Rendered,
        plan: impl FnOnce() -> Plan,
        regs: &LineRegs,
        cgram: &[u16; 256],
        vram: &[u16],
    ) {
        let bg = match rendered.layer {
            Layer::Bg1 => 0,
            Layer::Bg2 => 1,
            Layer::Bg3 => 2,
            Layer::Bg4 => 3,
            _ => return,
        };
        // 2 bits a pixel, not flipped, and not a Mode 7 plane
        if regs.bg_mode == 7
            || crate::bg::bg_bitplanes(regs.bg_mode, bg) != 2
            || rendered.pos & 0xC0 != 0
        {
            return;
        }
        let hit = match self.last {
            Some((address, hit)) if address == rendered.char => hit,
            _ => {
                let hit = font.glyph_at(vram, rendered.char);
                self.last = Some((rendered.char, hit));
                hit
            }
        };
        let Some(hit) = hit else { return };
        let (column, line) = (i32::from(rendered.pos & 7), i32::from(rendered.pos >> 3 & 7));
        let key = (x - column, row as i32 - line, hit.glyph);
        let at = match self.index.get(&key) {
            Some(&at) => at,
            None => {
                let plan = plan();
                let native = |color: u8| {
                    let pixel = Pixel { palette: rendered.palette, color, ..Pixel::TRANSPARENT };
                    resolve_color(cgram, rendered.layer, regs.bg_mode, false, pixel)
                };
                let shown = |color: u8| {
                    let shown = convert(plan.apply(native(color)), regs.brightness);
                    [shown.r, shown.g, shown.b]
                };
                let card = hit.glyph >= CARD_BASE;
                self.cells.push(TextCell {
                    x: key.0,
                    y: key.1,
                    glyph: hit.glyph,
                    half: hit.half,
                    clear: hit.clear,
                    bold: hit.bold,
                    layer: bg as u8,
                    fg: shown(hit.ink),
                    bg: shown(1),
                    mid: if card { shown(2) } else { [0; 3] },
                    white: (card && native(1) & 0x7FFF == 0x7FFF).then(|| shown(1)),
                    mask: 0,
                });
                self.index.insert(key, self.cells.len() - 1);
                self.cells.len() - 1
            }
        };
        self.cells[at].mask |= 1 << (line * 8 + column);
    }

    /// Takes in the cells another part of the frame noted (its pixels join the cells in both).
    pub(crate) fn merge(&mut self, other: &Self) {
        for cell in &other.cells {
            let key = (cell.x, cell.y, cell.glyph);
            match self.index.get(&key) {
                Some(&at) => self.cells[at].mask |= cell.mask,
                None => {
                    self.cells.push(*cell);
                    self.index.insert(key, self.cells.len() - 1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A cartridge image whose font has `H` (tile `$8A`) as a vertical bar of colour 3 and the
    /// space (tile `$01`) as colour 1 everywhere; the other tiles are empty.
    fn rom() -> Vec<u8> {
        let mut rom = vec![0_u8; FONT_OFFSET + FONT_TILES * 16];
        let at = |code: usize| FONT_OFFSET + code * 16;
        // Every other tile is its own: a tile of one word with its code in it
        for code in 0..FONT_TILES {
            rom[at(code) + 15] = (code % 250) as u8 + 1;
        }
        for row in 0..8 {
            rom[at(0x8A) + 2 * row] = 0b0001_1000;
            rom[at(0x8A) + 2 * row + 1] = 0b0001_1000;
            rom[at(0x01) + 2 * row] = 0xFF;
        }
        rom[at(0x01) + 15] = 0;
        rom
    }

    fn font() -> Font {
        Font::standard(&rom()).unwrap()
    }

    /// The shape of glyph `glyph` (its character and tiles high).
    fn shape(font: &Font, glyph: u8) -> Shape {
        font.shapes[usize::from(glyph)]
    }

    /// A 1 MiB cartridge image: the UI tiles of [`rom`] and the title stream at `00:DF39`, a stream
    /// of literals holding the tiles `tiles` (index and words; the other tiles are empty).
    fn title_rom(tiles: &[(usize, [u16; 8])]) -> Vec<u8> {
        let mut rom = rom();
        rom.resize(0x10_0000, 0);
        let mut decoded = vec![0_u8; TITLE_TILES * 16];
        for &(index, words) in tiles {
            for (row, word) in words.iter().enumerate() {
                // A tile's plane 0 rows, then its plane 1 rows: the layout `regroup` undoes
                decoded[index * 16 + row] = *word as u8;
                decoded[index * 16 + 8 + row] = (*word >> 8) as u8;
            }
        }
        let at = 0x5F39;
        rom[at..at + 2].copy_from_slice(&(decoded.len() as u16).to_le_bytes());
        let mut pos = at + 2;
        for group in decoded.chunks(8) {
            rom[pos] = 0xFF;
            pos += 1;
            rom[pos..pos + group.len()].copy_from_slice(group);
            pos += group.len();
        }
        rom
    }

    #[test]
    fn a_short_image_has_no_font_and_the_font_file_is_a_font() {
        assert!(matches!(Font::standard(&[0; 100]), Err(FontError::ShortRom)));
        assert!(matches!(Font::new(&rom(), b"not a font"), Err(FontError::Face)));
    }

    #[test]
    fn a_tile_is_a_glyph_when_its_words_are_those_of_a_font_tile() {
        let font = font();
        let mut vram = vec![0_u16; 64];
        for row in 0..8 {
            vram[16 + row] = 0x1818;
            vram[32 + row] = 0xFF;
        }
        let h = font.glyph_at(&vram, 16).expect("H").glyph;
        assert_eq!(shape(&font, h), ('H', 1));
        assert_eq!(
            font.glyph_at(&vram, 32).map(|hit| hit.glyph),
            Some(BLANK),
            "the all-colour-1 tile is the space"
        );
        assert_eq!(font.glyph_at(&vram, 0), None, "an empty tile is not a glyph");
        vram[19] = 0x1819;
        assert_eq!(font.glyph_at(&vram, 16), None, "one word different");
        assert_eq!(font.glyph_at(&vram, 60000), None, "outside VRAM");
    }

    #[test]
    fn the_glyph_masks_are_antialiased_ink_inside_the_cell() {
        let font = font();
        let h = font.shapes.iter().position(|&s| s == ('H', 1)).unwrap() as u8;
        for scale in 2..=8_usize {
            let mask = font.mask(scale, h, false);
            let side = (8 + 2 * MARGIN) * scale;
            assert_eq!((mask.w, mask.h), (side, side));
            let alpha = &mask.alpha;
            let ink = alpha.iter().filter(|&&a| a > 200).count();
            let partial = alpha.iter().filter(|&&a| a > 0 && a < 255).count();
            assert!(ink > 8 * scale, "scale {scale}: the stems are solid");
            assert!(partial > 0, "scale {scale}: the edges are anti-aliased");
            // The capital H of the original is 7 pixels tall in an 8-pixel cell: its ink is
            // within the cell's rows and columns, give or take the stroke
            let rows: Vec<usize> =
                (0..side).filter(|&y| alpha[y * side..][..side].iter().any(|&a| a > 200)).collect();
            let (first, last) = (rows[0], *rows.last().unwrap());
            assert_eq!(
                first / scale,
                MARGIN,
                "scale {scale}: the cap line is the cell's top ({rows:?})"
            );
            assert_eq!(last / scale, MARGIN + 6, "scale {scale}: the baseline is below row 6");
        }
        assert!(
            font.mask(4, BLANK, false).alpha.iter().all(|&a| a == 0),
            "the space draws nothing"
        );
    }

    #[test]
    fn a_bold_letter_is_two_tiles_of_two_colours_and_its_mask_is_sixteen_pixels_tall() {
        let top = words_of(&[0x0F_u8; 16]);
        let font = Font::new(&title_rom(&[(0x90, top)]), SHARE_TECH_MONO).unwrap();
        let hit = font.glyph_at(&top_vram(top), 0x20).expect("the top tile of A");
        assert_eq!(shape(&font, hit.glyph), ('A', 2));
        assert_eq!((hit.half, hit.ink, hit.clear), (0, 2, false));
        let mask = font.mask(4, hit.glyph, false);
        assert_eq!((mask.w, mask.h), ((8 + 2 * MARGIN) * 4, (16 + 2 * MARGIN) * 4));
    }

    #[test]
    fn a_title_mark_without_colour_one_is_a_clear_glyph() {
        // A comma of colours 2 and 3 only: its other pixels are colour 0 in either form
        let comma = [0xFF0C_u16; 8];
        let font = Font::new(&title_rom(&[(0x8C, comma)]), SHARE_TECH_MONO).unwrap();
        let mut vram = vec![0_u16; 0x200];
        vram[0x20..0x28].copy_from_slice(&comma);
        let hit = font.glyph_at(&vram, 0x20).expect("the comma");
        assert_eq!(shape(&font, hit.glyph), (',', 1));
        assert!(hit.clear, "a mark with no colour 1 shows the picture beside it");
    }

    #[test]
    fn a_clear_cells_shown_pixel_takes_the_picture_beneath_it() {
        let font = font();
        let cell = TextCell {
            x: 1,
            y: 1,
            glyph: BLANK,
            clear: true,
            bold: true,
            fg: [255; 3],
            bg: [1, 2, 3],
            mask: 1,
            ..TextCell::default()
        };
        let base = vec![Color::rgb(9, 9, 9); 16 * 16];
        let beneath = vec![Color::rgb(40, 50, 60); 16 * 16];
        let mut rgba = vec![5_u8; 32 * 32 * 4];
        font.draw_with(
            &mut rgba,
            32,
            2,
            &[cell],
            &base,
            16,
            [Some(&beneath[..]), None, None, None],
        );
        // The shown pixel (1, 1) takes the picture beneath it, not the ink or the colour beside it
        assert_eq!(&rgba[(2 * 32 + 2) * 4..][..3], [40, 50, 60]);
    }

    #[test]
    fn the_picture_under_a_clear_glyph_switches_off_only_its_own_layer() {
        let log = text_log();
        let cell = |layer: u8, clear: bool| TextCell { clear, layer, ..TextCell::default() };
        let none = beneath_logs(&log, &[cell(0, false)]);
        assert!(none.iter().all(Option::is_none), "no clear glyph, no beneath");
        let logs = beneath_logs(&log, &[cell(0, true), cell(2, true)]);
        assert!(logs[1].is_none() && logs[3].is_none(), "only the layers with clear glyphs");
        for bg in [0_usize, 2] {
            let blank = logs[bg].as_ref().expect("a beneath for the layer");
            for (before, after) in log.entries.iter().zip(&blank.entries) {
                let mut expected = before.regs.main_bg;
                expected[bg] = false;
                assert_eq!(after.regs.main_bg, expected, "only BG{} is off", bg + 1);
            }
        }
    }

    /// VRAM holding the eight words `words` at word 0x20 (a tile's place; the tile 16 characters
    /// under it is at 0xA0, and is empty).
    fn top_vram(words: [u16; 8]) -> Vec<u16> {
        let mut vram = vec![0_u16; 0x200];
        vram[0x20..0x28].copy_from_slice(&words);
        vram
    }

    #[test]
    fn a_bold_letter_binds_by_its_pair_of_halves_where_a_lower_half_is_shared() {
        // E and F share their lower half (as O, S, C, J and U do); their upper halves differ
        let lower = [0x0F0F_u16; 8];
        let upper_e = [0x3030_u16; 8];
        let upper_f = [0x0C0C_u16; 8];
        let rom = title_rom(&[(0x94, upper_e), (0x95, upper_f), (0xA4, lower), (0xA5, lower)]);
        let font = Font::new(&rom, SHARE_TECH_MONO).unwrap();
        let mut vram = vec![0_u16; 0x800];
        for (tile, words) in [(0x94, upper_e), (0x95, upper_f), (0xA4, lower), (0xA5, lower)] {
            vram[tile * 8..tile * 8 + 8].copy_from_slice(&words);
        }
        let at = |tile: usize| (tile * 8) as u16;
        for (upper, lower_tile, letter) in [(0x94, 0xA4, 'E'), (0x95, 0xA5, 'F')] {
            let top = font.glyph_at(&vram, at(upper)).expect("the upper half");
            let bottom = font.glyph_at(&vram, at(lower_tile)).expect("the lower half");
            assert_eq!(shape(&font, top.glyph), (letter, 2));
            assert_eq!((top.half, top.ink, bottom.half, bottom.ink), (0, 2, 1, 3));
            assert_eq!(top.glyph, bottom.glyph, "the two halves are one letter");
        }
    }

    #[test]
    fn the_title_set_binds_its_white_letters_through_colour_one_clear() {
        // The white A: plane 0 set in the first four pixels of each row, plane 1 in the 1st and 3rd
        let white = [0xA0F0_u16; 8];
        let font = Font::new(&title_rom(&[(0xDA, white)]), SHARE_TECH_MONO).unwrap();
        // VRAM holds the clear form: the pixels that were colour 1 read colour 0
        let clear = clear_colour_one(white);
        assert_eq!(clear, [0xA0A0; 8], "colour 1 pixels become colour 0");
        let hit = font.glyph_at(&top_vram(clear), 0x20).expect("the white A");
        assert_eq!(shape(&font, hit.glyph), ('A', 1));
        assert_eq!((hit.half, hit.ink, hit.clear), (0, 3, true));
        // The credits hold the same tile as the stream has it, colour 1 drawn: both forms bind
        let held = font.glyph_at(&top_vram(white), 0x20).expect("the held form of the A");
        assert_eq!((held.glyph, held.clear), (hit.glyph, false));
    }

    #[test]
    fn a_title_tile_of_the_cards_art_is_bound_in_both_forms_of_the_title_set() {
        // A separator's tile: colours 2 and 3 with colour 1 in between
        let tile = [0x0F3C_u16; 8];
        let with_title = title_rom(&[(0xCB, tile)]);
        let cards = Arc::new(CardArt::flat_title(&[0xCB]));
        let font = Font::new(&with_title, SHARE_TECH_MONO)
            .unwrap()
            .with_cards(&with_title, cards)
            .unwrap();
        let held = font.glyph_at(&top_vram(tile), 0x20).expect("the tile as the stream has it");
        assert_eq!(held.glyph, CARD_BASE);
        let clear =
            font.glyph_at(&top_vram(clear_colour_one(tile)), 0x20).expect("with colour 1 clear");
        assert_eq!(clear.glyph, CARD_BASE);
        // It is a tile of art, not a letter, and the UI set's tile of the same number is another tile
        assert_eq!((held.bold, held.clear), (false, false));
        let other = [0x1234_u16; 8];
        assert!(font.glyph_at(&top_vram(other), 0x20).is_none());
        // An image with no title stream skips it
        let short = rom();
        let font = Font::new(&short, SHARE_TECH_MONO)
            .unwrap()
            .with_cards(&short, Arc::new(CardArt::flat_title(&[0xCB])))
            .unwrap();
        assert!(font.glyph_at(&top_vram(tile), 0x20).is_none());
    }

    #[test]
    fn the_title_set_is_only_bound_when_the_image_has_its_stream() {
        let rom = rom();
        assert!(title_tiles(&rom).is_none(), "a short image has no title stream");
        let font = Font::standard(&rom).unwrap();
        assert!(font.shapes.iter().all(|&(_, tall)| tall == 1), "no bold letters");
    }

    #[test]
    fn the_shapes_of_the_text_sets_fit_below_the_card_glyphs() {
        let mut chars: Vec<char> = TILE_CHARS.iter().map(|&(_, c)| c).collect();
        chars.extend(TITLE_CHARS.iter().map(|&(_, c)| c));
        chars.sort_unstable();
        chars.dedup();
        let mut bold: Vec<char> = TITLE_TALL.iter().map(|&(_, c)| c).collect();
        bold.sort_unstable();
        bold.dedup();
        assert!(chars.len() + bold.len() < usize::from(CARD_BASE));
    }

    #[test]
    fn a_clear_glyph_pixel_takes_the_picture_beside_it_unless_it_is_ink() {
        let (width, ink) = (16, [255, 255, 255]);
        let picture = [10, 20, 30];
        let mut base = vec![Color::rgb(picture[0], picture[1], picture[2]); width * 4];
        for x in [5, 6] {
            base[width + x] = Color::rgb(255, 255, 255);
        }
        assert_eq!(under(&base, width, 4, 5, 1, ink), Some(picture), "the left of the ink");
        assert_eq!(under(&base, width, 4, 7, 1, ink), Some(picture), "the ink is skipped");
        // A colour the colour math moved a little is still the ink
        base[width + 4] = Color::rgb(250, 252, 255);
        assert_eq!(under(&base, width, 4, 5, 1, ink), Some(picture), "near the ink is ink");
        // A row of ink takes the picture above it
        for x in 0..width {
            base[width + x] = Color::rgb(255, 255, 255);
        }
        assert_eq!(under(&base, width, 4, 7, 1, ink), Some(picture), "the column above");
        assert_eq!(under(&base, width, 4, 40, 1, ink), None, "outside the picture");
    }

    #[test]
    fn a_cell_fills_its_pixels_and_draws_its_glyph_over_them() {
        let font = font();
        let h = font.shapes.iter().position(|&s| s == ('H', 1)).unwrap() as u8;
        let scale = 4;
        let (width, height) = (16 * scale, 16 * scale);
        let mut rgba = vec![9_u8; width * height * 4];
        let base = vec![Color::rgb(9, 9, 9); 16 * 16];
        // The cell at (4, 4), with its top half hidden by something in front
        let cell = TextCell {
            x: 4,
            y: 4,
            glyph: h,
            fg: [200, 0, 0],
            bg: [10, 20, 30],
            mask: !0 << 32,
            ..TextCell::default()
        };
        font.draw_with(&mut rgba, width, scale, &[cell], &base, 16, [None; 4]);
        let px = |x: usize, y: usize| {
            [
                rgba[(y * width + x) * 4],
                rgba[(y * width + x) * 4 + 1],
                rgba[(y * width + x) * 4 + 2],
            ]
        };
        // A hidden pixel of the cell is untouched; a shown one is the background or the ink
        assert_eq!(px(4 * scale, 4 * scale), [9, 9, 9]);
        assert_eq!(px(4 * scale, 7 * scale + 1), [9, 9, 9]);
        assert_eq!(px(4 * scale, 8 * scale + 1), [10, 20, 30]);
        let ink = (4 * scale..12 * scale)
            .flat_map(|x| (8 * scale..12 * scale).map(move |y| (x, y)))
            .filter(|&(x, y)| px(x, y)[0] > 150)
            .count();
        assert!(ink > 0, "the stems are red");
        // Nothing outside the cell and its margin changed
        assert_eq!(px(0, 0), [9, 9, 9]);
        assert_eq!(px(15 * scale, 15 * scale), [9, 9, 9]);
    }

    /// A letter over the bands of a non-black background: the picture is made from the frame without
    /// the letter, so the enlarging filter leaves no sub-pixel of the original ink beside the glyph,
    /// and the glyph is blended against the band it stands on.
    #[test]
    fn a_glyph_over_a_coloured_background_leaves_nothing_outside_its_footprint() {
        use crate::enhanced::{Capture, HdOptions, picture};
        let font = font();
        let h = font.shapes.iter().position(|&s| s == ('H', 1)).unwrap() as u8;
        let (scale, side) = (4_usize, 24_usize);
        // Vertical bands of blue, one a pixel wide, as the intro's
        let band = |x: usize| Color::rgb(0, 0, 40 + 12 * (x % 5) as u8);
        let mut base: Vec<Color> = (0..side * side).map(|at| band(at % side)).collect();
        // The original's letter: the colour-3 ink of a cell at (8, 8), a stroke diagonal enough for
        // the filter to round its corners
        let mut mask = 0_u64;
        for bit in 0..64_usize {
            let (column, row) = (bit % 8, bit / 8);
            if column == 1 || column == 6 || (row == 3 && column > 1) || (row + column == 9) {
                mask |= 1 << bit;
                base[(8 + row) * side + 8 + column] = Color::rgb(255, 255, 255);
            }
        }
        let cell = TextCell {
            x: 8,
            y: 8,
            glyph: h,
            clear: true,
            bold: true,
            fg: [255, 255, 255],
            bg: [0, 0, 0],
            mask,
            ..TextCell::default()
        };
        let cells = [cell];
        let clean = without_letters(&base, side, &cells, [None; 4]);
        let frame = crate::Frame { width: side, height: side, pixels: clean, sources: Vec::new() };
        let options = HdOptions { scale: scale as u32, look: crate::enhanced::Look::Crisp };
        let mut hd = picture(&frame, &Capture::default(), None, options);
        font.draw_with(&mut hd.rgba, hd.width as usize, scale, &cells, &base, side, [None; 4]);
        // The glyph's footprint: where its mask has any alpha
        let alpha = &font.mask(scale, h, true).alpha;
        let mask_width = (8 + 2 * MARGIN) * scale;
        let origin = (8 - MARGIN) * scale;
        let in_footprint = |x: usize, y: usize| {
            let (mx, my) = (x as i64 - origin as i64, y as i64 - origin as i64);
            mx >= 0
                && my >= 0
                && (mx as usize) < mask_width
                && alpha.get(my as usize * mask_width + mx as usize).is_some_and(|&a| a > 0)
        };
        let mut ink = 0;
        for y in 0..side * scale {
            for x in 0..side * scale {
                let at = (y * hd.width as usize + x) * 4;
                let shown = [hd.rgba[at], hd.rgba[at + 1], hd.rgba[at + 2]];
                if in_footprint(x, y) {
                    ink += usize::from(shown[0] > 200);
                    continue;
                }
                let want = band(x / scale);
                assert_eq!(shown, [want.r, want.g, want.b], "a speck at ({x}, {y})");
            }
        }
        assert!(ink > 0, "the glyph is drawn");
    }

    #[test]
    fn scale_one_and_blank_cells_draw_nothing_of_glyphs() {
        let font = font();
        let mut rgba = vec![5_u8; 16 * 16 * 4];
        let base = vec![Color::rgb(5, 5, 5); 16 * 16];
        let cell = TextCell {
            x: 2,
            y: 2,
            glyph: BLANK,
            fg: [255; 3],
            bg: [1, 2, 3],
            mask: 1,
            ..TextCell::default()
        };
        font.draw_with(&mut rgba, 16, 1, &[cell], &base, 16, [None; 4]);
        assert!(rgba.iter().all(|&v| v == 5), "scale 1 leaves the picture");
        let mut rgba = vec![5_u8; 32 * 32 * 4];
        font.draw_with(&mut rgba, 32, 2, &[cell], &base, 16, [None; 4]);
        assert_eq!(&rgba[(2 * 2 * 32 + 2 * 2) * 4..][..3], [1, 2, 3]);
        assert_eq!(
            rgba.iter().filter(|&&v| v != 5).count(),
            4 * 3,
            "one pixel of 2 x 2, its colour"
        );
    }

    #[test]
    fn a_pixel_of_a_glyph_tile_makes_a_cell_with_the_colours_the_game_showed() {
        let font = font();
        let mut vram = vec![0_u16; 64];
        for row in 0..8 {
            vram[16 + row] = 0x1818;
        }
        let mut cgram = [0_u16; 256];
        // Mode 1, BG3, palette 1: colour 1 and colour 3 through the 2-bit palette rows
        cgram[(1 << 2) | 1] = 0x7FFF;
        cgram[(1 << 2) | 3] = 0x001F;
        let regs = LineRegs { brightness: 15, bg_mode: 1, ..LineRegs::default() };
        let rendered = |color: u8, column: u8, row: u8| Rendered {
            color: u16::from(color),
            palette: 1,
            layer: Layer::Bg3,
            char: 16,
            pos: crate::pos(column, row, false, false),
            oam: crate::NO_OBJECT,
            priority: 7,
        };
        let plain = || Plan { force_black: false, math: None };
        let mut capture = TextCapture::default();
        // Row 3 of the frame, x 20: the tile's column 3, row 2
        capture.note(&font, 3, 20, &rendered(3, 3, 2), plain, &regs, &cgram, &vram);
        capture.note(&font, 3, 21, &rendered(1, 4, 2), plain, &regs, &cgram, &vram);
        capture.note(&font, 4, 20, &rendered(3, 3, 3), plain, &regs, &cgram, &vram);
        assert_eq!(capture.cells().len(), 1);
        let cell = capture.cells()[0];
        assert_eq!((cell.x, cell.y), (17, 1));
        assert_eq!(cell.fg, [255, 0, 0]);
        assert_eq!(cell.bg, [255, 255, 255]);
        assert_eq!(cell.mask, 1 << (2 * 8 + 3) | 1 << (2 * 8 + 4) | 1 << (3 * 8 + 3));
        // A flipped tile, another layer, a 4-bit layer and a tile that is not a glyph are not noted
        let mut flipped = rendered(3, 3, 2);
        flipped.pos |= 0x40;
        let mut object = rendered(3, 3, 2);
        object.layer = Layer::Obj;
        let mut four_bits = rendered(3, 3, 2);
        four_bits.layer = Layer::Bg1;
        let mut other = rendered(3, 3, 2);
        other.char = 0;
        for pixel in [flipped, object, four_bits, other] {
            capture.note(&font, 9, 40, &pixel, plain, &regs, &cgram, &vram);
        }
        assert_eq!(capture.cells().len(), 1);
        // Two parts of one frame that saw a cell each join
        let mut more = TextCapture::default();
        more.note(&font, 3, 22, &rendered(3, 5, 2), plain, &regs, &cgram, &vram);
        capture.merge(&more);
        assert_eq!(capture.cells().len(), 1);
        assert_eq!(capture.cells()[0].mask & (1 << (2 * 8 + 5)), 1 << (2 * 8 + 5));
    }

    /// A frame log of 224 lines of Mode 1 with BG3 only: tile 1 of the 2-bit character base
    /// `$1000` (the font's `H` of [`rom`]) in palette 1 at the top left.
    fn text_log() -> ssd_oracle::FrameLog {
        use ssd_oracle::{FrameLog, LineEntry, SpriteLine};
        let regs = LineRegs {
            brightness: 15,
            bg_mode: 1,
            main_bg: [false, false, true, false],
            bg_map_base: [0, 0, 0x0400, 0],
            bg_char_base: [0, 0, 0x1000, 0],
            ..LineRegs::default()
        };
        let entries = (1..=224)
            .map(|scanline| LineEntry {
                scanline,
                from_pixel: 0,
                regs,
                sprites: SpriteLine::default(),
            })
            .collect();
        let mut vram = vec![0_u16; 32768];
        for row in 0..8 {
            vram[0x1000 + 8 + row] = 0x1818;
        }
        vram[0x0400] = 1 | (1 << 10);
        let mut cgram = vec![0_u16; 256];
        cgram[(1 << 2) | 3] = 0x001F;
        FrameLog {
            complete: true,
            height: 224,
            h_hi_res: false,
            v_hi_res: false,
            entries,
            cgram_start: cgram,
            cgram_writes: Vec::new(),
            vram,
            vram_writes: Vec::new(),
            oam_low: vec![0; 512],
            oam_high: vec![0; 32],
            oam_writes: Vec::new(),
        }
    }

    #[test]
    fn a_renderer_with_the_font_draws_text_from_scale_two_and_leaves_scale_one_alone() {
        use crate::{HdOptions, Look, Renderer};
        let log = text_log();
        let options = |scale| HdOptions { scale, look: Look::Crisp };
        let mut plain = Renderer::new();
        let mut with = Renderer::new();
        with.set_font(Some(std::sync::Arc::new(font())));
        // Scale 1 is the PPU's frame, font or not
        let one = with.render_hd(&log, options(1)).unwrap();
        assert_eq!(one, plain.render_hd(&log, options(1)).unwrap());
        // Scale 4: the same picture but for the cell's rows, where the glyph is drawn
        let (a, b) =
            (plain.render_hd(&log, options(4)).unwrap(), with.render_hd(&log, options(4)).unwrap());
        assert_ne!(a, b);
        let row_bytes = 256 * 4 * 4;
        // The first line shows row 1 of the tile; the cell is at rows -1 to 6 of the frame, so
        // everything from picture row 6 * 4 + 8 (the margin) down is the look's own
        assert_eq!(a.rgba[(7 * 4 + 8) * row_bytes..], b.rgba[(7 * 4 + 8) * row_bytes..]);
        // A frame the font is switched off for is the look's again
        with.set_font(None);
        assert_eq!(a, with.render_hd(&log, options(4)).unwrap());
    }
}
