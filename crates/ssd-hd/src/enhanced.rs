//! The Enhanced picture: the frame the PPU drew, drawn again at a whole multiple of its resolution
//! from the same render log and nothing else (`--hd-scale`).
//!
//! The 1x renderer runs first, and while it draws it records what the picture needs of each pixel
//! ([`Capture`]): whether a Mode 7 plane is the top layer there, the colour math of the pixel, and
//! the state of its line the plane was drawn with. The picture is then the look ([`Look`]) over the
//! 1x frame, except on the plane pixels, which are sampled again at the picture's resolution: the
//! matrix is applied to each finer position, so the plane's diagonals are drawn on that grid (the
//! HD Mode 7). Every pass works row by row on the rayon pool and reads only what was recorded, so
//! the output does not depend on the threads.

use crate::art;
use crate::mode7::Plane;
use crate::screen::{Plan, convert, direct_color};
use crate::{Frame, Renderer, Unsupported, WIDTH};
use rayon::prelude::*;
use ssd_oracle::{Color, FrameLog, LineRegs};

/// The look of the Enhanced picture where it is not a Mode 7 plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Look {
    /// Pixel art made bigger: Scale2x (EPX) on the power-of-two part of the scale, then whole
    /// pixels, so every block is uniform and the diagonals are smoothed.
    #[default]
    Crisp,
    /// A smooth interpolation: a bicubic (Catmull-Rom) filter over the 1x frame.
    Smooth,
}

/// How an Enhanced picture is made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HdOptions {
    /// Pixels of the picture per pixel of the PPU's frame, 1 to [`crate::MAX_SCALE`].
    pub scale: u32,
    pub look: Look,
}

/// A rectangle of an [`HdFrame`], in its pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// An Enhanced picture: `width * height` RGBA pixels, row by row, alpha 255.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HdFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub scale: u32,
    /// Where the PPU's own picture sits in the frame: the whole frame here. Overlays keep their
    /// coordinates in this rectangle when the frame is wider than the picture (widescreen).
    pub logical: Rect,
}

impl Drop for HdFrame {
    /// The picture's bytes go back to the pool the next picture is made in (see [`crate::pool`]).
    fn drop(&mut self) {
        crate::pool::BYTES.give(std::mem::take(&mut self.rgba));
    }
}

/// The Mode 7 state of one line: the entry of the log it belongs to, the plane's parameters, the
/// CGRAM it reads and the VRAM copy it reads (`None`: the log's own VRAM).
#[derive(Clone, Copy)]
struct State {
    entry: usize,
    plane: Plane,
    cgram: [u16; 256],
    vram: Option<usize>,
}

/// What the picture needs of one pixel of the 1x frame.
#[derive(Clone, Copy)]
struct Record {
    /// 1 for BG1, 2 for BG2 (EXTBG), when a Mode 7 plane is the top layer; 0 otherwise.
    plane: u8,
    /// The index of the pixel's line in [`Capture::states`]; meaningful when `plane` is not 0.
    state: u32,
    plan: Plan,
}

impl Record {
    const NONE: Self = Self { plane: 0, state: 0, plan: Plan { force_black: false, math: None } };
}

/// What the 1x renderer records while it draws a frame for the Enhanced picture. It keeps its
/// buffers from one frame to the next.
#[derive(Default)]
pub(crate) struct Capture {
    /// One record per pixel of the frame, row by row.
    records: Vec<Record>,
    states: Vec<State>,
    /// Copies of VRAM, for the frames that write it; the first `copies` are in use.
    vram: Vec<Vec<u16>>,
    copies: usize,
    /// The copy that holds the VRAM the lines being drawn read, while it is current.
    current: Option<usize>,
    /// The frame writes no VRAM, so every line reads the log's VRAM.
    shared_vram: bool,
}

impl Capture {
    fn begin(&mut self, log: &FrameLog) {
        self.records.clear();
        self.records.resize(usize::from(log.height) * WIDTH, Record::NONE);
        self.states.clear();
        self.copies = 0;
        self.current = None;
        self.shared_vram = log.vram_writes.is_empty();
    }

    /// The VRAM changed: the next Mode 7 line copies it again.
    pub(crate) fn vram_changed(&mut self) {
        self.current = None;
    }

    /// Records the state of a Mode 7 line and returns its index.
    fn push_state(&mut self, entry: usize, plane: Plane, cgram: &[u16; 256], vram: &[u16]) -> u32 {
        let vram = if self.shared_vram {
            None
        } else if let Some(copy) = self.current {
            Some(copy)
        } else {
            let copy = self.copies;
            self.copies += 1;
            if copy == self.vram.len() {
                self.vram.push(vram.to_vec());
            } else {
                self.vram[copy].copy_from_slice(vram);
            }
            self.current = Some(copy);
            Some(copy)
        };
        self.states.push(State { entry, plane, cgram: *cgram, vram });
        (self.states.len() - 1) as u32
    }
}

