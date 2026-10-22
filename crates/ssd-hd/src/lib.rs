//! The frame-end renderer of the HD route (M6):
//! re-renders a frame from what the PPU read to draw it, a [`FrameLog`] (per-scanline registers,
//! the CGRAM writes, VRAM and OAM at the end of the display), without the machine.
//!
//! The renderer only reads the log. Its 1x output with no art swapped in is the PPU's own frame,
//! pixel for pixel: it models the layers the game uses (BG modes 0 and 1, Mode 7 with its per-line
//! matrices, objects with their per-line limits, windows, color math, mosaic, brightness and
//! forced blanking). What it does not model (hi-res, interlace, the other BG modes) it refuses
//! with [`Unsupported`], so a frame is never wrong silently.
//!
//! A [`Renderer`] keeps the line buffers the PPU keeps between scanlines: the object line buffer
//! is only rebuilt when the PPU rebuilt it, so the last frame's sprites can show on the first
//! line of the next one. Feed it the frames of a run in order.
//!
//! The HD pack (`pack`, written by `xtask hd-convert`) indexes the tiles of the game by their
//! content: [`Renderer::render_bound`] says, for each pixel, the character it shows and the pack
//! tile bound to it ([`Binder`], the run rule of `binding`), which is what the art swap needs.

mod art;
pub mod art_index;
mod bg;
pub mod bgart;
mod binding;
pub mod cards;
mod enhanced;
pub mod font;
pub mod mapart;
mod menu_art;
mod mode7;
pub mod mode7art;
pub mod pack;
pub mod png;
mod screen;
mod sprites;
mod tooltip;
mod transfer;
pub mod translate;
pub mod world;

use ssd_oracle::{CgramWrite, Color, FrameLog, LineEntry, OamWrite, VramWrite};
use std::sync::Arc;

pub use art::{
    ArtSource, ArtStats, COLOUR_CLASSES, Conditions, Drawn, DrawnPose, Fate, FrameTally, Image,
    ItemKey, KindPixels, PortraitUse, PoseShape, PoseStat, TallyCount, TallyItem, TallyLayer,
    colour_class,
};
pub use binding::Binder;
pub use cards::{CardArt, Preview as CardPreview};
pub use enhanced::{HdFrame, HdOptions, Look, Rect};
pub use font::{Font, FontError};
pub use menu_art::MenuPicture;
pub use pack::{Index, TileRef};
pub use ssd_oracle::{LineRegs, SpriteLine};

/// Width of the frame the renderer produces (and of the PPU's, without hi-res).
pub const WIDTH: usize = 256;

/// The version of what the renderer draws a frame from: bump it when the art matching or the
/// drawing changes which pixels the art covers, so the results kept of earlier runs (the parts of
/// `xtask hd-tour`) are not mistaken for the new ones.
pub const RENDERER_VERSION: u32 = 11;

/// The largest scale of an Enhanced picture (see [`Renderer::render_hd`]).
pub const MAX_SCALE: u32 = 8;

/// What the renderer does not model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unsupported {
    /// The frame was not logged from its first scanline to the end of its display.
    Incomplete,
    /// A hi-res (mode 5 or 6, pseudo-hi-res) or interlaced frame.
    HiRes,
    /// A BG mode other than 0, 1 and 7.
    BgMode(u8),
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Incomplete => f.write_str("the frame was not logged from its start"),
            Self::HiRes => f.write_str("hi-res or interlaced frame"),
            Self::BgMode(mode) => write!(f, "BG mode {mode}"),
        }
    }
}

impl std::error::Error for Unsupported {}

/// A rendered frame: `width * height` pixels, row by row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<Color>,
    /// What each pixel shows, row by row, when the frame was rendered with
    /// [`Renderer::render_bound`]; empty otherwise.
    pub sources: Vec<Source>,
}

