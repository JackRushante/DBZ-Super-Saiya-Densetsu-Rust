//! Drawing the art over the picture the look made: the pictures of portraits and faces, character
//! by character, and the poses of the battle, masked by what the original pixels showed.

use super::poses::{Pose, ROW_OF_CAPTURE};
use super::resample::Image;
use super::{ArtPixel, NO_PICTURE};
use crate::screen::Plan;
use crate::{Frame, NO_OBJECT};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use std::sync::Arc;

/// How many pixels of the original a pose's art may grow past the silhouette it was made from.
pub(crate) const GROWTH: i32 = 3;

/// The 8-bit colour of a 15-bit one, as the art's palettes are made.
fn expand(value: u16) -> [u8; 3] {
    let channel = |shift: u16| ((u32::from(value >> shift) & 31) * 255 + 15) / 31;
    [channel(0) as u8, channel(5) as u8, channel(10) as u8]
}

/// What the forced blanking, the colour math and the brightness do to an 8-bit colour: the
/// PPU's own steps, taken on the art's finer channels.
pub(crate) fn shade(plan: Plan, brightness: u8, color: [u8; 3]) -> [u8; 3] {
    let mut out = if plan.force_black { [0; 3] } else { color };
    if let Some(math) = plan.math {
        let other = expand(math.sub);
        for channel in 0..3 {
            let (a, b) = (u16::from(out[channel]), u16::from(other[channel]));
            let value = if math.subtract { a.saturating_sub(b) } else { (a + b).min(255) };
            out[channel] = (if math.divide { value >> 1 } else { value }) as u8;
        }
    }
    if brightness < 15 {
        for value in &mut out {
            *value = ((u32::from(*value) * u32::from(brightness) * 2 + 15) / 30) as u8;
        }
    }
    out
}

/// `art` over the picture pixel `base`, by the art's alpha.
fn blend(base: &mut [u8], color: [u8; 3], alpha: u8) {
    match alpha {
        0 => {}
        255 => base[..3].copy_from_slice(&color),
        alpha => {
            let (alpha, rest) = (u32::from(alpha), 255 - u32::from(alpha));
            for channel in 0..3 {
                base[channel] =
                    ((u32::from(color[channel]) * alpha + u32::from(base[channel]) * rest + 127)
                        / 255) as u8;
            }
        }
    }
}

/// Draws the pictures of the characters the frame shows: each 1x pixel with a picture is replaced
/// by the block of the picture's art it covers. `rgba` is the picture, `width` pixels wide at
/// `scale`; `pixels` has the frame's pixels, `frame_width` to a row. Returns the pixels drawn.
pub(crate) fn draw_pictures(
    rgba: &mut [u8],
    scale: usize,
    frame_width: usize,
    pixels: &[ArtPixel],
    images: &FxHashMap<(u32, u8), Arc<Image>>,
) -> u64 {
    let row_bytes = frame_width * scale * 4;
    rgba.par_chunks_mut(row_bytes * scale)
        .enumerate()
        .map(|(line, block)| {
            let mut drawn = 0_u64;
            for x in 0..frame_width {
                let pixel = pixels[line * frame_width + x];
                if pixel.picture == NO_PICTURE {
                    continue;
                }
                let Some(image) = images.get(&(pixel.picture, pixel.row)) else { continue };
                let (cell_x, cell_y) = (usize::from(pixel.cell % 4), usize::from(pixel.cell / 4));
                let (col, row) = (usize::from(pixel.pos & 7), usize::from((pixel.pos >> 3) & 7));
                let (x_flip, y_flip) = (pixel.pos & 0x40 != 0, pixel.pos & 0x80 != 0);
                for sy in 0..scale {
                    for sx in 0..scale {
                        let u = if x_flip { scale - 1 - sx } else { sx };
                        let v = if y_flip { scale - 1 - sy } else { sy };
                        let at = ((cell_y * 8 + row) * scale + v) * image.width
                            + (cell_x * 8 + col) * scale
                            + u;
                        let source = &image.rgba[at * 4..at * 4 + 4];
                        let color =
                            shade(pixel.plan, pixel.brightness, [source[0], source[1], source[2]]);
                        let target = (sy * frame_width * scale + x * scale + sx) * 4;
                        blend(&mut block[target..target + 4], color, source[3]);
                    }
                }
                drawn += 1;
            }
            drawn
        })
        .sum()
}

/// The colour the game's pixel `pix` had where the art was made: the source picture's, or (a pack
/// with none) the colour of the same CGRAM entry in the palette the art was made in.
fn reference_of(image: &crate::bgart::BgImage, pix: crate::bgart::BgPix) -> Option<[u8; 3]> {
    match (&image.source, &image.base) {
        (Some(source), _) => {
            let at = (usize::from(pix.y) * source.width + usize::from(pix.x)) * 4;
            let s = source.rgba.get(at..at + 4)?;
            (s[3] != 0).then_some([s[0], s[1], s[2]])
        }
        (None, Some(base)) if pix.index != crate::bgart::NO_INDEX => {
            let known = base[usize::from(pix.index & 255)];
            (known != crate::bgart::UNKNOWN_COLOUR).then(|| expand(known))
        }
        _ => None,
    }
}

/// Whether the pixel is drawn from the art of the background layers: a layer's pixel, the backdrop, or an object
/// that `hidden` names.
fn drawable(pixel: &ArtPixel, hidden: u128) -> bool {
    use crate::bgart::Kind;
    match pixel.bg.kind {
        Kind::Hidden | Kind::Under => false,
        Kind::Layer => true,
        Kind::Object => hidden >> (pixel.oam & 127) & 1 != 0,
    }
}

