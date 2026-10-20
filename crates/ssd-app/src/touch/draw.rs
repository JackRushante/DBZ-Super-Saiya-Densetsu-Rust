//! The drawing of the touch controls, with egui. The shapes are egui's own anti-aliased ones and the text is
//! Share Tech Mono (the font of the game's HD text and of the approved design), at sizes in dp. egui is the
//! overlay the app already has for the debug panel, and unlike the HD menus' draw lists (a fixed 1280 x 720 design
//! mapped onto the picture) it paints in screen points, so the buttons keep their size in dp whatever the picture is.

#![allow(clippy::many_single_char_names)]

use super::layout::{Btn, CREAM, DARK_RED, Item, MIN_DP, ORANGE, Shape};
use super::settings::{OPACITY_MAX, OPACITY_MIN, Orient, SCALE_MAX, SCALE_MIN};
use super::{Mode, PBtn, Panel, Touch};
use crate::theme;
use egui::{
    Align2, Color32, CornerRadius, Painter, Pos2, Rect, Stroke, StrokeKind, epaint::PathStroke,
    pos2, vec2,
};

const FILL: (u8, u8, u8) = (26, 5, 0);
const BAND: Color32 = Color32::from_rgb(0x1C, 0x14, 0x12);
const TEXT_DARK: Color32 = Color32::from_rgb(0x3A, 0x00, 0x00);
const NOTE: Color32 = Color32::from_rgb(0x5A, 0x2A, 0x2A);

fn alpha(c: Color32, a: f32) -> Color32 {
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a.clamp(0.0, 1.0) * 255.0).round() as u8)
}

fn radius(r: f32) -> CornerRadius {
    CornerRadius::same(r.round().clamp(0.0, 255.0) as u8)
}

/// The text of a control, centred.
fn label(p: &Painter, at: Pos2, text: &str, size: f32, color: Color32) {
    p.text(at, Align2::CENTER_CENTER, text, theme::game(size), color);
}

fn triangle(p: &Painter, c: Pos2, dir: (f32, f32), size: f32, color: Color32) {
    let (dx, dy) = dir;
    let tip = c + vec2(dx, dy) * size;
    let base = c - vec2(dx, dy) * size * 0.55;
    let side = vec2(-dy, dx) * size * 0.8;
    p.add(egui::Shape::convex_polygon(vec![tip, base + side, base - side], color, Stroke::NONE));
}

/// One button of the overlay.
fn button(p: &Painter, t: &Touch, item: &Item, text: &str) {
    let a = t.settings.opacity as f32 / 100.0;
    let down = t.is_down(item.btn);
    let (fill, ink, ring) = if down {
        (CREAM, DARK_RED, item.accent)
    } else {
        (
            alpha(Color32::from_rgb(FILL.0, FILL.1, FILL.2), 0.6 * a),
            alpha(CREAM, a),
            alpha(item.accent, a),
        )
    };
    let stroke = Stroke::new(3.0, ring);
    match item.shape {
        Shape::Pill(r, rad) => {
            p.rect_filled(r, radius(rad), fill);
            p.rect_stroke(r, radius(rad), stroke, StrokeKind::Inside);
            let size: f32 = match item.btn {
                Btn::Menu | Btn::Full | Btn::Min => 15.0,
                Btn::Start | Btn::Select => 13.0,
                _ => 18.0,
            };
            label(p, r.center(), text, size.min(r.height() * 0.4), ink);
        }
        Shape::Cell(r) => {
            p.rect_filled(r, radius(10.0 * r.width() / 48.0), fill);
            p.rect_stroke(r, radius(10.0 * r.width() / 48.0), stroke, StrokeKind::Inside);
            let dir = match item.btn {
                Btn::Up => (0.0, -1.0),
                Btn::Down => (0.0, 1.0),
                Btn::Left => (-1.0, 0.0),
                _ => (1.0, 0.0),
            };
            triangle(p, r.center(), dir, r.width() * 0.2, ink);
        }
        Shape::Circle(c, r) => {
            p.circle_filled(c, r, fill);
            p.circle_stroke(c, r - 1.5, stroke);
            label(p, c, text, (r * 0.72).clamp(14.0, 30.0), ink);
        }
    }
}

fn text_of(t: &Touch, btn: Btn) -> &'static str {
    match btn {
        Btn::A => "A",
        Btn::B => "B",
        Btn::X => "X",
        Btn::Y => "Y",
        Btn::L => "L",
        Btn::R => "R",
        Btn::Start => "START",
        Btn::Select => "SELECT",
        Btn::Speed => t.speed_label,
        Btn::Menu => "MENU",
        Btn::Full => "FULL",
        Btn::Min => "MIN",
        Btn::Up | Btn::Down | Btn::Left | Btn::Right => "",
    }
}

