//! The translated text of the Enhanced picture: while the message engine writes a string the translation has
//! (`ssd_catalogue::translation`, keyed by the engine's coordinates that `ssd_catalogue::engine` reads from work
//! RAM), the text of the original's window is painted over with the window's own colour and the translation is
//! drawn there with a Traditional Chinese font (the player's Noto Sans HK, loaded at run time), in the colour of the
//! game's letters, after the HD text pass.
//!
//! The window is the one the engine writes into: its text starts at the tile-map cell of `message_text_base`, is
//! as wide as the table's window (measured for the battle, dialogue and story windows; the page's longest line for
//! the others) and has as many lines as the original's page or the translation needs. The letters the frame shows
//! there (the [`TextCell`]s the font noted) give the window's colours and its place on the screen to the pixel. A
//! glyph is 12 pixels of the original square, the lines 16 pixels apart (two tile rows, as the engine's), the text
//! wrapped to the window's width with the basic rules of line breaking (no line starts with a closing mark, none
//! ends with an opening one). It appears as the engine writes: the share of the page's glyphs drawn is the share of
//! the original's cells written. The name and number slots take the names of the translation (the original's
//! when it has none) and the numbers the game wrote.
//!
//! The text stays a few frames after the engine's bytes stop naming it (between two pages, a frame where the
//! window flickers), as long as the window still has letters. A string the translation lacks keeps the original's.

use crate::cards::CARD_BASE;
use crate::font::TextCell;
use ab_glyph::{Font as _, FontVec, PxScale, point};
use rustc_hash::FxHashMap;
use ssd_catalogue::engine::OnScreen;
use ssd_catalogue::translation::{Slot, Translation, render};
use ssd_oracle::Color;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// The side of a glyph's em box, in pixels of the original.
const EM: f32 = 12.0;
/// From one line to the next, in pixels of the original: two tile rows, as the engine's lines.
const PITCH: i32 = 16;
/// How far the em box starts above the line's first tile row (the 12 pixels centre on the 8 of the cell and the
/// descenders under it).
const RISE: i32 = 1;
/// Where the baseline is in the em box, from its top: the ideographic em box of Noto Sans CJK.
const ASCENT: f32 = 0.88;
/// The glyph of a blank cell (the font's index 0).
const BLANK: u8 = 0;
/// Frames the text stays once the engine's bytes no longer name it, while the window still has letters.
const KEEP: u32 = 8;
/// The marks that never start a line, and those that never end one.
const NO_START: &str = "，。、！？：；」』）〕】》〉…—～·．,.!?:;)]}%";
const NO_END: &str = "「『（〔【《〈([{";

/// A Traditional Chinese font, rasterised at the picture's scale on first use of each glyph.
pub struct ChineseFace {
    face: FontVec,
    /// The glyphs drawn so far, by character, scale and em (in tenths of a pixel of the original).
    glyphs: Mutex<FxHashMap<(char, usize, u16), Arc<Glyph>>>,
}

impl std::fmt::Debug for ChineseFace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChineseFace").finish_non_exhaustive()
    }
}

/// One glyph at a scale: its coverage (`w` x `h`, from `left`, `top` of the em box's top left, in pixels of the
/// picture) and its advance in pixels of the original.
struct Glyph {
    w: usize,
    h: usize,
    left: i32,
    top: i32,
    alpha: Vec<u8>,
    advance: f32,
}

impl ChineseFace {
    /// The font of the TrueType or OpenType file `bytes`.
    ///
    /// # Errors
    ///
    /// The bytes are not a font.
    pub fn new(bytes: Vec<u8>) -> Result<Self, crate::FontError> {
        let face = FontVec::try_from_vec(bytes).map_err(|_| crate::FontError::Face)?;
        Ok(Self { face, glyphs: Mutex::new(FxHashMap::default()) })
    }

    /// The advance of `ch` in pixels of the original, at the em of the text.
    fn advance(&self, ch: char) -> f32 {
        self.advance_at(ch, EM)
    }

    /// The advance of `ch` in pixels of the original, at an em of `em` pixels of the original.
    fn advance_at(&self, ch: char, em: f32) -> f32 {
        let units = self.face.units_per_em().unwrap_or(1000.0);
        self.face.h_advance_unscaled(self.face.glyph_id(ch)) * em / units
    }

    /// The glyph of `ch` at `scale`, at the em of the text, drawn on first use.
    fn glyph(&self, ch: char, scale: usize) -> Arc<Glyph> {
        self.glyph_at(ch, scale, EM)
    }

