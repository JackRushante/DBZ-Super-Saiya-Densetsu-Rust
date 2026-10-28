//! A card of the hand put together from the tile words the game wrote for it: the HD menus' Cards screen reads the
//! 4 x 6 tile words of each card from the game's tile map (the tile, its palette row and its flips) and asks for
//! the whole card, or one of its parts (a ball, the kanji square), drawn from the art of those tiles. Nothing here
//! knows which tile is which value or symbol: the game's own drawing already chose them.

use super::{
    BLACK, BODY_COLOURS, CardArt, PREVIEW_SCALE, PREVIEW_TILES, Preview, SYMBOL_COLOURS, TILE,
};

/// Tile words in the picture of a card: 4 columns and 6 rows, row by row.
pub const WORDS: usize = PREVIEW_TILES.0 * PREVIEW_TILES.1;
/// The palette row of the black-on-yellow kanji squares (colour 3, 2, 1): black ink, a pink ring, yellow paper.
const YELLOW_COLOURS: [[u8; 3]; 3] = [[0, 0, 0], [0xFF, 0x63, 0xBD], [0xFF, 0xFF, 0x63]];
/// The palette rows of the kanji squares: blue ink on cream with an orange ring, and the yellow one.
const SYMBOL_ROW: u16 = 0;
const YELLOW_ROW: u16 = 6;
/// Art pixels between the sphere of a ball and the edge of its 2 x 2 tiles.
const BALL_MARGIN: f32 = 4.0;

/// A part of the card, in tiles from its top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Part {
    pub column: usize,
    pub row: usize,
    pub columns: usize,
    pub rows: usize,
}

impl Part {
    /// The whole card.
    pub const WHOLE: Self = Self { column: 0, row: 0, columns: 4, rows: 6 };
    /// The ball in the top-left corner: the attack value.
    pub const ATTACK: Self = Self { column: 0, row: 0, columns: 2, rows: 2 };
    /// The square in the middle: the kanji.
    pub const SYMBOL: Self = Self { column: 1, row: 2, columns: 2, rows: 2 };
    /// The ball in the bottom-right corner: the defence value.
    pub const DEFENCE: Self = Self { column: 2, row: 4, columns: 2, rows: 2 };
}

impl CardArt {
    /// `part` of the card whose tile words are `words` (a tile number in the low ten bits, the palette row in bits
    /// 10 to 12, horizontal and vertical flips in bits 14 and 15), drawn from the art of the tiles at 8x. A tile with
    /// no art is left clear; `None` when no tile of the part has art at all, or `words` is not a card's.
    #[must_use]
    pub fn compose(&self, words: &[u16], part: Part) -> Option<Preview> {
        if words.len() != WORDS
            || part.column + part.columns > PREVIEW_TILES.0
            || part.row + part.rows > PREVIEW_TILES.1
            || part.columns == 0
            || part.rows == 0
        {
            return None;
        }
        let side = 8 * PREVIEW_SCALE;
        let (width, height) = (part.columns * side, part.rows * side);
        let mut rgba = vec![0_u8; width * height * 4];
        let mut drawn = false;
        for row in 0..part.rows {
            for column in 0..part.columns {
                let word = words[(part.row + row) * PREVIEW_TILES.0 + part.column + column];
                let code = word & 0x3FF;
                let colours = match (word >> 10) & 7 {
                    SYMBOL_ROW => SYMBOL_COLOURS,
                    YELLOW_ROW => YELLOW_COLOURS,
                    _ => BODY_COLOURS,
                };
                let tile = Tile {
                    x0: column * side,
                    y0: row * side,
                    flip_x: word & 0x4000 != 0,
                    flip_y: word & 0x8000 != 0,
                    colours,
                };
                if code == BLACK {
                    tile.fill(&mut rgba, width, side);
                    continue;
                }
                let Some(art) = self.tiles.iter().find(|art| art.code == code && !art.title) else {
                    continue;
                };
                drawn |=
                    tile.draw(&mut rgba, width, side, art.base.as_deref(), art.roles.as_deref());
            }
        }
        if part == Part::ATTACK || part == Part::DEFENCE {
            round(&mut rgba, width, height);
        }
        drawn.then_some(Preview { width, height, rgba })
    }
}