fn dashed_rect(p: &Painter, r: Rect, color: Color32, width: f32) {
    let pts = vec![r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
    p.extend(egui::Shape::dashed_line(&pts, Stroke::new(width, color), 8.0, 5.0));
}

fn dashed_circle(p: &Painter, c: Pos2, r: f32, color: Color32) {
    let n = 48;
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = std::f32::consts::TAU * i as f32 / n as f32;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    p.extend(egui::Shape::dashed_line(&pts, Stroke::new(2.0, color), 3.0, 4.0));
}

/// The floating stick: the dashed zone while idle; the base, the dead zone and the knob under the thumb.
fn stick(p: &Painter, t: &Touch, zone: Rect) {
    let a = t.settings.opacity as f32 / 100.0;
    match t.stick() {
        None => {
            p.rect_stroke(
                zone,
                radius(16.0),
                Stroke::new(2.0, alpha(CREAM, 0.5 * a)),
                StrokeKind::Inside,
            );
            label(p, zone.center_bottom() - vec2(0.0, 14.0), "touch zone", 12.0, alpha(CREAM, a));
        }
        Some((origin, at)) => {
            p.circle_filled(
                origin,
                63.0,
                alpha(Color32::from_rgb(FILL.0, FILL.1, FILL.2), 0.45 * a),
            );
            p.circle_stroke(origin, 61.5, Stroke::new(3.0, alpha(CREAM, a)));
            dashed_circle(p, origin, 35.0, alpha(CREAM, 0.6 * a));
            let d = at - origin;
            let knob = origin + if d.length() > 37.0 { d.normalized() * 37.0 } else { d };
            p.circle_filled(knob, 26.0, alpha(ORANGE, 0.7));
            p.circle_stroke(knob, 24.5, Stroke::new(3.0, ORANGE));
        }
    }
}

/// Draws the whole overlay over the window.
pub fn paint(p: &Painter, t: &Touch) {
    if !t.enabled {
        return;
    }
    let geo = t.geometry();
    if let Some(band) = geo.band {
        p.rect_filled(band, 0.0, BAND);
    }
    if t.settings.show_fps
        && let Some(stats) = &t.stats
    {
        // At the top in the middle, clear of L and R; drawn even while the controls are hidden.
        let at = pos2(geo.screen.center().x, geo.screen.min.y + 14.0);
        let galley = p.layout_no_wrap(stats.clone(), theme::game(13.0), CREAM);
        let back = Rect::from_center_size(at, galley.size() + vec2(12.0, 6.0));
        p.rect_filled(back, radius(4.0), Color32::from_rgba_unmultiplied(0, 0, 0, 160));
        p.galley(back.min + vec2(6.0, 3.0), galley, CREAM);
    }
    if t.mode == Mode::Edit {
        // The picture dims while the layout is edited.
        p.rect_filled(geo.picture, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 140));
    }
    if t.hidden {
        if t.mode == Mode::Play {
            let r = t.show_chip();
            let a = 0.5;
            p.rect_filled(r, radius(22.0), alpha(Color32::from_rgb(FILL.0, FILL.1, FILL.2), 0.6));
            p.rect_stroke(r, radius(22.0), Stroke::new(2.0, alpha(CREAM, a)), StrokeKind::Inside);
            label(p, r.center(), "...", 18.0, alpha(CREAM, a + 0.2));
        }
        return;
    }
    if !t.visible() {
        return;
    }
    if let Some(zone) = geo.stick_zone {
        // The stick's zone is a hint while idle; in the edit mode it is the group's box.
        stick(p, t, zone);
    } else if let Some(dpad) = geo.group_bounds(super::settings::Group::Dpad) {
        let a = t.settings.opacity as f32 / 100.0;
        let cell = dpad.width() / 3.0;
        p.rect_filled(
            Rect::from_center_size(dpad.center(), vec2(cell, cell)),
            0.0,
            alpha(CREAM, 0.18 * a),
        );
    }
    for item in &geo.items {
        button(p, t, item, text_of(t, item.btn));
    }
    if t.mode == Mode::Edit
        && let Some(b) = geo.group_bounds(t.selected)
    {
        let b = b.expand(6.0);
        dashed_rect(p, b, CREAM, 2.0);
        for c in [b.left_top(), b.right_top(), b.left_bottom(), b.right_bottom()] {
            let h = Rect::from_center_size(c, vec2(16.0, 16.0));
            p.rect_filled(h, 0.0, ORANGE);
            p.rect_stroke(h, 0.0, Stroke::new(2.0, CREAM), StrokeKind::Inside);
        }
    }
    if let Some(panel) = t.panel() {
        if t.mode != Mode::Edit {
            p.rect_filled(geo.screen, 0.0, Color32::from_rgba_unmultiplied(0, 0, 0, 110));
        }
        window(p, t, &panel);
    }
}