/// The pairs of slot `slot` of the frame, from the art `image` of that slot: the colour the layer's pixel had where
/// the art was made and the colour it has now, of the pixels drawn from the art (see [`crate::transfer`]).
pub(crate) fn pairs_of(
    image: &crate::bgart::BgImage,
    slot: usize,
    pixels: &[ArtPixel],
    hidden: u128,
) -> crate::transfer::Pairs {
    use crate::bgart::NO_COLOUR;
    let mut pairs = crate::transfer::Pairs::default();
    for pixel in pixels.iter().filter(|pixel| drawable(pixel, hidden)) {
        let pix = pixel.bg.pix[slot];
        if !pix.is_set() || pix.colour == NO_COLOUR {
            continue;
        }
        if let Some(reference) = reference_of(image, pix) {
            pairs.add(reference, expand(pix.colour));
        }
    }
    pairs
}

/// Draws the art of the background layers: each 1x pixel the original showed a layer (or the
/// backdrop, or an object that is `hidden`) of is replaced by the block of the art it covers, in
/// the colours the game's layer has now, with the colour math and the brightness of the pixel.
/// `images[j]` is the art of the pixel's slot `j` (far to near); `colourings[j]` says how that art's colours
/// are transferred as it is drawn (see [`crate::transfer::Colouring`]). A layer that is not `fill`ed is drawn by its
/// alpha. Returns the pixels drawn (a pixel counts once, and only when it is one of a battle background's).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_backgrounds(
    rgba: &mut [u8],
    scale: usize,
    frame_width: usize,
    pixels: &[ArtPixel],
    images: &[Option<Arc<crate::bgart::BgImage>>],
    colourings: &[crate::transfer::Colouring],
    fills: &[bool],
    hidden: u128,
) -> u64 {
    use crate::transfer::Colouring;
    let drawable_of = |pixel: &ArtPixel| drawable(pixel, hidden);
    let row_bytes = frame_width * scale * 4;
    rgba.par_chunks_mut(row_bytes * scale)
        .enumerate()
        .map(|(line, block)| {
            let mut drawn = 0_u64;
            for x in 0..frame_width {
                let pixel = pixels[line * frame_width + x];
                if !drawable_of(&pixel) {
                    continue;
                }
                let mut any = false;
                for (j, image) in images.iter().enumerate() {
                    let (pix, Some(image)) = (pixel.bg.pix[j], image) else { continue };
                    if !pix.is_set() {
                        continue;
                    }
                    let (gx, gy) = (usize::from(pix.x), usize::from(pix.y));
                    // The art picture that holds the pixel (a field map's is in tiles)
                    let Some((art, ox, oy)) = image.block(gx, gy, scale) else { continue };
                    for sy in 0..scale {
                        for sx in 0..scale {
                            // A mosaic block is flat: every point of it takes the middle of the
                            // art's block of the game pixel
                            let (ty, tx) = if pix.flat { (scale / 2, scale / 2) } else { (sy, sx) };
                            let at = ((oy + ty) * art.width + ox + tx) * 4;
                            let Some(source) = art.rgba.get(at..at + 4) else { continue };
                            let alpha = if fills[j] { 255 } else { source[3] };
                            if alpha == 0 {
                                continue;
                            }
                            let mut color = [source[0], source[1], source[2]];
                            match &colourings[j] {
                                Colouring::Same => {}
                                Colouring::Each(transfer) => color = transfer.apply(color),
                                Colouring::Numbered { colours, transferred } => {
                                    color = transferred[usize::from(colours.number[at / 4])];
                                }
                            }
                            let color = shade(pix.plan, pixel.brightness, color);
                            let target = (sy * frame_width * scale + x * scale + sx) * 4;
                            blend(&mut block[target..target + 4], color, alpha);
                            any = true;
                        }
                    }
                }
                // (the pixels that count are the ones of the layers, not those under a sprite)
                drawn += u64::from(any && pixel.bg.battle);
            }
            drawn
        })
        .sum()
}

/// What the Mode 7 plane's art is drawn from.
pub(crate) struct PlaneDraw<'a> {
    /// The matrix of each line and the plane's VRAM.
    pub frame: &'a super::PlaneFrame,
    /// The CGRAM the frame started with: the colours the plane has now.
    pub cgram: &'a [u16],
    pub art: &'a crate::mode7art::PlaneArt,
    /// Where the window starts in the frame (the plane continues past it in a wide view).
    pub x_offset: i32,
    /// The slot of the plane in the pixels' background records.
    pub slot: usize,
    /// The objects (a bit for each OAM entry) that an HD pose hides.
    pub hidden: u128,
}

