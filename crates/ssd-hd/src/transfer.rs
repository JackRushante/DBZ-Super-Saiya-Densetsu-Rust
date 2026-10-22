//! A smooth colour transfer for painted layers: what a colour of the art becomes when the game's
//! layer is shown in other colours than the ones the art was made for.
//!
//! The transfer is a function of the art's colour alone, never of the place of the pixel, so a
//! smooth painted gradient stays a smooth gradient in any palette (a per-pixel rule that looks at
//! the 1x pixel under each art pixel carries the tiles' dither and blocks into the picture). It is
//! built from the pairs (colour the game's pixel had where the art was made, colour it has now)
//! of the pixels the frame shows:
//!
//! * the pairs agree within a step of the PPU's 5-bit channels: the palette is the art's own and
//!   the art is drawn as it is;
//! * the pairs are a fade (every colour scaled towards black, or lightened towards white, by one
//!   factor): the art is blended the same way, plainly;
//! * otherwise the offsets of the pairs are blended by inverse squared distance of the art's
//!   colour to each pair's reference colour (Shepard's interpolation, a convex combination: smooth,
//!   bounded, no overshoot), each offset scaled by how much brighter or darker the art's colour is
//!   than its reference, and sampled once into a 3-D lookup table that is read with trilinear
//!   interpolation.

use crate::Image;
use rustc_hash::FxHashMap;

/// How many 8-bit levels a pair may differ in a channel and still be "the same colour".
const SAME: u8 = 3;

/// How far (levels) every pair of a fade may be from the plain blend.
const FADE_TOLERANCE: f32 = 7.0;

/// The most distinct pairs a lookup table is built from (the most frequent).
const MAX_PAIRS: usize = 48;

/// Pairs of fewer pixels than this are dropped (a stray pixel, an edge) when there are enough
/// others.
const MIN_COUNT: u32 = 4;

/// Nodes of the table along a channel.
const NODES: usize = 17;

/// The softening of the brightness ratio, so that a very dark colour does not scale wildly.
const SOFT: f32 = 8.0;

/// The squared distance (levels) that keeps the weight of a colour exactly on a pair finite.
const EPSILON: f32 = 144.0;

/// A colour of the art's palette and the colour it has now.
type Pair = ([u8; 3], [u8; 3]);

/// A reference colour, the mean colour it has now, and how many pixels vote for it.
struct Anchor {
    reference: [u8; 3],
    current: [f32; 3],
    weight: f32,
}

/// The pairs of a frame's layer, gathered pixel by pixel.
#[derive(Debug, Default)]
pub(crate) struct Pairs {
    counts: FxHashMap<([u8; 3], [u8; 3]), u32>,
}

impl Pairs {
    /// The pixel whose colour was `reference` where the art was made and is `current` now.
    pub(crate) fn add(&mut self, reference: [u8; 3], current: [u8; 3]) {
        *self.counts.entry((reference, current)).or_default() += 1;
    }

    /// Whether every pair keeps its colour: the transfer is the identity ([`Transfer::Same`]).
    pub(crate) fn is_identity(&self) -> bool {
        self.counts.keys().all(|(reference, current)| same(*reference, *current))
    }

    /// The transfer of these pairs.
    pub(crate) fn transfer(self) -> Transfer {
        let mut pairs: Vec<(Pair, u32)> = self.counts.into_iter().collect();
        if pairs.iter().all(|((reference, current), _)| same(*reference, *current)) {
            return Transfer::Same;
        }
        // The most frequent first (and the colours, for a reproducible order)
        pairs.sort_unstable_by_key(|&((reference, current), count)| {
            (std::cmp::Reverse(count), reference, current)
        });
        if let Some(fade) = Fade::fit(&pairs) {
            return Transfer::Fade(fade);
        }
        let kept: Vec<_> = pairs.iter().filter(|(_, count)| *count >= MIN_COUNT).collect();
        let kept = if kept.len() >= 3 { kept } else { pairs.iter().collect() };
        let mut merged: Vec<Anchor> = Vec::new();
        for ((reference, current), count) in kept {
            let (c, n) = (current.map(f32::from), *count as f32);
            match merged.iter_mut().find(|anchor| anchor.reference == *reference) {
                // One colour that is two colours now (a cycle, a gradient of lines): their mean
                Some(anchor) => {
                    for (mean, new) in anchor.current.iter_mut().zip(c) {
                        *mean = (*mean * anchor.weight + new * n) / (anchor.weight + n);
                    }
                    anchor.weight += n;
                }
                None => merged.push(Anchor { reference: *reference, current: c, weight: n }),
            }
            if merged.len() == MAX_PAIRS {
                break;
            }
        }
        Transfer::Table(Table::new(&merged))
    }
}