    /// The glyph of `ch` at `scale` and an em of `em` pixels of the original, drawn on first use.
    fn glyph_at(&self, ch: char, scale: usize, em: f32) -> Arc<Glyph> {
        let tenths = (em * 10.0).round() as u16;
        let em = f32::from(tenths) / 10.0;
        let mut glyphs = self.glyphs.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let entry = glyphs.entry((ch, scale, tenths));
        Arc::clone(entry.or_insert_with(|| Arc::new(self.rasterise(ch, scale, em))))
    }

    fn rasterise(&self, ch: char, scale: usize, em: f32) -> Glyph {
        let advance = self.advance_at(ch, em);
        let empty = Glyph { w: 0, h: 0, left: 0, top: 0, alpha: Vec::new(), advance };
        let Some(units) = self.face.units_per_em() else { return empty };
        let em = em * scale as f32;
        // `PxScale` is the font's height (ascent to descent) in pixels, not its em
        let height = em * self.face.height_unscaled() / units;
        let glyph = self
            .face
            .glyph_id(ch)
            .with_scale_and_position(PxScale::from(height), point(0.0, ASCENT * em));
        let Some(outlined) = self.face.outline_glyph(glyph) else { return empty };
        let bounds = outlined.px_bounds();
        let (w, h) = (bounds.width().ceil() as usize, bounds.height().ceil() as usize);
        if w == 0 || h == 0 {
            return empty;
        }
        let mut alpha = vec![0_u8; w * h];
        outlined.draw(|x, y, coverage| {
            if let Some(cell) = alpha.get_mut(y as usize * w + x as usize) {
                *cell = (coverage.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
            }
        });
        // The strokes widened across by a quarter of an original pixel: the weight of the game's letters
        let reach = scale / 4;
        let wide = w + reach;
        let bold: Vec<u8> = (0..h * wide)
            .map(|at| {
                let (y, x) = (at / wide, at % wide);
                (x.saturating_sub(reach)..=x.min(w.saturating_sub(1)))
                    .map(|from| alpha[y * w + from])
                    .max()
                    .unwrap_or(0)
            })
            .collect();
        let (left, top) = (bounds.min.x as i32, bounds.min.y as i32);
        Glyph { w: wide, h, left, top, alpha: bold, advance }
    }
}

/// What the overlay draws from: the translation and the font.
#[derive(Debug)]
pub struct Overlay {
    table: Arc<Translation>,
    face: Arc<ChineseFace>,
    /// The names the game writes straight into its tile maps, as the screen spells them, and their translations
    /// (`ssd_catalogue::engine::screen_names`).
    names: HashMap<String, String>,
}

impl Overlay {
    #[must_use]
    pub fn new(table: Arc<Translation>, face: Arc<ChineseFace>) -> Self {
        Self { table, face, names: HashMap::new() }
    }

    /// The translation of the name `english` spells on screen: the name, or the one name it is the start of
    /// (a list cuts a long name at its column: `Piccol`) when it has four letters or more.
    fn name(&self, english: &str) -> Option<&str> {
        if let Some(name) = self.names.get(english) {
            return Some(name);
        }
        if english.chars().count() < 4 {
            return None;
        }
        let mut starting = self.names.iter().filter(|(name, _)| name.starts_with(english));
        let (_, first) = starting.next()?;
        starting.all(|(_, other)| other == first).then_some(first.as_str())
    }

    /// The overlay drawing the translations of `names` over the runs of letters that spell them.
    #[must_use]
    pub fn with_names(mut self, names: HashMap<String, String>) -> Self {
        self.names = names;
        self
    }