/// Draws the Mode 7 plane's art: each picture pixel of a 1x pixel the plane showed is the art at the
/// place the line's matrix maps that picture pixel to (the matrix is applied to the finer
/// position, so a perspective is the original's lines, not a stretch), bilinear, in the colour the
/// game's palette has now for that place (a fade or a cycle changes the art as it changed the
/// game's pixel), with the colour math and the brightness of the pixel. Where the plane is
/// transparent at the finer position the look's pixel stays. Returns the pixels drawn.
pub(crate) fn draw_plane(
    rgba: &mut [u8],
    scale: usize,
    frame_width: usize,
    pixels: &[ArtPixel],
    draw: &PlaneDraw<'_>,
) -> u64 {
    use crate::bgart::{Kind, adapt};
    use crate::mode7art::SCALE;
    let row_bytes = frame_width * scale * 4;
    let wide = scale as i64;
    rgba.par_chunks_mut(row_bytes * scale)
        .enumerate()
        .map(|(line, block)| {
            let Some(Some(plane)) = draw.frame.lines.get(line + 1) else { return 0 };
            let step = plane.footprint(wide, SCALE as i64);
            let mut drawn = 0_u64;
            for x in 0..frame_width {
                let pixel = pixels[line * frame_width + x];
                let drawable = match pixel.bg.kind {
                    Kind::Hidden | Kind::Under => false,
                    Kind::Layer => true,
                    Kind::Object => draw.hidden >> (pixel.oam & 127) & 1 != 0,
                };
                let pix = pixel.bg.pix[draw.slot];
                if !drawable || !pix.is_set() {
                    continue;
                }
                let window_x = x as i32 - draw.x_offset;
                let mut any = false;
                for sy in 0..scale {
                    for sx in 0..scale {
                        let (map_x, map_y) = plane.map_position(
                            wide,
                            (window_x, sx as i64),
                            ((line + 1) as u16, sy as i64),
                        );
                        let (index, in_plane) =
                            plane.fetch(&draw.frame.vram, map_x >> 8, map_y >> 8);
                        // (a position the plane does not hold, as the PPU's out-of-bounds setting says
                        // it, is the look's)
                        if !in_plane {
                            continue;
                        }
                        let [r, g, b, alpha] = draw.art.sample(map_x, map_y, step);
                        if alpha == 0 {
                            continue;
                        }
                        // Where the game's pixel is transparent (colour 0) the art's soft edge still
                        // draws, over the backdrop the look drew
                        let mut color = [r, g, b];
                        if index != 0
                            && let Some(&word) = draw.cgram.get(usize::from(index))
                        {
                            color =
                                adapt(color, draw.art.palette[usize::from(index)], expand(word));
                        }
                        let color = shade(pix.plan, pixel.brightness, color);
                        let target = (sy * frame_width * scale + x * scale + sx) * 4;
                        blend(&mut block[target..target + 4], color, alpha);
                        any = true;
                    }
                }
                drawn += u64::from(any);
            }
            drawn
        })
        .sum()
}

/// What to do with one pixel of a pose's silhouette.
#[derive(Clone, Copy)]
enum Cell {
    /// Leave the picture as it is.
    Skip,
    /// Draw the art, with the colour and shading of the pixel's original.
    Draw { color: [u8; 3], plan: Plan, brightness: u8 },
}

/// The pose's pixels in the frame: which of its silhouette the original showed, and which pixels
/// next to it the art may grow into.
fn cells(
    pose: &Pose,
    frame: &Frame,
    pixels: &[ArtPixel],
    x_offset: i32,
    clip: (i32, i32, i32, i32),
) -> (Vec<Cell>, u32) {
    let (w, h) = (pose.w as i32, pose.h as i32);
    let x_offset = if pose.bg { 0 } else { x_offset };
    let (x0, y0) = (pose.org.0 + x_offset, pose.org.1 + ROW_OF_CAPTURE);
    let record = |x: i32, y: i32| -> Option<(&ArtPixel, usize)> {
        let (fx, fy) = (x0 + x, y0 + y);
        let in_frame =
            (0..frame.width as i32).contains(&fx) && (0..frame.height as i32).contains(&fy);
        let in_clip = fx >= clip.0 && fx < clip.2 && fy >= clip.1 && fy < clip.3;
        (in_frame && in_clip).then(|| {
            let at = fy as usize * frame.width + fx as usize;
            (&pixels[at], at)
        })
    };
    // The pieces that are a shadow of their own are not the art's
    let sprites = pose.sprites & !pose.excluded;
    let owned = |x: i32, y: i32| {
        if pose.bg {
            // A fighter drawn as BG tiles owns the BG pixels of its own silhouette
            let inside = x >= 0 && y >= 0 && x < w && y < h && pose.arr[(y * w + x) as usize] != 0;
            return record(x, y).filter(|(pixel, _)| inside && pixel.oam == NO_OBJECT);
        }
        record(x, y).filter(|(pixel, _)| pixel.oam != NO_OBJECT && sprites >> pixel.oam & 1 != 0)
    };
    let mut out = vec![Cell::Skip; (w * h) as usize];
    let mut visible = 0;
    for y in 0..h {
        for x in 0..w {
            let at = (y * w + x) as usize;
            if pose.arr[at] != 0 {
                if let Some((pixel, index)) = owned(x, y) {
                    let c = frame.pixels[index];
                    out[at] = Cell::Draw {
                        color: [c.r, c.g, c.b],
                        plan: pixel.plan,
                        brightness: pixel.brightness,
                    };
                    visible += 1;
                }
                continue;
            }
            // Outside the silhouette: the art grows into pixels next to what the original showed,
            // where the pose would have been in front of what is there (not for BG tiles)
            if pose.bg {
                continue;
            }
            let mut nearest: Option<(i32, (&ArtPixel, usize))> = None;
            for dy in -GROWTH..=GROWTH {
                for dx in -GROWTH..=GROWTH {
                    if let Some(found) = owned(x + dx, y + dy) {
                        let distance = dx * dx + dy * dy;
                        if nearest.is_none_or(|(best, _)| distance < best) {
                            nearest = Some((distance, found));
                        }
                    }
                }
            }
            let (Some((_, (near, near_at))), Some((here, _))) = (nearest, record(x, y)) else {
                continue;
            };
            if here.beaten_by(pose.priority) && (here.oam == NO_OBJECT || here.oam > pose.first) {
                let c = frame.pixels[near_at];
                out[at] = Cell::Draw {
                    color: [c.r, c.g, c.b],
                    plan: near.plan,
                    brightness: near.brightness,
                };
            }
        }
    }
    (out, visible)
}