/// A ball on its own: its tiles hold the card's frame behind the sphere, so only the disc that fills the 2 x 2 tiles stays,
/// with a soft edge.
fn round(rgba: &mut [u8], width: usize, height: usize) {
    let (cx, cy) = (width as f32 / 2.0, height as f32 / 2.0);
    let radius = width.min(height) as f32 / 2.0 - BALL_MARGIN;
    for y in 0..height {
        for x in 0..width {
            let distance = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
            let keep = (radius - distance + 0.5).clamp(0.0, 1.0);
            let at = (y * width + x) * 4 + 3;
            rgba[at] = (f32::from(rgba[at]) * keep).round() as u8;
        }
    }
}

/// Where a tile goes in the picture, and how it is turned and coloured.
struct Tile {
    x0: usize,
    y0: usize,
    flip_x: bool,
    flip_y: bool,
    colours: [[u8; 3]; 3],
}

impl Tile {
    /// The tile as solid black.
    fn fill(&self, rgba: &mut [u8], width: usize, side: usize) {
        for y in self.y0..self.y0 + side {
            for x in self.x0..self.x0 + side {
                put(rgba, width, x, y, [0, 0, 0], 255);
            }
        }
    }

    /// The art of the tile (its fixed-colour layer, then the layer that takes the palette's colours); whether
    /// anything was drawn.
    fn draw(
        &self,
        rgba: &mut [u8],
        width: usize,
        side: usize,
        base: Option<&[u8]>,
        roles: Option<&[u8]>,
    ) -> bool {
        for y in 0..side {
            for x in 0..side {
                let sx = if self.flip_x { side - 1 - x } else { x };
                let sy = if self.flip_y { side - 1 - y } else { y };
                let at = (sy * TILE / side) * TILE + sx * TILE / side;
                if let Some(base) = base {
                    let p = &base[at * 4..at * 4 + 4];
                    put(rgba, width, self.x0 + x, self.y0 + y, [p[0], p[1], p[2]], p[3]);
                }
                if let Some(roles) = roles {
                    let p = &roles[at * 4..at * 4 + 4];
                    let weight = [u32::from(p[0]), u32::from(p[1]), u32::from(p[2])];
                    let total = (weight[0] + weight[1] + weight[2]).max(1);
                    let colour = [0, 1, 2].map(|c| {
                        let mixed = weight[0] * u32::from(self.colours[0][c])
                            + weight[1] * u32::from(self.colours[1][c])
                            + weight[2] * u32::from(self.colours[2][c]);
                        ((mixed + total / 2) / total) as u8
                    });
                    put(rgba, width, self.x0 + x, self.y0 + y, colour, p[3]);
                }
            }
        }
        base.is_some() || roles.is_some()
    }
}