    /// The text of the page `screen` shows, its slots filled in; `None` when the translation lacks the page.
    fn text(&self, screen: &OnScreen) -> Option<String> {
        let text = self.table.text(&screen.key)?;
        let plain = |text: &str| render(text, |_| String::new()).replace('\n', "");
        Some(render(text, |slot| match slot {
            Slot::Name(n) => screen
                .names
                .get(usize::from(n).wrapping_sub(1))
                .and_then(Option::as_ref)
                .map(|name| self.table.text(&name.key).map_or_else(|| name.english.clone(), plain))
                .unwrap_or_default(),
            Slot::Number(n) => {
                screen.numbers.get(usize::from(n).wrapping_sub(1)).cloned().unwrap_or_default()
            }
            Slot::Phrase(n) => {
                self.table.text(&format!("phrase:{n}")).map(plain).unwrap_or_default()
            }
        }))
    }
}

/// The page being drawn: its text and where and how far the engine is.
#[derive(Debug, Clone, PartialEq)]
struct Shown {
    key: String,
    text: String,
    table: u8,
    /// `message_table` and `message_string`.
    string: (u8, u16),
    /// The tile-map cell of the text's first cell.
    cell: (i32, i32),
    portrait: bool,
    /// The original page's lines, and its longest line in cells.
    lines: usize,
    longest: usize,
    progress: f32,
    /// The engine has written some of the page: its letters show where the window's text is.
    settled: bool,
}

/// What the overlay keeps from one frame to the next.
#[derive(Debug, Default)]
pub(crate) struct Kept {
    shown: Option<Shown>,
    /// Frames since the engine last named the page shown.
    age: u32,
    /// Where the window's text is on the screen and its colours, as last seen in its letters.
    place: Option<Place>,
    colours: Option<([u8; 3], [u8; 3])>,
}

/// Where a string's text is on the screen (a window may scroll from one string to the next): the string
/// (`message_table`, `message_string`), the screen's pixel of its first line's first cell, and the pixels from one
/// line to the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Place {
    string: (u8, u16),
    at: (i32, i32),
    pitch: i32,
}

/// The overlay of a renderer: what it draws from, the page the engine named at the start of the frame being drawn,
/// and what it keeps between frames.
#[derive(Debug)]
pub(crate) struct Translator {
    pub overlay: Arc<Overlay>,
    pub screen: Option<OnScreen>,
    kept: Kept,
}

impl Translator {
    pub(crate) fn new(overlay: Arc<Overlay>) -> Self {
        Self { overlay, screen: None, kept: Kept::default() }
    }

    /// Draws the frame's translation: the names written into the tile maps ([`draw_names`]), then the message
    /// engine's page ([`draw`]). `character` spells a glyph of the cells.
    pub(crate) fn draw(
        &mut self,
        target: &mut Target<'_>,
        cells: &[TextCell],
        frame: &crate::Frame,
        window_x: i32,
        character: &dyn Fn(u8) -> Option<char>,
    ) {
        draw_names(&self.overlay, target, cells, character);
        let screen = self.screen.as_ref();
        draw(
            &self.overlay,
            &mut self.kept,
            screen,
            target,
            cells,
            &frame.pixels,
            frame.width,
            window_x,
        );
    }
}

/// The cells of the table's window, across (`docs/re/data/text-engine.md`): measured for the battle (1), dialogue
/// (2) and story (3) windows, the page's longest line for the others.
fn window_cells(table: u8, longest: usize) -> usize {
    match table {
        1 => 17,
        2 => 20,
        3 => 25,
        _ => longest.max(4),
    }
}

/// Wraps `text` (lines at its `\n`) to `width` pixels of the original, each character `advance` wide: a line breaks
/// before the character that does not fit, except a mark that never starts a line (it stays at the end, past the
/// width); an opening mark that would end a line, and a number or Latin word that would be cut, go down to the
/// next line. Spaces that would start a wrapped line are dropped.
#[must_use]
pub fn wrap(text: &str, width: f32, advance: impl Fn(char) -> f32) -> Vec<Vec<char>> {
    let mut lines = Vec::new();
    for paragraph in text.split('\n') {
        let mut line: Vec<char> = Vec::new();
        let mut used = 0.0;
        for ch in paragraph.chars() {
            let wide = advance(ch);
            if used + wide > width + 0.01 && !line.is_empty() && !NO_START.contains(ch) {
                let mut next = Vec::new();
                let latin = |c: &char| c.is_ascii_alphanumeric();
                if latin(&ch) && line.last().is_some_and(latin) {
                    // A number or a Latin word is not cut: it goes down whole, when the line has more
                    let start = line.iter().rposition(|c| !latin(c)).map_or(0, |at| at + 1);
                    if start > 0 {
                        next = line.split_off(start);
                    }
                } else if line.len() > 1 && line.last().is_some_and(|&last| NO_END.contains(last)) {
                    next.extend(line.pop());
                }
                lines.push(std::mem::take(&mut line));
                line = next;
                used = line.iter().map(|&c| advance(c)).sum();
                if ch.is_whitespace() && line.is_empty() {
                    continue;
                }
            }
            line.push(ch);
            used += wide;
        }
        lines.push(line);
    }
    lines
}