/// What one pose drew, for [`draw_objects`]: its visible pixels, and (when measured) the
/// alpha-weighted sums of the colours it drew and of the colours the original showed there.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Drawn {
    pub visible: u32,
    pub weight: u64,
    pub shown: [u64; 3],
    pub original: [u64; 3],
    /// The alpha weight of the art's colours, and of the original's, by [`colour_class`].
    pub classes_shown: [u64; COLOUR_CLASSES],
    pub classes_original: [u64; COLOUR_CLASSES],
}

/// The coarse colour classes the on-model check compares: dark, white, grey, red, orange, skin,
/// brown, yellow, green, blue, purple.
pub const COLOUR_CLASSES: usize = 11;

/// The class of a colour: by lightness, saturation and hue, so shading does not move it far.
#[must_use]
pub fn colour_class(rgb: [u8; 3]) -> usize {
    let [r8, g8, b8] = rgb;
    let (max8, min8) = (r8.max(g8).max(b8), r8.min(g8).min(b8));
    let [r, g, b] = rgb.map(|c| f32::from(c) / 255.0);
    let max = f32::from(max8) / 255.0;
    let delta = f32::from(max8 - min8) / 255.0;
    let saturation = if max8 == 0 { 0.0 } else { delta / max };
    if max < 0.22 {
        return 0;
    }
    if saturation < 0.18 {
        return if max > 0.75 { 1 } else { 2 };
    }
    let hue = if max8 == min8 {
        0.0
    } else if max8 == r8 {
        60.0 * (((g - b) / delta).rem_euclid(6.0))
    } else if max8 == g8 {
        60.0 * ((b - r) / delta + 2.0)
    } else {
        60.0 * ((r - g) / delta + 4.0)
    };
    match hue {
        h if !(15.0..345.0).contains(&h) => 3,
        h if h < 45.0 => {
            if saturation < 0.62 && max > 0.7 {
                5
            } else if max < 0.55 {
                6
            } else {
                4
            }
        }
        h if h < 70.0 => {
            if max < 0.5 {
                6
            } else {
                7
            }
        }
        h if h < 170.0 => 8,
        h if h < 262.0 => 9,
        _ => 10,
    }
}

impl Drawn {
    fn add(mut self, other: Self) -> Self {
        self.weight += other.weight;
        for class in 0..COLOUR_CLASSES {
            self.classes_shown[class] += other.classes_shown[class];
            self.classes_original[class] += other.classes_original[class];
        }
        for channel in 0..3 {
            self.shown[channel] += other.shown[channel];
            self.original[channel] += other.original[channel];
        }
        self
    }
}

/// Draws each pose's art, `images[n]` for `poses[n]`, over `rgba`. `frame` is the 1x frame the
/// pixels were recorded from (with the sprites, so the colours the original gave are known), and
/// `x_offset` the x of the window in it (the wide view's margin). `clip` is the part of the frame
/// that may be drawn on: `(left, top, right, bottom)`. Returns what each pose drew; with
/// `measure` the colour sums too (they cost a little in the loop over the pixels drawn).
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_objects(
    rgba: &mut [u8],
    scale: usize,
    frame: &Frame,
    pixels: &[ArtPixel],
    poses: &[Placed<'_>],
    x_offset: i32,
    clip: (i32, i32, i32, i32),
    measure: bool,
) -> Vec<Drawn> {
    let width = frame.width * scale;
    let mut shown = Vec::with_capacity(poses.len());
    for &(pose, ref image, ref luma) in poses {
        let (grid, visible) = cells(pose, frame, pixels, x_offset, clip);
        let x_offset = if pose.bg { 0 } else { x_offset };
        let (x0, y0) = (pose.org.0 + x_offset, pose.org.1 + ROW_OF_CAPTURE);
        let (w, h) = (pose.w, pose.h);
        let rows = image.height.min(h * scale);
        let columns = image.width.min(w * scale);
        // Each row of the art is its own pixels of the picture: rows are independent
        let first = (y0 * scale as i32).max(0) as usize;
        let start = first * width * 4;
        if start >= rgba.len() {
            shown.push(Drawn { visible, ..Drawn::default() });
            continue;
        }
        let area = &mut rgba[start..];
        let skipped = (first as i32 - y0 * scale as i32).max(0) as usize;
        let sums = area
            .par_chunks_mut(width * 4)
            .take(rows.saturating_sub(skipped))
            .enumerate()
            .map(|(n, row)| {
                let mut sums = Drawn::default();
                let py = n + skipped;
                for px in 0..columns {
                    let target_x = x0 * scale as i32 + px as i32;
                    if target_x < 0 || target_x as usize >= width {
                        continue;
                    }
                    let Cell::Draw { color, plan, brightness } =
                        grid[(py / scale) * w + px / scale]
                    else {
                        continue;
                    };
                    let at = py * image.width + px;
                    let source = &image.rgba[at * 4..at * 4 + 4];
                    if source[3] == 0 {
                        continue;
                    }
                    let shown = if *luma {
                        luma_colour(color, image, at)
                    } else {
                        shade(plan, brightness, [source[0], source[1], source[2]])
                    };
                    if measure {
                        let weight = u64::from(source[3]);
                        sums.weight += weight;
                        for channel in 0..3 {
                            sums.shown[channel] += weight * u64::from(shown[channel]);
                            sums.original[channel] += weight * u64::from(color[channel]);
                        }
                        sums.classes_shown[colour_class(shown)] += weight;
                        sums.classes_original[colour_class(color)] += weight;
                    }
                    let target = target_x as usize * 4;
                    blend(&mut row[target..target + 4], shown, source[3]);
                }
                sums
            })
            .reduce(Drawn::default, Drawn::add);
        shown.push(Drawn { visible, ..sums });
    }
    shown
}

/// Draws the picture of a pose variant over `rgba`: `image` is the variant's picture at the picture's `scale`, its top left at
/// `origin` (frame coordinates, the margin included). Its opaque pixels are drawn in their own colours, dimmed only by the
/// screen's brightness: they are not the palette's (`recolour`) nor the colour math's. A pixel that shows a sprite of another
/// object is left as it is (that object is in front); the others, the pose's own sprites included, are drawn over the
/// picture, so the variant's margin shows past the sprites it grows from.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_variant(
    rgba: &mut [u8],
    scale: usize,
    frame: &Frame,
    pixels: &[ArtPixel],
    sprites: u128,
    image: &Image,
    origin: (i32, i32),
    clip: (i32, i32, i32, i32),
) {
    let width = frame.width * scale;
    let rows = rgba.len() / (width * 4);
    let plain = Plan { force_black: false, math: None };
    for py in 0..image.height {
        let y = origin.1 * scale as i32 + py as i32;
        if y < 0 || y as usize >= rows {
            continue;
        }
        for px in 0..image.width {
            let source = &image.rgba[(py * image.width + px) * 4..][..4];
            let x = origin.0 * scale as i32 + px as i32;
            if source[3] == 0 || x < 0 || x as usize >= width {
                continue;
            }
            let Some((pixel, _)) =
                at_frame(frame, pixels, clip, x / scale as i32, y / scale as i32)
            else {
                continue;
            };
            if pixel.oam != NO_OBJECT && sprites >> pixel.oam & 1 == 0 {
                continue;
            }
            let colour = shade(plain, pixel.brightness, [source[0], source[1], source[2]]);
            let at = (y as usize * width + x as usize) * 4;
            blend(&mut rgba[at..at + 4], colour, source[3]);
        }
    }
}

