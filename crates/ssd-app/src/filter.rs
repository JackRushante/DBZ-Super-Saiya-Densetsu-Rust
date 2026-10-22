//! The last pass over the picture the window shows: the 4:3 crop, the scanlines and the CRT filter of the Video
//! page. It works on the RGBA pixels of the game view before they are uploaded, never on the machine's or the
//! HD renderer's own picture, so with everything off the picture is the very one the renderer made (no copy).

use crate::canvas::GameView;

/// The scanlines and the CRT filter: 0 is off, 1 is Light (Soft), 2 is Strong (Full).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Filters {
    pub scanlines: u8,
    pub crt: u8,
}

impl Filters {
    /// Whether the pass changes nothing.
    #[must_use]
    pub const fn is_off(self) -> bool {
        self.scanlines == 0 && self.crt == 0
    }
}

/// What the pass does: crop to the original window (the 4:3 picture) and filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Shape {
    pub crop: bool,
    pub filters: Filters,
}

impl Shape {
    /// Whether the pass leaves the view as it is.
    #[must_use]
    pub const fn is_identity(self) -> bool {
        !self.crop && self.filters.is_off()
    }
}

/// The depth of the scanlines, in 256ths of the brightness the darkest row of a line loses.
const SCANLINE_DEPTH: [u32; 3] = [0, 64, 128];
/// The depth of the vignette (the corners' loss, in 256ths), the mask (the dim channels' loss) and the bleed
/// (the share of the neighbours mixed in, in 256ths) of the CRT filter.
const VIGNETTE: [u32; 3] = [0, 40, 84];
const MASK: [u32; 3] = [0, 20, 44];
const BLEED: [u32; 3] = [0, 48, 96];

/// Applies `shape` to `view`: the view itself when it is the identity, else a view of `buffer` holding the
/// cropped and filtered pixels.
pub fn shape<'a>(view: GameView<'a>, shape: Shape, buffer: &'a mut Vec<u8>) -> GameView<'a> {
    if shape.is_identity() {
        return view;
    }
    let (mut width, mut height) = (view.width as usize, view.height as usize);
    buffer.clear();
    let window = view
        .logical
        .filter(|_| shape.crop)
        .map(|[x, y, w, h]| [x as usize, y as usize, w as usize, h as usize])
        .filter(|&[x, y, w, h]| w > 0 && h > 0 && x + w <= width && y + h <= height);
    match window {
        Some([x, y, w, h]) => {
            buffer.reserve(w * h * 4);
            for row in y..y + h {
                let from = (row * width + x) * 4;
                buffer.extend_from_slice(&view.rgba[from..from + w * 4]);
            }
            (width, height) = (w, h);
        }
        None => buffer.extend_from_slice(view.rgba),
    }
    // How many pixels one line of the game is tall: the HD scale (1 for a Classic frame).
    let scale = (height / 224).max(1);
    apply(shape.filters, buffer, width, height, scale);
    // The pixels are the filters' of the shape as well as the picture's: the view names no picture of its own
    GameView {
        rgba: buffer,
        width: width as u32,
        height: height as u32,
        logical: None,
        serial: None,
        ..view
    }
}

/// Filters `rgba` (`width` x `height` pixels, `scale` pixels per line of the game) in place. Off changes nothing.
pub fn apply(filters: Filters, rgba: &mut [u8], width: usize, height: usize, scale: usize) {
    if filters.is_off() || width == 0 || height == 0 || rgba.len() < width * height * 4 {
        return;
    }
    let plan = Plan::new(filters, width, height, scale);
    let rows = &mut rgba[..width * height * 4];
    let threads = std::thread::available_parallelism().map_or(1, usize::from).min(4);
    let band = height.div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        for (index, chunk) in rows.chunks_mut(band * width * 4).enumerate() {
            let plan = &plan;
            scope.spawn(move || {
                let mut scratch = vec![0_u8; width * 4];
                for (offset, row) in chunk.chunks_exact_mut(width * 4).enumerate() {
                    plan.row(index * band + offset, row, &mut scratch);
                }
            });
        }
    });
}

