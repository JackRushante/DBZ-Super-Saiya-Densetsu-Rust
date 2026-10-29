//! The save editor's screens, at the positions of the approved canvas "DBZ save editor" (the 1280 x 720 space): the slot picker,
//! the party page, the bag pages, the picker of objects, the red question and the write sheet.

use std::fmt::Write as _;

use super::{
    Ask, Card, Editor, GRID_COLUMNS, Member, OBJECTS, Page, Row, Session, Stage, delta_of,
};
use crate::draw::{Align, Color, DrawList, Op, Rect};
use crate::layout::{
    CURSOR_HEIGHT, FRAME, HEIGHT, ITEM_PORTRAIT, LARGE_TEXT, LINE, MARGIN, PORTRAIT_FRAME,
    SMALL_TEXT, WIDTH, fit, palette, text_width,
};
use crate::view::Picture;

/// The ground the editor draws on: the navy of the design, as no field was frozen under it.
const GROUND: Color = Color::rgb(0x1F, 0x2D, 0x50);
const ROW_TINT: Color = Color([0xFF, 0x52, 0x00, 40]);
const VEIL: Color = Color([0, 0, 0, 150]);
/// A figure that changed since the slot was opened.
const CHANGED: Color = Color::rgb(0xE8, 0x4A, 0x00);
const SLOT_WINDOW: Rect = Rect::new(176.0, 30.0, 928.0, 580.0);
const SLOT_HELP: Rect = Rect::new(176.0, 626.0, 928.0, 84.0);
const LEFT_WINDOW: Rect = Rect::new(176.0, 28.0, 500.0, 626.0);
pub(super) const RIGHT_WINDOW: Rect = Rect::new(696.0, 28.0, 408.0, 626.0);

/// The editor's screen now.
#[must_use]
pub fn scene(editor: &Editor) -> DrawList {
    let mut list = DrawList::new();
    list.fill(Rect::new(0.0, 0.0, WIDTH, HEIGHT), GROUND);
    let session = editor.session();
    match (editor.stage(), session) {
        (Stage::Slots, _) | (_, None) => slots(&mut list, editor),
        (stage, Some(session)) => {
            match (stage, session.page) {
                (Stage::Add | Stage::Ask(Ask::Add { .. }, _), _) => add(&mut list, editor, session),
                (_, Page::Party) => party(&mut list, editor, session),
                _ => bag(&mut list, editor, session),
            }
            match stage {
                Stage::Write(choice) => write_sheet(&mut list, editor, session, choice),
                Stage::Ask(Ask::Remove { .. }, choice) => remove_ask(&mut list, choice),
                _ => {}
            }
        }
    }
    list
}

/// The small strip over the title: where the editor lives.
#[must_use]
pub fn title_strip() -> DrawList {
    let mut list = DrawList::new();
    let rect = Rect::new(500.0, 650.0, 280.0, 46.0);
    list.push(Op::Window { rect });
    let y = rect.middle_y();
    list.text(rect.x + 24.0, y, "Select", 20.0, palette::DARK_RED, Align::Left);
    list.text(rect.x + 114.0, y, "Edit saves", 22.0, palette::BLACK, Align::Left);
    list
}

fn number(slot: usize) -> String {
    format!("{:02}", slot + 1)
}

fn tab_row(list: &mut DrawList, page: Page, left: f32, y: f32) {
    for (tab, x) in [Page::Party, Page::All, Page::Allies, Page::Tools]
        .into_iter()
        .zip([0.0, 78.0, 146.0, 235.0])
    {
        let current = tab == page;
        let color = if current { palette::DARK_RED } else { palette::GREY };
        let label = tab.label();
        list.text(left + 9.0 + x, y, label, SMALL_TEXT, color, Align::Left);
        if current {
            let width = text_width(label, SMALL_TEXT) + 8.0;
            list.fill(Rect::new(left + 5.0 + x, y + 12.0, width, LINE * 2.0), palette::ORANGE);
        }
    }
}

/// The CHEATS tag at the right end of a slot's row.
fn badge(list: &mut DrawList, right: f32, mid: f32) {
    let rect = Rect::new(right - 80.0, mid - 11.0, 80.0, 22.0);
    list.fill(rect, palette::CREAM);
    list.push(Op::Outline { rect, width: LINE, color: palette::ORANGE });
    list.text(
        rect.x + 40.0,
        rect.middle_y(),
        "CHEATS",
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Centre,
    );
}