/// The colour of a pixel of luma art: the original's colour there, lit by the art's luminance.
fn luma_colour(original: [u8; 3], image: &Image, at: usize) -> [u8; 3] {
    let light = (image.luma(at) * 255 / u32::from(image.luma_ref.max(1))).min(255);
    [0, 1, 2].map(|c| ((u32::from(original[c]) * light + 127) / 255) as u8)
}

/// A pose with its art and whether the art is a luma shape, for [`draw_objects`].
pub(crate) type Placed<'a> = (&'a Pose, Arc<Image>, bool);

/// How far a shadow's soft edge reaches past its silhouette, in pixels of the original.
pub(super) const SHADOW_MARGIN: i32 = 1;

/// The width of a shadow ellipse's feathered edge, in pixels of the original: the alpha goes from
/// 1 to 0 across this distance, centred on the ellipse's outline.
const SHADOW_FEATHER: f32 = 0.9;

/// How much of the picture's light a shadow leaves at the least, per channel (0 to 1): a shadow
/// that was black in the original still darkens the HD picture by multiplying it, so the water's
/// ripples and the grass show through.
const SHADOW_FLOOR: f32 = 0.14;

/// How far around a shadow's box the original's picture is read to learn how dark the shadow is, in
/// pixels of the original.
const SHADOW_SURROUND: i32 = 3;

/// The fewest pixels of picture around a shadow that measure its darkness.
const SHADOW_SAMPLES: u32 = 6;

/// The smooth ellipse of a ground shadow: the box of its silhouette, in pixels of the original
/// measured from the corner of the shadow's box with its margin.
#[derive(Clone, Copy)]
pub(crate) struct Ellipse {
    centre: (f32, f32),
    radii: (f32, f32),
}

impl Ellipse {
    /// The ellipse of the box of `pose` (`margin` pixels in from the corner of the grid), when the
    /// pose's silhouette is close to it: the pixels the two agree on are at least 80 percent of the
    /// pixels either covers. A silhouette of another shape is drawn as it is.
    pub(crate) fn fitted(w: usize, h: usize, arr: &[u8], margin: i32) -> Option<Self> {
        if w == 0 || h == 0 {
            return None;
        }
        let ellipse = Self {
            centre: (margin as f32 + w as f32 / 2.0, margin as f32 + h as f32 / 2.0),
            radii: (w as f32 / 2.0, h as f32 / 2.0),
        };
        let (mut both, mut either) = (0_u32, 0_u32);
        for y in 0..h {
            for x in 0..w {
                let inside = ellipse
                    .outline(margin as f32 + x as f32 + 0.5, margin as f32 + y as f32 + 0.5)
                    <= 0.0;
                let drawn = arr[y * w + x] != 0;
                both += u32::from(inside && drawn);
                either += u32::from(inside || drawn);
            }
        }
        (either > 0 && both * 10 >= either * 8).then_some(ellipse)
    }

    /// How far the point `(x, y)` is from the outline, in pixels of the original (negative inside,
    /// to first order: the ellipse's own measure divided by its gradient).
    fn outline(&self, x: f32, y: f32) -> f32 {
        let (dx, dy) = ((x - self.centre.0) / self.radii.0, (y - self.centre.1) / self.radii.1);
        let r = dx.hypot(dy);
        if r < 1e-4 {
            return -self.radii.0.min(self.radii.1);
        }
        let gradient = (dx / self.radii.0).hypot(dy / self.radii.1) / r;
        (r - 1.0) / gradient
    }

    /// The alpha of the ellipse at every pixel of a `cols` x `rows` grid of the original enlarged
    /// `scale` times: 255 inside, 0 outside, a smooth edge of [`SHADOW_FEATHER`] on the outline.
    fn alpha(&self, cols: usize, rows: usize, scale: usize) -> Vec<u8> {
        let mut out = vec![0_u8; cols * scale * rows * scale];
        for j in 0..rows * scale {
            let y = (j as f32 + 0.5) / scale as f32;
            for i in 0..cols * scale {
                let x = (i as f32 + 0.5) / scale as f32;
                let t = (0.5 - self.outline(x, y) / SHADOW_FEATHER).clamp(0.0, 1.0);
                out[j * cols * scale + i] = (t * t * (3.0 - 2.0 * t) * 255.0 + 0.5) as u8;
            }
        }
        out
    }
}