/// What a pixel of a frame shows: the layer it was drawn from, and for a character its VRAM
/// address and the pack tile it is bound to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The backdrop: nothing is drawn there, or the line is forced blank.
    Backdrop,
    /// A Mode 7 plane pixel: the plane is not made of characters.
    Plane,
    /// A BG character (modes 0 and 1) or an object tile. `address` is its VRAM word address,
    /// `bpp` its bit depth, and `tile` the pack tile bound to it, if any.
    Char { object: bool, bpp: u8, address: u16, tile: Option<TileRef> },
}

/// VRAM and OAM (low table as bytes, high table) as the entry being rendered sees them.
#[derive(Clone, Copy)]
pub(crate) struct Memories<'a> {
    pub vram: &'a [u16],
    pub oam: (&'a [u8], &'a [u8]),
}

/// One pixel of a BG or object line buffer. `char` is the VRAM word address of the character
/// the pixel comes from (0 in Mode 7).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Pixel {
    pub palette: u8,
    pub color: u8,
    pub priority: u8,
    pub char: u16,
    /// Where in its character the pixel is: column, row (both in the character's own pixels,
    /// flips applied) and the two flips, see [`pos`].
    pub pos: u8,
    /// The OAM entry an object pixel comes from; [`NO_OBJECT`] for the other layers.
    pub oam: u8,
}

/// `Pixel::oam` of a pixel that is not an object's.
pub(crate) const NO_OBJECT: u8 = 0xFF;

/// `Pixel::pos`: the column and row of the pixel in its character (0 to 7, as the character
/// stores them: the flips are already undone) and whether the picture shows it flipped.
pub(crate) fn pos(col: u8, row: u8, x_flip: bool, y_flip: bool) -> u8 {
    col | row << 3 | u8::from(x_flip) << 6 | u8::from(y_flip) << 7
}

impl Pixel {
    pub const TRANSPARENT: Self =
        Self { palette: 0, color: 0, priority: 0, char: 0, pos: 0, oam: NO_OBJECT };

    pub fn is_transparent(self) -> bool {
        self.color == 0
    }
}

/// The renderer: line buffers and the CGRAM of the entry being rendered.
pub struct Renderer {
    pub(crate) bg_pixels: [[Pixel; WIDTH]; 4],
    pub(crate) obj_pixels: [Pixel; WIDTH],
    pub(crate) cgram: [u16; 256],
    /// VRAM as the frame changes it, when it does.
    vram_work: Vec<u16>,
    /// OAM (low table as bytes, high table) as the frame changes it, when it does.
    oam_work: (Vec<u8>, Vec<u8>),
    pub(crate) main: screen::ScreenBuffer,
    pub(crate) sub: screen::ScreenBuffer,
    /// The window position of the first pixel of the rows drawn: 0 for the PPU's own frame. A
    /// chunk of the wide view sits at another place in the window, so the windows and Mode 7 are
    /// evaluated at `origin + x` (positions outside the screen's 256 pixels are in no window).
    pub(crate) origin: i32,
    /// What the Enhanced picture needs, recorded while `capturing` (see `enhanced`).
    capture: enhanced::Capture,
    capturing: bool,
    /// Objects (bit `n` for OAM entry `n`) whose pixels are left out of the object line buffer:
    /// the picture under the sprites an HD pose replaces. They still take their places in the
    /// 32 sprites and 34 tiles a line the PPU counts.
    pub(crate) hidden_objects: u128,
    /// The frame rows (first, past the last) the picture under the poses is drawn for; the other
    /// lines are left black (see [`art::pose_rows`]). `None` draws every line.
    pub(crate) only_rows: Option<(usize, usize)>,
    /// The pack's art, when the pictures draw it (see [`Renderer::set_art`]).
    pub(crate) art: Option<Box<art::ArtState>>,
    /// The HD text font, when the pictures draw their text from it (see [`Renderer::set_font`]).
    font: Option<Arc<Font>>,
    /// The text cells of the frame being drawn, noted while `text_on`.
    text: font::TextCapture,
    text_on: bool,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    #[must_use]
    pub fn new() -> Self {
        Self {
            bg_pixels: [[Pixel::TRANSPARENT; WIDTH]; 4],
            obj_pixels: [Pixel::TRANSPARENT; WIDTH],
            cgram: [0; 256],
            vram_work: Vec::new(),
            oam_work: (Vec::new(), Vec::new()),
            main: screen::ScreenBuffer::new(),
            sub: screen::ScreenBuffer::new(),
            origin: 0,
            capture: enhanced::Capture::default(),
            capturing: false,
            hidden_objects: 0,
            only_rows: None,
            art: None,
            font: None,
            text: font::TextCapture::default(),
            text_on: false,
        }
    }