impl Renderer {
    /// Records, for the pixels `from..` of line `line` drawn by entry `index` of the log, what the
    /// Enhanced picture needs (see [`Capture`]). Called after the line is drawn, blank or not: a
    /// blank line has no plane.
    pub(crate) fn capture_line(
        &mut self,
        index: usize,
        line: usize,
        from: usize,
        regs: &LineRegs,
        vram: &[u16],
    ) {
        let mut capture = std::mem::take(&mut self.capture);
        let blank = regs.forced_blanking;
        let state = (regs.bg_mode == 7 && !blank)
            .then(|| capture.push_state(index, Plane::of(regs), &self.cgram, vram));
        for x in from..WIDTH {
            let plane = if blank { 0 } else { self.plane_layer(regs, x) };
            capture.records[(line - 1) * WIDTH + x] = if plane == 0 {
                Record::NONE
            } else {
                Record { plane, state: state.unwrap_or(0), plan: self.plan(regs, x) }
            };
        }
        self.capture = capture;
    }

    /// Renders the frame `log` describes as the Enhanced picture (see the module documentation).
    /// The picture of scale 1, with either look, is the frame of [`Renderer::render`].
    ///
    /// # Errors
    ///
    /// The frame uses something the renderer does not model ([`Unsupported`]).
    ///
    /// # Panics
    ///
    /// `options.scale` is not in `1..=MAX_SCALE`.
    pub fn render_hd(
        &mut self,
        log: &FrameLog,
        options: HdOptions,
    ) -> Result<HdFrame, Unsupported> {
        assert!((1..=crate::MAX_SCALE).contains(&options.scale), "the scale is 1 to 8");
        if self.art.as_ref().is_some_and(|art| art.has_art()) {
            return self.render_hd_art(log, options);
        }
        let mut capture = std::mem::take(&mut self.capture);
        capture.begin(log);
        self.capture = capture;
        self.capturing = true;
        self.begin_text(options);
        let frame = self.render_log(log, None, false);
        self.capturing = false;
        self.text_on = false;
        let capture = std::mem::take(&mut self.capture);
        let beneath = self.title_beneath(log, options);
        let picture = frame.map(|frame| {
            let mut hd = {
                let shown = self.without_text(&frame, &beneath);
                picture(shown.as_ref().unwrap_or(&frame), &capture, Some(log), options)
            };
            self.draw_text(&mut hd, &frame, options, &beneath);
            hd
        });
        self.capture = capture;
        picture
    }

    /// Starts noting the text cells of the frame about to be drawn, when the pictures draw text
    /// from a font and the scale is 2 or more.
    fn begin_text(&mut self, options: HdOptions) {
        self.text.clear();
        self.text_on = self.font.is_some() && options.scale >= 2;
    }

    /// The pictures under the title's text, one per BG layer that holds clear title glyphs: the 1x
    /// frame drawn again with only that layer off (see [`crate::font::beneath_logs`]). A fresh
    /// renderer draws them, so the frame's own state is left as it is.
    fn title_beneath(&self, log: &FrameLog, options: HdOptions) -> [Option<Frame>; 4] {
        if self.font.is_none() || options.scale < 2 {
            return [None, None, None, None];
        }
        crate::font::beneath_logs(log, self.text.cells())
            .map(|blank| blank.and_then(|blank| Renderer::new().render(&blank).ok()))
    }

    /// The 1x `frame` as the filter that enlarges it should see it: with the letters of the text
    /// the font redraws painted over by what they stand on (see [`crate::font::without_letters`]),
    /// so that no sub-pixel of an original letter is left beside the HD glyph. The frame itself
    /// when no text is redrawn.
    fn without_text(&self, frame: &Frame, beneath: &[Option<Frame>; 4]) -> Option<Frame> {
        if self.font.is_none() || self.text.cells().is_empty() {
            return None;
        }
        let beneath_pixels: [Option<&[Color]>; 4] =
            std::array::from_fn(|bg| beneath[bg].as_ref().map(|frame| &frame.pixels[..]));
        let pixels = crate::font::without_letters(
            &frame.pixels,
            frame.width,
            self.text.cells(),
            beneath_pixels,
        );
        Some(Frame { pixels, sources: Vec::new(), ..*frame })
    }

    /// Draws the text cells noted while the 1x `frame` was drawn into its picture `hd`, over the
    /// pictures under the title's text (`beneath`, per BG layer).
    fn draw_text(
        &self,
        hd: &mut HdFrame,
        frame: &Frame,
        options: HdOptions,
        beneath: &[Option<Frame>; 4],
    ) {
        if let (Some(font), true) = (&self.font, options.scale >= 2) {
            let beneath_pixels: [Option<&[Color]>; 4] =
                std::array::from_fn(|bg| beneath[bg].as_ref().map(|frame| &frame.pixels[..]));
            font.draw_with(
                &mut hd.rgba,
                hd.width as usize,
                options.scale as usize,
                self.text.cells(),
                &frame.pixels,
                frame.width,
                beneath_pixels,
            );
        }
    }

