//! The rows window of the Controls page (the approved design): the device, then the SNES buttons and the
//! shortcuts as key boxes, two to a line. The categories window, the help strip and the questions are the
//! Options screen's own (see [`crate::screens::options_screen`]).

use crate::draw::{Align, DrawList, Op, Rect};
use crate::layout::{FRAME, LINE, SMALL_TEXT, palette, text_width};
use crate::options::controls::{self, ACTION_DEFS, ACTIONS, BUTTONS, Device};
use crate::view::OptionsView;

const TOP: f32 = 32.0;
const HEIGHT: f32 = 522.0;
const LEFT_EDGE: f32 = 362.0;
const WIDTH: f32 = 878.0;
const SIDE: f32 = 18.0;
const PAD_TOP: f32 = 12.0;
const GROUP: f32 = 28.0;
const ROW: f32 = 40.0;
const GAP: f32 = 2.0;
const COLUMN_GAP: f32 = 24.0;
const LABEL_TEXT: f32 = 20.0;
const GROUP_TEXT: f32 = 15.0;
const BOX_HEIGHT: f32 = 34.0;
const BOX_LEAST: f32 = 150.0;
const BOX_PADDING: f32 = 20.0;
const DEVICE_BUTTON_HEIGHT: f32 = 40.0;
const TINT: crate::draw::Color = crate::draw::Color([0xFF, 0x52, 0x00, 36]);

/// Draws the rows window of the Controls page for `view` (its values are the codes of the 35 rows).
pub(crate) fn rows(list: &mut DrawList, view: &OptionsView) {
    let window = Rect::new(LEFT_EDGE, TOP, WIDTH, HEIGHT);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + SIDE;
    let right = window.right() - FRAME - SIDE;
    let column = (right - left - COLUMN_GAP) / 2.0;
    let mut y = window.y + FRAME + PAD_TOP;
    let at = position(view.row);
    let device = Device::from_index(view.values.first().copied().unwrap_or(0));

    let page = view.values.first().copied().unwrap_or(0);

    // The device row, alone on its line.
    let middle = y + ROW / 2.0;
    cursor_row(list, left, right, y, at == 0);
    list.text(left + 26.0, middle, "Device", LABEL_TEXT, palette::BLACK, Align::Left);
    let widths = [136.0, 120.0, 100.0, 100.0];
    let mut x = right - widths.iter().sum::<f32>() - 6.0;
    for (index, (text, width)) in
        ["Keyboard", "Gamepad", "Touch", "Turbo"].into_iter().zip(widths).enumerate()
    {
        let rect = Rect::new(x, middle - DEVICE_BUTTON_HEIGHT / 2.0, width, DEVICE_BUTTON_HEIGHT);
        if index == page {
            list.fill(rect, palette::ORANGE);
            list.push(Op::Outline { rect, width: LINE, color: palette::DARK_RED });
        }
        list.text(rect.x + width / 2.0, middle, text, SMALL_TEXT, palette::BLACK, Align::Centre);
        x += width + 2.0;
    }
    y += ROW + GAP;

    if let Some((first, count)) = controls::list_rows(page) {
        // The touch controls' or the turbo rows: a list under the device row, drawn like any page's rows. The view's
        // cursor is a row of the table, so the generic list tints the row it is on.
        crate::screens::option_row_list(list, view, first..first + count, left, right, y);
        return;
    }

    // The two groups of the SNES buttons: the d-pad and the shoulders on the left, the buttons on the right.
    for (index, title) in ["D-pad and shoulders", "Buttons"].into_iter().enumerate() {
        group(list, left + index as f32 * (column + COLUMN_GAP), y, title);
    }
    y += GROUP + GAP;
    for line in 0..BUTTONS / 2 {
        key_line(list, view, device, (left, right, column), y, line);
        y += ROW + GAP;
    }
    group(list, left, y, "Shortcuts");
    y += GROUP + GAP;
    for line in BUTTONS / 2..ACTIONS.div_ceil(2) {
        key_line(list, view, device, (left, right, column), y, line);
        y += ROW + GAP;
    }
}

/// The place of the cursor in the grid (0 the device row, 1 to 17 the actions) of a row of the table.
fn position(row: usize) -> usize {
    if row == 0 { 0 } else { (row - 1) % ACTIONS + 1 }
}

/// The group's title at the left of a column.
fn group(list: &mut DrawList, left: f32, y: f32, title: &str) {
    list.text(
        left + 26.0,
        y + GROUP - 9.0,
        &format!("-- {title} --"),
        GROUP_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
}

/// The tint and the arrow of the row under the cursor.
fn cursor_row(list: &mut DrawList, left: f32, right: f32, y: f32, under: bool) {
    if under {
        list.fill(Rect::new(left, y, right - left, ROW), TINT);
        list.push(Op::Cursor { x: left, y: y + ROW / 2.0 - crate::layout::CURSOR_HEIGHT / 2.0 });
    }
}

/// One line of two key boxes: actions `2 * line` and `2 * line + 1`.
fn key_line(
    list: &mut DrawList,
    view: &OptionsView,
    device: Device,
    (left, right, column): (f32, f32, f32),
    y: f32,
    line: usize,
) {
    let at = position(view.row);
    for side in 0..2 {
        let action = line * 2 + side;
        if action >= ACTIONS {
            break;
        }
        let x = left + side as f32 * (column + COLUMN_GAP);
        let edge = if side == 0 { x + column } else { right };
        let middle = y + ROW / 2.0;
        // The cell's own tint and arrow, so the cursor shows in its half of the line only.
        cursor_row(list, x, edge, y, at == action + 1);
        list.text(
            x + 26.0,
            middle,
            ACTION_DEFS[action].label,
            LABEL_TEXT,
            palette::BLACK,
            Align::Left,
        );
        let row = controls::row_of(device, action);
        let code = view.values.get(row).copied().unwrap_or(0);
        let code = u8::try_from(code).unwrap_or(0);
        let refused = view
            .controls
            .as_ref()
            .and_then(|state| state.refused.as_ref())
            .filter(|refusal| refusal.position == action + 1);
        let listening =
            at == action + 1 && view.controls.as_ref().is_some_and(|state| state.listening);
        let text = if refused.is_some() {
            "Key in use".to_owned()
        } else if listening {
            match device {
                Device::Keyboard => "Press a key...".to_owned(),
                Device::Gamepad => "Press a button...".to_owned(),
            }
        } else {
            controls::name(device, code)
        };
        let width = (text_width(&text, SMALL_TEXT) + BOX_PADDING).max(BOX_LEAST);
        let rect = Rect::new(edge - width, middle - BOX_HEIGHT / 2.0, width, BOX_HEIGHT);
        let (fill, line_width) = match (refused, listening) {
            (Some(refusal), _) => (if refusal.blink { palette::ORANGE } else { palette::CAP }, 3.0),
            (None, true) => (palette::CAP, LINE),
            (None, false) => (palette::WHITE, LINE),
        };
        list.fill(rect, fill);
        list.push(Op::Outline { rect, width: line_width, color: palette::DARK_RED });
        list.text(rect.x + width / 2.0, middle, &text, SMALL_TEXT, palette::BLACK, Align::Centre);
    }
}
