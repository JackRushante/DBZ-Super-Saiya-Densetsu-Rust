//! Where the touch controls are: the shapes of the buttons of each layout (landscape, minimal for battles,
//! portrait) worked out from the screen, the player's places and sizes, and the area left to the game picture.
//! All sizes are egui points, which on Android are density-independent pixels (dp). The design is the approved
//! mock-up (932 x 430 dp landscape, 430 x 932 dp portrait).

#![allow(clippy::many_single_char_names)]

use super::settings::{Group, Orient, Picture, Settings};
use egui::{Color32, Pos2, Rect, Vec2, pos2, vec2};

/// The smallest size of a control, in dp.
pub const MIN_DP: f32 = 48.0;

/// Every button the overlay has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Btn {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
    X,
    Y,
    L,
    R,
    Start,
    Select,
    Speed,
    Menu,
    /// Minimal layout only: back to all the controls.
    Full,
    /// The full layout's button for the minimal one.
    Min,
}

/// The shape of a button.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
    /// A pill (a rounded rectangle) with this corner radius.
    Pill(Rect, f32),
    /// A cell of the d-pad.
    Cell(Rect),
    Circle(Pos2, f32),
}

impl Shape {
    #[must_use]
    pub fn bounds(self) -> Rect {
        match self {
            Self::Pill(r, _) | Self::Cell(r) => r,
            Self::Circle(c, r) => Rect::from_center_size(c, vec2(2.0 * r, 2.0 * r)),
        }
    }