fn slots(list: &mut DrawList, editor: &Editor) {
    list.push(Op::Window { rect: SLOT_WINDOW });
    let (left, right) = (206.0, 1074.0);
    list.text(left, 69.0, "Edit saves", LARGE_TEXT, palette::BLACK, Align::Left);
    list.text(right, 69.0, "Choose a slot", SMALL_TEXT, palette::DARK_RED, Align::Right);
    let rules = [99.0, 262.0, 421.0];
    let mids = [181.0, 341.0, 501.0];
    for rule in rules {
        list.fill(Rect::new(left, rule, right - left, LINE), palette::DARK_RED);
    }
    for (index, card) in editor.cards().iter().enumerate() {
        let mid = mids[index];
        if index == editor.cursor() {
            list.fill(Rect::new(190.0, rules[index] + 2.0, 900.0, 159.0), ROW_TINT);
            list.push(Op::Cursor { x: 194.0, y: mid - CURSOR_HEIGHT / 2.0 });
        }
        list.text(230.0, mid, &number(index), LARGE_TEXT, palette::BLACK, Align::Left);
        match card {
            Card::Empty => list.text(330.0, mid, "Empty", 26.0, palette::GREY, Align::Left),
            Card::Damaged => list.text(330.0, mid, "Damaged", 26.0, palette::GREY, Align::Left),
            Card::Game { summary, cheats } => {
                for (n, face) in summary.faces.iter().take(5).enumerate() {
                    let rect = Rect::new(330.0 + 70.0 * n as f32, mid - 30.0, 60.0, 60.0);
                    list.push(Op::Portrait { rect, picture: face.map(Picture::Face) });
                }
                if let Some(place) = summary.place {
                    list.text(720.0, mid - 12.0, place, 28.0, palette::BLACK, Align::Left);
                    list.text(
                        720.0,
                        mid + 22.0,
                        &summary.stats(),
                        SMALL_TEXT,
                        palette::DARK_RED,
                        Align::Left,
                    );
                } else {
                    list.text(
                        720.0,
                        mid,
                        &summary.stats(),
                        SMALL_TEXT,
                        palette::DARK_RED,
                        Align::Left,
                    );
                }
                if *cheats {
                    badge(list, right, mid + 44.0);
                }
            }
        }
    }
    list.push(Op::Window { rect: SLOT_HELP });
    let line = if let Some(text) = editor.notice() {
        text.to_owned()
    } else if let Some(slot) = editor.written() {
        format!("Slot {} written.", number(slot))
    } else {
        let n = number(editor.cursor());
        match editor.cards()[editor.cursor()] {
            Card::Empty => format!("Slot {n} is empty. Nothing to edit."),
            Card::Damaged => format!("Slot {n} is damaged and cannot be edited."),
            Card::Game { .. } => format!("Slot {n} holds a game. Editing changes the save itself."),
        }
    };
    list.text(left, 655.0, &fit(&line, 22.0, 868.0), 22.0, palette::BLACK, Align::Left);
    for (x, text) in [(left, "A  Edit"), (326.0, "B  Back")] {
        list.text(x, 688.0, text, SMALL_TEXT, palette::DARK_RED, Align::Left);
    }
}

fn changed_color(changed: bool) -> Color {
    if changed { CHANGED } else { palette::BLACK }
}

/// An arrow of a stepper, vertically centred on `y`.
fn arrow(list: &mut DrawList, x: f32, y: f32, left: bool) {
    list.push(Op::Arrow { x, y: y - CURSOR_HEIGHT / 2.0, left });
}

/// `value` between two arrows, centred on `centre`.
fn stepper(list: &mut DrawList, centre: f32, y: f32, value: &str, size: f32, color: Color) {
    let half = text_width(value, size) / 2.0;
    arrow(list, centre - half - 30.0, y, true);
    list.text(centre, y, value, size, color, Align::Centre);
    arrow(list, centre + half + 12.0, y, false);
}