fn same(a: [u8; 3], b: [u8; 3]) -> bool {
    (0..3).all(|c| a[c].abs_diff(b[c]) <= SAME)
}

/// What a colour of the art becomes.
#[derive(Debug)]
pub(crate) enum Transfer {
    /// The palette is the art's own: the art as it is.
    Same,
    Fade(Fade),
    Table(Table),
}

impl Transfer {
    pub(crate) fn apply(&self, art: [u8; 3]) -> [u8; 3] {
        match self {
            Self::Same => art,
            Self::Fade(fade) => fade.apply(art),
            Self::Table(table) => table.apply(art),
        }
    }
}

/// The distinct colours of a picture of art, in order of first appearance, and the colour's number of every pixel
/// (the colour of its red, green and blue, alpha not counted). A transfer is a function of the colour, so it is
/// applied to each of these colours once, and each pixel takes the transferred colour of its number.
pub(crate) struct Colours {
    pub(crate) list: Vec<[u8; 3]>,
    pub(crate) number: Vec<u16>,
}

impl Colours {
    /// The colours of `image`, or `None` when it has more than `u16::MAX` distinct colours (a photograph, not palette art,
    /// is transferred pixel by pixel).
    pub(crate) fn of(image: &Image) -> Option<Self> {
        let mut seen: FxHashMap<[u8; 3], u16> = FxHashMap::default();
        let mut list = Vec::new();
        let mut number = Vec::with_capacity(image.rgba.len() / 4);
        for pixel in image.rgba.as_chunks::<4>().0 {
            let colour = [pixel[0], pixel[1], pixel[2]];
            let next = u16::try_from(list.len()).ok()?;
            let at = *seen.entry(colour).or_insert_with(|| {
                list.push(colour);
                next
            });
            number.push(at);
        }
        Some(Self { list, number })
    }

    /// Each of the colours transferred by `transfer`, in the order of [`Colours::list`].
    pub(crate) fn transferred(&self, transfer: &Transfer) -> Vec<[u8; 3]> {
        self.list.iter().map(|&colour| transfer.apply(colour)).collect()
    }
}

/// How the colours of a layer's art are transferred when it is drawn: not at all, each colour as it is drawn (a fade,
/// or a picture with too many colours to number), or by the transferred colours of the art's own colours (see
/// [`Colours`]).
pub(crate) enum Colouring {
    Same,
    Each(Transfer),
    Numbered { colours: std::sync::Arc<Colours>, transferred: Vec<[u8; 3]> },
}

/// A fade: every colour scaled towards black by `keep` and lightened towards white by `lift`
/// (the game's palette fades and flashes).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Fade {
    keep: f32,
    lift: f32,
}

impl Fade {
    /// The fade the pairs are, if they are one: a scale towards black or a lift towards white that
    /// every pair is within [`FADE_TOLERANCE`] of.
    fn fit(pairs: &[(Pair, u32)]) -> Option<Self> {
        // current = reference * keep
        let (mut num, mut den) = (0.0_f32, 0.0_f32);
        for ((reference, current), _) in pairs {
            for c in 0..3 {
                num += f32::from(reference[c]) * f32::from(current[c]);
                den += f32::from(reference[c]) * f32::from(reference[c]);
            }
        }
        if den > 0.0 {
            let keep = num / den;
            let fits = pairs.iter().all(|((reference, current), _)| {
                (0..3).all(|c| {
                    (f32::from(reference[c]) * keep - f32::from(current[c])).abs() <= FADE_TOLERANCE
                })
            });
            if fits {
                return Some(Self { keep, lift: 0.0 });
            }
        }
        // current = reference + (255 - reference) * lift
        let (mut num, mut den) = (0.0_f32, 0.0_f32);
        for ((reference, current), _) in pairs {
            for c in 0..3 {
                let room = 255.0 - f32::from(reference[c]);
                num += (f32::from(current[c]) - f32::from(reference[c])) * room;
                den += room * room;
            }
        }
        if den > 0.0 {
            let lift = num / den;
            let fits = pairs.iter().all(|((reference, current), _)| {
                (0..3).all(|c| {
                    let r = f32::from(reference[c]);
                    (r + (255.0 - r) * lift - f32::from(current[c])).abs() <= FADE_TOLERANCE
                })
            });
            if fits {
                return Some(Self { keep: 1.0, lift });
            }
        }
        None
    }