    /// [`Renderer::render_hd`] with the pack's art (see the `art` module): the frame is drawn
    /// once to see what shows where, and again without the sprites an HD pose replaces.
    fn render_hd_art(
        &mut self,
        log: &FrameLog,
        options: HdOptions,
    ) -> Result<HdFrame, Unsupported> {
        let scale = options.scale as usize;
        let height = usize::from(log.height);
        let mut art = self.art.take().expect("art is on");
        art.begin(WIDTH, height, scale);
        art.begin_bg(log);
        art.active = true;
        let mut binder = art.binder.take().expect("the art's binder");
        self.art = Some(art);
        let mut capture = std::mem::take(&mut self.capture);
        capture.begin(log);
        self.capture = capture;
        self.capturing = true;
        let before = self.obj_pixels;
        self.begin_text(options);
        let first = self.render_log(log, Some(&mut binder), false);
        self.capturing = false;
        self.text_on = false;
        let mut art = self.art.take().expect("art is on");
        art.binder = Some(binder);
        art.active = false;
        let first = match first {
            Ok(frame) => frame,
            Err(reason) => {
                self.art = Some(art);
                return Err(reason);
            }
        };
        let found = art.find_poses(log);
        let mut under = None;
        if !found.is_empty() {
            let after = self.obj_pixels;
            self.obj_pixels = before;
            self.hidden_objects = found.iter().fold(0, |all, (pose, _)| all | pose.sprites);
            let rows = art::pose_rows(&found, height);
            self.only_rows = rows;
            under = self.render_log(log, None, false).ok();
            self.only_rows = None;
            self.hidden_objects = 0;
            if let (Some(under), Some((first_row, past))) = (under.as_mut(), rows) {
                // The lines the poses do not reach are the first picture's
                for row in (0..first_row).chain(past..height) {
                    let span = row * WIDTH..(row + 1) * WIDTH;
                    under.pixels[span.clone()].copy_from_slice(&first.pixels[span]);
                }
            }
            self.obj_pixels = after;
        }
        let capture = std::mem::take(&mut self.capture);
        let beneath = self.title_beneath(log, options);
        let drawn = under.as_ref().unwrap_or(&first);
        let shown = self.without_text(drawn, &beneath);
        let mut hd = picture(shown.as_ref().unwrap_or(drawn), &capture, Some(log), options);
        self.capture = capture;
        art.compose(&mut hd, &first, &found, 0, (0, 0, WIDTH as i32, height as i32));
        self.art = Some(art);
        self.draw_text(&mut hd, &first, options, &beneath);
        Ok(hd)
    }
}

/// The picture of the 1x `frame` (drawn from `log`, recorded in `capture`) at the options.
pub(crate) fn picture(
    frame: &Frame,
    capture: &Capture,
    log: Option<&FrameLog>,
    options: HdOptions,
) -> HdFrame {
    let scale = options.scale as usize;
    let lines = frame.height;
    let wide = frame.width;
    // A crisp picture of a power of two with no plane to sample again: the Scale2x passes write the
    // picture's bytes directly
    if log.is_none() && options.look == Look::Crisp && scale.is_power_of_two() && scale > 1 {
        let rgba = crisp_rgba(&frame.pixels, wide, lines, scale);
        let (width, height) = ((wide * scale) as u32, (lines * scale) as u32);
        return HdFrame {
            rgba,
            width,
            height,
            scale: options.scale,
            logical: Rect { x: 0, y: 0, width, height },
        };
    }
    let source: Vec<u32> = frame.pixels.iter().map(|c| pack(c.r, c.g, c.b)).collect();
    let filtered = match options.look {
        Look::Crisp => crisp(&source, wide, lines, scale),
        Look::Smooth => smooth(&source, wide, lines, scale),
    };
    let width = wide * scale;
    let height = lines * scale;
    let mut rgba = crate::pool::BYTES.take(width * height * 4);
    rgba.par_chunks_mut(width * 4).enumerate().for_each(|(y, row)| {
        let line = y / scale;
        for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let planed = log.and_then(|log| {
                let record = *capture.records.get(line * wide + x / scale)?;
                plane_colour(capture, log, record, (x / scale, x % scale), (line, y % scale), scale)
            });
            let [r, g, b] = planed.unwrap_or_else(|| unpack(filtered[y * width + x]));
            *pixel = [r, g, b, 0xFF];
        }
    });
    let (width, height) = (width as u32, height as u32);
    HdFrame {
        rgba,
        width,
        height,
        scale: options.scale,
        logical: Rect { x: 0, y: 0, width, height },
    }
}

/// The colour of a Mode 7 plane pixel of the picture: `x` is the pixel of the line and the
/// sub-position of the picture inside it, `y` the line (0-based) and the sub-row. `None` where the
/// pixel is not a plane pixel, the plane is transparent there, or the line is mosaic (the look
/// draws those). At `scale` 1 it is the colour the 1x renderer drew.
fn plane_colour(
    capture: &Capture,
    log: &FrameLog,
    record: Record,
    x: (usize, usize),
    y: (usize, usize),
    scale: usize,
) -> Option<[u8; 3]> {
    if record.plane == 0 {
        return None;
    }
    let state = &capture.states[record.state as usize];
    let regs = &log.entries[state.entry].regs;
    if regs.mosaic_bg[0] {
        return None;
    }
    let vram = match state.vram {
        Some(copy) => &capture.vram[copy][..],
        None => &log.vram[..],
    };
    // The PPU's line numbers start at 1: the first picture row is scanline 1
    let scale = scale as i64;
    let byte =
        state.plane.sample(vram, scale, (x.0 as i32, x.1 as i64), ((y.0 + 1) as u16, y.1 as i64));
    let color = if record.plane == 2 { byte & 0x7F } else { byte };
    if color == 0 {
        return None;
    }
    let colour =
        if regs.direct_color { direct_color(0, color) } else { state.cgram[usize::from(color)] };
    let shown = convert(record.plan.apply(colour), regs.brightness);
    Some([shown.r, shown.g, shown.b])
}