/// The most frequent of `values`.
fn most<T: Copy + Eq + std::hash::Hash>(values: impl Iterator<Item = T>) -> Option<T> {
    let mut counts: FxHashMap<T, usize> = FxHashMap::default();
    for value in values {
        *counts.entry(value).or_default() += 1;
    }
    counts.into_iter().max_by_key(|&(_, count)| count).map(|(value, _)| value)
}

/// The offset of a cell from the grid of `origin` with a pitch of `pitch` pixels, from `-pitch / 2` up.
fn off_grid(at: i32, origin: i32, pitch: i32) -> i32 {
    let off = (at - origin).rem_euclid(pitch);
    if off >= pitch / 2 { off - pitch } else { off }
}

/// The picture being drawn into: `width` pixels of `scale` times the 1x frame.
pub(crate) struct Target<'a> {
    pub rgba: &'a mut [u8],
    pub width: usize,
    pub scale: usize,
}

impl Target<'_> {
    fn height(&self) -> usize {
        self.rgba.len() / 4 / self.width
    }

    /// Fills the 1x pixels `x0..x1`, `y0..y1` with `colour`.
    fn fill(&mut self, (x0, y0): (i32, i32), (x1, y1): (i32, i32), colour: [u8; 3]) {
        let s = self.scale as i32;
        let (w, h) = (self.width as i32, self.height() as i32);
        for y in (y0 * s).max(0)..(y1 * s).min(h) {
            for x in (x0 * s).max(0)..(x1 * s).min(w) {
                let at = (y as usize * self.width + x as usize) * 4;
                self.rgba[at..at + 3].copy_from_slice(&colour);
            }
        }
    }

    /// Blends `glyph` in `ink` with its em box's top left at the picture's pixel (`x`, `y`).
    fn glyph(&mut self, glyph: &Glyph, (x, y): (i32, i32), ink: [u8; 3]) {
        let (w, h) = (self.width as i32, self.height() as i32);
        for gy in 0..glyph.h {
            let py = y + glyph.top + gy as i32;
            if !(0..h).contains(&py) {
                continue;
            }
            for gx in 0..glyph.w {
                let px = x + glyph.left + gx as i32;
                let alpha = u32::from(glyph.alpha[gy * glyph.w + gx]);
                if alpha == 0 || !(0..w).contains(&px) {
                    continue;
                }
                let at = (py as usize * self.width + px as usize) * 4;
                for (channel, &ink) in ink.iter().enumerate() {
                    let under = u32::from(self.rgba[at + channel]);
                    self.rgba[at + channel] =
                        ((under * (255 - alpha) + u32::from(ink) * alpha + 127) / 255) as u8;
                }
            }
        }
    }
}

/// The em of a name drawn over a run of letters, at most: a line of the tile map is 8 pixels high.
const NAME_EM: f32 = 10.0;

/// Draws, over each part of `cells` (spelled by `character`, see [`name_parts`]) that is a name of the overlay's,
/// its translation: the part's cells take their ground colour and the name is drawn across them in their ink, its
/// em shrunk to fit the part.
fn draw_names(
    overlay: &Overlay,
    target: &mut Target<'_>,
    cells: &[TextCell],
    character: &dyn Fn(u8) -> Option<char>,
) {
    if overlay.names.is_empty() {
        return;
    }
    for (english, part) in name_parts(cells, character) {
        if let Some(name) = overlay.name(english.trim_end()) {
            draw_name(&overlay.face, target, &part, name);
        }
    }
}

/// The parts of `cells` (spelled by `character`) a name could be, with their cells. A run is the letters of one
/// row, 8 pixels apart, or 16 where a space has no tile (prose: the opening's scroll); a part is the whole run, or
/// a piece of it between two or more blank cells (a list's columns). So a word of a sentence is never a part.
fn name_parts<'a>(
    cells: &'a [TextCell],
    character: &dyn Fn(u8) -> Option<char>,
) -> Vec<(String, Vec<&'a TextCell>)> {
    let mut row: Vec<(&TextCell, char)> = cells
        .iter()
        .filter(|cell| cell.glyph < CARD_BASE)
        .filter_map(|cell| Some((cell, character(cell.glyph)?)))
        .collect();
    row.sort_unstable_by_key(|(cell, _)| (cell.y, cell.x));
    // the row's letters, with `None` for a space that has no tile
    let mut slots: Vec<Option<(&TextCell, char)>> = Vec::new();
    let mut parts = Vec::new();
    for at in 0..=row.len() {
        if at > 0 {
            let (prev, before) = row[at - 1];
            let next = row.get(at).copied();
            let same_row = next.is_some_and(|(next, _)| next.y == prev.y);
            // (a number beside a name is its own: a box's line or a frame's tile is between them)
            let digit = before.is_ascii_digit() || next.is_some_and(|(_, ch)| ch.is_ascii_digit());
            match next {
                Some((next, _)) if same_row && next.x == prev.x + 8 => {}
                Some((next, _)) if same_row && !digit && next.x == prev.x + 16 => slots.push(None),
                _ => {
                    run_parts(&slots, &mut parts);
                    slots.clear();
                }
            }
        }
        if let Some(&letter) = row.get(at) {
            slots.push(Some(letter));
        }
    }
    parts
}