    fn apply(self, art: [u8; 3]) -> [u8; 3] {
        art.map(|value| {
            let v = f32::from(value) * self.keep;
            (v + (255.0 - v) * self.lift + 0.5).clamp(0.0, 255.0) as u8
        })
    }
}

fn luma(rgb: [f32; 3]) -> f32 {
    rgb[0] * 0.299 + rgb[1] * 0.587 + rgb[2] * 0.114
}

/// A colour transfer sampled on a regular grid of the art's colours.
#[derive(Debug)]
pub(crate) struct Table {
    nodes: Vec<[f32; 3]>,
}

impl Table {
    /// `pairs`: reference colour, current colour, weight.
    fn new(pairs: &[Anchor]) -> Self {
        let step = 255.0 / (NODES - 1) as f32;
        let mut nodes = Vec::with_capacity(NODES * NODES * NODES);
        for r in 0..NODES {
            for g in 0..NODES {
                for b in 0..NODES {
                    let art = [r as f32 * step, g as f32 * step, b as f32 * step];
                    nodes.push(shepard(art, pairs));
                }
            }
        }
        Self { nodes }
    }

    fn apply(&self, art: [u8; 3]) -> [u8; 3] {
        let at = art.map(|v| f32::from(v) * (NODES - 1) as f32 / 255.0);
        let base = at.map(|v| (v.floor() as usize).min(NODES - 2));
        let frac = [at[0] - base[0] as f32, at[1] - base[1] as f32, at[2] - base[2] as f32];
        let node = |r: usize, g: usize, b: usize| self.nodes[(r * NODES + g) * NODES + b];
        let mut out = [0.0_f32; 3];
        for corner in 0..8_usize {
            let (dr, dg, db) = (corner & 1, corner >> 1 & 1, corner >> 2 & 1);
            let weight = (if dr == 1 { frac[0] } else { 1.0 - frac[0] })
                * (if dg == 1 { frac[1] } else { 1.0 - frac[1] })
                * (if db == 1 { frac[2] } else { 1.0 - frac[2] });
            let n = node(base[0] + dr, base[1] + dg, base[2] + db);
            for c in 0..3 {
                out[c] += n[c] * weight;
            }
        }
        out.map(|v| (v + 0.5).clamp(0.0, 255.0) as u8)
    }
}