fn wrapped(p: &Painter, at: Pos2, text: &str, size: f32, width: f32, color: Color32) {
    let galley = p.layout(text.to_owned(), theme::game(size), color, width);
    p.galley(at, galley, color);
}

/// The cream window of the MENU, the options and the edit mode.
fn window(p: &Painter, t: &Touch, panel: &Panel) {
    let r = panel.rect;
    p.rect_filled(r, 0.0, CREAM);
    p.rect_stroke(r, 0.0, Stroke::new(4.0, ORANGE), StrokeKind::Inside);
    p.rect_stroke(r.shrink(4.0), 0.0, Stroke::new(2.0, DARK_RED), StrokeKind::Inside);
    p.text(
        r.min + vec2(16.0, 24.0),
        Align2::LEFT_CENTER,
        panel.title,
        theme::game(22.0),
        TEXT_DARK,
    );
    for b in &panel.buttons {
        let down = t.panel_down(b.id);
        let (fill, line, ink) = if down {
            (CREAM, DARK_RED, DARK_RED)
        } else if b.primary {
            (ORANGE, DARK_RED, Color32::from_rgb(0x1A, 0, 0))
        } else {
            (DARK_RED, ORANGE, CREAM)
        };
        p.rect_filled(b.rect, 0.0, fill);
        p.rect_stroke(b.rect, 0.0, Stroke::new(2.0, line), StrokeKind::Inside);
        let centred = matches!(b.id, PBtn::SizeDown | PBtn::SizeUp | PBtn::OpDown | PBtn::OpUp);
        let mut size = if centred { 22.0 } else { 17.0 };
        // Long labels shrink to fit the button.
        let probe = p.layout_no_wrap(b.label.clone(), theme::game(size), ink);
        let room = b.rect.width() - 20.0;
        if probe.size().x > room {
            size *= room / probe.size().x;
        }
        if centred {
            label(p, b.rect.center(), &b.label, size, ink);
        } else {
            p.text(
                pos2(b.rect.min.x + 10.0, b.rect.center().y),
                Align2::LEFT_CENTER,
                &b.label,
                theme::game(size),
                ink,
            );
        }
    }
    if t.mode == Mode::Edit {
        p.text(
            r.min + vec2(16.0, 46.0),
            Align2::LEFT_CENTER,
            format!("Selected: {}", t.selected.name()),
            theme::game(15.0),
            DARK_RED,
        );
        let place = t.settings.place(t.orient(), t.selected);
        let pct = place.scale.unwrap_or(100);
        bar(p, r.min + vec2(0.0, 62.0), "Size", pct, SCALE_MIN, SCALE_MAX);
        bar(p, r.min + vec2(0.0, 118.0), "Opacity", t.settings.opacity, OPACITY_MIN, OPACITY_MAX);
    }
    let note_y = match t.mode {
        Mode::Edit => r.min.y + 176.0,
        // under the last row of buttons
        _ => panel.buttons.iter().map(|b| b.rect.max.y).fold(r.min.y + 50.0, f32::max) + 8.0,
    };
    if t.mode != Mode::Edit || note_y < r.max.y {
        wrapped(p, pos2(r.min.x + 14.0, note_y), panel.note, 12.0, r.width() - 28.0, NOTE);
    }
    let _ = (Orient::Landscape, MIN_DP);
}

/// A row of the edit panel: its name, the bar between the buttons and the value.
fn bar(p: &Painter, origin: Pos2, name: &str, value: u32, min: u32, max: u32) {
    let mid = origin.y + MIN_DP / 2.0;
    p.text(pos2(origin.x + 16.0, mid), Align2::LEFT_CENTER, name, theme::game(14.0), TEXT_DARK);
    let track = Rect::from_min_size(pos2(origin.x + 140.0, mid - 5.0), vec2(120.0, 10.0));
    p.rect_filled(track, 0.0, Color32::from_rgb(0xD8, 0xC9, 0xA6));
    p.rect_stroke(track, 0.0, Stroke::new(1.0, DARK_RED), StrokeKind::Inside);
    let k = (value.clamp(min, max) - min) as f32 / (max - min) as f32;
    p.rect_filled(Rect::from_min_size(track.min, vec2(track.width() * k, 10.0)), 0.0, DARK_RED);
    p.text(
        pos2(origin.x + 324.0, mid),
        Align2::LEFT_CENTER,
        format!("{value}%"),
        theme::game(15.0),
        TEXT_DARK,
    );
}

#[allow(dead_code)]
fn _unused(_: PathStroke) {}