    /// Whether `p` is on the shape grown by `slop`.
    #[must_use]
    pub fn hit(self, p: Pos2, slop: f32) -> bool {
        match self {
            Self::Pill(r, _) | Self::Cell(r) => r.expand(slop).contains(p),
            Self::Circle(c, r) => c.distance(p) <= r + slop,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Item {
    pub btn: Btn,
    pub group: Group,
    pub shape: Shape,
    /// The ring and the glyph's colour.
    pub accent: Color32,
}

pub const CREAM: Color32 = Color32::from_rgb(0xFF, 0xEF, 0xCE);
pub const ORANGE: Color32 = Color32::from_rgb(0xFF, 0x52, 0x00);
pub const DARK_RED: Color32 = Color32::from_rgb(0x84, 0x00, 0x00);
pub const RED: Color32 = Color32::from_rgb(0xFF, 0x6B, 0x6B);
pub const YELLOW: Color32 = Color32::from_rgb(0xFF, 0xD2, 0x3F);
pub const BLUE: Color32 = Color32::from_rgb(0x5A, 0xB0, 0xFF);
pub const GREEN: Color32 = Color32::from_rgb(0x62, 0xD8, 0x7A);

/// Everything the overlay needs to draw and to hit-test one frame.
#[derive(Debug, Clone)]
pub struct Geometry {
    pub orient: Orient,
    pub screen: Rect,
    /// The area the game picture is fitted in.
    pub picture: Rect,
    /// Portrait: the opaque band under the picture.
    pub band: Option<Rect>,
    pub items: Vec<Item>,
    /// The floating stick's touch zone (instead of the d-pad's cells).
    pub stick_zone: Option<Rect>,
    /// The d-pad's centre and its half size (also the stick zone's centre).
    pub dpad: (Pos2, f32),
}

#[must_use]
pub fn orient_of(screen: Rect) -> Orient {
    if screen.width() >= screen.height() { Orient::Landscape } else { Orient::Portrait }
}

/// The space kept between two groups a short screen pushes together, in dp.
const GAP: f32 = 8.0;
/// The size of the controls of the Large picture and of the full screen, on top of the player's: smaller, so the
/// picture has the room.
const COMPACT: f32 = 0.85;
/// The parts of the portrait band its rows are at (the design's band is 270..932): the top row, the d-pad and the
/// diamond, Start/Select.
const ROW: f32 = 0.0906;
const PADS: f32 = 0.459;
const START: f32 = 0.831;
/// The SNES picture's height for its width (256 x 224): a picture this tall spans the whole width.
const FULL_WIDTH: f32 = 224.0 / 256.0;

/// The size of the controls for the picture size: [`COMPACT`] for the Large picture and the full screen.
fn compact(picture: Picture) -> f32 {
    if picture == Picture::Standard { 1.0 } else { COMPACT }
}

/// The least portrait band the controls of size `k` fit in: the top row, MIN under it, the d-pad (three cells of
/// 56 dp), Start/Select and the gaps between them, at the band's own proportions.
fn least_band(k: f32) -> f32 {
    let stack = 56.0 + MIN_DP / 2.0 + GAP + 84.0 + 84.0 + GAP + MIN_DP / 2.0 + MIN_DP / 2.0;
    (stack * k + GAP) / (1.0 - ROW)
}

/// The top of the portrait band under the picture, from the screen's top: the design's 16:9 area under a 28 dp
/// margin (Standard); the whole width of the picture as far as the band of compact controls leaves room, and never
/// less than the design (Large); the controls' least band at the bottom of the screen, over the picture (Full).
fn band_top(w: f32, h: f32, picture: Picture) -> f32 {
    let standard = 28.0 + w * 9.0 / 16.0;
    match picture {
        Picture::Standard => standard,
        Picture::Large => (28.0 + w * FULL_WIDTH).min(h - least_band(COMPACT)).max(standard),
        Picture::Full => (h - least_band(COMPACT)).max(0.0),
    }
}

/// The default centre of a group on `screen`: the design's place at its size (932 x 430, 430 x 932), and on a
/// shorter screen (split screen, a window) the d-pad, the face buttons and Start/Select kept under the top
/// buttons, and in portrait under the picture. The Large picture and the full screen pack the controls closer to
/// the edges, at [`COMPACT`] size.
fn default_centre(o: Orient, g: Group, s: Rect, picture: Picture) -> Pos2 {
    let (w, h) = (s.width(), s.height());
    let k = compact(picture);
    let at = |x: f32, y: f32| pos2(s.min.x + x, s.min.y + y);
    match o {
        Orient::Landscape => {
            // MIN's bottom (under the speed button) and MENU's, then the half heights of the d-pad and the diamond
            let (min_bottom, menu_bottom) =
                ((94.0 + 56.0 + MIN_DP / 2.0) * k, (94.0 + MIN_DP / 2.0) * k);
            match g {
                Group::Dpad => at(80.0 * k, (0.619 * h).max(min_bottom + GAP + 72.0 * k)),
                Group::Face => at(w - 82.0 * k, (0.633 * h).max(menu_bottom + GAP + 75.0 * k)),
                Group::L => at(65.0 * k, 34.0 * k),
                Group::R => at(w - 65.0 * k, 34.0 * k),
                Group::Speed => at(46.0 * k, 94.0 * k),
                Group::Menu => at(w - 65.0 * k, 94.0 * k),
                Group::StartSelect => at(w / 2.0, h - 28.0 * k),
            }
        }
        Orient::Portrait => {
            let top = band_top(w, h, picture);
            let band = (h - top).max(0.0);
            let row = top + ROW * band;
            // under MIN (a row under the top one), then the d-pad's half height (cells of 56 dp)
            let pads = (top + PADS * band).max(row + (56.0 + MIN_DP / 2.0 + 84.0) * k + GAP);
            match g {
                Group::Dpad => at(0.256 * w, pads),
                Group::Face => at(0.749 * w, pads),
                Group::L => at(0.137 * w, row),
                Group::Speed => at(0.379 * w, row),
                Group::Menu => at(0.62 * w, row),
                Group::R => at(0.863 * w, row),
                Group::StartSelect => {
                    at(0.5 * w, (top + START * band).max(pads + (84.0 + MIN_DP / 2.0) * k + GAP))
                }
            }
        }
    }
}

/// The size a group is drawn at on top of the player's: the portrait layout has bigger cells for a thumb.
fn base_scale(o: Orient, g: Group) -> f32 {
    match (o, g) {
        (Orient::Portrait, Group::Dpad) => 56.0 / 48.0,
        (Orient::Portrait, Group::Face) => 1.15,
        _ => 1.0,
    }
}

/// The centre and the size factor of `g` for the player's choices, kept on the screen.
#[must_use]
pub fn placed(o: Orient, g: Group, s: Rect, settings: &Settings) -> (Pos2, f32) {
    let p = settings.place(o, g);
    let mut c = default_centre(o, g, s, settings.picture);
    if let Some(x) = p.x {
        c.x = s.min.x + s.width() * x as f32 / 1000.0;
    }
    if let Some(y) = p.y {
        c.y = s.min.y + s.height() * y as f32 / 1000.0;
    }
    let k = base_scale(o, g) * compact(settings.picture) * p.scale.unwrap_or(100) as f32 / 100.0;
    (pos2(c.x.clamp(s.min.x, s.max.x), c.y.clamp(s.min.y, s.max.y)), k)
}

fn pill(c: Pos2, w: f32, h: f32) -> Shape {
    Shape::Pill(Rect::from_center_size(c, vec2(w, h)), h / 2.0)
}

fn push(items: &mut Vec<Item>, btn: Btn, group: Group, shape: Shape, accent: Color32) {
    items.push(Item { btn, group, shape, accent });
}

/// The picture area and the controls for `screen`.
#[must_use]
pub fn compute(screen: Rect, minimal: bool, settings: &Settings) -> Geometry {
    let o = orient_of(screen);
    let (w, h) = (screen.width(), screen.height());
    let (picture, band) = match (o, settings.picture) {
        // The full screen: the picture as big as the screen holds (in portrait from the top, the width's height),
        // the controls over it.
        (Orient::Landscape, Picture::Full) => (screen, None),
        (Orient::Portrait, Picture::Full) => {
            let bottom = screen.min.y + h.min(w * FULL_WIDTH);
            (Rect::from_min_max(screen.min, pos2(screen.max.x, bottom)), None)
        }
        (Orient::Landscape, size) => {
            let (top, bottom) = if size == Picture::Large {
                (0.0, 56.0 * COMPACT)
            } else {
                (8.0_f32.max(0.02 * h), 56.0)
            };
            let area = Rect::from_min_max(
                pos2(screen.min.x, screen.min.y + top),
                pos2(screen.max.x, screen.max.y - bottom),
            );
            (area, None)
        }
        (Orient::Portrait, size) => {
            let top = screen.min.y + 28.0;
            let bottom = screen.min.y + band_top(w, h, size);
            let area = Rect::from_min_max(pos2(screen.min.x, top), pos2(screen.max.x, bottom));
            (area, Some(Rect::from_min_max(pos2(screen.min.x, bottom), screen.max)))
        }
    };
    let mut items = Vec::new();
    let mut stick_zone = None;
    let portrait = o == Orient::Portrait;

    // The d-pad: four cells of 48 dp in a cross, or the floating stick's zone.
    let (dc, dk) = placed(o, Group::Dpad, screen, settings);
    let cell = 48.0 * dk;
    if settings.stick {
        let zone = Rect::from_center_size(dc, vec2(150.0 * dk, 246.0 * dk));
        stick_zone = Some(zone.intersect(screen));
    } else {
        for (btn, dx, dy) in [
            (Btn::Up, 0.0, -1.0),
            (Btn::Left, -1.0, 0.0),
            (Btn::Right, 1.0, 0.0),
            (Btn::Down, 0.0, 1.0),
        ] {
            let r = Rect::from_center_size(dc + vec2(dx, dy) * cell, vec2(cell, cell));
            push(&mut items, btn, Group::Dpad, Shape::Cell(r), CREAM);
        }
    }

    // A, B, X, Y in the SNES diamond (A right, B bottom, X top, Y left); the minimal layout has a big A and a B.
    let (fc, fk) = placed(o, Group::Face, screen, settings);
    if minimal {
        let big = 88.0 * fk.max(1.0);
        let small = 68.0 * fk.max(1.0);
        let a_at = fc + vec2(if portrait { 8.0 } else { 0.0 }, -28.0 * fk);
        push(&mut items, Btn::A, Group::Face, Shape::Circle(a_at, big / 2.0), RED);
        let b_at = a_at + vec2(-(big / 2.0 + small / 2.0 + 4.0), small * 0.6);
        push(&mut items, Btn::B, Group::Face, Shape::Circle(b_at, small / 2.0), YELLOW);
    } else {
        let d = 50.0 * fk;
        for (btn, dx, dy, accent) in [
            (Btn::A, 1.0, 0.0, RED),
            (Btn::B, 0.0, 1.0, YELLOW),
            (Btn::X, 0.0, -1.0, BLUE),
            (Btn::Y, -1.0, 0.0, GREEN),
        ] {
            push(
                &mut items,
                btn,
                Group::Face,
                Shape::Circle(fc + vec2(dx, dy) * d, d / 2.0),
                accent,
            );
        }
    }

    // The top buttons.
    let wide = if portrait { 90.0 } else { 110.0 };
    let (mc, mk) = placed(o, Group::Menu, screen, settings);
    if minimal {
        // Fixed to the corners: FULL (back to all controls) top-left, MENU top-right.
        push(
            &mut items,
            Btn::Full,
            Group::L,
            pill(pos2(screen.min.x + 52.0, screen.min.y + 34.0), 84.0, MIN_DP),
            ORANGE,
        );
        let at = if portrait { mc } else { pos2(screen.max.x - 65.0, screen.min.y + 34.0) };
        push(&mut items, Btn::Menu, Group::Menu, pill(at, wide * mk, MIN_DP * mk), ORANGE);
    } else {
        let (lc, lk) = placed(o, Group::L, screen, settings);
        push(&mut items, Btn::L, Group::L, pill(lc, wide * lk, MIN_DP * lk), CREAM);
        let (rc, rk) = placed(o, Group::R, screen, settings);
        push(&mut items, Btn::R, Group::R, pill(rc, wide * rk, MIN_DP * rk), CREAM);
        let (sc, sk) = placed(o, Group::Speed, screen, settings);
        let sw = if portrait { 90.0 } else { 72.0 };
        push(&mut items, Btn::Speed, Group::Speed, pill(sc, sw * sk, MIN_DP * sk), ORANGE);
        push(
            &mut items,
            Btn::Min,
            Group::Speed,
            pill(sc + vec2(0.0, 56.0 * sk), sw * sk, MIN_DP * sk),
            ORANGE,
        );
        push(&mut items, Btn::Menu, Group::Menu, pill(mc, wide * mk, MIN_DP * mk), ORANGE);
        let (cc, ck) = placed(o, Group::StartSelect, screen, settings);
        for (btn, dx) in [(Btn::Select, -64.0), (Btn::Start, 64.0)] {
            push(
                &mut items,
                btn,
                Group::StartSelect,
                pill(cc + vec2(dx * ck, 0.0), 100.0 * ck, MIN_DP * ck),
                CREAM,
            );
        }
    }
    Geometry { orient: o, screen, picture, band, items, stick_zone, dpad: (dc, 72.0 * dk) }
}

impl Geometry {
    /// The items of one group.
    pub fn group(&self, g: Group) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(move |i| i.group == g)
    }

    /// The box around a group (the stick's zone for the stick).
    #[must_use]
    pub fn group_bounds(&self, g: Group) -> Option<Rect> {
        if g == Group::Dpad
            && let Some(zone) = self.stick_zone
        {
            return Some(zone);
        }
        self.group(g).map(|i| i.shape.bounds()).reduce(Rect::union)
    }

    #[must_use]
    pub fn item(&self, btn: Btn) -> Option<&Item> {
        self.items.iter().find(|i| i.btn == btn)
    }
}

/// The d-pad directions a finger at offset `d` from the centre holds: 8-way, with a dead zone of `dead` dp.
#[must_use]
pub fn directions(d: Vec2, dead: f32) -> [bool; 4] {
    let len = d.length();
    if len < dead {
        return [false; 4];
    }
    // tan(22.5 degrees): a diagonal begins where the minor axis is this much of the vector.
    let t = 0.4142 * len;
    // up, down, left, right
    [
        d.y < -t.max(dead * 0.5),
        d.y > t.max(dead * 0.5),
        d.x < -t.max(dead * 0.5),
        d.x > t.max(dead * 0.5),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn screen(w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, vec2(w, h))
    }

    #[test]
    fn the_default_landscape_layout_matches_the_design() {
        let g = compute(screen(932.0, 430.0), false, &Settings::default());
        let a = g.item(Btn::A).unwrap().shape.bounds();
        // The design has A at x 875..925, y 247..297 and B below the diamond's centre.
        assert!((a.min.x - 875.0).abs() < 1.5 && (a.min.y - 247.0).abs() < 4.0, "{a:?}");
        let up = g.item(Btn::Up).unwrap().shape.bounds();
        assert!((up.min.x - 54.0).abs() < 4.0, "{up:?}");
        let menu = g.item(Btn::Menu).unwrap().shape.bounds();
        assert!(menu.width() >= MIN_DP && menu.height() >= MIN_DP);
        assert!(g.picture.height() > 300.0 && g.band.is_none());
    }

    #[test]
    fn no_control_is_smaller_than_48_dp_and_none_leaves_the_screen_at_the_defaults() {
        for (w, h) in [(932.0, 430.0), (800.0, 360.0), (430.0, 932.0)] {
            for minimal in [false, true] {
                let s = screen(w, h);
                let g = compute(s, minimal, &Settings::default());
                for i in &g.items {
                    let b = i.shape.bounds();
                    assert!(b.width() >= MIN_DP - 0.01 && b.height() >= MIN_DP - 0.01, "{i:?}");
                    assert!(s.expand(1.0).contains_rect(b), "{:?} off the {w}x{h} screen", i.btn);
                }
            }
        }
    }

    #[test]
    fn portrait_has_the_picture_on_top_and_an_opaque_band_below() {
        let g = compute(screen(430.0, 932.0), false, &Settings::default());
        let band = g.band.unwrap();
        assert!((g.picture.max.y - band.min.y).abs() < 0.01);
        assert!(g.items.iter().all(|i| i.shape.bounds().min.y >= band.min.y));
    }

    #[test]
    fn on_split_screen_sizes_no_group_covers_another_or_the_portrait_picture() {
        // the designs, a phone (both ways), a Fold's inner screen (both ways, then halved) and a short window
        let sizes = [
            (932.0, 430.0),
            (430.0, 932.0),
            (411.0, 891.0),
            (891.0, 411.0),
            (787.0, 873.0),
            (873.0, 787.0),
            (420.0, 673.0),
            (841.0, 337.0),
            (800.0, 360.0),
        ];
        let pictures = [Picture::Standard, Picture::Large, Picture::Full];
        for ((w, h), picture) in sizes.into_iter().flat_map(|s| pictures.map(|p| (s, p))) {
            let s = screen(w, h);
            let g = compute(s, false, &Settings { picture, ..Settings::default() });
            let groups = [
                Group::Dpad,
                Group::Face,
                Group::L,
                Group::R,
                Group::Speed,
                Group::Menu,
                Group::StartSelect,
            ];
            let bounds: Vec<(Group, Rect)> =
                groups.iter().map(|&k| (k, g.group_bounds(k).unwrap())).collect();
            for (i, &(a, ra)) in bounds.iter().enumerate() {
                assert!(
                    s.expand(1.0).contains_rect(ra),
                    "{a:?} off the {w}x{h} screen ({picture:?})"
                );
                for &(b, rb) in &bounds[i + 1..] {
                    assert!(
                        !ra.intersects(rb),
                        "{a:?} {ra:?} covers {b:?} {rb:?} on {w}x{h} ({picture:?})"
                    );
                }
                if g.orient == Orient::Portrait && picture != Picture::Full {
                    assert!(
                        ra.min.y >= g.picture.max.y,
                        "{a:?} {ra:?} on the picture on {w}x{h} ({picture:?})"
                    );
                }
            }
        }
    }

    #[test]
    fn the_large_picture_takes_the_width_where_it_can_and_the_full_screen_is_the_screen() {
        let large = Settings { picture: Picture::Large, ..Settings::default() };
        let full = Settings { picture: Picture::Full, ..Settings::default() };
        // a phone in portrait: the whole width, so no bars at the sides
        let phone = compute(screen(411.0, 891.0), false, &large);
        assert!(phone.picture.height() >= 411.0 * FULL_WIDTH - 0.5, "{:?}", phone.picture);
        // a Fold's inner screen: much more picture than the design gives it
        let standard = compute(screen(787.0, 873.0), false, &Settings::default()).picture;
        let unfolded = compute(screen(787.0, 873.0), false, &large).picture;
        assert!(unfolded.height() > standard.height() * 1.2, "{unfolded:?} against {standard:?}");
        // the controls are smaller, never tiny
        for i in &phone.items {
            let b = i.shape.bounds();
            assert!(b.width().min(b.height()) >= MIN_DP * COMPACT - 0.01, "{i:?}");
        }
        assert_eq!(compute(screen(891.0, 411.0), false, &full).picture, screen(891.0, 411.0));
        let tall = compute(screen(787.0, 873.0), false, &full);
        assert!(tall.band.is_none() && tall.picture.min.y == 0.0);
        assert!(
            (tall.picture.height() - 787.0 * FULL_WIDTH).abs() < 0.5,
            "the width's height, from the top"
        );
    }

    #[test]
    fn a_stored_place_moves_and_resizes_a_group() {
        let mut s = Settings::default();
        *s.place_mut(Orient::Landscape, Group::Face) =
            super::super::settings::Place { x: Some(500), y: Some(500), scale: Some(150) };
        let g = compute(screen(1000.0, 400.0), false, &s);
        let b = g.group_bounds(Group::Face).unwrap();
        assert!((b.center().x - 500.0).abs() < 1.0 && (b.center().y - 200.0).abs() < 1.0);
        assert!((b.width() - 150.0 * 1.5).abs() < 1.0, "{b:?}");
    }

    #[test]
    fn directions_are_eight_way_with_a_dead_zone() {
        assert_eq!(directions(vec2(3.0, -2.0), 20.0), [false; 4]);
        assert_eq!(directions(vec2(0.0, -40.0), 20.0), [true, false, false, false]);
        assert_eq!(directions(vec2(40.0, 5.0), 20.0), [false, false, false, true]);
        assert_eq!(directions(vec2(30.0, 30.0), 20.0), [false, true, false, true]);
        assert_eq!(directions(vec2(-30.0, -30.0), 20.0), [true, false, true, false]);
    }
}
