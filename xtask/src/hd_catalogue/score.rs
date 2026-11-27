//! The scores of a piece of HD art against the native picture it is made for.
//!
//! The art is shrunk to the native size by averaging each block of pixels (`factor` x `factor`),
//! weighting colours by their opacity; a native pixel is covered when at least half of the art
//! pixels of its block are opaque.

/// The fewest interior pixels a colour score is measured over.
const MIN_PIXELS: u32 = 40;

/// An art image shrunk to the native size.
#[derive(Debug, Clone)]
pub struct Down {
    pub w: usize,
    pub h: usize,
    /// Share of the block that is opaque (0 to 1), per native pixel.
    pub alpha: Vec<f32>,
    /// Mean colour of the opaque art pixels of the block.
    pub rgb: Vec<[f32; 3]>,
}

impl Down {
    /// Whether the art covers native pixel `n`.
    #[must_use]
    pub fn covers(&self, n: usize) -> bool {
        self.alpha[n] >= 0.5
    }

    /// The image flipped left to right.
    #[must_use]
    pub fn mirrored(&self) -> Self {
        let mut out = self.clone();
        for y in 0..self.h {
            for x in 0..self.w {
                out.alpha[y * self.w + x] = self.alpha[y * self.w + (self.w - 1 - x)];
                out.rgb[y * self.w + x] = self.rgb[y * self.w + (self.w - 1 - x)];
            }
        }
        out
    }
}

impl Down {
    /// The image flipped top to bottom.
    #[must_use]
    pub fn flipped(&self) -> Self {
        let mut out = self.clone();
        for y in 0..self.h {
            let from = (self.h - 1 - y) * self.w;
            out.alpha[y * self.w..(y + 1) * self.w]
                .copy_from_slice(&self.alpha[from..from + self.w]);
            out.rgb[y * self.w..(y + 1) * self.w].copy_from_slice(&self.rgb[from..from + self.w]);
        }
        out
    }
}

/// The art `rgba` (`aw` x `ah`) shrunk to `w` x `h`; `None` when the art is not a whole multiple
/// of that size.
#[must_use]
pub fn downsample(rgba: &[u8], aw: usize, ah: usize, w: usize, h: usize) -> Option<Down> {
    if w == 0
        || h == 0
        || !aw.is_multiple_of(w)
        || !ah.is_multiple_of(h)
        || aw / w != ah / h
        || rgba.len() != aw * ah * 4
    {
        return None;
    }
    let factor = aw / w;
    let mut alpha = vec![0.0_f32; w * h];
    let mut rgb = vec![[0.0_f32; 3]; w * h];
    for y in 0..h {
        for x in 0..w {
            let (mut opaque, mut sum) = (0_u32, [0_u32; 3]);
            for dy in 0..factor {
                for dx in 0..factor {
                    let at = ((y * factor + dy) * aw + x * factor + dx) * 4;
                    if rgba[at + 3] > 127 {
                        opaque += 1;
                        for c in 0..3 {
                            sum[c] += u32::from(rgba[at + c]);
                        }
                    }
                }
            }
            alpha[y * w + x] = opaque as f32 / (factor * factor) as f32;
            if opaque > 0 {
                rgb[y * w + x] = [
                    sum[0] as f32 / opaque as f32,
                    sum[1] as f32 / opaque as f32,
                    sum[2] as f32 / opaque as f32,
                ];
            }
        }
    }
    Some(Down { w, h, alpha, rgb })
}

/// The share of the union of the native silhouette and the art's that both cover.
#[must_use]
pub fn iou(native: &[u8], art: &Down) -> f32 {
    let (mut both, mut either) = (0_u32, 0_u32);
    for (n, &index) in native.iter().enumerate() {
        let (a, b) = (index != 0, art.covers(n));
        both += u32::from(a && b);
        either += u32::from(a || b);
    }
    if either == 0 { 1.0 } else { both as f32 / either as f32 }
}

/// How the colours of the art fall on the palette indexes of the native picture.
#[derive(Debug, Clone, Default)]
pub struct ColourScore {
    /// Share of them whose art colour is nearest to the colour of their own index.
    pub share: f32,
    /// Mean distance (RGB, 0 to 441) of the art colour from the colour of the pixel's index.
    pub distance: f32,
    /// Per palette index: pixels, mean distance, share that quantises back.
    pub per_index: Vec<(u8, u32, f32, f32)>,
}