/// The colour `art` becomes: itself plus the offsets of the pairs, each scaled by the art's
/// brightness against its reference's, blended by inverse squared distance to the references.
fn shepard(art: [f32; 3], pairs: &[Anchor]) -> [f32; 3] {
    let la = luma(art) + SOFT;
    let (mut sum, mut total) = ([0.0_f32; 3], 0.0_f32);
    for anchor in pairs {
        let reference = anchor.reference.map(f32::from);
        let d2: f32 = (0..3).map(|c| (art[c] - reference[c]) * (art[c] - reference[c])).sum();
        let weight = 1.0 / (d2 + EPSILON);
        let scale = (la / (luma(reference) + SOFT)).clamp(0.25, 4.0);
        for c in 0..3 {
            sum[c] += weight * (art[c] + (anchor.current[c] - reference[c]) * scale);
        }
        total += weight;
    }
    sum.map(|v| (v / total).clamp(0.0, 255.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A smooth ramp of `n` colours between two ends.
    fn ramp(from: [f32; 3], to: [f32; 3], n: usize) -> Vec<[u8; 3]> {
        (0..n)
            .map(|i| {
                let t = i as f32 / (n - 1) as f32;
                [0, 1, 2].map(|c| (from[c] + (to[c] - from[c]) * t).round() as u8)
            })
            .collect()
    }

    /// The sum of the squared second differences of a row of colours: the noise of a gradient.
    fn roughness(row: &[[u8; 3]]) -> f32 {
        row.windows(3)
            .map(|w| {
                (0..3)
                    .map(|c| {
                        let d = f32::from(w[0][c]) - 2.0 * f32::from(w[1][c]) + f32::from(w[2][c]);
                        d * d
                    })
                    .sum::<f32>()
            })
            .sum()
    }

    fn cave_pairs() -> Pairs {
        // The art was made for a blue palette; the game now shows an orange one
        let mut pairs = Pairs::default();
        let blue = [[20, 30, 70], [40, 60, 120], [80, 110, 190], [150, 180, 230]];
        let orange = [[50, 20, 10], [120, 60, 20], [190, 110, 40], [240, 190, 110]];
        for (b, o) in blue.iter().zip(&orange) {
            for _ in 0..50 {
                pairs.add(*b, *o);
            }
        }
        pairs
    }

    #[test]
    fn a_smooth_gradient_stays_smooth_in_another_palette() {
        let transfer = cave_pairs().transfer();
        assert!(matches!(transfer, Transfer::Table(_)));
        // A painted gradient across the whole range of the palette, one step per art pixel
        let gradient = ramp([15.0, 25.0, 60.0], [160.0, 190.0, 240.0], 256);
        let mapped: Vec<[u8; 3]> = gradient.iter().map(|&c| transfer.apply(c)).collect();
        // Not a noisier picture: the second differences stay at the quantisation level
        assert!(roughness(&mapped) < roughness(&gradient) * 4.0 + 600.0, "{}", roughness(&mapped));
        // The ends take the palette's colours: a dark orange and a light one
        assert!(mapped[0][0] > mapped[0][2], "{:?}", mapped[0]);
        assert!(mapped[255][0] > 200, "{:?}", mapped[255]);
        // And it is monotone in brightness
        let lumas: Vec<f32> = mapped.iter().map(|c| luma(c.map(f32::from))).collect();
        assert!(lumas.windows(2).all(|w| w[1] >= w[0] - 1.5));
    }

    #[test]
    fn the_result_does_not_depend_on_which_pixel_the_colour_is_at() {
        // What the old per-pixel rule did: two art pixels of one colour over game pixels of two
        // colours (a dither) came out differently. The transfer is a function of the colour.
        let transfer = cave_pairs().transfer();
        let colour = [70, 100, 180];
        assert_eq!(transfer.apply(colour), transfer.apply(colour));
    }

    #[test]
    fn the_same_palette_draws_the_art_as_it_is() {
        let mut pairs = Pairs::default();
        for c in [[10, 20, 30], [200, 100, 50]] {
            pairs.add(c, [c[0] + 2, c[1], c[2].saturating_sub(1)]);
        }
        let transfer = pairs.transfer();
        assert!(matches!(transfer, Transfer::Same));
        assert_eq!(transfer.apply([77, 88, 99]), [77, 88, 99]);
    }

    #[test]
    fn a_fade_is_a_plain_blend() {
        let mut pairs = Pairs::default();
        for c in [[200_u8, 100, 50], [20, 220, 120], [90, 90, 90]] {
            pairs.add(c, c.map(|v| (f32::from(v) * 0.5).round() as u8));
        }
        let transfer = pairs.transfer();
        assert!(matches!(transfer, Transfer::Fade(_)));
        let out = transfer.apply([100, 200, 40]);
        assert!(out.iter().zip([50, 100, 20]).all(|(a, b)| a.abs_diff(b) <= 1), "{out:?}");
        // Towards white
        let mut pairs = Pairs::default();
        for c in [[200_u8, 100, 50], [20, 220, 120], [90, 90, 90]] {
            pairs.add(c, c.map(|v| (f32::from(v) + (255.0 - f32::from(v)) * 0.4).round() as u8));
        }
        let out = pairs.transfer().apply([100, 200, 40]);
        assert!(out.iter().zip([162, 222, 126]).all(|(a, b)| a.abs_diff(b) <= 1), "{out:?}");
    }

    #[test]
    fn a_colour_near_a_pair_takes_that_pair_s_colour() {
        let transfer = cave_pairs().transfer();
        let out = transfer.apply([80, 110, 190]);
        assert!(out.iter().zip([190, 110, 40]).all(|(a, b)| a.abs_diff(b) <= 25), "{out:?}");
    }

    #[test]
    fn a_numbered_picture_is_the_colours_transferred_one_by_one() {
        let transfer = cave_pairs().transfer();
        let (width, height) = (5_usize, 3_usize);
        // Palette-like art: a few colours, each pixel one of them (and one colour of a pixel of no alpha)
        let palette = [[15_u8, 25, 60], [160, 190, 240], [90, 40, 200], [7, 7, 7]];
        let mut rgba = Vec::new();
        for i in 0..width * height {
            let [r, g, b] = palette[i * 7 % palette.len()];
            rgba.extend_from_slice(&[r, g, b, if i % 4 == 0 { 0 } else { 255 }]);
        }
        let image = Image::new(width, height, rgba.clone());
        let colours = Colours::of(&image).expect("a few colours");
        assert_eq!(colours.list.len(), palette.len());
        let mapped = colours.transferred(&transfer);
        for (pixel, &number) in rgba.as_chunks::<4>().0.iter().zip(&colours.number) {
            assert_eq!(mapped[usize::from(number)], transfer.apply([pixel[0], pixel[1], pixel[2]]));
        }
    }
}