fn pack(r: u8, g: u8, b: u8) -> u32 {
    u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
}

fn unpack(packed: u32) -> [u8; 3] {
    [(packed >> 16) as u8, (packed >> 8) as u8, packed as u8]
}

/// The crisp look: Scale2x on the power-of-two part of `scale`, then whole pixels for its odd part.
fn crisp(source: &[u32], width: usize, height: usize, scale: usize) -> Vec<u32> {
    let doublings = scale.trailing_zeros();
    let whole = scale >> doublings;
    let mut image = source.to_vec();
    let (mut w, mut h) = (width, height);
    for _ in 0..doublings {
        image = scale2x(&image, w, h);
        w *= 2;
        h *= 2;
    }
    if whole == 1 {
        return image;
    }
    let out_w = w * whole;
    let mut out = vec![0_u32; out_w * h * whole];
    out.par_chunks_mut(out_w).enumerate().for_each(|(y, row)| {
        let source_row = &image[(y / whole) * w..][..w];
        for (x, pixel) in row.iter_mut().enumerate() {
            *pixel = source_row[x / whole];
        }
    });
    out
}

/// The four pixels Scale2x makes of `centre` with its neighbours above, right, left and below
/// (upper left, upper right, lower left, lower right): a corner takes the colour of a neighbour
/// when the two neighbours on that corner agree and the pixel differs from both.
#[inline(always)]
#[allow(clippy::needless_bitwise_bool)] // (the `&` keeps the corner rules free of branches: lanes are selected)
fn epx(centre: u32, up: u32, right: u32, left: u32, down: u32) -> [u32; 4] {
    // (`&` and not `&&`: no branch, so the compiler can select lanes)
    let e0 = if (left == up) & (left != down) & (up != right) { up } else { centre };
    let e1 = if (up == right) & (up != left) & (right != down) { right } else { centre };
    let e2 = if (down == left) & (down != right) & (left != up) { left } else { centre };
    let e3 = if (right == down) & (right != up) & (down != left) { down } else { centre };
    [e0, e1, e2, e3]
}

/// The source rows one task of the Scale2x passes takes, at least and at most.
const BAND_MIN: usize = 4;
const BAND_MAX: usize = 32;

/// How many source rows one task takes: about one band for each worker of the pool.
fn band_rows(height: usize) -> usize {
    height.div_ceil(rayon::current_num_threads().max(1)).clamp(BAND_MIN, BAND_MAX)
}

/// Scale2x of the row `cur` of an image, with the rows `up` and `down` around it: the two rows
/// (`upper`, `lower`) of twice the width that it becomes. The ends of a row repeat their pixel.
/// Written so that the inside of the row, which is nearly all of it, is a loop over slices of equal
/// length with no index to check: the compiler turns it into vector code.
#[inline(always)]
fn epx_row(up: &[u32], cur: &[u32], down: &[u32], upper: &mut [u32], lower: &mut [u32]) {
    let width = cur.len();
    assert!(up.len() == width && down.len() == width);
    assert!(upper.len() == 2 * width && lower.len() == 2 * width);
    let edge = |x: usize, upper: &mut [u32], lower: &mut [u32]| {
        let left = cur[x.saturating_sub(1)];
        let right = cur[(x + 1).min(width - 1)];
        let [e0, e1, e2, e3] = epx(cur[x], up[x], right, left, down[x]);
        upper[2 * x] = e0;
        upper[2 * x + 1] = e1;
        lower[2 * x] = e2;
        lower[2 * x + 1] = e3;
    };
    if width < 3 {
        for x in 0..width {
            edge(x, upper, lower);
        }
        return;
    }
    edge(0, upper, lower);
    edge(width - 1, upper, lower);
    let inside = width - 2;
    let (left, centre, right) = (&cur[..inside], &cur[1..=inside], &cur[2..]);
    let (above, below) = (&up[1..=inside], &down[1..=inside]);
    let upper = upper[2..2 * width - 2].as_chunks_mut::<2>().0;
    let lower = lower[2..2 * width - 2].as_chunks_mut::<2>().0;
    for ((((((&l, &c), &r), &u), &d), upper), lower) in
        left.iter().zip(centre).zip(right).zip(above).zip(below).zip(upper).zip(lower)
    {
        let [e0, e1, e2, e3] = epx(c, u, r, l, d);
        *upper = [e0, e1];
        *lower = [e2, e3];
    }
}