/// The light a shadow of the colour `shadow` leaves of a picture of the colour `around` (what the
/// original showed next to it): the ratio of their luminances, never below [`SHADOW_FLOOR`]. One
/// number for the three channels, so a shadow darkens the picture without tinting it (the
/// original's shadow colour is a dark colour of its own palette, which is not a hue the HD picture
/// should take: a purple shadow on grass).
fn shadow_light(shadow: [u8; 3], around: [f32; 3]) -> f32 {
    let luma = |c: [f32; 3]| c[0] * 0.299 + c[1] * 0.587 + c[2] * 0.114;
    (luma(shadow.map(f32::from)) / luma(around).max(24.0)).clamp(SHADOW_FLOOR, 1.0)
}

/// `base` darkened towards the light `light` by `alpha`: a multiplication, so what is under the
/// shadow still shows.
fn darken(base: &mut [u8], light: f32, alpha: u8) {
    let alpha = f32::from(alpha) / 255.0;
    let factor = 1.0 - alpha * (1.0 - light);
    for value in &mut base[..3] {
        *value = (f32::from(*value) * factor + 0.5) as u8;
    }
}

/// A ground shadow to draw: the box of its silhouette with [`SHADOW_MARGIN`] pixels around it, in
/// pixels of the frame, and for each pixel of the box the colour the shadow is drawn in where it
/// may be drawn, and whether the pixel is part of the silhouette.
pub(crate) struct Shape {
    pub(crate) x0: i32,
    pub(crate) y0: i32,
    pub(crate) w: i32,
    pub(crate) h: i32,
    pub(crate) grid: Vec<Option<[u8; 3]>>,
    pub(crate) solid: Vec<f32>,
    pub(crate) smooth: Option<Ellipse>,
    pub(crate) colour: [u8; 3],
}

/// Draws the ground shadows, `shadows`, over `rgba`: each one's silhouette, drawn as a smooth
/// ellipse (see [`paint`]) when it is one. The silhouette goes under the fighters: the pixels a
/// pose of `others` (the sprites drawn as art) covers are the shadow's where its silhouette is,
/// since those poses are drawn after it. Elsewhere only the pixels the original's shadow showed
/// are drawn, and the margin beside them where nothing is in front.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_shadows(
    rgba: &mut [u8],
    scale: usize,
    frame: &Frame,
    pixels: &[ArtPixel],
    shadows: &[&Pose],
    others: u128,
    x_offset: i32,
    clip: (i32, i32, i32, i32),
) {
    for &pose in shadows {
        let (w, h) = (pose.w as i32 + 2 * SHADOW_MARGIN, pose.h as i32 + 2 * SHADOW_MARGIN);
        let (x0, y0) =
            (pose.org.0 + x_offset - SHADOW_MARGIN, pose.org.1 + ROW_OF_CAPTURE - SHADOW_MARGIN);
        let smooth = Ellipse::fitted(pose.w, pose.h, &pose.arr, SHADOW_MARGIN);
        let mut grid: Vec<Option<[u8; 3]>> = vec![None; (w * h) as usize];
        let mut solid = vec![0.0_f32; (w * h) as usize];
        let mut covered = Vec::new();
        let mut known: Option<[u8; 3]> = None;
        for y in 0..h {
            for x in 0..w {
                let at = (y * w + x) as usize;
                let Some((here, index)) = at_frame(frame, pixels, clip, x0 + x, y0 + y) else {
                    continue;
                };
                let inside = x >= SHADOW_MARGIN
                    && y >= SHADOW_MARGIN
                    && x < w - SHADOW_MARGIN
                    && y < h - SHADOW_MARGIN
                    && pose.arr[((y - SHADOW_MARGIN) * pose.w as i32 + x - SHADOW_MARGIN) as usize]
                        != 0;
                let ours = here.oam != NO_OBJECT && pose.sprites >> here.oam & 1 != 0;
                if inside && ours {
                    let c = frame.pixels[index];
                    grid[at] = Some([c.r, c.g, c.b]);
                    known.get_or_insert([c.r, c.g, c.b]);
                    solid[at] = 1.0;
                } else if (inside || smooth.is_some())
                    && here.oam != NO_OBJECT
                    && others >> here.oam & 1 != 0
                {
                    // Under a fighter drawn from art: the shadow is there, in the shadow's colour
                    covered.push(at);
                    solid[at] = f32::from(u8::from(inside));
                } else if here.oam == NO_OBJECT && here.beaten_by(pose.priority) {
                    // Beside the silhouette, over a picture it is in front of: the soft edge
                    grid[at] = Some([0; 3]);
                    covered.push(at);
                }
            }
        }
        let Some(colour) = known else { continue };
        for at in covered {
            grid[at] = Some(colour);
        }
        let shape = Shape { x0, y0, w, h, grid, solid, smooth, colour };
        paint(rgba, scale, frame, pixels, clip, &shape);
    }
}

/// The pixel of the frame at `(fx, fy)` and its index, when it is in the frame and the clip.
pub(crate) fn at_frame<'a>(
    frame: &Frame,
    pixels: &'a [ArtPixel],
    clip: (i32, i32, i32, i32),
    fx: i32,
    fy: i32,
) -> Option<(&'a ArtPixel, usize)> {
    let in_frame = (0..frame.width as i32).contains(&fx) && (0..frame.height as i32).contains(&fy);
    let in_clip = fx >= clip.0 && fx < clip.2 && fy >= clip.1 && fy < clip.3;
    (in_frame && in_clip).then(|| {
        let at = fy as usize * frame.width + fx as usize;
        (&pixels[at], at)
    })
}