    /// Makes the Enhanced pictures of a scale of 2 or more ([`Renderer::render_hd`]) draw the
    /// game's UI text from `font` (see [`font`]); `None` (the start) leaves the text as the look
    /// draws it. The 1x frame of [`Renderer::render`] never has HD text.
    pub fn set_font(&mut self, font: Option<Arc<Font>>) {
        self.font = font;
    }

    /// Makes the Enhanced pictures ([`Renderer::render_hd`]) draw the art of `pack`; `None` (the
    /// start) draws none. The 1x frame of [`Renderer::render`] never has art.
    pub fn set_art(&mut self, pack: Option<Arc<pack::Pack>>) {
        self.art = pack.map(|pack| Box::new(art::ArtState::new(pack)));
    }

    /// Makes the pictures with art measure the colours the poses draw against the original's
    /// ([`ArtStats::drawn`]).
    pub fn set_art_measure(&mut self, on: bool) {
        if let Some(art) = self.art.as_mut() {
            art.measure = on;
        }
    }

    /// The conditions of the frame the next picture with art is drawn from: the pack's pose variants whose condition holds
    /// are drawn ([`Conditions::of_work_ram`] reads them from the work RAM).
    pub fn set_conditions(&mut self, conditions: Conditions) {
        if let Some(art) = self.art.as_mut() {
            art.conditions = conditions;
        }
    }

    /// What the art did in the last picture drawn with it.
    #[must_use]
    pub fn art_stats(&self) -> ArtStats {
        self.art.as_ref().map(|art| art.stats.clone()).unwrap_or_default()
    }

    /// Renders the frame `log` describes, at 1x with no art swapped in.
    ///
    /// # Errors
    ///
    /// The frame uses something the renderer does not model ([`Unsupported`]).
    pub fn render(&mut self, log: &FrameLog) -> Result<Frame, Unsupported> {
        self.render_log(log, None, false)
    }

    /// [`Renderer::render`], and also what each pixel shows and the pack tile of its character
    /// ([`Frame::sources`]). The binding follows the VRAM of each line: a VRAM write during the
    /// frame rebinds from the line it applies to.
    ///
    /// # Errors
    ///
    /// As for [`Renderer::render`].
    pub fn render_bound(
        &mut self,
        log: &FrameLog,
        binder: &mut Binder<'_>,
    ) -> Result<Frame, Unsupported> {
        self.render_log(log, Some(binder), true)
    }