/// The rows around source row `y` (above, the row, below), the edges repeating their row.
#[inline(always)]
fn rows3(source: &[u32], width: usize, height: usize, y: usize) -> (&[u32], &[u32], &[u32]) {
    (
        &source[y.saturating_sub(1) * width..][..width],
        &source[y * width..][..width],
        &source[(y + 1).min(height - 1) * width..][..width],
    )
}

/// Scale2x (EPX) of a `width` x `height` image: each pixel becomes a 2 x 2 block whose corners
/// take the colour of a neighbour when the two neighbours on that corner agree and the pixel
/// differs from both.
fn scale2x(source: &[u32], width: usize, height: usize) -> Vec<u32> {
    let mut out = vec![0_u32; 4 * width * height];
    scale2x_into(source, width, height, &mut out);
    out
}

/// [`scale2x`] written into `out`, `4 * width * height` words.
fn scale2x_into(source: &[u32], width: usize, height: usize, out: &mut [u32]) {
    let band = band_rows(height);
    out.par_chunks_mut(4 * width * band).enumerate().for_each(|(n, rows)| {
        for (k, rows) in rows.chunks_exact_mut(4 * width).enumerate() {
            let (upper, lower) = rows.split_at_mut(2 * width);
            let (up, cur, down) = rows3(source, width, height, n * band + k);
            epx_row(up, cur, down, upper, lower);
        }
    });
}

/// Two Scale2x passes at once, a 4x picture of the `width` x `height` image `source`, written into
/// `out` (`16 * width * height` words). Each task takes a band of source rows and makes into a
/// scratch it keeps the rows of the first pass that its second pass reads (one above the band's
/// own and one below, so that the second pass sees exactly the rows of the whole first picture),
/// then writes the second pass's rows straight to `out`: the first picture, a megabyte or more, is
/// never written out.
fn scale4_into(source: &[u32], width: usize, height: usize, out: &mut [u32]) {
    let band = band_rows(height);
    let (mid_width, mid_height) = (2 * width, 2 * height);
    out.par_chunks_mut(16 * width * band).enumerate().for_each_init(Vec::new, |mid, (n, rows)| {
        let (first, past) = (n * band, (n * band + band).min(height));
        // The rows of the first picture the band's second pass reads: from the one above its first
        // row to the one below its last, inside the picture
        let needed_from = (2 * first).saturating_sub(1);
        let needed_to = (2 * past).min(mid_height - 1);
        // They are made by these source rows, each making a pair of rows
        let (source_from, source_to) = (needed_from / 2, needed_to / 2);
        let base = 2 * source_from;
        mid.clear();
        mid.resize(2 * (source_to - source_from + 1) * mid_width, 0);
        for (k, pair) in mid.chunks_exact_mut(2 * mid_width).enumerate() {
            let (upper, lower) = pair.split_at_mut(mid_width);
            let (up, cur, down) = rows3(source, width, height, source_from + k);
            epx_row(up, cur, down, upper, lower);
        }
        let mid_row = |row: usize| &mid[(row - base) * mid_width..][..mid_width];
        for (k, pair) in rows.chunks_exact_mut(4 * mid_width).enumerate() {
            let row = 2 * first + k;
            let (upper, lower) = pair.split_at_mut(2 * mid_width);
            epx_row(
                mid_row(row.saturating_sub(1)),
                mid_row(row),
                mid_row((row + 1).min(mid_height - 1)),
                upper,
                lower,
            );
        }
    });
}

/// The word of the RGBA bytes `[r, g, b, 255]` of a colour, as they are in memory.
#[inline(always)]
fn rgba_word(colour: Color) -> u32 {
    u32::from_ne_bytes([colour.r, colour.g, colour.b, 0xFF])
}

/// The crisp look of a power-of-two `scale` above 1, as RGBA bytes (alpha 255): Scale2x on the
/// colours as words of their bytes (the filter only compares colours, so any one-to-one packing
/// gives the same picture), the last passes written straight into the picture's bytes. The bytes
/// are a buffer of [`crate::pool::BYTES`].
fn crisp_rgba(pixels: &[Color], width: usize, height: usize, scale: usize) -> Vec<u8> {
    let doublings = scale.trailing_zeros();
    let mut image = crate::pool::WORDS.take(width * height);
    for (word, &colour) in image.iter_mut().zip(pixels) {
        *word = rgba_word(colour);
    }
    let (mut w, mut h) = (width, height);
    // All the passes but the last two (or one) make a picture of their own
    for _ in 0..doublings.saturating_sub(2) {
        let mut next = crate::pool::WORDS.take(4 * w * h);
        scale2x_into(&image, w, h, &mut next);
        crate::pool::WORDS.give(std::mem::replace(&mut image, next));
        w *= 2;
        h *= 2;
    }
    let mut rgba = crate::pool::BYTES.take(4 * width * scale * height * scale);
    let last = |image: &[u32], words: &mut [u32]| {
        if doublings == 1 {
            scale2x_into(image, w, h, words);
        } else {
            scale4_into(image, w, h, words);
        }
    };
    match bytemuck::try_cast_slice_mut::<u8, u32>(&mut rgba) {
        Ok(words) => last(&image, words),
        // (a buffer whose address is not a multiple of four: the allocator never gives one)
        Err(_) => {
            let mut words = vec![0_u32; rgba.len() / 4];
            last(&image, &mut words);
            for (bytes, word) in rgba.as_chunks_mut::<4>().0.iter_mut().zip(words) {
                *bytes = word.to_ne_bytes();
            }
        }
    }
    crate::pool::WORDS.give(image);
    rgba
}