/// Paints one shadow over `rgba`: through the alpha of its smooth ellipse (or, for another
/// silhouette, its own shape smoothly enlarged and blurred over about one pixel of the original,
/// so what is under it shows through the edge). An ellipse darkens the picture by multiplying
/// it with the light the shadow left in the original (measured against the original's picture
/// around it, never under [`SHADOW_FLOOR`]), so the HD picture's texture shows through it; a
/// shadow that was not darker than its surroundings, or one that has none around it to measure
/// against, is drawn in its own colour.
pub(crate) fn paint(
    rgba: &mut [u8],
    scale: usize,
    frame: &Frame,
    pixels: &[ArtPixel],
    clip: (i32, i32, i32, i32),
    shape: &Shape,
) {
    let Shape { x0, y0, w, h, grid, solid, smooth, colour } = shape;
    let (x0, y0, w, h) = (*x0, *y0, *w, *h);
    let width = frame.width * scale;
    let alpha = match smooth {
        Some(ellipse) => ellipse.alpha(w as usize, h as usize, scale),
        None => soft_alpha(solid, w as usize, h as usize, scale),
    };
    // The picture the original showed around the shadow: beside an ellipse's outline, or (a
    // silhouette of another shape) outside the silhouette and the pixel ring next to it
    let ring = |x: i32, y: i32| -> bool {
        match smooth {
            Some(ellipse) => ellipse.outline(x as f32 + 0.5, y as f32 + 0.5) >= 0.6,
            None => {
                let near = |dx: i32, dy: i32| {
                    let (nx, ny) = (x + dx, y + dy);
                    (0..w).contains(&nx)
                        && (0..h).contains(&ny)
                        && solid[(ny * w + nx) as usize] > 0.0
                };
                !(-1..=1).any(|dy| (-1..=1).any(|dx| near(dx, dy)))
            }
        }
    };
    let light = {
        let mut sum = [0_u32; 3];
        let mut count = 0_u32;
        for y in -SHADOW_SURROUND..h + SHADOW_SURROUND {
            for x in -SHADOW_SURROUND..w + SHADOW_SURROUND {
                if !ring(x, y) {
                    continue;
                }
                let Some((around, index)) = at_frame(frame, pixels, clip, x0 + x, y0 + y) else {
                    continue;
                };
                if around.oam == NO_OBJECT && around.bg.shadow == crate::bgart::NO_COLOUR {
                    let c = frame.pixels[index];
                    sum =
                        [sum[0] + u32::from(c.r), sum[1] + u32::from(c.g), sum[2] + u32::from(c.b)];
                    count += 1;
                }
            }
        }
        (count >= SHADOW_SAMPLES).then(|| sum.map(|total| total as f32 / count as f32))
    };
    let light = light.map(|around| shadow_light(*colour, around)).filter(|light| *light < 0.98);
    let (hw, hh) = (w as usize * scale, h as usize * scale);
    for j in 0..hh {
        let target_y = y0 * scale as i32 + j as i32;
        if target_y < 0 || target_y as usize >= rgba.len() / (width * 4) {
            continue;
        }
        for i in 0..hw {
            let target_x = x0 * scale as i32 + i as i32;
            if target_x < 0 || target_x as usize >= width {
                continue;
            }
            let Some(colour) = grid[(j / scale) * w as usize + i / scale] else { continue };
            let value = alpha[j * hw + i];
            if value == 0 {
                continue;
            }
            let at = (target_y as usize * width + target_x as usize) * 4;
            match light {
                Some(light) => darken(&mut rgba[at..at + 4], light, value),
                None => blend(&mut rgba[at..at + 4], colour, value),
            }
        }
    }
}

/// The alpha of a silhouette `solid` (1 inside, 0 outside, `cols` x `rows` pixels of the
/// original), enlarged `scale` times with bilinear weights and blurred over about one pixel of the
/// original, so the edge goes from 0 to 255 over that distance. `cols * scale` x `rows * scale`
/// values.
fn soft_alpha(solid: &[f32], cols: usize, rows: usize, scale: usize) -> Vec<u8> {
    let (wide, high) = (cols * scale, rows * scale);
    let at = |x: i32, y: i32| -> f32 {
        if x < 0 || y < 0 || x >= cols as i32 || y >= rows as i32 {
            0.0
        } else {
            solid[y as usize * cols + x as usize]
        }
    };
    // Bilinear: the picture pixel's centre in pixels of the original, minus the cell's centre
    let mut image = vec![0.0_f32; wide * high];
    for row in 0..high {
        let up = (row as f32 + 0.5) / scale as f32 - 0.5;
        let (y, below) = (up.floor() as i32, up - up.floor());
        for column in 0..wide {
            let left = (column as f32 + 0.5) / scale as f32 - 0.5;
            let (x, right) = (left.floor() as i32, left - left.floor());
            let top = at(x, y) * (1.0 - right) + at(x + 1, y) * right;
            let bottom = at(x, y + 1) * (1.0 - right) + at(x + 1, y + 1) * right;
            image[row * wide + column] = top * (1.0 - below) + bottom * below;
        }
    }
    // A box blur twice each way
    let radius = (scale / 2).max(1);
    let mut scratch = vec![0.0_f32; wide * high];
    for _ in 0..2 {
        box_blur(&image, &mut scratch, wide, high, radius, true);
        box_blur(&scratch, &mut image, wide, high, radius, false);
    }
    image.iter().map(|&value| (value.clamp(0.0, 1.0) * 255.0 + 0.5) as u8).collect()
}