    fn render_log(
        &mut self,
        log: &FrameLog,
        mut binder: Option<&mut Binder<'_>>,
        sources: bool,
    ) -> Result<Frame, Unsupported> {
        if !log.complete {
            return Err(Unsupported::Incomplete);
        }
        if log.h_hi_res || log.v_hi_res {
            return Err(Unsupported::HiRes);
        }
        for entry in &log.entries {
            let regs = &entry.regs;
            if regs.interlaced || regs.pseudo_h_hi_res || regs.pseudo_obj_hi_res {
                return Err(Unsupported::HiRes);
            }
            if !matches!(regs.bg_mode, 0 | 1 | 7) {
                return Err(if regs.bg_mode == 5 || regs.bg_mode == 6 {
                    Unsupported::HiRes
                } else {
                    Unsupported::BgMode(regs.bg_mode)
                });
            }
        }

        let height = usize::from(log.height);
        let sourced = binder.is_some() && sources;
        let mut frame = Frame {
            width: WIDTH,
            height,
            pixels: vec![Color::BLACK; WIDTH * height],
            sources: if sourced { vec![Source::Backdrop; WIDTH * height] } else { Vec::new() },
        };
        self.cgram.copy_from_slice(&log.cgram_start);
        if let Some(binder) = binder.as_deref_mut() {
            binder.update(&log.vram);
        }
        // The game turns the display off and loads VRAM mid-frame: those writes apply from the
        // entry they precede, to a copy of VRAM
        let mut work = std::mem::take(&mut self.vram_work);
        if !log.vram_writes.is_empty() {
            work.clear();
            work.extend_from_slice(&log.vram);
        }
        let mut oam = std::mem::take(&mut self.oam_work);
        if !log.oam_writes.is_empty() {
            oam.0.clear();
            oam.0.extend_from_slice(&log.oam_low);
            oam.1.clear();
            oam.1.extend_from_slice(&log.oam_high);
        }
        let mut cgram_writes = log.cgram_writes.iter().peekable();
        let mut vram_writes = log.vram_writes.iter().peekable();
        let mut oam_writes = log.oam_writes.iter().peekable();
        let mut stale = false;
        for (index, entry) in log.entries.iter().enumerate() {
            while let Some(&CgramWrite { index: slot, value, .. }) =
                cgram_writes.next_if(|write| usize::from(write.entry) <= index)
            {
                self.cgram[usize::from(slot)] = value;
            }
            let mut vram_changed = false;
            while let Some(&VramWrite { address, value, .. }) =
                vram_writes.next_if(|write| usize::from(write.entry) <= index)
            {
                work[usize::from(address)] = value;
                vram_changed = true;
            }
            stale |= vram_changed;
            if vram_changed {
                self.capture.vram_changed();
            }
            while let Some(&OamWrite { high, index, value, .. }) =
                oam_writes.next_if(|write| usize::from(write.entry) <= index)
            {
                let index = usize::from(index);
                if high {
                    oam.1[index] = value as u8;
                } else {
                    oam.0[2 * index..2 * index + 2].copy_from_slice(&value.to_le_bytes());
                }
            }
            let vram = if log.vram_writes.is_empty() { &log.vram } else { &work };
            let objects = if log.oam_writes.is_empty() {
                (&log.oam_low[..], &log.oam_high[..])
            } else {
                (&oam.0[..], &oam.1[..])
            };
            // The binding is only made for a line that shows something
            if stale && !entry.regs.forced_blanking {
                if let Some(binder) = binder.as_deref_mut() {
                    binder.update(vram);
                }
                stale = false;
            }
            self.render_entry(
                Memories { vram, oam: objects },
                entry,
                index,
                &mut frame,
                binder.as_deref(),
                sourced,
            );
        }
        self.vram_work = work;
        self.oam_work = oam;
        Ok(frame)
    }