/// `colour` over the pixel at (`x`, `y`) by `alpha`, straight alpha.
fn put(rgba: &mut [u8], width: usize, x: usize, y: usize, colour: [u8; 3], alpha: u8) {
    let at = (y * width + x) * 4;
    let under = u32::from(rgba[at + 3]);
    let a = u32::from(alpha);
    let out = a * 255 + under * (255 - a);
    if out == 0 {
        return;
    }
    for c in 0..3 {
        let mixed = u32::from(colour[c]) * a * 255 + u32::from(rgba[at + c]) * under * (255 - a);
        rgba[at + c] = ((mixed + out / 2) / out) as u8;
    }
    rgba[at + 3] = ((out + 127) / 255) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::Art;

    fn flat(code: u16, roles: [u8; 4]) -> Art {
        let fill = |p: [u8; 4]| p.iter().copied().cycle().take(TILE * TILE * 4).collect();
        Art { code, title: false, carve: false, base: None, roles: Some(fill(roles)) }
    }

    fn cards(tiles: Vec<Art>) -> CardArt {
        CardArt::new(tiles, Vec::new())
    }

    fn pixel(card: &Preview, x: usize, y: usize) -> [u8; 4] {
        let at = (y * card.width + x) * 4;
        card.rgba[at..at + 4].try_into().expect("four bytes")
    }

    /// 24 words, all one tile.
    fn words(word: u16) -> Vec<u16> {
        vec![word; WORDS]
    }

    #[test]
    fn a_part_is_as_big_as_its_tiles_and_the_whole_card_is_four_by_six() {
        let art = cards(vec![flat(0x96, [0, 255, 0, 255])]);
        let whole = art.compose(&words(0x96), Part::WHOLE).expect("a card");
        assert_eq!((whole.width, whole.height), (4 * 64, 6 * 64));
        let ball = art.compose(&words(0x96), Part::ATTACK).expect("a ball");
        assert_eq!((ball.width, ball.height), (2 * 64, 2 * 64));
    }

    #[test]
    fn the_palette_row_of_a_tile_word_picks_its_colours() {
        let art = cards(vec![flat(0x96, [0, 255, 0, 255])]);
        // The middle colour: orange in the body row (3), orange-red in row 0, pink in row 6
        let body = art.compose(&words(0x96 | 3 << 10), Part::SYMBOL).unwrap();
        assert_eq!(pixel(&body, 5, 5), [0xFF, 0x8B, 0x00, 255]);
        let ink = art.compose(&words(0x96), Part::SYMBOL).unwrap();
        assert_eq!(pixel(&ink, 5, 5), [0xFF, 0x52, 0x00, 255]);
        let yellow = art.compose(&words(0x96 | 6 << 10), Part::SYMBOL).unwrap();
        assert_eq!(pixel(&yellow, 5, 5), [0xFF, 0x63, 0xBD, 255]);
    }

    #[test]
    fn the_plain_black_tile_is_solid_and_a_tile_without_art_is_clear() {
        let art = cards(vec![flat(0x96, [255, 0, 0, 255])]);
        let mut hand = words(0x1F0);
        // The kanji square's tiles: row 2, columns 1 and 2, and the row under it
        hand[9] = BLACK;
        hand[10] = 0x96;
        let card = art.compose(&hand, Part::SYMBOL).unwrap();
        assert_eq!(pixel(&card, 5, 5), [0, 0, 0, 255], "black");
        assert_eq!(pixel(&card, 64 + 5, 5)[3], 255, "the art");
        assert_eq!(pixel(&card, 5, 64 + 5)[3], 0, "no art: clear");
    }

    #[test]
    fn a_part_whose_tiles_have_no_art_is_none() {
        let art = cards(vec![flat(0x96, [255, 0, 0, 255])]);
        assert!(art.compose(&words(0x1F0), Part::WHOLE).is_none());
        assert!(art.compose(&words(0x96)[..10], Part::WHOLE).is_none(), "not a card's words");
    }

    #[test]
    fn a_flipped_tile_is_drawn_mirrored() {
        // Art covering the left half of the tile only
        let mut half = vec![0_u8; TILE * TILE * 4];
        for y in 0..TILE {
            for x in 0..TILE / 2 {
                half[(y * TILE + x) * 4..][..4].copy_from_slice(&[255, 0, 0, 255]);
            }
        }
        let tile = Art { code: 0x96, title: false, carve: false, base: None, roles: Some(half) };
        let art = cards(vec![tile]);
        let plain = art.compose(&words(0x96), Part::SYMBOL).unwrap();
        let flipped = art.compose(&words(0x96 | 0x4000), Part::SYMBOL).unwrap();
        assert_eq!(pixel(&plain, 2, 2)[3], 255);
        assert_eq!(pixel(&plain, 125, 2)[3], 0);
        assert_eq!(pixel(&flipped, 2, 2)[3], 0);
        assert_eq!(pixel(&flipped, 125, 2)[3], 255);
    }

    #[test]
    fn a_ball_keeps_its_disc_and_loses_the_cards_frame_around_it() {
        let art = cards(vec![flat(0x96, [255, 0, 0, 255])]);
        let ball = art.compose(&words(0x96), Part::ATTACK).unwrap();
        assert_eq!(pixel(&ball, 64, 64)[3], 255, "the middle of the ball");
        assert_eq!(pixel(&ball, 2, 2)[3], 0, "the corner of the tiles is clear");
        assert_eq!(pixel(&ball, 125, 64)[3], 0, "past the sphere");
        let square = art.compose(&words(0x96), Part::SYMBOL).unwrap();
        assert_eq!(pixel(&square, 2, 2)[3], 255, "the kanji square keeps its corners");
    }
}