/// A box blur of `radius` along rows (`across`) or columns, the edges counting as 0.
fn box_blur(
    from: &[f32],
    to: &mut [f32],
    width: usize,
    height: usize,
    radius: usize,
    across: bool,
) {
    let (lines, length) = if across { (height, width) } else { (width, height) };
    let at = |line: usize, n: usize| if across { line * width + n } else { n * width + line };
    let norm = 1.0 / (2 * radius + 1) as f32;
    for line in 0..lines {
        let mut sum: f32 = (0..=radius.min(length - 1)).map(|n| from[at(line, n)]).sum();
        for n in 0..length {
            to[at(line, n)] = sum * norm;
            if n + radius + 1 < length {
                sum += from[at(line, n + radius + 1)];
            }
            if n >= radius {
                sum -= from[at(line, n - radius)];
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pixels of a stepped ellipse of widths 6, 10, 10, 6 in a 10 x 4 box: what the game draws.
    fn stepped() -> Pose {
        let mut arr = Vec::new();
        for width in [6_usize, 10, 10, 6] {
            arr.extend((0..10).map(|x| u8::from(x >= (10 - width) / 2 && x < 5 + width / 2)));
        }
        Pose {
            key: 0,
            fighter: false,
            field: false,
            cut: false,
            sprites: 1,
            first: 0,
            priority: 0,
            palette: 3,
            w: 10,
            h: 4,
            arr,
            rgba: Vec::new(),
            org: (0, 0),
            shadow: true,
            excluded: 0,
            recolour: None,
            bg: false,
        }
    }

    #[test]
    fn a_stepped_ellipse_fits_a_smooth_one_of_its_box_and_other_shapes_do_not() {
        let pose = stepped();
        let ellipse = Ellipse::fitted(pose.w, pose.h, &pose.arr, 1).expect("fits");
        assert_eq!((ellipse.centre, ellipse.radii), ((6.0, 3.0), (5.0, 2.0)));
        let mut square = stepped();
        square.arr = (0..40).map(|n| u8::from(n % 10 < 2 || n / 10 == 3)).collect();
        assert!(
            Ellipse::fitted(square.w, square.h, &square.arr, 1).is_none(),
            "an L is not an ellipse"
        );
    }

    #[test]
    fn the_smooth_ellipse_is_solid_inside_clear_outside_and_goes_across_the_edge_by_steps() {
        let ellipse = {
            let p = stepped();
            Ellipse::fitted(p.w, p.h, &p.arr, 1)
        }
        .unwrap();
        let (cols, rows, scale) = (12, 6, 4);
        let alpha = ellipse.alpha(cols, rows, scale);
        let at = |x: usize, y: usize| alpha[y * cols * scale + x];
        assert_eq!(at(6 * scale, 3 * scale), 255, "the middle");
        assert_eq!(at(0, 0), 0, "the corner of the box");
        assert_eq!(at(cols * scale - 1, rows * scale - 1), 0);
        // Along the vertical through the middle the alpha falls from 255 to 0 without a jump of a
        // whole pixel of the original at once, and never rises
        let column: Vec<u8> = (0..rows * scale).map(|y| at(6 * scale, y)).collect();
        let top = &column[..column.len() / 2];
        assert!(top.windows(2).all(|pair| pair[0] <= pair[1]), "{top:?}");
        assert!(top.iter().filter(|&&v| v > 0 && v < 255).count() >= 3, "a feathered edge {top:?}");
        // Symmetric left to right
        for y in 0..rows * scale {
            assert_eq!(at(3 * scale + 1, y), at(9 * scale - 2, y), "row {y}");
        }
    }

    #[test]
    fn a_shadow_leaves_the_light_it_left_of_the_original_and_never_less_than_the_floor() {
        let around = [100.0, 200.0, 250.0];
        let light = shadow_light([50, 50, 50], around);
        assert!((light - 50.0 / 176.3).abs() < 1e-2, "{light}");
        assert!(
            (shadow_light([0; 3], around) - SHADOW_FLOOR).abs() < 1e-6,
            "a black shadow still shows"
        );
        assert!(
            (shadow_light([255; 3], around) - 1.0).abs() < 1e-6,
            "a lighter one darkens nothing"
        );
    }

    #[test]
    fn a_shadow_of_a_hue_darkens_without_tinting() {
        // The original's shadow is a dark purple; the picture around it is green grass
        let light = shadow_light([60, 20, 90], [60.0, 160.0, 30.0]);
        let mut grass = [60, 160, 30, 255];
        darken(&mut grass, light, 255);
        let before = [60.0_f32, 160.0, 30.0];
        let ratios: Vec<f32> = (0..3).map(|c| f32::from(grass[c]) / before[c]).collect();
        assert!(ratios.iter().all(|r| (r - ratios[0]).abs() < 0.03), "{grass:?}");
        assert!(grass[1] > grass[0] && grass[1] > grass[2], "still green: {grass:?}");
    }

    #[test]
    fn darkening_multiplies_what_is_under_the_shadow() {
        let light = 0.5;
        let mut water = [200, 100, 80, 255];
        darken(&mut water, light, 255);
        assert_eq!(water, [100, 50, 40, 255]);
        let mut grass = [40, 200, 20, 255];
        darken(&mut grass, light, 255);
        assert_eq!(grass, [20, 100, 10, 255], "the texture under it stays");
        let mut half = [200, 100, 80, 255];
        darken(&mut half, light, 128);
        assert!(half[0] > 100 && half[0] < 200 && half[1] > 50 && half[1] < 100, "{half:?}");
        let mut clear = [200, 100, 80, 255];
        darken(&mut clear, light, 0);
        assert_eq!(clear, [200, 100, 80, 255]);
    }
}