    /// Draws the entry `index` of the log: the pixels of its line from its first one on.
    fn render_entry(
        &mut self,
        memories: Memories<'_>,
        entry: &LineEntry,
        index: usize,
        frame: &mut Frame,
        binder: Option<&Binder<'_>>,
        sourced: bool,
    ) {
        let line = usize::from(entry.scanline);
        if line == 0 || line > frame.height {
            return;
        }
        let from = usize::from(entry.from_pixel);
        if from >= WIDTH {
            return;
        }
        let span = (line - 1) * WIDTH..line * WIDTH;
        let regs = &entry.regs;

        if entry.from_pixel == 0 && entry.sprites.rebuilt {
            self.build_sprite_line(&memories, &entry.sprites, 0);
        }

        if self.only_rows.is_some_and(|(first, past)| !(first..past).contains(&(line - 1))) {
            return;
        }
        if regs.forced_blanking {
            frame.pixels[span.clone()][from..].fill(Color::BLACK);
            if self.capturing {
                self.capture_line(index, line, from, regs, memories.vram);
            }
            return;
        }
        self.render_bg_layers(memories.vram, regs, entry.scanline, from);
        self.render_scanline(regs, &mut frame.pixels[span.clone()], from);
        if self.capturing {
            self.capture_line(index, line, from, regs, memories.vram);
        }
        if let (true, Some(binder)) = (sourced, binder) {
            self.sources_of_line(regs, binder, &mut frame.sources[span], from);
        }
        self.capture_art(regs, binder, line, from);
        if self.text_on {
            self.capture_text(regs, memories.vram, line, from);
        }
    }

    /// Notes the text cells of the line just drawn (see [`font`]).
    fn capture_text(&mut self, regs: &LineRegs, vram: &[u16], line: usize, from: usize) {
        let Some(font) = self.font.clone() else { return };
        let mut text = std::mem::take(&mut self.text);
        text.new_entry();
        let slots = self.art.as_ref().map_or(&[][..], |art| art.bg_slots());
        for x in from..WIDTH {
            if let Some(bg) = bg::of_layer(self.main.rendered[x].layer)
                && bgart::covers(slots, regs, bg, self.origin + x as i32, line)
            {
                continue;
            }
            text.note(
                &font,
                line - 1,
                x as i32,
                &self.main.rendered[x],
                || self.plan(regs, x),
                regs,
                &self.cgram,
                vram,
            );
        }
        self.text = text;
    }