/// The tables of one pass, worked out once for the picture's size.
struct Plan {
    width: usize,
    height: usize,
    /// The brightness of each pixel row of a line of the game, in 256ths.
    line: Vec<u32>,
    /// The vignette's brightness at each column, in 256ths.
    across: Vec<u32>,
    mask: u32,
    bleed: u32,
    vignette: u32,
    /// The pixels a neighbour of the bleed is away.
    reach: usize,
}

impl Plan {
    fn new(filters: Filters, width: usize, height: usize, scale: usize) -> Self {
        let depth = SCANLINE_DEPTH[usize::from(filters.scanlines).min(2)];
        let line = (0..scale)
            .map(|i| {
                // The top row of a line is lit fully; the darkest row loses `depth`, growing as the square.
                let t = if scale > 1 { (i * i * 256) / ((scale - 1) * (scale - 1)) } else { 0 };
                256 - (depth * t as u32) / 256
            })
            .collect();
        let vignette = VIGNETTE[usize::from(filters.crt).min(2)];
        let across = (0..width).map(|x| 256 - (vignette * edge(x, width)) / 256).collect();
        Self {
            width,
            height,
            line,
            across,
            mask: MASK[usize::from(filters.crt).min(2)],
            bleed: BLEED[usize::from(filters.crt).min(2)],
            vignette,
            reach: (scale / 2).max(1),
        }
    }

    /// Filters pixel row `y` in place; `scratch` holds a copy of the unfiltered row.
    fn row(&self, y: usize, row: &mut [u8], scratch: &mut [u8]) {
        let lit = self.line[y % self.line.len()];
        let down = 256 - (self.vignette * edge(y, self.height)) / 256;
        let crt = self.mask > 0;
        if crt {
            scratch.copy_from_slice(row);
        }
        for x in 0..self.width {
            let at = x * 4;
            let mut value = [u32::from(row[at]), u32::from(row[at + 1]), u32::from(row[at + 2])];
            if crt {
                // The bleed: a share of the neighbours, a little way left and right.
                let left = (x.saturating_sub(self.reach)) * 4;
                let right = ((x + self.reach).min(self.width - 1)) * 4;
                for (c, v) in value.iter_mut().enumerate() {
                    let around =
                        u32::midpoint(u32::from(scratch[left + c]), u32::from(scratch[right + c]));
                    *v = (*v * (256 - self.bleed) + around * self.bleed) / 256;
                }
            }
            let mut gain = lit;
            if crt {
                gain = gain * self.across[x] / 256 * down / 256;
            }
            for (c, v) in value.iter_mut().enumerate() {
                let mut g = gain;
                if crt {
                    // The mask dims two channels of three, and a gain makes up for the light it takes.
                    let dim = if x % 3 == c { 0 } else { self.mask };
                    g = g * (256 - dim) / 256 * (256 + self.mask * 2 / 3) / 256;
                }
                row[at + c] = ((*v * g) / 256).min(255) as u8;
            }
        }
    }
}