fn distance(a: [f32; 3], b: [u8; 3]) -> f32 {
    let d = [a[0] - f32::from(b[0]), a[1] - f32::from(b[1]), a[2] - f32::from(b[2])];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// The colour score of `art` over the pixels of `native` (palette indexes, `w` wide): for each
/// index the art's colour against `reference[index]`, over the interior pixels (those whose four
/// neighbours have the same index, so that the art's blended outlines do not count). `None` when
/// fewer than 40 pixels are measured.
#[must_use]
pub fn colour(
    native: &[u8],
    w: usize,
    art: &Down,
    reference: &[Option<[u8; 3]>; 16],
) -> Option<ColourScore> {
    let h = native.len() / w;
    let used: Vec<u8> = (1..16_u8)
        .filter(|&i| reference[usize::from(i)].is_some() && native.contains(&i))
        .collect();
    let mut per: [(u32, f64, u32); 16] = [(0, 0.0, 0); 16];
    for y in 1..h.saturating_sub(1) {
        for x in 1..w.saturating_sub(1) {
            let n = y * w + x;
            let index = native[n];
            let Some(own) = reference[usize::from(index & 15)].filter(|_| index != 0) else {
                continue;
            };
            let interior = [n - 1, n + 1, n - w, n + w].iter().all(|&m| native[m] == index);
            if !interior || !art.covers(n) {
                continue;
            }
            let c = art.rgb[n];
            let own_distance = distance(c, own);
            let nearest = used
                .iter()
                .filter_map(|&i| reference[usize::from(i)].map(|colour| distance(c, colour)))
                .fold(f32::MAX, f32::min);
            let slot = &mut per[usize::from(index & 15)];
            slot.0 += 1;
            slot.1 += f64::from(own_distance);
            slot.2 += u32::from(own_distance <= nearest + 0.5);
        }
    }
    let pixels: u32 = per.iter().map(|p| p.0).sum();
    if pixels < MIN_PIXELS {
        return None;
    }
    let share = per.iter().map(|p| p.2).sum::<u32>() as f32 / pixels as f32;
    let distance = (per.iter().map(|p| p.1).sum::<f64>() / f64::from(pixels)) as f32;
    let per_index = per
        .iter()
        .enumerate()
        .filter(|(_, p)| p.0 > 0)
        .map(|(i, p)| (i as u8, p.0, (p.1 / f64::from(p.0)) as f32, p.2 as f32 / p.0 as f32))
        .collect();
    Some(ColourScore { share, distance, per_index })
}

fn luma(c: [f32; 3]) -> f32 {
    0.299 * c[0] + 0.587 * c[1] + 0.114 * c[2]
}

/// The correlation (-1 to 1) of the luminance of the native picture and the art's, over all
/// pixels: the structure of the picture (where it is dark and light) whatever the colours.
#[must_use]
pub fn luma_correlation(native: &[u8], palette: &[[u8; 3]; 16], art: &Down) -> f32 {
    let a: Vec<f32> =
        native.iter().map(|&i| luma(palette[usize::from(i & 15)].map(f32::from))).collect();
    let b: Vec<f32> =
        (0..native.len()).map(|n| if art.covers(n) { luma(art.rgb[n]) } else { 0.0 }).collect();
    let (ma, mb) = (a.iter().sum::<f32>() / a.len() as f32, b.iter().sum::<f32>() / b.len() as f32);
    let (mut cov, mut va, mut vb) = (0.0_f32, 0.0_f32, 0.0_f32);
    for (x, y) in a.iter().zip(&b) {
        cov += (x - ma) * (y - mb);
        va += (x - ma) * (x - ma);
        vb += (y - mb) * (y - mb);
    }
    if va <= f32::EPSILON || vb <= f32::EPSILON { 0.0 } else { cov / (va.sqrt() * vb.sqrt()) }
}

/// How well the palette indexes of the native picture explain the light and dark of the art, whatever their
/// colours: the correlation of the art's luminance with its own mean over the pixels of each index (1 when
/// every index is one flat tone of the art). For pictures whose palette is not known.
#[must_use]
pub fn index_structure(native: &[u8], art: &Down) -> f32 {
    let lumas: Vec<f32> =
        (0..native.len()).map(|n| if art.covers(n) { luma(art.rgb[n]) } else { 0.0 }).collect();
    let mut sums = [(0.0_f32, 0_u32); 16];
    for (&i, &l) in native.iter().zip(&lumas) {
        let slot = &mut sums[usize::from(i & 15)];
        slot.0 += l;
        slot.1 += 1;
    }
    let means: Vec<f32> = native
        .iter()
        .map(|&i| {
            let (sum, count) = sums[usize::from(i & 15)];
            sum / count.max(1) as f32
        })
        .collect();
    let n = lumas.len() as f32;
    let (ma, mb) = (lumas.iter().sum::<f32>() / n, means.iter().sum::<f32>() / n);
    let (mut cov, mut va, mut vb) = (0.0_f32, 0.0_f32, 0.0_f32);
    for (x, y) in lumas.iter().zip(&means) {
        cov += (x - ma) * (y - mb);
        va += (x - ma) * (x - ma);
        vb += (y - mb) * (y - mb);
    }
    if va <= f32::EPSILON || vb <= f32::EPSILON { 0.0 } else { cov / (va.sqrt() * vb.sqrt()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An art of `w` x `h` blocks of 2 x 2 pixels, each block the colour `colour(x, y)` or clear.
    fn art(
        w: usize,
        h: usize,
        colour: impl Fn(usize, usize) -> Option<[u8; 3]>,
    ) -> (Vec<u8>, usize, usize) {
        let (aw, ah) = (w * 2, h * 2);
        let mut rgba = vec![0_u8; aw * ah * 4];
        for y in 0..ah {
            for x in 0..aw {
                if let Some(c) = colour(x / 2, y / 2) {
                    rgba[(y * aw + x) * 4..][..4].copy_from_slice(&[c[0], c[1], c[2], 255]);
                }
            }
        }
        (rgba, aw, ah)
    }

    #[test]
    fn the_silhouette_of_the_same_shape_is_one_and_of_another_is_less() {
        let native: Vec<u8> = (0..16).map(|n| u8::from(n % 4 < 2)).collect();
        let (rgba, aw, ah) = art(4, 4, |x, _| (x < 2).then_some([9, 9, 9]));
        let down = downsample(&rgba, aw, ah, 4, 4).unwrap();
        assert!((iou(&native, &down) - 1.0).abs() < 1e-6);
        let (rgba, aw, ah) = art(4, 4, |x, _| (1..3).contains(&x).then_some([9, 9, 9]));
        let down = downsample(&rgba, aw, ah, 4, 4).unwrap();
        assert!((iou(&native, &down) - 1.0 / 3.0).abs() < 1e-6);
    }

    #[test]
    fn a_size_that_is_not_a_multiple_is_refused() {
        assert!(downsample(&[0; 6 * 6 * 4], 6, 6, 4, 4).is_none());
        assert!(downsample(&[0; 8 * 4 * 4], 8, 4, 4, 4).is_none());
    }

    #[test]
    fn colours_that_quantise_back_score_one_and_swapped_ones_zero() {
        // A 12 x 12 picture, left half index 1, right half index 2
        let native: Vec<u8> = (0..144).map(|n| if n % 12 < 6 { 1 } else { 2 }).collect();
        let mut reference = [None; 16];
        reference[1] = Some([200, 0, 0]);
        reference[2] = Some([0, 0, 200]);
        let (rgba, aw, ah) =
            art(12, 12, |x, _| Some(if x < 6 { [200, 0, 0] } else { [0, 0, 200] }));
        let down = downsample(&rgba, aw, ah, 12, 12).unwrap();
        let score = colour(&native, 12, &down, &reference).unwrap();
        assert!((score.share - 1.0).abs() < 1e-6 && score.distance < 1e-3, "{score:?}");
        let (rgba, aw, ah) =
            art(12, 12, |x, _| Some(if x < 6 { [0, 0, 200] } else { [200, 0, 0] }));
        let down = downsample(&rgba, aw, ah, 12, 12).unwrap();
        let score = colour(&native, 12, &down, &reference).unwrap();
        assert!(score.share < 1e-6 && score.distance > 250.0, "{score:?}");
        // Too few interior pixels to say anything
        let (rgba, aw, ah) = art(6, 6, |_, _| Some([200, 0, 0]));
        let down = downsample(&rgba, aw, ah, 6, 6).unwrap();
        assert!(colour(&[1; 36], 6, &down, &reference).is_none());
    }

    #[test]
    fn the_indexes_explain_an_art_whose_tones_follow_them_and_not_one_that_ignores_them() {
        let native: Vec<u8> = (0..16).map(|n| if n % 4 < 2 { 1 } else { 2 }).collect();
        let (rgba, aw, ah) =
            art(4, 4, |x, _| Some(if x < 2 { [250, 240, 100] } else { [20, 0, 30] }));
        let down = downsample(&rgba, aw, ah, 4, 4).unwrap();
        assert!(index_structure(&native, &down) > 0.99);
        let (rgba, aw, ah) =
            art(4, 4, |_, y| Some(if y < 2 { [250, 240, 100] } else { [20, 0, 30] }));
        let down = downsample(&rgba, aw, ah, 4, 4).unwrap();
        assert!(index_structure(&native, &down).abs() < 0.01);
    }

    #[test]
    fn the_structure_of_light_and_dark_correlates_whatever_the_hue() {
        let native: Vec<u8> = (0..16).map(|n| if n % 4 < 2 { 1 } else { 2 }).collect();
        let mut palette = [[0_u8; 3]; 16];
        palette[1] = [250, 250, 250];
        palette[2] = [10, 10, 10];
        let (rgba, aw, ah) =
            art(4, 4, |x, _| Some(if x < 2 { [250, 240, 100] } else { [20, 0, 30] }));
        let down = downsample(&rgba, aw, ah, 4, 4).unwrap();
        assert!(luma_correlation(&native, &palette, &down) > 0.95);
        let (rgba, aw, ah) =
            art(4, 4, |x, _| Some(if x < 2 { [20, 0, 30] } else { [250, 240, 100] }));
        let down = downsample(&rgba, aw, ah, 4, 4).unwrap();
        assert!(luma_correlation(&native, &palette, &down) < -0.95);
    }
}