fn party(list: &mut DrawList, editor: &Editor, session: &Session) {
    list.push(Op::Window { rect: LEFT_WINDOW });
    list.push(Op::Window { rect: RIGHT_WINDOW });
    let left = LEFT_WINDOW.x + 30.0;
    list.text(
        left,
        69.0,
        &format!("Slot {}", number(session.slot)),
        LARGE_TEXT,
        palette::BLACK,
        Align::Left,
    );
    let count = format!("{}/9", session.members.len());
    list.text(
        LEFT_WINDOW.right() - 30.0,
        69.0,
        &count,
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Right,
    );
    tab_row(list, session.page, left, 105.0);
    list.fill(Rect::new(left, 125.0, LEFT_WINDOW.w - 60.0, LINE), palette::DARK_RED);
    for (index, member) in session.members.iter().enumerate() {
        let mid = 159.0 + 50.0 * index as f32;
        let on = index == session.member;
        if on {
            list.fill(Rect::new(left - 16.0, mid - 25.0, LEFT_WINDOW.w - 28.0, 50.0), ROW_TINT);
            list.push(Op::Cursor { x: left - 12.0, y: mid - CURSOR_HEIGHT / 2.0 });
        }
        list.push(Op::Portrait {
            rect: Rect::new(left + 6.0, mid - 22.0, 44.0, 44.0),
            picture: member.face.map(Picture::Face),
        });
        let changed = member.changed();
        list.text(left + 64.0, mid - 8.0, &member.name, 26.0, palette::BLACK, Align::Left);
        let lv_color = if changed { CHANGED } else { palette::DARK_RED };
        list.text(
            left + 64.0,
            mid + 14.0,
            &format!("LV {}", member.record.level()),
            SMALL_TEXT,
            lv_color,
            Align::Left,
        );
        list.text(
            LEFT_WINDOW.right() - 30.0,
            mid,
            &format!("BP {}", member.record.bp()),
            18.0,
            if changed { CHANGED } else { palette::DARK_RED },
            Align::Right,
        );
    }
    if session.members.is_empty() {
        list.text(left + 20.0, 170.0, "Nobody in the party.", 22.0, palette::GREY, Align::Left);
    }
    for (x, text) in
        [(left, "L R  Member"), (left + 150.0, "B  Back"), (left + 270.0, "Start  Finish")]
    {
        list.text(x, 622.0, text, SMALL_TEXT, palette::DARK_RED, Align::Left);
    }
    if let Some(member) = session.members.get(session.member) {
        member_panel(list, editor, session, member);
    }
}