/// The Catmull-Rom weights of the four taps around a picture pixel, in whole units, and the index
/// of the first tap. The picture pixel `index` of `scale` centres its source position at
/// `(2 * index + 1 - scale) / (2 * scale)` source pixels; the weights sum to `2 * d^3` with `d` the
/// denominator `2 * scale`.
fn taps(index: usize, scale: i64) -> (isize, [i64; 4]) {
    let d = 2 * scale;
    let t = 2 * index as i64 + 1 - scale;
    let first = t.div_euclid(d);
    let m = t.rem_euclid(d);
    let (m2, m3) = (m * m, m * m * m);
    let weights = [
        -m3 + 2 * m2 * d - m * d * d,
        3 * m3 - 5 * m2 * d + 2 * d * d * d,
        -3 * m3 + 4 * m2 * d + m * d * d,
        m3 - m2 * d,
    ];
    ((first - 1) as isize, weights)
}

/// The smooth look: a separable Catmull-Rom filter, the rows first and then the columns, in
/// integer arithmetic with the weights of [`taps`] and a rounded division at the end.
fn smooth(source: &[u32], width: usize, height: usize, scale: usize) -> Vec<u32> {
    let scale = scale as i64;
    let out_w = width * scale as usize;
    let out_h = height * scale as usize;
    let whole = 2 * (2 * scale).pow(3);
    let denominator = whole * whole;
    let columns: Vec<(isize, [i64; 4])> = (0..out_w).map(|x| taps(x, scale)).collect();
    let rows: Vec<(isize, [i64; 4])> = (0..out_h).map(|y| taps(y, scale)).collect();
    let clamp = |index: isize, size: usize| index.clamp(0, size as isize - 1) as usize;

    // The rows, filtered across: units of `whole`
    let mut across = vec![[0_i64; 3]; height * out_w];
    across.par_chunks_mut(out_w).enumerate().for_each(|(y, row)| {
        let source_row = &source[y * width..][..width];
        for (x, sample) in row.iter_mut().enumerate() {
            let (first, weights) = columns[x];
            let mut sum = [0_i64; 3];
            for (k, &weight) in weights.iter().enumerate() {
                let packed = source_row[clamp(first + k as isize, width)];
                let [r, g, b] = unpack(packed);
                for (total, channel) in sum.iter_mut().zip([r, g, b]) {
                    *total += weight * i64::from(channel);
                }
            }
            *sample = sum;
        }
    });

    let mut out = vec![0_u32; out_w * out_h];
    out.par_chunks_mut(out_w).enumerate().for_each(|(y, row)| {
        let (first, weights) = rows[y];
        for (x, pixel) in row.iter_mut().enumerate() {
            let mut sum = [0_i64; 3];
            for (k, &weight) in weights.iter().enumerate() {
                let sample = across[clamp(first + k as isize, height) * out_w + x];
                for (total, value) in sum.iter_mut().zip(sample) {
                    *total += weight * value;
                }
            }
            let [r, g, b] = sum.map(|value| {
                ((value + denominator / 2).div_euclid(denominator)).clamp(0, 255) as u8
            });
            *pixel = pack(r, g, b);
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_SCALE;
    use ssd_oracle::{LineEntry, SpriteLine};

    /// A frame log of 224 lines with the same registers on each and the given VRAM, empty OAM.
    fn mode_7_log(regs: LineRegs, vram: Vec<u16>, cgram: &[u16; 256]) -> FrameLog {
        let entries = (1..=224)
            .map(|scanline| LineEntry {
                scanline,
                from_pixel: 0,
                regs,
                sprites: SpriteLine::default(),
            })
            .collect();
        FrameLog {
            complete: true,
            height: 224,
            h_hi_res: false,
            v_hi_res: false,
            entries,
            cgram_start: cgram.to_vec(),
            cgram_writes: Vec::new(),
            vram,
            vram_writes: Vec::new(),
            oam_low: vec![0; 512],
            oam_high: vec![0; 32],
            oam_writes: Vec::new(),
        }
    }

    /// Mode 7, BG1 only, the identity matrix at `matrix` (8.8 fixed point), brightness full.
    fn plane_regs(matrix: u16) -> LineRegs {
        LineRegs {
            brightness: 15,
            bg_mode: 7,
            main_bg: [true, false, false, false],
            m7_a: matrix,
            m7_d: matrix,
            ..LineRegs::default()
        }
    }

    /// A plane whose row 0 of tile 0 has colour `(column % 8) + 1` and whose tile map is zero.
    fn stripes() -> Vec<u16> {
        let mut vram = vec![0_u16; 32768];
        for (column, word) in vram.iter_mut().take(8).enumerate() {
            *word = ((column as u16) + 1) << 8;
        }
        vram
    }

    fn cgram_identity() -> [u16; 256] {
        let mut cgram = [0_u16; 256];
        for (index, entry) in cgram.iter_mut().enumerate() {
            *entry = index as u16;
        }
        cgram
    }

    fn rgb(pixel: &[u8]) -> [u8; 3] {
        [pixel[0], pixel[1], pixel[2]]
    }

    #[test]
    fn scale_one_is_the_ppu_frame_for_both_looks_and_for_a_plane() {
        let log = mode_7_log(plane_regs(0x0080), stripes(), &cgram_identity());
        let mut renderer = Renderer::new();
        let frame = renderer.render(&log).unwrap();
        for look in [Look::Crisp, Look::Smooth] {
            let hd = renderer.render_hd(&log, HdOptions { scale: 1, look }).unwrap();
            assert_eq!((hd.width, hd.height, hd.scale), (256, 224, 1));
            for (pixel, expected) in hd.rgba.as_chunks::<4>().0.iter().zip(&frame.pixels) {
                assert_eq!(rgb(pixel), [expected.r, expected.g, expected.b], "{look:?}");
            }
        }
    }

    #[test]
    fn a_plane_is_sampled_at_the_picture_resolution() {
        // The matrix halves the plane: each screen pixel covers half a plane column, so the
        // plane's column is the picture's x divided by 8 at scale 4
        let log = mode_7_log(plane_regs(0x0080), stripes(), &cgram_identity());
        let hd =
            Renderer::new().render_hd(&log, HdOptions { scale: 4, look: Look::Crisp }).unwrap();
        assert_eq!((hd.width, hd.height), (1024, 896));
        for x in 0..1024_usize {
            let column = (x / 8) % 8;
            let expected = convert(column as u16 + 1, 15);
            let pixel = &hd.rgba[x * 4..x * 4 + 3];
            assert_eq!(rgb(pixel), [expected.r, expected.g, expected.b], "x {x}");
        }
    }

    #[test]
    fn crisp_is_scale2x_on_a_checker_diagonal() {
        // The top-left block takes the top-left pixel's colour, its corner the colour of the
        // pixel to its right, its lower-left corner stays; the top-right block takes the left
        let source = [1, 0, 0, 1];
        let out = scale2x(&source, 2, 2);
        let top_left = [out[0], out[1], out[4], out[5]];
        let top_right = [out[2], out[3], out[6], out[7]];
        assert_eq!(top_left, [1, 1, 1, 0]);
        assert_eq!(top_right, [0, 0, 1, 0]);
    }

    #[test]
    fn odd_scales_are_whole_pixels_and_the_power_of_two_part_is_smoothed() {
        let source = [1, 2, 3, 4];
        for scale in [3_usize, 5, 7] {
            let out = crisp(&source, 2, 2, scale);
            let width = 2 * scale;
            for y in (0..2 * scale).step_by(scale) {
                for x in (0..width).step_by(scale) {
                    let block = out[y * width + x];
                    for dy in 0..scale {
                        for dx in 0..scale {
                            assert_eq!(out[(y + dy) * width + x + dx], block, "scale {scale}");
                        }
                    }
                }
            }
        }
        // Scale 6: Scale2x, then whole pixels of 3
        let six = crisp(&source, 2, 2, 6);
        assert_eq!(six.len(), 12 * 12);
        assert_eq!(six[0], scale2x(&source, 2, 2)[0]);
        assert!(six[..3].iter().all(|&p| p == six[0]));
    }

    /// A small deterministic generator for the equality tests.
    struct Random(u64);

    impl Random {
        fn next(&mut self) -> u32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 16) as u32
        }
    }

    /// The corner rules of the first version of the code, with the branches.
    fn epx_original(centre: u32, up: u32, right: u32, left: u32, down: u32) -> [u32; 4] {
        let (mut e0, mut e1, mut e2, mut e3) = (centre, centre, centre, centre);
        if left == up && left != down && up != right {
            e0 = up;
        }
        if up == right && up != left && right != down {
            e1 = right;
        }
        if down == left && down != right && left != up {
            e2 = left;
        }
        if right == down && right != up && down != left {
            e3 = down;
        }
        [e0, e1, e2, e3]
    }

    /// The Scale2x of the first version of the code: one serial pass, every neighbour looked up
    /// with its index clamped.
    fn scale2x_reference(source: &[u32], width: usize, height: usize) -> Vec<u32> {
        let mut out = vec![0_u32; 4 * width * height];
        for y in 0..height {
            for x in 0..width {
                let at = |x: usize, y: usize| source[y * width + x];
                let [e0, e1, e2, e3] = epx_original(
                    at(x, y),
                    at(x, y.saturating_sub(1)),
                    at((x + 1).min(width - 1), y),
                    at(x.saturating_sub(1), y),
                    at(x, (y + 1).min(height - 1)),
                );
                out[2 * y * 2 * width + 2 * x] = e0;
                out[2 * y * 2 * width + 2 * x + 1] = e1;
                out[(2 * y + 1) * 2 * width + 2 * x] = e2;
                out[(2 * y + 1) * 2 * width + 2 * x + 1] = e3;
            }
        }
        out
    }

    /// The first version of the crisp picture of a power-of-two scale: the colours packed as
    /// `0xRRGGBB`, Scale2x for every doubling but the last, which unpacks into RGBA bytes.
    fn crisp_rgba_reference(
        pixels: &[Color],
        width: usize,
        height: usize,
        scale: usize,
    ) -> Vec<u8> {
        let mut image: Vec<u32> = pixels.iter().map(|c| pack(c.r, c.g, c.b)).collect();
        let (mut across, mut down) = (width, height);
        for _ in 1..scale.trailing_zeros() {
            image = scale2x_reference(&image, across, down);
            across *= 2;
            down *= 2;
        }
        let last = scale2x_reference(&image, across, down);
        last.iter()
            .flat_map(|&packed| {
                let [r, g, b] = unpack(packed);
                [r, g, b, 0xFF]
            })
            .collect()
    }

    /// An image of `colours` distinct colours, in runs and noise, so every Scale2x condition comes up.
    fn random_pixels(random: &mut Random, count: usize, colours: u32) -> Vec<Color> {
        let palette: Vec<Color> = (0..colours)
            .map(|_| Color {
                r: random.next() as u8,
                g: random.next() as u8,
                b: random.next() as u8,
                a: random.next() as u8,
            })
            .collect();
        (0..count).map(|_| palette[(random.next() % colours) as usize]).collect()
    }

    #[test]
    fn scale2x_equals_the_serial_version_on_random_images() {
        let mut random = Random(0x9E37_79B9_7F4A_7C15);
        for (width, height) in
            [(1, 1), (1, 5), (2, 2), (2, 7), (3, 3), (4, 1), (5, 9), (17, 6), (64, 33), (372, 40)]
        {
            for colours in [2, 3, 5, 200] {
                let source: Vec<u32> =
                    (0..width * height).map(|_| random.next() % colours).collect();
                assert_eq!(
                    scale2x(&source, width, height),
                    scale2x_reference(&source, width, height),
                    "{width} x {height}, {colours} colours"
                );
            }
        }
    }

    #[test]
    fn the_crisp_picture_equals_the_first_version_at_every_power_of_two() {
        let mut random = Random(0xD1B5_4A32_D192_ED03);
        for (width, height) in
            [(1, 1), (2, 3), (3, 2), (5, 4), (16, 9), (37, 21), (96, 70), (372, 31)]
        {
            for colours in [2, 3, 6, 400] {
                let pixels = random_pixels(&mut random, width * height, colours);
                for scale in [2, 4, 8] {
                    if width * height * scale * scale > 400_000 {
                        continue;
                    }
                    let fast = crisp_rgba(&pixels, width, height, scale);
                    let reference = crisp_rgba_reference(&pixels, width, height, scale);
                    assert!(
                        fast == reference,
                        "{width} x {height}, {colours} colours, scale {scale}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_crisp_picture_is_the_same_in_a_pool_of_any_size() {
        // The buffers come back dirty: a picture made after others is the same as the first
        let mut random = Random(77);
        let pixels = random_pixels(&mut random, 372 * 24, 4);
        let first = crisp_rgba(&pixels, 372, 24, 4);
        let mut dirty = crisp_rgba(&random_pixels(&mut random, 372 * 24, 90), 372, 24, 4);
        dirty.fill(0xA5);
        crate::pool::BYTES.give(dirty);
        assert!(crisp_rgba(&pixels, 372, 24, 4) == first, "a dirty buffer changed the picture");
    }

    #[test]
    fn smooth_keeps_flat_colours_and_the_source_at_scale_one() {
        let flat = vec![pack(10, 200, 30); 6];
        assert!(smooth(&flat, 3, 2, 4).iter().all(|&p| p == pack(10, 200, 30)));
        let source = [pack(1, 2, 3), pack(4, 5, 6), pack(7, 8, 9), pack(10, 11, 12)];
        assert_eq!(smooth(&source, 2, 2, 1), source);
    }

    #[test]
    fn every_scale_renders_and_the_logical_rect_is_the_whole_picture() {
        let log = mode_7_log(plane_regs(0x0100), stripes(), &cgram_identity());
        let mut renderer = Renderer::new();
        for scale in 1..=MAX_SCALE {
            for look in [Look::Crisp, Look::Smooth] {
                let hd = renderer.render_hd(&log, HdOptions { scale, look }).unwrap();
                let (w, h) = (256 * scale, 224 * scale);
                assert_eq!((hd.width, hd.height), (w, h));
                assert_eq!(hd.rgba.len(), (w * h * 4) as usize);
                assert_eq!(hd.logical, Rect { x: 0, y: 0, width: w, height: h });
                assert!(hd.rgba.as_chunks::<4>().0.iter().all(|p| p[3] == 0xFF));
            }
        }
    }

    #[test]
    fn a_picture_of_a_frame_that_is_not_modelled_is_refused() {
        let mut log = mode_7_log(plane_regs(0x0100), stripes(), &cgram_identity());
        log.complete = false;
        let refused = Renderer::new().render_hd(&log, HdOptions { scale: 2, look: Look::Crisp });
        assert_eq!(refused, Err(Unsupported::Incomplete));
    }
}