    /// Records what the art passes need of the line just drawn, when they are on.
    fn capture_art(
        &mut self,
        regs: &LineRegs,
        binder: Option<&Binder<'_>>,
        line: usize,
        from: usize,
    ) {
        if let (Some(binder), true) = (binder, self.art.as_ref().is_some_and(|art| art.active)) {
            let mut art = self.art.take().expect("art is on");
            art.capture_line(self, regs, binder, line, from);
            self.art = Some(art);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame log of 224 lines with the same registers on each, empty memories.
    fn log(regs: LineRegs, sprites: SpriteLine) -> FrameLog {
        let entries = (1..=224)
            .map(|scanline| LineEntry { scanline, from_pixel: 0, regs, sprites })
            .collect();
        FrameLog {
            complete: true,
            height: 224,
            h_hi_res: false,
            v_hi_res: false,
            entries,
            cgram_start: vec![0; 256],
            cgram_writes: Vec::new(),
            vram: vec![0; 32768],
            vram_writes: Vec::new(),
            oam_low: vec![0; 512],
            oam_high: vec![0; 32],
            oam_writes: Vec::new(),
        }
    }

    fn mode_1() -> LineRegs {
        LineRegs {
            brightness: 15,
            bg_mode: 1,
            color_math: 3,
            bg_map_base: [0x0400, 0, 0, 0],
            bg_char_base: [0x1000, 0, 0, 0],
            main_bg: [true, false, false, false],
            ..LineRegs::default()
        }
    }

    fn rgb(color: Color) -> [u8; 3] {
        [color.r, color.g, color.b]
    }

    #[test]
    fn the_backdrop_through_brightness() {
        let mut log = log(mode_1(), SpriteLine::default());
        log.cgram_start[0] = 0x7FFF;
        let mut renderer = Renderer::new();
        let frame = renderer.render(&log).unwrap();
        assert_eq!((frame.width, frame.height), (256, 224));
        assert!(frame.pixels.iter().all(|&p| rgb(p) == [255; 3]));

        for entry in &mut log.entries {
            entry.regs.brightness = 1;
        }
        let frame = renderer.render(&log).unwrap();
        // round(1 / 15 * 31 * 255 / 31) = 17
        assert!(frame.pixels.iter().all(|&p| rgb(p) == [17; 3]));
    }

    #[test]
    fn forced_blanking_is_black_and_a_cgram_write_applies_from_its_line() {
        let mut log = log(mode_1(), SpriteLine::default());
        log.cgram_start[0] = 0x001F;
        log.entries[2].regs.forced_blanking = true;
        log.cgram_writes.push(ssd_oracle::CgramWrite { entry: 100, index: 0, value: 0x03E0 });
        let frame = Renderer::new().render(&log).unwrap();
        assert_eq!(rgb(frame.pixels[0]), [255, 0, 0]);
        assert_eq!(rgb(frame.pixels[2 * WIDTH + 5]), [0, 0, 0]);
        assert_eq!(rgb(frame.pixels[99 * WIDTH]), [255, 0, 0]);
        assert_eq!(rgb(frame.pixels[100 * WIDTH]), [0, 255, 0]);
    }

    #[test]
    fn the_cgram_index_of_a_pixel_is_the_entry_its_colour_comes_from() {
        use crate::screen::{Layer, resolve_color, resolve_index};
        let mut cgram = [0_u16; 256];
        for (at, word) in cgram.iter_mut().enumerate() {
            *word = (at as u16) * 3 + 1;
        }
        for mode in 0..2 {
            for layer in [Layer::Bg1, Layer::Bg2, Layer::Bg3, Layer::Bg4, Layer::Obj] {
                for palette in 0..8 {
                    for color in 1..16 {
                        let pixel =
                            Pixel { palette, color, priority: 0, char: 0, pos: 0, oam: NO_OBJECT };
                        let index = resolve_index(layer, mode, pixel);
                        assert_eq!(
                            cgram[usize::from(index)],
                            resolve_color(&cgram, layer, mode, false, pixel),
                            "{layer:?} mode {mode} palette {palette} colour {color}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn a_bg_tile_with_flips_and_the_palette_row() {
        let mut log = log(mode_1(), SpriteLine::default());
        // Tile 1 at character base 0x1000: its row 1 has pixel 0 (left) = 2, pixel 7 = 1. The
        // first frame row is scanline 1: a BG shows its row 1 there, as the PPU does
        let tile = 0x1000 + 16 + 1;
        log.vram[tile] = 0b1000_0000_0000_0000 | 0b0000_0001; // plane 0 bit 7, plane 1 bit 0
        // Map entry at (0, 0): tile 1, palette 2, h-flip
        log.vram[0x0400] = 1 | (2 << 10) | 0x4000;
        log.cgram_start[(2 << 4) | 1] = 0x001F;
        log.cgram_start[(2 << 4) | 2] = 0x7C00;
        let frame = Renderer::new().render(&log).unwrap();
        // Flipped: pixel 0 shows the old pixel 7, which is colour 1
        assert_eq!(rgb(frame.pixels[0]), [255, 0, 0]);
        assert_eq!(rgb(frame.pixels[7]), [0, 0, 255]);
        // The next row of the tile is empty: transparent over the backdrop (black)
        assert_eq!(rgb(frame.pixels[WIDTH]), [0, 0, 0]);
    }

    #[test]
    fn sprites_are_limited_to_thirty_two_a_line_and_the_first_is_on_top() {
        let mut regs = mode_1();
        regs.main_bg = [false; 4];
        regs.main_obj = true;
        let sprites = SpriteLine { eval_line: 0, rebuilt: true, ..SpriteLine::default() };
        let mut log = log(regs, sprites);
        // The second line draws from a line buffer rebuilt with no sprite
        log.entries[1].sprites = SpriteLine { blank: true, rebuilt: true, ..sprites };
        // 40 8x8 sprites at y = 0 (a row at scanline 0), x = 2 * n, palette n % 8 + 1... colour 1
        for n in 0..40_usize {
            log.oam_low[n * 4] = (n * 6) as u8;
            log.oam_low[n * 4 + 1] = 0;
            log.oam_low[n * 4 + 2] = 0;
            log.oam_low[n * 4 + 3] = 0;
        }
        for n in 40..128_usize {
            log.oam_low[n * 4 + 1] = 0xF0;
        }
        // Tile 0 of the object character base (word 0): row 0 is all colour 1
        log.vram[0] = 0x00FF;
        log.cgram_start[0x80 + 1] = 0x7FFF;
        let frame = Renderer::new().render(&log).unwrap();
        // Sprites 0..32 are drawn (x = 0 .. 8 and so on, 6 apart: overlapping), 32..40 are not
        assert_eq!(rgb(frame.pixels[0]), [255; 3]);
        assert_eq!(rgb(frame.pixels[31 * 6 + 7]), [255; 3]);
        assert_eq!(rgb(frame.pixels[31 * 6 + 8]), [0; 3]);
        assert_eq!(rgb(frame.pixels[WIDTH]), [0; 3]);
    }

    #[test]
    fn an_oam_write_during_the_frame_shows_in_the_sprites_of_the_lines_after_it() {
        let mut regs = mode_1();
        regs.main_bg = [false; 4];
        regs.main_obj = true;
        let sprites = SpriteLine { eval_line: 0, rebuilt: true, ..SpriteLine::default() };
        let mut log = log(regs, sprites);
        for n in 0..128_usize {
            log.oam_low[n * 4 + 1] = 0xF0;
        }
        log.vram[0] = 0x00FF;
        log.cgram_start[0x80 + 1] = 0x7FFF;
        let frame = Renderer::new().render(&log).unwrap();
        assert_eq!(rgb(frame.pixels[0]), [0; 3]);

        // Sprite 0 moves to the top left (word 0 holds X, then Y) before the first line
        log.oam_writes.push(ssd_oracle::OamWrite { entry: 0, high: false, index: 0, value: 0 });
        let frame = Renderer::new().render(&log).unwrap();
        assert_eq!(rgb(frame.pixels[0]), [255; 3]);
        assert_eq!(log.oam_low[1], 0xF0, "the log itself is untouched");
    }

    #[test]
    fn mode_7_with_an_identity_matrix_reads_the_plane() {
        let mut regs = mode_1();
        regs.bg_mode = 7;
        regs.m7_a = 0x0100;
        regs.m7_d = 0x0100;
        let mut log = log(regs, SpriteLine::default());
        // Plane tile (0, 0) is character 3; its pixel at row 1 (the first frame row is
        // scanline 1), column 1 is colour 9
        log.vram[0] = 3;
        log.vram[64 * 3 + 8 + 1] = 9 << 8;
        log.cgram_start[9] = 0x7FFF;
        let frame = Renderer::new().render(&log).unwrap();
        assert_eq!(rgb(frame.pixels[1]), [255; 3]);
        assert_eq!(rgb(frame.pixels[0]), [0; 3]);
    }

    #[test]
    fn a_vram_write_during_the_frame_applies_from_the_entry_after_it() {
        let mut regs = mode_1();
        regs.bg_mode = 7;
        regs.m7_a = 0x0100;
        regs.m7_d = 0x0100;
        let mut log = log(regs, SpriteLine::default());
        // The frame row r reads the plane row r + 1: colour 1 in column 0 of rows 1 to 7
        for row in 1..8 {
            log.vram[8 * row] = 1 << 8;
        }
        log.cgram_start[1] = 0x001F;
        log.cgram_start[2] = 0x03E0;
        // Plane row 3 becomes colour 2 after the second entry was rendered; plane row 2 after
        // the fourth, too late for its frame row (1)
        log.vram_writes.push(VramWrite { entry: 2, address: 8 * 3, value: 2 << 8 });
        log.vram_writes.push(VramWrite { entry: 4, address: 8 * 2, value: 2 << 8 });
        let frame = Renderer::new().render(&log).unwrap();
        let column = |row: usize| rgb(frame.pixels[row * WIDTH]);
        assert_eq!(column(0), [255, 0, 0]);
        assert_eq!(column(1), [255, 0, 0]);
        assert_eq!(column(2), [0, 255, 0]);
        // The write of plane row 2 shows from then on, and the frame log itself is untouched
        assert_eq!(log.vram[8 * 3], 1 << 8);
    }

    #[test]
    fn the_window_and_color_math() {
        let mut regs = mode_1();
        regs.main_bg = [false; 4];
        regs.color_math = 3;
        regs.backdrop_color_math = true;
        regs.fixed_color = 0x0001;
        // Window 1 over x = 10..=20 clips the color math: outside it only
        regs.window_left = [10, 0];
        regs.window_right = [20, 0];
        regs.math_window_area = [1, 0];
        regs.color_math = 1;
        let mut log = log(regs, SpriteLine::default());
        log.cgram_start[0] = 0x0002;
        let frame = Renderer::new().render(&log).unwrap();
        // Outside the window: 2 + 1 = 3 in red; inside: 2
        assert_eq!(frame.pixels[0].r, channel_for_test(3));
        assert_eq!(frame.pixels[15].r, channel_for_test(2));
    }

    fn channel_for_test(value: u16) -> u8 {
        ((u32::from(value) * 255 + 15) / 31) as u8
    }

    #[test]
    fn a_bg_pixel_names_its_character_and_the_pack_tile_bound_to_it() {
        let mut log = log(mode_1(), SpriteLine::default());
        // BG1 shows tile 1 of the 4-bit character base 0x1000 at the top left: row 1 has colour 2
        // at column 0 (plane 1), which is CGRAM entry 2
        let address = 0x1000 + 16;
        log.vram[0x0400] = 1;
        log.vram[address + 1] = 0x8000;
        log.cgram_start[2] = 0x001F;
        let key = crate::pack::word_key(&log.vram[address..address + 16]);
        let asset = crate::pack::Asset {
            id: "test/tile".to_owned(),
            bpp: 4,
            mode: crate::pack::Mode::Indexed,
            columns: 1,
            keys: vec![key],
            art: "base/test.png".to_owned(),
        };
        let index = Index::new(vec![asset]);

        let mut binder = Binder::new(&index);
        let frame = Renderer::new().render_bound(&log, &mut binder).unwrap();
        assert_eq!(frame.sources.len(), WIDTH * frame.height);
        assert_eq!(
            frame.sources[0],
            Source::Char {
                object: false,
                bpp: 4,
                address: address as u16,
                tile: Some(TileRef { asset: 0, tile: 0 }),
            }
        );
        assert_eq!(frame.sources[1], Source::Backdrop, "a transparent pixel is the backdrop");
        assert_eq!(rgb(frame.pixels[0]), [255, 0, 0]);

        // Without the tile in the pack, the same pixel is still a character, unbound
        let empty = Index::new(Vec::new());
        let frame = Renderer::new().render_bound(&log, &mut Binder::new(&empty)).unwrap();
        assert!(matches!(frame.sources[0], Source::Char { tile: None, .. }));
        assert!(Renderer::new().render(&log).unwrap().sources.is_empty(), "no sources asked");
    }

    #[test]
    fn a_frame_the_renderer_does_not_model_is_refused() {
        let mut log = log(mode_1(), SpriteLine::default());
        log.entries[7].regs.bg_mode = 5;
        assert_eq!(Renderer::new().render(&log), Err(Unsupported::HiRes));
        log.entries[7].regs.bg_mode = 3;
        assert_eq!(Renderer::new().render(&log), Err(Unsupported::BgMode(3)));
        log.complete = false;
        assert_eq!(Renderer::new().render(&log), Err(Unsupported::Incomplete));
    }
}