/// How far `i` of `n` is toward the edge, squared, from 0 (the middle) to 256 (the edge).
fn edge(i: usize, n: usize) -> u32 {
    if n < 2 {
        return 0;
    }
    let middle = (n - 1) as f32 / 2.0;
    let d = (i as f32 - middle).abs() / middle;
    (d * d * 256.0) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(width: usize, height: usize) -> Vec<u8> {
        (0..width * height * 4)
            .map(|i| if i % 4 == 3 { 255 } else { (i * 7 % 251) as u8 })
            .collect()
    }

    fn view(rgba: &[u8], width: u32, height: u32, logical: Option<[u32; 4]>) -> GameView<'_> {
        GameView { rgba, width, height, pixel_aspect: 1.0, smooth: true, logical, serial: None }
    }

    #[test]
    fn everything_off_is_the_identity_and_copies_nothing() {
        let rgba = picture(30, 12);
        let mut buffer = Vec::new();
        let shown = shape(view(&rgba, 30, 12, Some([0, 0, 10, 12])), Shape::default(), &mut buffer);
        assert!(
            std::ptr::eq(shown.rgba.as_ptr(), rgba.as_ptr()),
            "the very pixels of the renderer"
        );
        assert_eq!(buffer.len(), 0);
        let mut copy = rgba.clone();
        apply(Filters::default(), &mut copy, 30, 12, 4);
        assert_eq!(copy, rgba);
    }

    #[test]
    fn the_crop_keeps_the_original_window_and_nothing_else() {
        let (width, height) = (20, 6);
        let rgba = picture(width, height);
        let mut buffer = Vec::new();
        let shape_4_3 = Shape { crop: true, filters: Filters::default() };
        let shown = shape(view(&rgba, 20, 6, Some([5, 1, 8, 4])), shape_4_3, &mut buffer);
        assert_eq!((shown.width, shown.height), (8, 4));
        for row in 0..4 {
            let from = ((row + 1) * width + 5) * 4;
            assert_eq!(&shown.rgba[row * 32..(row + 1) * 32], &rgba[from..from + 32]);
        }
        // No window to crop to (a Classic frame), or one that is not inside the picture: the whole picture.
        let mut other = Vec::new();
        let whole = shape(view(&rgba, 20, 6, None), shape_4_3, &mut other);
        assert_eq!((whole.width, whole.height), (20, 6));
        let mut out = Vec::new();
        let bad = shape(view(&rgba, 20, 6, Some([15, 0, 9, 6])), shape_4_3, &mut out);
        assert_eq!((bad.width, bad.height), (20, 6));
    }

    #[test]
    fn scanlines_leave_the_top_row_of_each_line_and_darken_the_rest_more_when_strong() {
        let (width, height, scale) = (6, 8, 4);
        let rgba = vec![200_u8; width * height * 4];
        let level = |strength: u8| {
            let mut copy = rgba.clone();
            apply(Filters { scanlines: strength, crt: 0 }, &mut copy, width, height, scale);
            (0..height).map(|y| copy[y * width * 4]).collect::<Vec<_>>()
        };
        let (light, strong) = (level(1), level(2));
        assert_eq!((light[0], light[4]), (200, 200), "the top row of each line is untouched");
        assert!(light[1] < 200 && light[2] < light[1] && light[3] < light[2], "{light:?}");
        assert_eq!(light[0..4], light[4..8], "the pattern repeats every line");
        assert!(strong[3] < light[3], "{strong:?} against {light:?}");
        let mut copy = rgba.clone();
        apply(Filters { scanlines: 2, crt: 0 }, &mut copy, width, height, scale);
        assert!(copy.chunks(4).all(|p| p[3] == 200), "the alpha stays");
    }

    #[test]
    fn the_crt_filter_darkens_the_corners_more_than_the_middle_and_more_when_full() {
        let (width, height) = (60, 40);
        let rgba = vec![180_u8; width * height * 4];
        let at = |pixels: &[u8], x: usize, y: usize| pixels[(y * width + x) * 4 + 1];
        let mut soft = rgba.clone();
        apply(Filters { scanlines: 0, crt: 1 }, &mut soft, width, height, 1);
        let mut full = rgba.clone();
        apply(Filters { scanlines: 0, crt: 2 }, &mut full, width, height, 1);
        assert!(at(&soft, 0, 0) < at(&soft, 30, 20), "the edge is darker");
        assert!(at(&full, 0, 0) < at(&soft, 0, 0), "and darker when full");
        assert!(at(&full, 30, 20) > 100, "the middle stays lit");
        // The filter is deterministic: the same picture twice gives the same pixels.
        let mut again = rgba.clone();
        apply(Filters { scanlines: 0, crt: 2 }, &mut again, width, height, 1);
        assert_eq!(again, full);
    }

    #[test]
    fn a_picture_shorter_than_its_size_says_is_left_alone() {
        let mut rgba = vec![9_u8; 10];
        apply(Filters { scanlines: 2, crt: 2 }, &mut rgba, 8, 8, 2);
        assert_eq!(rgba, vec![9_u8; 10]);
    }
}