fn member_panel(list: &mut DrawList, editor: &Editor, session: &Session, member: &Member) {
    let x = RIGHT_WINDOW.x + 26.0;
    list.push(Op::Portrait {
        rect: Rect::new(x, 52.0, 140.0, 140.0),
        picture: member.face.map(Picture::Face),
    });
    list.text(x + 162.0, 84.0, &member.name, LARGE_TEXT, palette::BLACK, Align::Left);
    let place = format!("Party member {} of {}", session.member + 1, session.members.len());
    list.text(x + 162.0, 118.0, &place, SMALL_TEXT, palette::DARK_RED, Align::Left);
    list.text(x, 220.0, "-- Edit --", SMALL_TEXT, palette::DARK_RED, Align::Left);
    let growth = editor.labels().growth.get(usize::from(member.id)).cloned().unwrap_or_default();
    let (record, opened) = (&member.record, &member.opened);
    // The level row.
    let lv_band = Rect::new(709.0, 234.0, 382.0, 59.0);
    let bp_band = Rect::new(709.0, 309.0, 382.0, 59.0);
    let (on_lv, on_bp) = (session.row == Row::Level, session.row == Row::Bp);
    for (band, on) in [(lv_band, on_lv), (bp_band, on_bp)] {
        if on {
            list.fill(band, ROW_TINT);
            list.push(Op::Cursor { x: band.x + 5.0, y: band.y + 12.0 });
        }
    }
    list.text(x + 20.0, 258.0, "LV", LARGE_TEXT, palette::BLACK, Align::Left);
    let level_changed = record.level() != opened.level();
    if growth.has_levels() {
        stepper(
            list,
            980.0,
            258.0,
            &record.level().to_string(),
            LARGE_TEXT,
            changed_color(level_changed),
        );
        let stops = format!("stops at {}", growth.cap);
        let line = if level_changed {
            format!("was {}  \u{b7}  {stops}", opened.level())
        } else if growth.min > 1 {
            format!("starts at {}  \u{b7}  {stops}", growth.min)
        } else {
            stops
        };
        list.text(x + 20.0, 285.0, &line, SMALL_TEXT, palette::DARK_RED, Align::Left);
    } else {
        list.text(
            980.0,
            258.0,
            &record.level().to_string(),
            LARGE_TEXT,
            palette::GREY,
            Align::Centre,
        );
        list.text(x + 20.0, 285.0, "no level table: fixed", SMALL_TEXT, palette::GREY, Align::Left);
    }
    // The BP row.
    list.text(x + 20.0, 333.0, "BP", LARGE_TEXT, palette::BLACK, Align::Left);
    let bp_changed = record.bp() != opened.bp();
    stepper(list, 975.0, 333.0, &record.bp().to_string(), LARGE_TEXT, changed_color(bp_changed));
    let mut parts = Vec::new();
    if bp_changed {
        parts.push(format!("was {}", opened.bp()));
    }
    if growth.at(record.level()).is_some() {
        let delta = delta_of(&growth, record);
        parts.push(match delta {
            0 => "the game's own  (Y)".to_owned(),
            d if d > 0 => format!("+{d} over the game's  (Y)"),
            d => format!("{d} under the game's  (Y)"),
        });
    }
    // The note must stay in the window: a far delta gets the short phrase, and a line still too long is cut.
    let room = RIGHT_WINDOW.w - 52.0 - 20.0;
    let mut note = parts.join("  \u{b7}  ");
    if text_width(&note, SMALL_TEXT) > room
        && let Some(last) = parts.last_mut()
        && let Some(cut) = last.find(" over the game's").or_else(|| last.find(" under the game's"))
    {
        last.truncate(cut);
        last.push_str("  (Y)");
        note = parts.join("  \u{b7}  ");
    }
    list.text(
        x + 20.0,
        360.0,
        &fit(&note, SMALL_TEXT, room),
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    list.fill(Rect::new(x, 385.0, RIGHT_WINDOW.w - 52.0, LINE), palette::DARK_RED);
    list.text(x, 403.0, "-- Follows the level --", SMALL_TEXT, palette::DARK_RED, Align::Left);
    let next = |level: u8, exp: u32| -> Option<u32> {
        growth
            .at(level.saturating_add(1))
            .filter(|_| level < growth.cap)
            .map(|row| row.exp.saturating_sub(exp))
    };
    let rows: [(&str, String, String); 4] = [
        ("Max HP", opened.max_hp().to_string(), record.max_hp().to_string()),
        ("Max KI", opened.max_ki().to_string(), record.max_ki().to_string()),
        ("EXP", opened.exp().to_string(), record.exp().to_string()),
        (
            "To next LV",
            next(opened.level(), opened.exp()).map_or("-".to_owned(), |n| n.to_string()),
            next(record.level(), record.exp()).map_or("-".to_owned(), |n| n.to_string()),
        ),
    ];
    for (n, (label, old, new)) in rows.iter().enumerate() {
        let y = 439.0 + 39.7 * n as f32;
        list.text(x + 20.0, y, label, 28.0, palette::BLACK, Align::Left);
        if old != new {
            list.text(OLD_RIGHT, y, old, SMALL_TEXT, palette::GREY, Align::Right);
            list.text(ARROW_LEFT, y, ">", SMALL_TEXT, palette::GREY, Align::Left);
        }
        list.text(NEW_RIGHT, y, new, 28.0, palette::BLACK, Align::Right);
    }
    let note = editor.notice().unwrap_or("HP and KI are filled to the new maximums.");
    let note_color = if editor.notice().is_some() { palette::STORY } else { palette::DARK_RED };
    list.text(
        x,
        607.0,
        &fit(note, SMALL_TEXT, RIGHT_WINDOW.w - 52.0),
        SMALL_TEXT,
        note_color,
        Align::Left,
    );
    list.text(
        x,
        632.0,
        "Left Right  Change      Up Down  Row",
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
}

/// The columns of the "follows the level" rows: the old value ends at `OLD_RIGHT`, the `>` starts at `ARROW_LEFT` and the new
/// value ends at `NEW_RIGHT`; six digits of the new value (28 px) still start right of the `>`.
pub(super) const OLD_RIGHT: f32 = 954.0;
pub(super) const ARROW_LEFT: f32 = 968.0;
pub(super) const NEW_RIGHT: f32 = 1076.0;

/// The foot window of the bag pages: the keys (or the refusal) and the count of changes.
fn foot(list: &mut DrawList, editor: &Editor, keys: &str) {
    let window = Rect::new(MARGIN, 628.0, WIDTH - 2.0 * MARGIN, 56.0);
    list.push(Op::Window { rect: window });
    let y = window.middle_y();
    match editor.notice() {
        Some(text) => list.text(window.x + 28.0, y, text, 20.0, palette::STORY, Align::Left),
        None => list.text(window.x + 28.0, y, keys, 18.0, palette::BLACK, Align::Left),
    }
    let n = editor.change_count();
    let changes = match n {
        0 => String::new(),
        1 => "1 change   ".to_owned(),
        n => format!("{n} changes   "),
    };
    list.text(
        window.right() - 28.0,
        y,
        &format!("{changes}Start  Finish"),
        18.0,
        palette::BLACK,
        Align::Right,
    );
}

fn bag(list: &mut DrawList, editor: &Editor, session: &Session) {
    let window = Rect::new(MARGIN, 36.0, 640.0, 570.0);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + 22.0;
    let right = window.right() - FRAME - 22.0;
    let header = window.y + FRAME + 32.0;
    list.text(left, header, "Bag", LARGE_TEXT, palette::BLACK, Align::Left);
    let used = format!("{:02}/{:02}", session.bag.stacks().len(), session.bag.rules().slots);
    list.text(right, header, &used, SMALL_TEXT, palette::DARK_RED, Align::Right);
    tab_row(list, session.page, left - 4.0, header + 34.0);
    list.fill(Rect::new(left, header + 54.0, right - left, LINE), palette::DARK_RED);
    let stacks = editor.stacks(session.page);
    let column = (right - left - 30.0) / 2.0;
    let first = header + 54.0 + 14.0 + 23.0;
    let per_page = 8;
    let page = session.cursor / per_page;
    for index in 0..per_page {
        let Some(stack) = stacks.get(page * per_page + index) else { continue };
        let (col, row) = (index / 4, index % 4);
        let x = left + col as f32 * (column + 30.0);
        let mid = first + row as f32 * 46.0;
        let on = page * per_page + index == session.cursor;
        let story = super::story(editor.labels(), stack.object);
        let changed = session.opened_bag.stack(stack.id).is_none_or(|old| old.count != stack.count);
        let text_color = if story { palette::STORY } else { palette::BLACK };
        let count_color = if changed { CHANGED } else { text_color };
        if on {
            list.fill(Rect::new(x - 6.0, mid - 22.0, column + 12.0, 44.0), ROW_TINT);
            list.push(Op::Cursor { x, y: mid - CURSOR_HEIGHT / 2.0 });
        }
        let units = format!("\u{d7}{:02}", stack.count);
        let room = column - 26.0 - 100.0;
        let name = fit(&super::object_name(editor.labels(), stack.object), LARGE_TEXT, room);
        list.text(
            x + 26.0,
            mid,
            &name,
            LARGE_TEXT,
            if on && changed { CHANGED } else { text_color },
            Align::Left,
        );
        if on && stack.class.stackable {
            arrow(list, x + column - 74.0, mid, true);
            list.text(x + column - 38.0, mid, &units, SMALL_TEXT, count_color, Align::Centre);
            arrow(list, x + column - 20.0, mid, false);
        } else {
            list.text(x + column, mid, &units, SMALL_TEXT, count_color, Align::Right);
        }
    }
    if stacks.is_empty() {
        list.text(left + 26.0, first, "Nothing here.", LARGE_TEXT, palette::GREY, Align::Left);
    }
    let pages = stacks.len().div_ceil(per_page).max(1);
    list.text(
        right,
        window.bottom() - FRAME - 24.0,
        &format!("Page {}/{pages}", page + 1),
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Right,
    );
    // The stack under the cursor.
    let x = WIDTH - MARGIN - 538.0;
    let picture = Rect::new(x, 36.0, ITEM_PORTRAIT, ITEM_PORTRAIT);
    let selected = stacks.get(session.cursor).copied();
    list.push(Op::Portrait { rect: picture, picture: selected.map(|s| Picture::Object(s.object)) });
    let count_box = Rect::new(x, picture.bottom() + 4.0, ITEM_PORTRAIT, 46.0);
    list.fill(count_box, palette::PRICE);
    list.push(Op::Outline { rect: count_box, width: PORTRAIT_FRAME, color: palette::BLACK });
    if let Some(stack) = selected {
        let single = !stack.class.stackable;
        if single {
            list.text(
                count_box.x + 75.0,
                count_box.middle_y(),
                "\u{d7}01",
                LARGE_TEXT,
                palette::PRICE_DIGITS,
                Align::Centre,
            );
        } else {
            arrow(list, count_box.x + 14.0, count_box.middle_y(), true);
            list.text(
                count_box.x + 75.0,
                count_box.middle_y(),
                &format!("{:02}", stack.count),
                LARGE_TEXT,
                palette::PRICE_DIGITS,
                Align::Centre,
            );
            arrow(list, count_box.right() - 30.0, count_box.middle_y(), false);
        }
    }
    let help =
        Rect::new(picture.right() + 22.0, 36.0, WIDTH - MARGIN - (picture.right() + 22.0), 240.0);
    list.push(Op::Window { rect: help });
    if let Some(stack) = selected {
        let label = editor.labels().objects.get(usize::from(stack.object));
        let hx = help.x + FRAME + 22.0;
        let room = help.w - 2.0 * (FRAME + 22.0);
        let top = help.y + FRAME + 16.0 + LARGE_TEXT / 2.0;
        list.text(
            hx,
            top,
            &fit(&super::object_name(editor.labels(), stack.object), LARGE_TEXT, room),
            LARGE_TEXT,
            palette::BLACK,
            Align::Left,
        );
        if let Some(label) = label {
            for (n, line) in label.help.iter().take(3).enumerate() {
                list.text(
                    hx,
                    top + 48.0 * (n + 1) as f32,
                    &fit(line, LARGE_TEXT, room),
                    LARGE_TEXT,
                    palette::BLACK,
                    Align::Left,
                );
            }
        }
        let was = session
            .opened_bag
            .stack(stack.id)
            .map_or_else(|| "new".to_owned(), |old| format!("was \u{d7}{:02}", old.count));
        list.text(
            hx,
            help.bottom() - FRAME - 24.0,
            &format!("Stack {} of {}  \u{b7}  {was}", stack.count, session.bag.rules().stack),
            SMALL_TEXT,
            palette::DARK_RED,
            Align::Left,
        );
    }
    for (rect, text) in [
        (Rect::new(x, 301.0, 262.0, 46.0), "X  Add object"),
        (Rect::new(x + 273.0, 301.0, 265.0, 46.0), "Y  Remove stack"),
    ] {
        list.fill(rect, palette::CREAM);
        list.push(Op::Outline { rect, width: 3.0, color: palette::ORANGE });
        list.text(
            rect.x + rect.w / 2.0,
            rect.middle_y(),
            text,
            24.0,
            palette::BLACK,
            Align::Centre,
        );
    }
    foot(list, editor, "Left Right  Count      X  Add      Y  Remove      L R  Tab      B  Back");
}

const CELL: f32 = 81.0;
const GRID: (f32, f32) = (70.0, 90.0);

fn add(list: &mut DrawList, editor: &Editor, session: &Session) {
    let left = Rect::new(MARGIN, 32.0, 700.0, 576.0);
    list.push(Op::Window { rect: left });
    list.text(left.x + 26.0, 70.0, "-- Add object --", SMALL_TEXT, palette::DARK_RED, Align::Left);
    list.text(
        left.right() - 26.0,
        70.0,
        "\u{d7}N in the bag",
        SMALL_TEXT,
        palette::GREY,
        Align::Right,
    );
    let labels = editor.labels();
    for object in 0..OBJECTS {
        let (col, row) = (object % GRID_COLUMNS, object / GRID_COLUMNS);
        let cell = Rect::new(GRID.0 + CELL * col as f32, GRID.1 + CELL * row as f32, CELL, CELL);
        list.push(Op::Portrait {
            rect: Rect::new(cell.x + 2.0, cell.y + 2.0, CELL - 4.0, CELL - 4.0),
            picture: Some(Picture::Object(object as u8)),
        });
        if super::story(labels, object as u8) {
            list.push(Op::Outline { rect: cell, width: 3.0, color: palette::STORY });
        }
        let held = session.bag.units_of(object as u8);
        if held > 0 {
            let tag = Rect::new(cell.right() - 44.0, cell.bottom() - 22.0, 42.0, 20.0);
            list.fill(tag, palette::BLACK);
            list.text(
                tag.x + 21.0,
                tag.middle_y(),
                &format!("\u{d7}{held:02}"),
                14.0,
                palette::WHITE,
                Align::Centre,
            );
        }
    }
    let cursor = Rect::new(
        GRID.0 + CELL * (session.pick % GRID_COLUMNS) as f32,
        GRID.1 + CELL * (session.pick / GRID_COLUMNS) as f32,
        CELL,
        CELL,
    );
    list.push(Op::Outline { rect: cursor, width: 5.0, color: palette::ORANGE });
    list.text(
        left.x + 26.0,
        left.bottom() - 40.0,
        "Red frame: the story checks this object.   Orange: under the cursor.",
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    // The object under the cursor.
    let object = session.pick as u8;
    let label = labels.objects.get(usize::from(object));
    let top = Rect::new(760.0, 32.0, 480.0, 390.0);
    list.push(Op::Window { rect: top });
    let x = top.x + 30.0;
    list.text(x, 70.0, "-- Add to the bag --", SMALL_TEXT, palette::DARK_RED, Align::Left);
    list.push(Op::Portrait {
        rect: Rect::new(x, 96.0, 140.0, 140.0),
        picture: Some(Picture::Object(object)),
    });
    let tx = x + 162.0;
    let story = super::story(labels, object);
    let name_color = if story { palette::STORY } else { palette::BLACK };
    list.text(
        tx,
        122.0,
        &fit(&super::object_name(labels, object), LARGE_TEXT, top.right() - 30.0 - tx),
        LARGE_TEXT,
        name_color,
        Align::Left,
    );
    if let Some(label) = label {
        for (n, line) in label.help.iter().take(3).enumerate() {
            list.text(
                tx,
                166.0 + 40.0 * n as f32,
                &fit(line, 26.0, top.right() - 30.0 - tx),
                26.0,
                palette::BLACK,
                Align::Left,
            );
        }
    }
    let held = session.bag.units_of(object);
    let will = (held + u32::from(session.how_many)).min(u32::from(super::STACK_MAX));
    list.text(
        x,
        290.0,
        &format!("In the bag: \u{d7}{held:02}  >  \u{d7}{will:02}"),
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    list.text(x, 362.0, "How many", LARGE_TEXT, palette::BLACK, Align::Left);
    stepper(list, 1130.0, 362.0, &session.how_many.to_string(), LARGE_TEXT, palette::BLACK);
    let max = if super::class_of(labels, object).stackable { "max 99" } else { "one at a time" };
    list.text(top.right() - 22.0, 400.0, max, SMALL_TEXT, palette::GREY, Align::Right);
    // The bag line, or the red question.
    let low = Rect::new(760.0, 438.0, 480.0, 170.0);
    list.push(Op::Window { rect: low });
    if let Stage::Ask(Ask::Add { .. }, choice) = editor.stage() {
        story_warning(list, low, "Adding", "Add anyway", choice);
    } else {
        let lx = low.x + 30.0;
        list.text(lx, 474.0, "-- Bag --", SMALL_TEXT, palette::DARK_RED, Align::Left);
        let used = format!(
            "{:02} of {} stacks used.",
            session.bag.stacks().len(),
            session.bag.rules().slots
        );
        list.text(lx, 508.0, &used, SMALL_TEXT, palette::BLACK, Align::Left);
        list.text(
            lx,
            536.0,
            "Added units join the stack of the object,",
            SMALL_TEXT,
            palette::BLACK,
            Align::Left,
        );
        list.text(lx, 560.0, "or take a free slot.", SMALL_TEXT, palette::BLACK, Align::Left);
    }
    let window = Rect::new(MARGIN, 630.0, WIDTH - 2.0 * MARGIN, 58.0);
    list.push(Op::Window { rect: window });
    let y = window.middle_y();
    match editor.notice() {
        Some(text) => list.text(window.x + 28.0, y, text, 20.0, palette::STORY, Align::Left),
        None => {
            let keys = if matches!(editor.stage(), Stage::Ask(..)) {
                "A  Choose     Left Right  Add anyway or Cancel     B  Back"
            } else {
                "A  Add     Left Right  How many     Up Down  L R  Pick     Y  Back to 1     B  Back to the bag"
            };
            list.text(window.x + 28.0, y, keys, 17.0, palette::BLACK, Align::Left);
        }
    }
}

/// The red warning of the picker and the question of a removal: the window's inside goes pink, the title, three lines (15 px,
/// the longest 397 px: they fit the 420 px of the picker's window) and the two chips, with the cursor on `choice` (0 the
/// go-on chip, 1 Cancel). `verb` is what the story objects must not suffer ("Adding", "Removing").
fn story_warning(list: &mut DrawList, window: Rect, verb: &str, go_on: &str, choice: u8) {
    let inner = window.grow(-(FRAME + 6.0));
    list.fill(inner, palette::WARNING);
    let x = window.x + 30.0;
    list.text(x, window.y + 34.0, "! STORY ITEM", LARGE_TEXT, palette::STORY, Align::Left);
    let lines = [
        format!("The story checks this object. {verb} it can"),
        "block a scene or run one twice. Save a copy".to_owned(),
        "of the slot first: B on the sheet writes nothing.".to_owned(),
    ];
    for (n, line) in lines.iter().enumerate() {
        list.text(x, window.y + 64.0 + 20.0 * n as f32, line, 15.0, palette::BLACK, Align::Left);
    }
    for (n, label) in [go_on, "Cancel"].into_iter().enumerate() {
        let chip = Rect::new(
            window.right() - 30.0 - 360.0 + 190.0 * n as f32,
            window.bottom() - 48.0,
            170.0,
            36.0,
        );
        let on = n as u8 == choice;
        list.fill(chip, if on { palette::ORANGE } else { palette::WHITE });
        list.push(Op::Outline { rect: chip, width: LINE, color: palette::DARK_RED });
        list.text(
            chip.x + chip.w / 2.0,
            chip.middle_y(),
            label,
            20.0,
            if on { palette::CREAM } else { palette::BLACK },
            Align::Centre,
        );
    }
}

/// The question before a story stack is removed.
fn remove_ask(list: &mut DrawList, choice: u8) {
    list.fill(Rect::new(0.0, 0.0, WIDTH, HEIGHT), VEIL);
    let window = Rect::new(380.0, 230.0, 520.0, 200.0);
    list.push(Op::Window { rect: window });
    story_warning(list, window, "Removing", "Remove anyway", choice);
}

fn write_sheet(list: &mut DrawList, editor: &Editor, session: &Session, choice: u8) {
    list.fill(Rect::new(0.0, 0.0, WIDTH, HEIGHT), VEIL);
    let window = Rect::new(320.0, 150.0, 640.0, 400.0);
    list.push(Op::Window { rect: window });
    let x = window.x + 38.0;
    list.text(
        x,
        193.0,
        &format!("Write the changes to slot {}?", number(session.slot)),
        LARGE_TEXT,
        palette::BLACK,
        Align::Left,
    );
    list.fill(Rect::new(x, 224.0, window.w - 76.0, LINE), palette::DARK_RED);
    // Up to three summary lines (18 px, 25 apart): the changed members, then the bag. A fourth member or more is
    // counted on the last party line, so the lines never reach the note below them.
    let (changed, added, removed) = editor.bag_changes(session);
    let bag_changed = (changed, added, removed) != (0, 0, 0);
    let mut party: Vec<String> = session
        .members
        .iter()
        .filter(|m| m.changed())
        .map(|member| {
            let (new, old) = (&member.record, &member.opened);
            let mut line = member.name.clone();
            if new.level() != old.level() {
                let _ = write!(line, " LV {} > {}", old.level(), new.level());
            }
            if new.bp() != old.bp() {
                let _ = write!(line, " (BP {} > {})", old.bp(), new.bp());
            }
            format!("Party: {line}")
        })
        .collect();
    let room = if bag_changed { 2 } else { 3 };
    if party.len() > room {
        let more = party.len() - (room - 1);
        party.truncate(room - 1);
        party.push(format!("Party: {more} more members changed"));
    }
    let mut lines = party;
    if bag_changed {
        let mut parts = Vec::new();
        if changed > 0 {
            parts.push(format!("{changed} stack{} changed", if changed == 1 { "" } else { "s" }));
        }
        if added > 0 {
            parts.push(format!("{added} added"));
        }
        if removed > 0 {
            parts.push(format!("{removed} removed"));
        }
        lines.push(format!("Bag: {}", parts.join(", ")));
    }
    for (n, line) in lines.iter().enumerate() {
        list.text(
            x,
            247.0 + 25.0 * n as f32,
            &fit(line, 18.0, window.w - 76.0),
            18.0,
            palette::DARK_RED,
            Align::Left,
        );
    }
    // The note, in two lines (each 16 px line is under the sheet's 564 px).
    let slot = number(session.slot);
    for (n, line) in [
        format!("Writes slot {slot} and its backup copy; the game, the"),
        "bag, cheats and options of the slot keep their link.".to_owned(),
    ]
    .iter()
    .enumerate()
    {
        list.text(x, 332.0 + 22.0 * n as f32, line, SMALL_TEXT, palette::DARK_RED, Align::Left);
    }
    let rows = [
        format!("Write to slot {}", number(session.slot)),
        "Discard the changes".to_owned(),
        "Keep editing".to_owned(),
    ];
    for (n, text) in rows.iter().enumerate() {
        let mid = 396.0 + 46.0 * n as f32;
        if n as u8 == choice {
            list.fill(Rect::new(window.x + 20.0, mid - 22.0, window.w - 40.0, 44.0), ROW_TINT);
            list.push(Op::Cursor { x: window.x + 32.0, y: mid - CURSOR_HEIGHT / 2.0 });
        }
        list.text(window.x + 74.0, mid, text, LARGE_TEXT, palette::BLACK, Align::Left);
    }
}