/// The parts of one run's `slots` (see [`name_parts`]), added to `parts`.
fn run_parts<'a>(
    slots: &[Option<(&'a TextCell, char)>],
    parts: &mut Vec<(String, Vec<&'a TextCell>)>,
) {
    let blank = |at: usize| slots.get(at).is_none_or(|slot| slot.is_none_or(|(_, ch)| ch == ' '));
    let mut at = 0;
    while at < slots.len() {
        if blank(at) {
            at += 1;
            continue;
        }
        // a part: from a letter to before two blanks in a row (or the run's end)
        let mut end = at;
        while end < slots.len() && !(blank(end) && blank(end + 1)) {
            end += 1;
        }
        let english = slots[at..end].iter().map(|slot| slot.map_or(' ', |(_, ch)| ch)).collect();
        parts.push((english, slots[at..end].iter().flatten().map(|&(cell, _)| cell).collect()));
        at = end;
    }
}

/// Draws `name` over the cells `part`, in their colours.
fn draw_name(face: &ChineseFace, target: &mut Target<'_>, part: &[&TextCell], name: &str) {
    let (first, last) = (part[0], part[part.len() - 1]);
    let (ink, paper) = (first.fg, first.bg);
    let width = (last.x + 8 - first.x) as f32;
    let natural: f32 = name.chars().map(|ch| face.advance_at(ch, NAME_EM)).sum();
    let em = if natural > width { NAME_EM * width / natural } else { NAME_EM };
    // (a pixel more around: the HD letters reach a little past their cells)
    target.fill((first.x - 1, first.y - 1), (last.x + 9, first.y + 9), paper);
    let scale = target.scale;
    let top = first.y as f32 + (8.0 - em) / 2.0;
    let mut pen = first.x as f32;
    for ch in name.chars() {
        let glyph = face.glyph_at(ch, scale, em);
        if !ch.is_whitespace() {
            let at = ((pen * scale as f32).round() as i32, (top * scale as f32).round() as i32);
            target.glyph(&glyph, at, ink);
        }
        pen += glyph.advance;
    }
}

/// Draws the translation of the page `screen` names (the engine's bytes of the frame; `None` when they name none)
/// over the picture `target`, whose 1x frame is `frame` (`frame_width` pixels a row) with the original's window at
/// `window_x` and the text `cells` the font noted in it. `kept` carries the page from one frame to the next.
#[allow(clippy::too_many_arguments)]
pub(crate) fn draw(
    overlay: &Overlay,
    kept: &mut Kept,
    screen: Option<&OnScreen>,
    target: &mut Target<'_>,
    cells: &[TextCell],
    frame: &[Color],
    frame_width: usize,
    window_x: i32,
) {
    match screen {
        Some(screen) => {
            kept.shown = overlay.text(screen).map(|text| Shown {
                key: screen.key.clone(),
                text,
                table: screen.table,
                string: (screen.table, screen.string),
                cell: {
                    let (column, row) = screen.base_cell();
                    (i32::from(column), i32::from(row))
                },
                portrait: screen.portrait,
                lines: screen.lines.len(),
                longest: screen.lines.iter().copied().max().unwrap_or(0),
                progress: screen.progress,
                settled: screen.line > 0 || screen.cell > 0 || !matches!(screen.state, 1 | 5),
            });
            kept.age = 0;
        }
        None => kept.age += 1,
    }
    if kept.age > KEEP {
        kept.shown = None;
    }
    let Some(shown) = kept.shown.clone() else { return };
    let frame_height = (frame.len() / frame_width.max(1)) as i32;
    let cells_wide = window_cells(shown.table, shown.longest) as i32;
    let left = window_x + 8 * shown.cell.0;
    let top = 8 * shown.cell.1;
    let face = &overlay.face;
    let lines = wrap(&shown.text, (8 * cells_wide) as f32, |ch| face.advance(ch));
    let rows = shown.lines.max(lines.len()) as i32;
    // The window's letters (a blank cell is the window's ground, whose colour 3 may be its frame's): where its text
    // is on the screen (the layer's scroll moves it from its tile-map place, and the battle's window shows its
    // lines closer than the engine writes them), and its colours
    let letters: Vec<&TextCell> = cells
        .iter()
        .filter(|cell| {
            cell.glyph != BLANK
                && cell.glyph < CARD_BASE
                && (left - 8..left + 8 * cells_wide + 8).contains(&cell.x)
                && (top - 2 * PITCH - 4..top + PITCH * (rows + 1)).contains(&cell.y)
        })
        .collect();
    if letters.is_empty() && kept.age > 0 {
        // The window was cleared or closed while the engine named nothing
        kept.shown = None;
        return;
    }
    // (only once the engine has written some of the page: a window that opens may still scroll the last text)
    let first = letters.iter().map(|cell| cell.y).min().filter(|_| shown.settled);
    if let Some(first) = first {
        let dx = most(letters.iter().map(|cell| off_grid(cell.x, left, 8))).unwrap_or(0);
        let mut ys: Vec<i32> = letters.iter().map(|cell| cell.y).collect();
        ys.sort_unstable();
        ys.dedup();
        let known = kept.place.filter(|place| place.string == shown.string);
        let pitch = ys
            .windows(2)
            .map(|pair| pair[1] - pair[0])
            .filter(|gap| (10..=2 * PITCH).contains(gap))
            .min()
            .or(known.map(|place| place.pitch))
            .unwrap_or(PITCH);
        kept.place = Some(Place { string: shown.string, at: (left + dx, first), pitch });
        let fg = most(letters.iter().map(|cell| cell.fg));
        let bg = most(letters.iter().map(|cell| cell.bg));
        kept.colours = fg.zip(bg);
    }
    // No letter of this string was seen yet: the original shows none of it either
    let Some(place) = kept.place.filter(|place| place.string == shown.string) else { return };
    let Some((ink, paper)) = kept.colours else { return };
    let ((x0, y0), pitch) = (place.at, place.pitch);
    // The original's letters go: the text's lines, from the hanging cell of a portrait's first line to the
    // window's width
    let visible_rows = rows.min((frame_height - y0 + pitch - 1) / pitch).max(0);
    let hang = if shown.portrait { 8 } else { 0 };
    let right = (x0 + 8 * cells_wide).min(frame_width as i32);
    target.fill(
        (x0 - hang, y0 - RISE),
        (right, y0 + pitch * (visible_rows - 1) + EM as i32 - RISE),
        paper,
    );
    // The translation, as far as the engine has written
    let total = lines.iter().flatten().filter(|ch| !ch.is_whitespace()).count();
    let mut left_to_draw = (shown.progress * total as f32).ceil() as usize;
    let scale = target.scale;
    for (row, line) in lines.iter().enumerate().take(visible_rows as usize) {
        let mut pen = 0.0;
        for &ch in line {
            if left_to_draw == 0 {
                return;
            }
            let glyph = face.glyph(ch, scale);
            if !ch.is_whitespace() {
                let x = ((x0 as f32 + pen) * scale as f32).round() as i32;
                let y = (y0 - RISE + pitch * row as i32) * scale as i32;
                target.glyph(&glyph, (x, y), ink);
                left_to_draw -= 1;
            }
            pen += glyph.advance;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(wrapped: &[Vec<char>]) -> Vec<String> {
        wrapped.iter().map(|line| line.iter().collect()).collect()
    }

    #[test]
    fn a_line_wraps_at_the_width_and_keeps_its_own_breaks() {
        let wrapped = wrap("一二三四五\n六", 36.0, |_| 12.0);
        assert_eq!(lines(&wrapped), ["一二三", "四五", "六"]);
    }

    #[test]
    fn a_closing_mark_never_starts_a_line_and_an_opening_one_never_ends_one() {
        assert_eq!(lines(&wrap("一二三。四", 36.0, |_| 12.0)), ["一二三。", "四"]);
        assert_eq!(lines(&wrap("一二「三四", 36.0, |_| 12.0)), ["一二", "「三四"]);
        assert_eq!(lines(&wrap("一二！？三", 24.0, |_| 12.0)), ["一二！？", "三"]);
    }

    #[test]
    fn narrow_letters_take_their_own_width_and_a_wrapped_line_drops_its_leading_space() {
        let advance = |ch: char| if ch.is_ascii() { 6.0 } else { 12.0 };
        assert_eq!(lines(&wrap("得到120點", 36.0, advance)), ["得到", "120點"]);
        assert_eq!(lines(&wrap("12345678", 24.0, advance)), ["1234", "5678"], "too long: cut");
        assert_eq!(lines(&wrap("ab cd", 12.0, advance)), ["ab", "cd"]);
    }

    #[test]
    fn the_offset_from_the_grid_is_the_nearest_one() {
        assert_eq!(off_grid(185, 184, 16), 1);
        assert_eq!(off_grid(199, 184, 16), -1);
        assert_eq!(off_grid(184 + 32, 184, 16), 0);
    }

    const PAPER: [u8; 3] = [0xFF, 0xEF, 0xCE];
    const INK: [u8; 3] = [0x00, 0x00, 0x31];

    /// An overlay that draws the made-up page `str3:0x00` ("AB" / "C") with the Latin font of the HD text (the tests
    /// have no Chinese font).
    fn overlay() -> Overlay {
        let table = Translation::parse("key\tzh\nstr3:0x00\tAB\\nC{auto}\n").unwrap();
        let face = ChineseFace::new(crate::font::SHARE_TECH_MONO.to_vec()).unwrap();
        Overlay::new(Arc::new(table), Arc::new(face))
    }

    /// The story window's first page of string 0, `progress` written.
    fn screen(progress: f32) -> OnScreen {
        OnScreen {
            table: 3,
            string: 0,
            page: 0,
            key: "str3:0x00".to_owned(),
            state: 1,
            line: 0,
            cell: 1,
            base: 0x05C8,
            portrait: false,
            lines: vec![9, 4],
            progress,
            names: [None, None, None],
            numbers: Default::default(),
        }
    }

    /// A letter of the original at the text's first cell (4, 23).
    fn letter() -> TextCell {
        TextCell { x: 32, y: 184, glyph: 1, fg: INK, bg: PAPER, mask: !0, ..TextCell::default() }
    }

    /// Draws a frame at scale 2 over a grey picture and a 1x frame of paper.
    fn drawn(
        overlay: &Overlay,
        kept: &mut Kept,
        screen: Option<&OnScreen>,
        cells: &[TextCell],
    ) -> Vec<u8> {
        let mut rgba = [7_u8, 7, 7, 255].repeat(512 * 448);
        let frame = vec![Color::rgb(0xFF, 0xEF, 0xCE); 256 * 224];
        let mut target = Target { rgba: &mut rgba, width: 512, scale: 2 };
        draw(overlay, kept, screen, &mut target, cells, &frame, 256, 0);
        rgba
    }

    fn pixel(rgba: &[u8], x: usize, y: usize) -> [u8; 3] {
        let at = (y * 512 + x) * 4;
        [rgba[at], rgba[at + 1], rgba[at + 2]]
    }

    /// Whether a pixel of the text's first line holds ink.
    fn inked(rgba: &[u8]) -> bool {
        (64..128).any(|x| (366..390).any(|y| pixel(rgba, x, y)[2] < 0x80))
    }

    #[test]
    fn the_window_is_painted_over_and_the_text_drawn_as_far_as_the_engine_wrote() {
        let overlay = overlay();
        let mut kept = Kept::default();
        let rgba = drawn(&overlay, &mut kept, Some(&screen(0.0)), &[letter()]);
        assert_eq!(pixel(&rgba, 2 * 32 + 2, 2 * 184 + 2), PAPER, "the original's letter is gone");
        assert_eq!(pixel(&rgba, 2 * 200, 2 * 200), PAPER, "the second line too");
        assert_eq!(pixel(&rgba, 2 * 240, 2 * 186), [7, 7, 7], "right of the 25 cells: untouched");
        assert!(!inked(&rgba), "nothing is written yet");
        let rgba = drawn(&overlay, &mut kept, Some(&screen(1.0)), &[letter()]);
        assert!(inked(&rgba));
    }

    #[test]
    fn the_text_stays_while_the_window_keeps_letters_and_goes_when_it_is_cleared() {
        let overlay = overlay();
        let mut kept = Kept::default();
        drawn(&overlay, &mut kept, Some(&screen(1.0)), &[letter()]);
        assert!(inked(&drawn(&overlay, &mut kept, None, &[letter()])), "held between pages");
        let cleared = drawn(&overlay, &mut kept, None, &[]);
        assert_eq!(pixel(&cleared, 2 * 32 + 2, 2 * 184 + 2), [7, 7, 7], "nothing drawn");
        assert!(kept.shown.is_none());
    }

    #[test]
    fn a_page_not_begun_in_a_window_never_seen_is_left_alone() {
        // (a window that opens may still show the last text, scrolling)
        let overlay = overlay();
        let mut kept = Kept::default();
        let fresh = OnScreen { cell: 0, ..screen(0.0) };
        let rgba = drawn(&overlay, &mut kept, Some(&fresh), &[letter()]);
        assert!(rgba.chunks(4).all(|p| p[..3] == [7, 7, 7]));
        drawn(&overlay, &mut kept, Some(&screen(0.5)), &[letter()]);
        let rgba = drawn(&overlay, &mut kept, Some(&fresh), &[letter()]);
        assert_eq!(pixel(&rgba, 2 * 32 + 2, 2 * 184 + 2), PAPER, "its place is known now");
    }

    #[test]
    fn a_page_the_translation_lacks_keeps_the_originals_text() {
        let overlay = overlay();
        let mut kept = Kept::default();
        let other = OnScreen { key: "str3:0x01".to_owned(), ..screen(1.0) };
        let rgba = drawn(&overlay, &mut kept, Some(&other), &[letter()]);
        assert!(rgba.chunks(4).all(|p| p[..3] == [7, 7, 7]));
    }

    #[test]
    fn a_name_is_found_whole_or_by_the_one_name_a_cut_column_starts() {
        let names =
            [("Piccolo", "笛子魔童"), ("Goku", "孫悟空"), ("Gohan", "孫悟飯"), ("Nappa", "立巴")]
                .map(|(english, name)| (english.to_owned(), name.to_owned()));
        let overlay = overlay().with_names(names.into_iter().collect());
        assert_eq!(overlay.name("Goku"), Some("孫悟空"));
        assert_eq!(overlay.name("Piccol"), Some("笛子魔童"));
        assert_eq!(overlay.name("Go"), None, "too short to stand for a name");
        assert_eq!(overlay.name("Napp"), Some("立巴"));
        assert_eq!(overlay.name("Talk"), None);
    }

    #[test]
    fn a_word_of_a_sentence_is_no_part_even_where_its_spaces_have_no_tiles() {
        // `text` at row `y`: a letter is a cell (its glyph its place in `LETTERS`), a space none (the opening's
        // scroll draws no tile for it)
        const LETTERS: &str = "ADEFGHIKLNOPRSYabcehiklnopsu.01";
        let row = |text: &str, y: i32| -> Vec<TextCell> {
            (0..)
                .zip(text.chars())
                .filter(|&(_, ch)| ch != ' ')
                .map(|(at, ch)| {
                    let glyph = u8::try_from(LETTERS.find(ch).unwrap()).unwrap();
                    TextCell { x: 8 * at, y, glyph, ..TextCell::default() }
                })
                .collect()
        };
        let character = |glyph: u8| LETTERS.chars().nth(usize::from(glyph));
        let english = |cells: &[TextCell]| -> Vec<String> {
            name_parts(cells, &character).into_iter().map(|(english, _)| english).collect()
        };
        assert_eq!(
            english(&row("A FROG HIDES IN A PILE OF REEDS.", 0)),
            ["A FROG HIDES IN A PILE OF REEDS."]
        );
        assert_eq!(english(&row("FOR", 0)), ["FOR"], "a word alone is still a part");
        assert_eq!(english(&row("Goku   Piccolo", 0)), ["Goku", "Piccolo"], "a list's columns");
        assert_eq!(
            english(&row("Piccol 01", 0)),
            ["Piccol", "01"],
            "a frame's tile before the level"
        );
        let two_rows = [row("Goku", 0), row("Gohan", 8)].concat();
        assert_eq!(english(&two_rows), ["Goku", "Gohan"]);
        let parts = name_parts(&two_rows, &character);
        assert_eq!(parts[1].1.len(), 5, "a part keeps its cells");
    }

    #[test]
    fn the_measured_windows_have_their_widths_and_the_others_their_longest_line() {
        assert_eq!(window_cells(3, 9), 25);
        assert_eq!(window_cells(1, 30), 17);
        assert_eq!(window_cells(7, 13), 13);
        assert_eq!(window_cells(7, 1), 4);
    }
}
