//! The HD screens: the field's command window with its help, the Menu list, and the item list with
//! its detail. Each builds a [`DrawList`] from a [`MenuView`] at the positions of the approved design.
//! The positions are the design's; the text they hold is the game's names and our own English help.

use crate::cheats::Row;
use crate::draw::{Align, Color, DrawList, Op, Rect};
use crate::layout::{
    ADVANCE, BAR_HEIGHT, COMMAND_ROW, CORNER_PORTRAIT, CURSOR_HEIGHT, CURSOR_WIDTH, FRAME, HEIGHT,
    HELP_LINE, ITEM_PORTRAIT, ITEM_ROW, LARGE_TEXT, LINE, MARGIN, PADDING, PORTRAIT_FRAME,
    SMALL_TEXT, WIDTH, fit, palette, text_width,
};
use crate::nav::{Entry, MENU_ROWS, ModsAsk, command_rows};
use crate::options::{Category, Kind};
use crate::view::{
    BagView, CheatsView, MenuView, ModsView, OptionsView, PackState, Picture, Screen, Selected,
    Sheet, SpawnerView, Tab, VideoAsk,
};

/// The items shown on one page of the item list.
pub const PAGE_SIZE: usize = 8;
/// The capacity the item list shows in its header: the original list's 24 entries.
pub const CAPACITY: usize = 24;
/// The space a name keeps from the text beside it (the count of a stack, the figures of a party member).
const NAME_GAP: f32 = 12.0;
/// The figures of a party member on the Use sheet: small enough that a name of the party fits beside them.
const FIGURE_TEXT: f32 = 14.0;
/// Distance of the corner pictures of the field menu from the sides of the space.
const CORNER_INSET: f32 = 32.0;
/// Distance of the corner pictures of the field menu from the top.
const CORNER_TOP: f32 = 28.0;

/// The rows of the command window of `view`: the field's, a town's or the flying ones.
#[must_use]
pub fn commands(view: &MenuView) -> &'static [Entry] {
    command_rows(view.flying, view.town)
}

/// The draw list of the HD screen of `view`. The cheats screen has its own view: see [`cheats_screen`].
#[must_use]
pub fn scene(view: &MenuView) -> DrawList {
    match view.screen {
        Screen::Command => command(view),
        Screen::Menu => menu_list(view),
        Screen::Items => items(view),
        Screen::Cheats | Screen::Options | Screen::Mods => DrawList::new(),
    }
}

/// The field's command window, with the party's corners and the money.
#[must_use]
pub fn command(view: &MenuView) -> DrawList {
    let mut list = DrawList::new();
    backdrop(&mut list);
    party(&mut list, view);
    zeni(&mut list, view.zeni);
    let rows = commands(view);
    list_window(&mut list, rows, view.cursor);
    help_window(&mut list, rows, view.cursor);
    list
}

/// The Menu list: the party's corners and the money as on the field, then the list of Cards, Status,
/// Order, Text and Save, with Cheats and Options greyed out.
#[must_use]
pub fn menu_list(view: &MenuView) -> DrawList {
    let mut list = DrawList::new();
    backdrop(&mut list);
    party(&mut list, view);
    zeni(&mut list, view.zeni);
    list_window(&mut list, &MENU_ROWS, view.cursor);
    help_window(&mut list, &MENU_ROWS, view.cursor);
    list
}

/// The bag: its stacks in two columns, the tabs, the sheet over the list and the object under the
/// cursor with its picture, price and help. Without a bag, the original's entries.
#[must_use]
pub fn items(view: &MenuView) -> DrawList {
    let mut list = DrawList::new();
    backdrop(&mut list);
    match &view.bag {
        Some(bag) => {
            bag_window(&mut list, view, bag);
            zeni_window(&mut list, view.zeni);
            detail(&mut list, view.selected.as_ref());
            if let Some(sheet) = sheet_window(bag) {
                sheet_draw(&mut list, &sheet);
            }
        }
        None => {
            bag_entries(&mut list, view);
            zeni_window(&mut list, view.zeni);
            detail(&mut list, view.selected.as_ref());
        }
    }
    list
}

/// The ground of the screens: the field as it was when the menu opened, which the app paints.
fn backdrop(list: &mut DrawList) {
    list.push(Op::Backdrop);
}

/// The party's corner pictures with their HP and KI bars, in party order (two corners).
fn party(list: &mut DrawList, view: &MenuView) {
    for (member, &x) in view.party.iter().take(2).zip(CORNERS.iter()) {
        let frame = Rect::new(x, CORNER_TOP, CORNER_PORTRAIT, CORNER_PORTRAIT);
        list.push(Op::Portrait { rect: frame, picture: member.face.map(Picture::Face) });
        let hp = frame.bottom() + 14.0;
        let ki = hp + 20.0;
        gauge_row(
            list,
            GaugeRow { x, y: hp, width: frame.w, label: "HP", fill: palette::HP },
            (member.hp, member.max_hp),
        );
        gauge_row(
            list,
            GaugeRow { x, y: ki, width: frame.w, label: "KI", fill: palette::KI },
            (member.ki, member.max_ki),
        );
    }
}

/// Where the corner pictures of the field menu sit: the left one and the right one, 28 px down.
const CORNERS: [f32; 2] = [CORNER_INSET, WIDTH - CORNER_INSET - CORNER_PORTRAIT];

/// One row of a member's gauges: where it starts, how wide the column is, its label and its fill.
struct GaugeRow {
    x: f32,
    y: f32,
    width: f32,
    label: &'static str,
    fill: Color,
}

/// A member's HP or KI under the corner picture: its label, in white with a black shadow, and its
/// bar. `amount` is the value and the maximum.
fn gauge_row(list: &mut DrawList, row: GaugeRow, amount: (u16, u16)) {
    let GaugeRow { x, y, width, label, fill } = row;
    list.text(x + 2.0, y + 2.0, label, SMALL_TEXT, palette::BLACK, Align::Left);
    list.text(x, y, label, SMALL_TEXT, palette::WHITE, Align::Left);
    let (value, max) = amount;
    let fraction = if max == 0 { 0.0 } else { (f32::from(value) / f32::from(max)).min(1.0) };
    let bar = Rect::new(x + 28.0, y - BAR_HEIGHT / 2.0, width - 28.0, BAR_HEIGHT);
    list.push(Op::Bar { rect: bar, fraction, color: fill });
}

/// The money, centred at the top of the field menu.
fn zeni(list: &mut DrawList, zeni: u16) {
    let window = Rect::new((WIDTH - 240.0) / 2.0, 36.0, 240.0, 72.0);
    list.push(Op::Window { rect: window });
    let text = format!("Zeni {zeni:03}P");
    list.text(
        window.x + window.w / 2.0,
        window.middle_y(),
        &text,
        LARGE_TEXT,
        palette::BLACK,
        Align::Centre,
    );
}

/// A list window at the bottom left, one row per entry, with the cursor on the row under it. An
/// entry that is not enabled is greyed out and says it is coming soon.
fn list_window(list: &mut DrawList, rows: &[Entry], cursor: u8) {
    let height = 2.0 * FRAME + 2.0 * 14.0 + COMMAND_ROW * rows.len() as f32;
    // A greyed entry carries its "coming soon" note after its label: the window makes room for it.
    let width = if rows.iter().any(|entry| !entry.enabled()) { 360.0 } else { 220.0 };
    let window = Rect::new(MARGIN, HEIGHT - MARGIN - height, width, height);
    list.push(Op::Window { rect: window });
    let first = window.y + FRAME + 14.0;
    for (row, entry) in rows.iter().enumerate() {
        let middle = first + COMMAND_ROW * row as f32 + COMMAND_ROW / 2.0;
        if usize::from(cursor) == row {
            list.push(Op::Cursor { x: window.x + FRAME + 14.0, y: middle - CURSOR_HEIGHT / 2.0 });
        }
        let colour = if entry.enabled() { palette::BLACK } else { palette::GREY };
        list.text(
            window.x + FRAME + 14.0 + CURSOR_WIDTH + 10.0,
            middle,
            entry.label(),
            LARGE_TEXT,
            colour,
            Align::Left,
        );
        if !entry.enabled() {
            list.text(
                window.right() - FRAME - 14.0,
                middle,
                "coming soon",
                SMALL_TEXT,
                palette::GREY,
                Align::Right,
            );
        }
    }
}

/// The help window at the bottom right: the help of the entry under the cursor.
fn help_window(list: &mut DrawList, rows: &[Entry], cursor: u8) {
    let help = rows.get(usize::from(cursor)).map_or("", |entry| entry.help());
    let height = help_height(1);
    let window = Rect::new(WIDTH - MARGIN - 560.0, HEIGHT - MARGIN - height, 560.0, height);
    list.push(Op::Window { rect: window });
    list.text(
        window.x + FRAME + 22.0,
        window.y + FRAME + PADDING + HELP_LINE / 2.0,
        help,
        LARGE_TEXT,
        palette::BLACK,
        Align::Left,
    );
    hints(list, window);
}

/// The height of a help window with `lines` lines of text.
#[must_use]
fn help_height(lines: usize) -> f32 {
    2.0 * FRAME + 2.0 * PADDING + HELP_LINE * lines.max(1) as f32 + 8.0 + SMALL_TEXT
}

/// The key hints along the bottom of a help window.
fn hints(list: &mut DrawList, window: Rect) {
    let y = window.bottom() - FRAME - PADDING - SMALL_TEXT / 2.0;
    for (i, hint) in ["A  OK", "B  Back", "Start  Close"].iter().enumerate() {
        list.text(
            window.x + FRAME + 22.0 + 120.0 * i as f32,
            y,
            hint,
            SMALL_TEXT,
            palette::DARK_RED,
            Align::Left,
        );
    }
}

/// The bag window: the header with the slots in use, the tabs, the stacks of the page in two
/// columns with their counts, and the page with the notice at its foot.
fn bag_window(list: &mut DrawList, view: &MenuView, bag: &BagView) {
    let window = Rect::new(MARGIN, 36.0, 640.0, 570.0);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + 22.0;
    let right = window.right() - FRAME - 22.0;
    let header = window.y + FRAME + 16.0 + 16.0;
    list.text(left, header, "Items", LARGE_TEXT, palette::BLACK, Align::Left);
    let count = format!("{:02}/{:02}", bag.used, bag.slots);
    list.text(right, header, &count, SMALL_TEXT, palette::DARK_RED, Align::Right);
    // The tabs: the current one is marked with an orange bar.
    let tabs_y = header + 34.0;
    let mut tab_x = left;
    for tab in [Tab::All, Tab::Allies, Tab::Tools] {
        let current = tab == bag.tab;
        let label = tab.label();
        let width = 40.0 + 16.0 * label.len() as f32;
        let colour = if current { palette::DARK_RED } else { palette::GREY };
        list.text(tab_x + 8.0, tabs_y, label, SMALL_TEXT, colour, Align::Left);
        if current {
            list.fill(Rect::new(tab_x, tabs_y + 12.0, width - 16.0, LINE * 2.0), palette::ORANGE);
        }
        tab_x += width;
    }
    let rule_y = tabs_y + 20.0;
    list.fill(Rect::new(left, rule_y, right - left, LINE), palette::DARK_RED);

    let column_width = (right - left - 30.0) / 2.0;
    let first_row = rule_y + 14.0 + ITEM_ROW / 2.0;
    let page = usize::from(view.page);
    for index in 0..PAGE_SIZE {
        let column = index / 4;
        let row = index % 4;
        let x = left + column as f32 * (column_width + 30.0);
        let middle = first_row + row as f32 * ITEM_ROW;
        let Some(stack) = bag.stacks.get(page * PAGE_SIZE + index) else { continue };
        if usize::from(view.cursor) == index {
            list.push(Op::Cursor { x, y: middle - CURSOR_HEIGHT / 2.0 });
        }
        let colour = if stack.usable { palette::BLACK } else { palette::GREY };
        let units = format!("\u{d7}{:02}", stack.count);
        // The name stops before the count, which is right-aligned at the end of the column.
        let room = column_width - 26.0 - text_width(&units, SMALL_TEXT) - NAME_GAP;
        let name = fit(&stack.name, LARGE_TEXT, room);
        list.text(x + 26.0, middle, &name, LARGE_TEXT, colour, Align::Left);
        list.text(x + column_width, middle, &units, SMALL_TEXT, colour, Align::Right);
    }
    let pages = bag.stacks.len().div_ceil(PAGE_SIZE).max(1);
    let footer_y = window.bottom() - FRAME - PADDING - SMALL_TEXT / 2.0;
    let footer = format!("Page {}/{pages}", page + 1);
    list.text(right, footer_y, &footer, SMALL_TEXT, palette::DARK_RED, Align::Right);
    match (bag.notice, bag.full) {
        (Some(notice), _) => {
            list.text(left, footer_y, notice, SMALL_TEXT, palette::DARK_RED, Align::Left);
        }
        (None, true) => list.text(
            left,
            footer_y,
            "Bag full: nothing more can be taken.",
            SMALL_TEXT,
            palette::DARK_RED,
            Align::Left,
        ),
        (None, false) => {}
    }
}

/// The original's entries, when there is no bag yet: one row each, the same window.
fn bag_entries(list: &mut DrawList, view: &MenuView) {
    let window = Rect::new(MARGIN, 36.0, 640.0, 570.0);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + 22.0;
    let right = window.right() - FRAME - 22.0;
    let header = window.y + FRAME + 16.0 + 16.0;
    list.text(left, header, "Items", LARGE_TEXT, palette::BLACK, Align::Left);
    let count = format!("{:02}/{CAPACITY}", view.items.len());
    list.text(right, header, &count, SMALL_TEXT, palette::DARK_RED, Align::Right);
    list.fill(Rect::new(left, header + 26.0, right - left, LINE), palette::DARK_RED);
    let column_width = (right - left - 30.0) / 2.0;
    let first_row = header + 26.0 + 14.0 + ITEM_ROW / 2.0;
    let page = usize::from(view.page);
    for index in 0..PAGE_SIZE {
        let column = index / 4;
        let row = index % 4;
        let x = left + column as f32 * (column_width + 30.0);
        let middle = first_row + row as f32 * ITEM_ROW;
        let Some(slot) = view.items.get(page * PAGE_SIZE + index) else { continue };
        if usize::from(view.cursor) == index {
            list.push(Op::Cursor { x, y: middle - CURSOR_HEIGHT / 2.0 });
        }
        let name = fit(&slot.name, LARGE_TEXT, column_width - 26.0 - NAME_GAP);
        list.text(x + 26.0, middle, &name, LARGE_TEXT, palette::BLACK, Align::Left);
    }
    let pages = view.items.len().div_ceil(PAGE_SIZE).max(1);
    let footer = format!("Page {}/{pages}", page + 1);
    let footer_y = window.bottom() - FRAME - PADDING - SMALL_TEXT / 2.0;
    list.text(right, footer_y, &footer, SMALL_TEXT, palette::DARK_RED, Align::Right);
}

/// A sheet over the list: its frame, its title and its rows, with the cursor on one of them.
struct SheetWindow {
    rect: Rect,
    title: String,
    /// Each row: its label, its figures (the use-on list's LV, HP, KI, BP) and whether it is enabled.
    rows: Vec<(String, Option<String>, bool)>,
    cursor: Option<usize>,
    /// Distance between two rows: [`SHEET_ROW`], or less when the rows would run past the space.
    pitch: f32,
}

/// Top of the sheets, under the detail's picture and help.
const SHEET_TOP: f32 = 300.0;
/// Distance between two rows of a sheet.
const SHEET_ROW: f32 = 40.0;

/// The sheet open on the bag, if one is: Use or Drop, the party's use-on list, or Drop's Yes and No.
/// It takes the place of the detail's help, on the right.
fn sheet_window(bag: &BagView) -> Option<SheetWindow> {
    let x = 702.0;
    let width = 538.0;
    let (title, rows, cursor) = match bag.sheet {
        Sheet::Browse => return None,
        Sheet::Choose => (
            "Use or drop?".to_owned(),
            vec![
                ("Use".to_owned(), None, bag.usable_here),
                ("Drop".to_owned(), None, bag.droppable_here),
            ],
            Some(usize::from(bag.choice)),
        ),
        Sheet::Confirm => (
            "Drop one?".to_owned(),
            vec![("Yes".to_owned(), None, true), ("No".to_owned(), None, true)],
            Some(usize::from(bag.choice)),
        ),
    };
    // Above the rows: the frame, the padding, the title and its gap.
    let head = 2.0 * FRAME + 2.0 * PADDING + LARGE_TEXT + 18.0;
    // The rows are 40 px apart; a party too long for the space below the detail takes closer rows,
    // so that the nine records a party can have still end above the margin.
    let room = HEIGHT - MARGIN - SHEET_TOP - head;
    let pitch = (room / rows.len().max(1) as f32).min(SHEET_ROW);
    let height = (head + pitch * rows.len() as f32).max(150.0);
    Some(SheetWindow { rect: Rect::new(x, SHEET_TOP, width, height), title, rows, cursor, pitch })
}

/// Paints a sheet: its frame, its title, and its rows with the cursor on the chosen one.
fn sheet_draw(list: &mut DrawList, window: &SheetWindow) {
    list.push(Op::Window { rect: window.rect });
    let left = window.rect.x + FRAME + 22.0;
    let top = window.rect.y + FRAME + PADDING;
    list.text(left, top + LARGE_TEXT / 2.0, &window.title, LARGE_TEXT, palette::BLACK, Align::Left);
    for (row, (label, figures, enabled)) in window.rows.iter().enumerate() {
        let middle = top + LARGE_TEXT + 18.0 + window.pitch * row as f32 + 20.0;
        if window.cursor == Some(row) {
            list.push(Op::Cursor { x: left - 4.0, y: middle - CURSOR_HEIGHT / 2.0 });
        }
        let colour = if *enabled { palette::BLACK } else { palette::GREY };
        let name_left = left + CURSOR_WIDTH + 10.0;
        let right = window.rect.right() - FRAME - 22.0;
        // The figures take the end of the row, and the name stops before them.
        let figures_width = figures.as_deref().map_or(0.0, |text| text_width(text, FIGURE_TEXT));
        let name = fit(label, LARGE_TEXT, right - figures_width - NAME_GAP - name_left);
        list.text(name_left, middle, &name, LARGE_TEXT, colour, Align::Left);
        if let Some(figures) = figures {
            list.text(right, middle, figures, FIGURE_TEXT, palette::DARK_RED, Align::Right);
        }
    }
}

/// The money at the bottom left, under the bag.
fn zeni_window(list: &mut DrawList, zeni: u16) {
    let window = Rect::new(MARGIN, 628.0, 640.0, 56.0);
    list.push(Op::Window { rect: window });
    list.text(
        window.x + FRAME + 20.0,
        window.middle_y(),
        "Zeni",
        LARGE_TEXT,
        palette::BLACK,
        Align::Left,
    );
    let amount = format!("{zeni:03}P");
    list.text(
        window.right() - FRAME - 20.0,
        window.middle_y(),
        &amount,
        LARGE_TEXT,
        palette::BLACK,
        Align::Right,
    );
}

/// The object under the cursor: its picture and price box on the left, its help on the right.
fn detail(list: &mut DrawList, selected: Option<&Selected>) {
    let x = WIDTH - MARGIN - 538.0;
    let picture = Rect::new(x, 36.0, ITEM_PORTRAIT, ITEM_PORTRAIT);
    list.push(Op::Portrait { rect: picture, picture: selected.map(|s| Picture::Object(s.object)) });
    let price = Rect::new(x, picture.bottom() + 4.0, ITEM_PORTRAIT, 46.0);
    list.fill(price, palette::PRICE);
    list.push(Op::Outline { rect: price, width: PORTRAIT_FRAME, color: palette::BLACK });
    // An object that is not sold shows no figure and no unit, as in the original (its box holds
    // a small kanji there instead); an empty list shows an empty box
    if let Some(figure) = selected.and_then(|s| s.price) {
        // The figure ends a 4 px gap before the unit, which ends 10 px from the box's edge.
        let unit_left = price.right() - 10.0 - text_width("Pt", SMALL_TEXT);
        list.text(
            unit_left - 4.0,
            price.middle_y(),
            &figure.to_string(),
            LARGE_TEXT,
            palette::PRICE_DIGITS,
            Align::Right,
        );
        list.text(
            price.right() - 10.0,
            price.middle_y(),
            "Pt",
            SMALL_TEXT,
            palette::PRICE_DIGITS,
            Align::Right,
        );
    }

    let Some(selected) = selected else { return };
    let window =
        Rect::new(picture.right() + 22.0, 36.0, WIDTH - MARGIN - (picture.right() + 22.0), 240.0);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + 22.0;
    // Every line of the panel stops at its inner edge.
    let room = window.right() - FRAME - 22.0 - left;
    let top = window.y + FRAME + PADDING + LARGE_TEXT / 2.0;
    let name = fit(&selected.name, LARGE_TEXT, room);
    list.text(left, top, &name, LARGE_TEXT, palette::BLACK, Align::Left);
    for (line, text) in selected.help.iter().take(3).enumerate() {
        let text = fit(text, LARGE_TEXT, room);
        list.text(
            left,
            top + HELP_LINE * (line + 1) as f32,
            &text,
            LARGE_TEXT,
            palette::BLACK,
            Align::Left,
        );
    }
    let usable = match (selected.field, selected.battle) {
        (true, true) => "Usable: on the map and in battle",
        (true, false) => "Usable: on the map",
        (false, true) => "Usable: in battle",
        (false, false) => "Not usable from the menu",
    };
    list.text(
        left,
        window.bottom() - FRAME - PADDING - SMALL_TEXT / 2.0,
        &fit(usable, SMALL_TEXT, room),
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
}

/// The grid of the Cheats screen, in logical pixels, as the approved design sets it: two panels of
/// [`CHEATS_COLUMN`] px and the preview column of [`CHEATS_SIDE`] px, [`CHEATS_GAP`] px apart, from
/// [`CHEATS_EDGE`] px below the top of the space to the help window at its foot.
const CHEATS_COLUMN: f32 = 443.0;
const CHEATS_SIDE: f32 = 270.0;
const CHEATS_GAP: f32 = 22.0;
const CHEATS_EDGE: f32 = 32.0;
/// Height of the help window at the foot.
const CHEATS_HELP: f32 = 58.0;
/// Height of the window of the next card: its title, the card and its caption.
const CHEATS_CARD: f32 = 286.0;
/// The panels' padding: 12 px above their first group, 18 px at the sides.
const PANEL_TOP: f32 = 12.0;
const PANEL_SIDE: f32 = 18.0;
/// A group's title: 8 px above it, 15 px high, 2 px below it.
const GROUP_TITLE_TOP: f32 = 8.0;
const GROUP_TITLE_HEIGHT: f32 = 18.0;
const GROUP_TITLE_BELOW: f32 = 2.0;
/// The text sizes of the screen, as the design sets them.
const GROUP_TEXT: f32 = 15.0;
const ROW_TEXT: f32 = 18.0;
const CAPTION_TEXT: f32 = 14.0;
/// A row of a panel is at least 44 px high; the rows and the titles are 2 px apart.
const ROW_HEIGHT: f32 = 44.0;
const LINE_GAP: f32 = 2.0;
/// The choice buttons are 40 px high and at least 40 px wide (their text has 4 px of padding each side); a
/// cycler's arrows sit in 44 px boxes, 4 px from the value.
const CHOICE_HEIGHT: f32 = 40.0;
const CHOICE_MIN: f32 = 40.0;
const CHOICE_PADDING: f32 = 8.0;
const ARROW_BOX: f32 = 44.0;
const CYCLER_GAP: f32 = 4.0;
/// The value of a cycler is at least 120 px wide.
const CYCLER_VALUE: f32 = 120.0;
/// The card of the preview: 124 x 184 px with a 6 px white border; its balls are 42 px across.
const CARD_WIDTH: f32 = 124.0;
const CARD_HEIGHT: f32 = 184.0;
const CARD_BORDER: f32 = 6.0;
const BALL: f32 = 42.0;
/// The window's inner padding: 16 px at the sides, 14 px above and below.
const WINDOW_SIDE: f32 = 16.0;
const WINDOW_TOP: f32 = 14.0;
/// The characters a line of the captions and of the On list's empty text holds: a line of 14 px text is about
/// 15 characters in the side column, 16 px text about 14.
const CAPTION_CHARS: usize = 15;
const EMPTY_CHARS: usize = 14;
/// The Reset all button: 44 px high, at the foot of the On window.
const RESET_HEIGHT: f32 = 44.0;

/// The Cheats screen: the choices of the run in two panels (Battle and Cards on the left, Resources and
/// Encounters in the middle), the preview of the next card, the On list with Reset all on the right, and the
/// help of the row under the cursor at the foot. The choice that is on is orange with white text; the row
/// under the cursor has the orange cursor. Everything lies inside the space.
#[must_use]
pub fn cheats_screen(view: &CheatsView) -> DrawList {
    if let Some(spawner) = &view.spawner {
        return spawner_screen(spawner);
    }
    let mut list = DrawList::new();
    backdrop(&mut list);
    let bottom = HEIGHT - CHEATS_EDGE - CHEATS_HELP - CHEATS_GAP;
    let height = bottom - CHEATS_EDGE;
    let left = Rect::new(MARGIN, CHEATS_EDGE, CHEATS_COLUMN, height);
    let middle = Rect::new(left.right() + CHEATS_GAP, CHEATS_EDGE, CHEATS_COLUMN, height);
    let side_x = middle.right() + CHEATS_GAP;
    let card = Rect::new(side_x, CHEATS_EDGE, CHEATS_SIDE, CHEATS_CARD);
    let on = Rect::new(
        side_x,
        card.bottom() + CHEATS_GAP,
        CHEATS_SIDE,
        bottom - card.bottom() - CHEATS_GAP,
    );
    cheats_panel(
        &mut list,
        view,
        left,
        &[&[Row::Damage, Row::Defence, Row::Exp], &[Row::ZCards, Row::ZFor]],
    );
    cheats_panel(
        &mut list,
        view,
        middle,
        &[
            &[Row::MaxZeni, Row::ZeniGained, Row::EndlessItems, Row::ItemSpawner],
            &[Row::RandomBattles, Row::HowOften, Row::Enemies, Row::EnemyPower],
            &[Row::UnlockChapters],
        ],
    );
    next_card(&mut list, view, card);
    on_window(&mut list, view, on);
    help_strip(&mut list, view.row);
    list
}

/// The width of the item spawner's list.
const SPAWNER_LIST: f32 = 600.0;
/// The height of the item spawner's object window (the warning's window is under it).
const SPAWNER_OBJECT: f32 = 330.0;

/// The item spawner: the page of objects (story objects in red, with their tag), the object under the cursor with its
/// help and how many to add, the story warning while it asks, and the keys.
fn spawner_screen(view: &SpawnerView) -> DrawList {
    let mut list = DrawList::new();
    backdrop(&mut list);
    let bottom = HEIGHT - CHEATS_EDGE - CHEATS_HELP - CHEATS_GAP;
    let left = Rect::new(MARGIN, CHEATS_EDGE, SPAWNER_LIST, bottom - CHEATS_EDGE);
    list.push(Op::Window { rect: left });
    let x = left.x + FRAME + PANEL_SIDE;
    let right = left.right() - FRAME - PANEL_SIDE;
    let title = left.y + FRAME + PANEL_TOP + GROUP_TITLE_TOP + GROUP_TITLE_HEIGHT / 2.0;
    list.text(x + 26.0, title, "-- Item spawner --", GROUP_TEXT, palette::DARK_RED, Align::Left);
    list.text(right, title, "In bag", GROUP_TEXT, palette::GREY, Align::Right);
    let mut y = title + GROUP_TITLE_HEIGHT / 2.0 + GROUP_TITLE_BELOW + LINE_GAP;
    for row in &view.rows {
        let middle = y + ROW_HEIGHT / 2.0;
        if row.object == view.object {
            list.fill(
                Rect::new(x + 22.0, y + 3.0, right - x - 22.0, ROW_HEIGHT - 6.0),
                palette::CAP,
            );
            list.push(Op::Cursor { x, y: middle - CURSOR_HEIGHT / 2.0 });
        }
        let colour = if row.story { palette::STORY } else { palette::BLACK };
        list.text(x + 34.0, middle, &row.name, ROW_TEXT, colour, Align::Left);
        if row.story {
            let tag = Rect::new(x + 230.0, middle - 12.0, 80.0, 24.0);
            list.fill(tag, palette::STORY);
            list.text(
                tag.x + tag.w / 2.0,
                middle,
                "STORY",
                SMALL_TEXT,
                palette::WHITE,
                Align::Centre,
            );
        }
        list.text(
            right - 20.0,
            middle,
            &row.in_bag.to_string(),
            ROW_TEXT,
            palette::GREY,
            Align::Right,
        );
        y += ROW_HEIGHT;
    }
    // The object under the cursor.
    let side_x = left.right() + CHEATS_GAP;
    let side_w = WIDTH - MARGIN - side_x;
    let object = Rect::new(side_x, CHEATS_EDGE, side_w, SPAWNER_OBJECT);
    list.push(Op::Window { rect: object });
    let ox = object.x + FRAME + PANEL_SIDE;
    let oright = object.right() - FRAME - PANEL_SIDE;
    let mut oy = object.y + FRAME + PANEL_TOP + GROUP_TITLE_TOP + GROUP_TITLE_HEIGHT / 2.0;
    list.text(ox + 8.0, oy, "-- Add to the bag --", GROUP_TEXT, palette::DARK_RED, Align::Left);
    oy += 40.0;
    let colour = if view.story { palette::STORY } else { palette::BLACK };
    list.text(ox + 8.0, oy, &view.name, LARGE_TEXT, colour, Align::Left);
    oy += 42.0;
    for line in view.help.iter().take(3) {
        list.text(ox + 8.0, oy, line, ROW_TEXT, palette::BLACK, Align::Left);
        oy += 28.0;
    }
    let count_y = object.bottom() - FRAME - 50.0;
    list.text(ox + 8.0, count_y, "How many", ROW_TEXT, palette::BLACK, Align::Left);
    let limit = if view.story { "one at a time" } else { "max 99" };
    list.text(oright, count_y, limit, SMALL_TEXT, palette::GREY, Align::Right);
    cycler(&mut list, &view.count.to_string(), oright - 150.0, count_y, 60.0);
    // The story warning, or the note.
    let note = Rect::new(
        side_x,
        object.bottom() + CHEATS_GAP,
        side_w,
        bottom - object.bottom() - CHEATS_GAP,
    );
    list.push(Op::Window { rect: note });
    let nx = note.x + FRAME + PANEL_SIDE;
    let nright = note.right() - FRAME - PANEL_SIDE;
    let mut ny = note.y + FRAME + PANEL_TOP + GROUP_TITLE_TOP + GROUP_TITLE_HEIGHT / 2.0;
    if let Some(choice) = view.confirm {
        let inner = Rect::new(
            note.x + FRAME + 6.0,
            note.y + FRAME + 6.0,
            note.w - 2.0 * FRAME - 12.0,
            note.h - 2.0 * FRAME - 12.0,
        );
        list.fill(inner, palette::WARNING);
        list.text(nx + 8.0, ny + 6.0, "! STORY ITEM", LARGE_TEXT, palette::STORY, Align::Left);
        ny += 44.0;
        for line in [
            "Spawning it can break the story completely:",
            "an event may never happen, or happen twice.",
            "Save the game first.",
        ] {
            list.text(nx + 8.0, ny, line, SMALL_TEXT, palette::BLACK, Align::Left);
            ny += 24.0;
        }
        let texts = ["Add anyway".to_owned(), "Cancel".to_owned()];
        choices(
            &mut list,
            &texts,
            usize::from(choice),
            nright,
            note.bottom() - FRAME - 34.0,
            palette::WHITE,
        );
    } else {
        list.text(nx + 8.0, ny, "-- Note --", GROUP_TEXT, palette::DARK_RED, Align::Left);
        ny += 34.0;
        for line in
            ["A game that uses the spawner gets", "the CHEATS badge, as the other", "cheats do."]
        {
            list.text(nx + 8.0, ny, line, SMALL_TEXT, palette::BLACK, Align::Left);
            ny += 24.0;
        }
    }
    // The keys.
    let strip =
        Rect::new(MARGIN, HEIGHT - CHEATS_EDGE - CHEATS_HELP, WIDTH - 2.0 * MARGIN, CHEATS_HELP);
    list.push(Op::Window { rect: strip });
    let keys = if view.confirm.is_some() {
        "A  choose   Left Right  Add anyway or Cancel"
    } else {
        "A  add   Left Right  how many   L R  page   Y  back to 1"
    };
    list.text(
        strip.x + FRAME + 20.0,
        strip.middle_y(),
        keys,
        ROW_TEXT,
        palette::BLACK,
        Align::Left,
    );
    let back = strip.right() - FRAME - 20.0;
    list.text(back, strip.middle_y(), "B  Back", SMALL_TEXT, palette::DARK_RED, Align::Right);
    list
}

/// The frame of the Data packs screen (the approved canvas, in the space of the screen): the list window on the left,
/// the details window on the right, and the help strip along the foot.
const MODS_EDGE: f32 = 30.0;
const MODS_TOP: f32 = 30.0;
const MODS_BOTTOM: f32 = 630.0;
const MODS_GAP: f32 = 20.0;
const MODS_LIST: f32 = 620.0;
/// The height of a row of the list, and the top of the first one under the window's top.
const MODS_ROW: f32 = 50.0;
const MODS_ROWS_TOP: f32 = 62.0;
/// The side of a pack's on and off box.
const MODS_BOX: f32 = 22.0;
/// The text of a pack's name and of a detail's title, and of the other lines of the details.
const MODS_NAME_TEXT: f32 = 22.0;
const MODS_LINE_TEXT: f32 = 18.0;
/// The characters a line of the details holds (at the line's size, inside the details window).
const MODS_DETAIL_CHARS: usize = 50;

/// The Data packs screen: the packs with their boxes and versions, then the chapters of the packs that are on, on the left;
/// the details of the line under the cursor and the notice on the right; the keys along the foot. A question (Apply, a
/// chapter's start, Main game) is drawn over both, with the cursor on its choice.
#[must_use]
pub fn mods_screen(view: &ModsView) -> DrawList {
    let mut list = DrawList::new();
    backdrop(&mut list);
    let left = Rect::new(MODS_EDGE, MODS_TOP, MODS_LIST, MODS_BOTTOM - MODS_TOP);
    let right_x = left.right() + MODS_GAP;
    let right = Rect::new(right_x, MODS_TOP, WIDTH - MODS_EDGE - right_x, MODS_BOTTOM - MODS_TOP);
    mods_list(&mut list, view, left);
    mods_details(&mut list, view, right);
    let strip = Rect::new(MODS_EDGE, MODS_BOTTOM + 15.0, WIDTH - 2.0 * MODS_EDGE, 60.0);
    list.push(Op::Window { rect: strip });
    list.text(
        strip.x + FRAME + 20.0,
        strip.middle_y(),
        "A on/off   Y + Up/Down move (order)   X Apply",
        MODS_LINE_TEXT + 1.0,
        palette::BLACK,
        Align::Left,
    );
    list.text(
        strip.right() - FRAME - 20.0,
        strip.middle_y(),
        "B Back",
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Right,
    );
    if let Some((ask, choice)) = view.ask {
        mods_question(&mut list, ask, choice);
    }
    list
}

/// The list window: the packs (a box, the name, the version; a refused pack in red with a cross), "Install a pack...", then
/// the chapters under their header, each a name, grey when it is locked; the cursor on the line under it; and the note and
/// Apply at the foot.
fn mods_list(list: &mut DrawList, view: &ModsView, window: Rect) {
    list.push(Op::Window { rect: window });
    let x = window.x + FRAME + 16.0;
    list.text(
        x + 10.0,
        window.y + 36.0,
        "-- Data packs --",
        MODS_LINE_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    list.text(window.x + 390.0, window.y + 36.0, "Version", SMALL_TEXT, palette::GREY, Align::Left);
    let mut line = 0;
    for pack in &view.packs {
        let middle = window.y + MODS_ROWS_TOP + MODS_ROW * line as f32 + 13.0;
        mods_row_band(list, view, line, window, middle);
        let refused = pack.state == PackState::Refused;
        let box_rect = Rect::new(window.x + 50.0, middle - MODS_BOX / 2.0, MODS_BOX, MODS_BOX);
        list.push(Op::Outline { rect: box_rect, width: LINE, color: palette::DARK_RED });
        match pack.state {
            PackState::On => {
                list.fill(box_rect.grow(-5.0), palette::ORANGE);
            }
            PackState::Off => {}
            PackState::Refused => {
                let (a, b) = (box_rect.grow(-4.0), box_rect);
                list.push(Op::Line {
                    from: (a.x, a.y),
                    to: (a.right(), a.bottom()),
                    width: 3.0,
                    color: palette::STORY,
                });
                list.push(Op::Line {
                    from: (a.right(), a.y),
                    to: (a.x, a.bottom()),
                    width: 3.0,
                    color: palette::STORY,
                });
                list.push(Op::Outline { rect: b, width: LINE, color: palette::STORY });
            }
        }
        let colour = if refused { palette::STORY } else { palette::BLACK };
        list.text(
            window.x + 86.0,
            middle,
            &fit(&pack.name, MODS_NAME_TEXT, 290.0),
            MODS_NAME_TEXT,
            colour,
            Align::Left,
        );
        list.text(
            window.x + 390.0,
            middle,
            &pack.version,
            MODS_NAME_TEXT,
            palette::GREY,
            Align::Left,
        );
        line += 1;
    }
    let middle = window.y + MODS_ROWS_TOP + MODS_ROW * line as f32 + 13.0;
    mods_row_band(list, view, view.install_line(), window, middle);
    list.text(
        window.x + 50.0,
        middle,
        "+ Install a pack...",
        MODS_NAME_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    line += 1;
    let mut chapter_top = window.y + MODS_ROWS_TOP + MODS_ROW * line as f32;
    if !view.chapters.is_empty() {
        list.text(
            x + 10.0,
            chapter_top + 10.0,
            "-- Chapters --",
            MODS_LINE_TEXT,
            palette::DARK_RED,
            Align::Left,
        );
        chapter_top += 34.0;
    }
    for (at, chapter) in view.chapters.iter().enumerate() {
        let index = view.install_line() + 1 + at;
        let middle = chapter_top + MODS_ROW * at as f32 + 13.0;
        mods_row_band(list, view, index, window, middle);
        let colour = if chapter.locked { palette::GREY } else { palette::BLACK };
        list.text(
            window.x + 50.0,
            middle,
            &fit(&chapter.name, MODS_NAME_TEXT, 420.0),
            MODS_NAME_TEXT,
            colour,
            Align::Left,
        );
        if chapter.locked {
            list.text(
                window.right() - 24.0,
                middle,
                "locked",
                SMALL_TEXT,
                palette::GREY,
                Align::Right,
            );
        }
    }
    list.text(
        window.x + 40.0,
        window.bottom() - 100.0,
        "Classic is never changed by packs.",
        SMALL_TEXT,
        palette::GREY,
        Align::Left,
    );
    let apply = Rect::new(window.x + 40.0, window.bottom() - 72.0, 230.0, 28.0);
    list.push(Op::Outline { rect: apply, width: LINE, color: palette::DARK_RED });
    list.text(
        apply.x + 115.0,
        apply.middle_y(),
        "Apply",
        SMALL_TEXT,
        palette::BLACK,
        Align::Centre,
    );
}

/// The band and the cursor of a list line (the pale one under the cursor).
fn mods_row_band(list: &mut DrawList, view: &ModsView, index: usize, window: Rect, middle: f32) {
    if view.cursor != index {
        return;
    }
    list.fill(
        Rect::new(window.x + 40.0, middle - 19.0, window.w - 70.0, 38.0),
        Color::rgb(0xFF, 0xD9, 0xA8),
    );
    list.push(Op::Cursor { x: window.x + 14.0, y: middle - CURSOR_HEIGHT / 2.0 });
}

/// The details window: the pack under the cursor (its name, author, description, what it needs, its size, and why it is
/// refused), or the chapter under it (its name, description and the Start button), and the notice at its foot.
fn mods_details(list: &mut DrawList, view: &ModsView, window: Rect) {
    list.push(Op::Window { rect: window });
    let x = window.x + 40.0;
    list.text(x, window.y + 36.0, "-- Details --", MODS_LINE_TEXT, palette::DARK_RED, Align::Left);
    let mut lines: Vec<(String, f32, Color)> = Vec::new();
    let mut button = None;
    if let Some(pack) = view.packs.get(view.cursor) {
        lines.push((
            fit(&format!("{}  {}", pack.name, pack.version), MODS_NAME_TEXT, 500.0),
            MODS_NAME_TEXT,
            palette::BLACK,
        ));
        if !pack.author.is_empty() {
            lines.push((format!("by {}", pack.author), MODS_LINE_TEXT, palette::GREY));
        }
        lines.push((String::new(), MODS_LINE_TEXT, palette::BLACK));
        for line in wrap(&pack.description, MODS_DETAIL_CHARS) {
            lines.push((line, MODS_LINE_TEXT, palette::BLACK));
        }
        lines.push((String::new(), MODS_LINE_TEXT, palette::BLACK));
        lines.push((format!("Needs: {}", pack.needs), MODS_LINE_TEXT, palette::BLACK));
        lines.push((
            format!("Size: {} KB", pack.size.div_ceil(1024)),
            MODS_LINE_TEXT,
            palette::BLACK,
        ));
        if let Some(reason) = &pack.reason {
            lines.push((String::new(), MODS_LINE_TEXT, palette::BLACK));
            lines.push(("REFUSED:".to_owned(), MODS_LINE_TEXT, palette::STORY));
            for line in wrap(reason, MODS_DETAIL_CHARS) {
                lines.push((line, MODS_LINE_TEXT, palette::BLACK));
            }
            lines.push((String::new(), MODS_LINE_TEXT, palette::BLACK));
            for line in wrap(
                "It stays off. Remove it or ask its author for a new version.",
                MODS_DETAIL_CHARS,
            ) {
                lines.push((line, MODS_LINE_TEXT, palette::BLACK));
            }
        }
    } else if view.cursor == view.install_line() {
        lines.push(("Install a pack".to_owned(), MODS_NAME_TEXT, palette::BLACK));
        lines.push((String::new(), MODS_LINE_TEXT, palette::BLACK));
        for line in wrap(
            "A picks a pack file (.ssdpack) and copies it into the mods folder. It is listed off: switch it on and apply.",
            MODS_DETAIL_CHARS,
        ) {
            lines.push((line, MODS_LINE_TEXT, palette::BLACK));
        }
    } else if let Some(chapter) = view.chapter_at_cursor() {
        lines.push((fit(&chapter.name, MODS_NAME_TEXT, 500.0), MODS_NAME_TEXT, palette::BLACK));
        lines.push((String::new(), MODS_LINE_TEXT, palette::BLACK));
        for line in wrap(&chapter.description, MODS_DETAIL_CHARS) {
            lines.push((line, MODS_LINE_TEXT, palette::BLACK));
        }
        if chapter.locked {
            lines.push((String::new(), MODS_LINE_TEXT, palette::BLACK));
            lines.push((
                "Locked: the game must be finished first.".to_owned(),
                MODS_LINE_TEXT,
                palette::GREY,
            ));
        } else {
            button = Some(if chapter.main { "Go" } else { "Start" });
        }
    }
    let mut y = window.y + 80.0;
    for (text, size, colour) in &lines {
        list.text(x, y, text, *size, *colour, Align::Left);
        y += 26.0;
    }
    if let Some(label) = button {
        let rect = Rect::new(x, y + 14.0, 150.0, 36.0);
        list.push(Op::Outline { rect, width: LINE, color: palette::DARK_RED });
        list.text(
            rect.x + 75.0,
            rect.middle_y(),
            label,
            MODS_LINE_TEXT,
            palette::BLACK,
            Align::Centre,
        );
    }
    if let Some(notice) = &view.notice {
        let texts = wrap(notice, MODS_DETAIL_CHARS);
        let mut at = window.bottom() - 30.0 - 26.0 * texts.len() as f32;
        for text in texts {
            list.text(x, at, &text, MODS_LINE_TEXT, palette::STORY, Align::Left);
            at += 26.0;
        }
    }
}

/// The question over the Data packs screen: its title and lines, its two buttons (the confirm one red, Cancel outlined), and
/// the cursor on the choice.
fn mods_question(list: &mut DrawList, ask: ModsAsk, choice: u8) {
    let window = Rect::new(330.0, 230.0, 620.0, 240.0);
    list.push(Op::Window { rect: window });
    let (title, lines, confirm): (&str, &[&str], &str) = match ask {
        ModsAsk::Apply => (
            "Apply the packs?",
            &["The game goes back to the title screen.", "Save first: unsaved progress is lost."],
            "Apply",
        ),
        ModsAsk::Chapter(_) => {
            ("Start this chapter?", &["The game goes to the chapter's own saves."], "Start")
        }
        ModsAsk::Main => ("Back to the main game?", &["The game goes to the main saves."], "Go"),
    };
    list.text(window.x + 40.0, window.y + 34.0, title, LARGE_TEXT, palette::BLACK, Align::Left);
    for (at, line) in lines.iter().enumerate() {
        list.text(
            window.x + 40.0,
            window.y + 80.0 + 30.0 * at as f32,
            line,
            MODS_LINE_TEXT,
            palette::BLACK,
            Align::Left,
        );
    }
    let confirm_rect = Rect::new(window.x + 230.0, window.y + 180.0, 160.0, 36.0);
    let cancel_rect = Rect::new(window.x + 410.0, window.y + 180.0, 160.0, 36.0);
    list.fill(confirm_rect, palette::STORY);
    list.text(
        confirm_rect.x + 80.0,
        confirm_rect.middle_y(),
        confirm,
        MODS_LINE_TEXT,
        palette::WHITE,
        Align::Centre,
    );
    list.push(Op::Outline { rect: cancel_rect, width: LINE, color: palette::DARK_RED });
    list.text(
        cancel_rect.x + 80.0,
        cancel_rect.middle_y(),
        "Cancel",
        MODS_LINE_TEXT,
        palette::BLACK,
        Align::Centre,
    );
    let chosen = if choice == 0 { confirm_rect } else { cancel_rect };
    list.push(Op::Cursor { x: chosen.x + 12.0, y: chosen.middle_y() - CURSOR_HEIGHT / 2.0 });
}

/// A panel of choices: its window, and each group with its title and its rows.
fn cheats_panel(list: &mut DrawList, view: &CheatsView, window: Rect, groups: &[&[Row]]) {
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + PANEL_SIDE;
    let right = window.right() - FRAME - PANEL_SIDE;
    let mut y = window.y + FRAME + PANEL_TOP;
    for rows in groups {
        y += GROUP_TITLE_TOP;
        let title = format!("-- {} --", rows[0].group());
        let middle = y + GROUP_TITLE_HEIGHT / 2.0;
        list.text(left + 26.0, middle, &title, GROUP_TEXT, palette::DARK_RED, Align::Left);
        y += GROUP_TITLE_HEIGHT + GROUP_TITLE_BELOW + LINE_GAP;
        for &row in *rows {
            cheat_row(list, view, row, left, right, y + ROW_HEIGHT / 2.0);
            y += ROW_HEIGHT + LINE_GAP;
        }
    }
}

/// One row of a panel: the cursor when it is under it, the label, and the row's choices at its right end (a
/// cycler for the "For" row).
fn cheat_row(list: &mut DrawList, view: &CheatsView, row: Row, left: f32, right: f32, middle: f32) {
    if view.row == row {
        list.push(Op::Cursor { x: left, y: middle - CURSOR_HEIGHT / 2.0 });
    }
    list.text(left + 26.0, middle, row.label(), ROW_TEXT, palette::BLACK, Align::Left);
    let texts: Vec<String> =
        (0..row.choices()).map(|choice| row.choice_text(choice, &view.names)).collect();
    let chosen = row.index(&view.cheats).min(texts.len().saturating_sub(1));
    if row == Row::ZFor {
        cycler(list, &texts[chosen], right, middle, CYCLER_VALUE);
    } else {
        choices(list, &texts, chosen, right, middle, palette::WHITE);
    }
}

/// The choices of a row, from `right` leftwards, each in a button as wide as its text needs (at least
/// [`CHOICE_MIN`]). The chosen one is orange with `on_colour` text (white on the Cheats screen, black on the Options) and a dark-red line.
fn choices(
    list: &mut DrawList,
    texts: &[String],
    chosen: usize,
    right: f32,
    middle: f32,
    on_colour: Color,
) {
    let widths: Vec<f32> = texts
        .iter()
        .map(|text| (SMALL_TEXT * text.chars().count() as f32 + CHOICE_PADDING).max(CHOICE_MIN))
        .collect();
    let total = widths.iter().sum::<f32>() + LINE_GAP * widths.len().saturating_sub(1) as f32;
    let mut x = right - total;
    for (index, (text, width)) in texts.iter().zip(widths).enumerate() {
        let rect = Rect::new(x, middle - CHOICE_HEIGHT / 2.0, width, CHOICE_HEIGHT);
        let on = index == chosen;
        if on {
            list.fill(rect, palette::ORANGE);
            list.push(Op::Outline { rect, width: LINE, color: palette::DARK_RED });
        }
        let colour = if on { on_colour } else { palette::BLACK };
        list.text(rect.x + width / 2.0, middle, text, SMALL_TEXT, colour, Align::Centre);
        x += width + LINE_GAP;
    }
}

/// A cycler: an arrow to the left, the value between two 4 px gaps, and an arrow to the right, the value
/// at least [`CYCLER_VALUE`] px wide and the whole ending at `right`.
fn cycler(list: &mut DrawList, value: &str, right: f32, middle: f32, least: f32) {
    let width = (ROW_TEXT * value.chars().count() as f32).max(least);
    let left = right - (2.0 * ARROW_BOX + 2.0 * CYCLER_GAP + width);
    let inset = (ARROW_BOX - CURSOR_WIDTH) / 2.0;
    let y = middle - CURSOR_HEIGHT / 2.0;
    list.push(Op::Arrow { x: left + inset, y, left: true });
    let value_left = left + ARROW_BOX + CYCLER_GAP;
    list.text(value_left + width / 2.0, middle, value, ROW_TEXT, palette::BLACK, Align::Centre);
    let arrow = value_left + width + CYCLER_GAP;
    list.push(Op::Arrow { x: arrow + inset, y, left: false });
}

/// The bar of a slider row (the approved design): an arrow, the bar (a 200 x 16 px well with a white edge and a
/// dark red line around it, filled orange in proportion), the value in percent and an arrow, ending at `right`.
fn slider(list: &mut DrawList, chosen: usize, steps: u8, right: f32, middle: f32) {
    const BAR_WIDTH: f32 = 200.0;
    const BAR_HEIGHT: f32 = 16.0;
    const VALUE_WIDTH: f32 = 52.0;
    let last = usize::from(steps).saturating_sub(1).max(1);
    let percent = chosen.min(last) * 100 / last;
    let inset = (ARROW_BOX - CURSOR_WIDTH) / 2.0;
    let y = middle - CURSOR_HEIGHT / 2.0;
    list.push(Op::Arrow { x: right - ARROW_BOX + inset, y, left: false });
    let value_right = right - ARROW_BOX - CYCLER_GAP;
    list.text(
        value_right,
        middle,
        &percent.to_string(),
        OPT_VALUE_TEXT,
        palette::BLACK,
        Align::Right,
    );
    let bar_right = value_right - VALUE_WIDTH - CYCLER_GAP;
    let bar = Rect::new(bar_right - BAR_WIDTH, middle - BAR_HEIGHT / 2.0, BAR_WIDTH, BAR_HEIGHT);
    list.fill(Rect::new(bar.x - 1.0, bar.y - 1.0, bar.w + 2.0, bar.h + 2.0), palette::DARK_RED);
    list.fill(bar, palette::WHITE);
    let well = Rect::new(bar.x + 2.0, bar.y + 2.0, bar.w - 4.0, bar.h - 4.0);
    list.fill(well, palette::BLACK);
    if percent > 0 {
        list.fill(Rect::new(well.x, well.y, well.w * percent as f32 / 100.0, well.h), palette::HP);
    }
    list.push(Op::Arrow { x: bar.x - CYCLER_GAP - ARROW_BOX + inset, y, left: true });
}

/// The preview of the next card: its window, the card with its balls and its symbol, and under it the
/// corners the cards are Z in. The attack ball is the top left, the defence ball the bottom right.
fn next_card(list: &mut DrawList, view: &CheatsView, window: Rect) {
    list.push(Op::Window { rect: window });
    let title_middle = window.y + FRAME + WINDOW_TOP + GROUP_TITLE_HEIGHT / 2.0;
    let title_left = window.x + FRAME + WINDOW_SIDE;
    list.text(
        title_left,
        title_middle,
        "-- Next card --",
        GROUP_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    let card = Rect::new(
        window.x + (window.w - CARD_WIDTH) / 2.0,
        window.y + FRAME + WINDOW_TOP + GROUP_TITLE_HEIGHT + 10.0,
        CARD_WIDTH,
        CARD_HEIGHT,
    );
    // The card: the HD art of the battle's cards where the pack has it, else the pixel-style card
    let mode = view.cheats.z_mode();
    list.push(Op::Card { rect: card, attack_z: mode.attack(), defence_z: mode.defence() });
    let caption = match (mode.attack(), mode.defence()) {
        (true, true) => "Z in attack and defence",
        (true, false) => "Z in attack",
        (false, true) => "Z in defence",
        (false, false) => "A card as the game deals it",
    };
    let first = card.bottom() + 6.0 + CAPTION_TEXT / 2.0;
    for (line, text) in wrap(caption, CAPTION_CHARS).iter().enumerate() {
        let y = first + 16.0 * line as f32;
        list.text(
            window.x + window.w / 2.0,
            y,
            text,
            CAPTION_TEXT,
            palette::DARK_RED,
            Align::Centre,
        );
    }
}

/// The card of the preview drawn in the pixel style, for a backend that has no card art: a black line
/// outside a white border, a black face with an orange outline, the balls in the attack and defence
/// corners and the symbol in a cream box with an orange border. `card` is the outside of the white border.
#[must_use]
pub fn pixel_card(card: Rect, attack_z: bool, defence_z: bool) -> DrawList {
    let mut list = DrawList::new();
    list.fill(card.grow(2.0), palette::BLACK);
    list.fill(card, palette::WHITE);
    let face = card.grow(-CARD_BORDER);
    list.fill(face, palette::BLACK);
    list.push(Op::Outline { rect: face.grow(-10.0), width: LINE, color: palette::CARD });
    let attack = Rect::new(face.x + 2.0, face.y + 2.0, BALL, BALL);
    let defence = Rect::new(face.right() - 2.0 - BALL, face.bottom() - 2.0 - BALL, BALL, BALL);
    ball(&mut list, attack, attack_z, true);
    ball(&mut list, defence, defence_z, false);
    // The symbol in the middle: a kanji in a cream box with an orange border.
    let symbol = Rect::new(face.x + face.w / 2.0 - 29.0, face.y + 56.0, 58.0, 58.0);
    list.fill(symbol, palette::ORANGE);
    list.fill(symbol.grow(-4.0), palette::CREAM);
    list.text(symbol.x + 29.0, symbol.y + 29.0, "必", 40.0, palette::KI, Align::Centre);
    list
}

/// A ball of the card: a black ring around an orange disc, with a Z (when the corner is Z), four pips (an
/// attack that is not Z) or the kanji seven (a defence that is not Z) in it.
fn ball(list: &mut DrawList, at: Rect, zed: bool, attack: bool) {
    list.push(Op::Disc { rect: at, color: palette::BLACK });
    list.push(Op::Disc { rect: at.grow(-2.0), color: palette::CARD });
    let (x, y) = (at.x + at.w / 2.0, at.y + at.h / 2.0);
    if zed {
        // The Z of the design: M5 5 h14 L6 19 h14 in a 24 px box, drawn with a 4 px stroke.
        let (x0, y0) = (x - 12.0, y - 12.0);
        let point = |px: f32, py: f32| (x0 + px, y0 + py);
        let stroke = 4.0;
        let color = palette::BLACK;
        list.push(Op::Line { from: point(5.0, 5.0), to: point(19.0, 5.0), width: stroke, color });
        list.push(Op::Line { from: point(19.0, 5.0), to: point(6.0, 19.0), width: stroke, color });
        list.push(Op::Line { from: point(6.0, 19.0), to: point(20.0, 19.0), width: stroke, color });
    } else if attack {
        for (dx, dy) in [(-4.0, -4.0), (4.0, -4.0), (-4.0, 4.0), (4.0, 4.0)] {
            let pip = Rect::new(x + dx - 3.0, y + dy - 3.0, 6.0, 6.0);
            list.push(Op::Disc { rect: pip, color: palette::BLACK });
        }
    } else {
        list.text(x, y, "七", 22.0, palette::BLACK, Align::Centre);
    }
}

/// The On window: the number of settings on, each one's label and value, and Reset all at its foot (orange
/// while the cursor is on it).
fn on_window(list: &mut DrawList, view: &CheatsView, window: Rect) {
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + WINDOW_SIDE;
    let right = window.right() - FRAME - WINDOW_SIDE;
    let title = format!("-- On: {} --", view.cheats.on_count());
    let title_middle = window.y + FRAME + WINDOW_TOP + GROUP_TITLE_HEIGHT / 2.0;
    list.text(left, title_middle, &title, GROUP_TEXT, palette::DARK_RED, Align::Left);
    let button = Rect::new(
        left,
        window.bottom() - FRAME - WINDOW_TOP - RESET_HEIGHT,
        right - left,
        RESET_HEIGHT,
    );
    let first = window.y + FRAME + WINDOW_TOP + GROUP_TITLE_HEIGHT + 6.0;
    let lines = view.cheats.lines(&view.names);
    if lines.is_empty() {
        let text = "None. The game plays as the original.";
        for (line, part) in wrap(text, EMPTY_CHARS).iter().enumerate() {
            let middle = first + 11.0 + 22.0 * line as f32;
            list.text(left, middle, part, SMALL_TEXT, palette::BLACK, Align::Left);
        }
    } else {
        // The lines take the room above the button, 24 px apart at most and closer when there are many.
        let room = button.y - 6.0 - first;
        let pitch = (room / lines.len() as f32).min(24.0);
        for (line, (label, value)) in lines.iter().enumerate() {
            let middle = first + pitch * line as f32 + pitch / 2.0;
            // The value stops before the label, with a gap of 8 px, and is cut when the two would meet.
            let value = fit(value, SMALL_TEXT, right - left - text_width(label, SMALL_TEXT) - 8.0);
            list.text(left, middle, label, SMALL_TEXT, palette::BLACK, Align::Left);
            list.text(right, middle, &value, SMALL_TEXT, palette::BLACK, Align::Right);
        }
    }
    let focused = view.row == Row::ResetAll;
    list.fill(button, if focused { palette::ORANGE } else { palette::CREAM });
    list.push(Op::Outline { rect: button, width: LINE, color: palette::DARK_RED });
    let colour = if focused { palette::WHITE } else { palette::BLACK };
    list.text(
        button.x + button.w / 2.0,
        button.middle_y(),
        "Reset all",
        ROW_TEXT,
        colour,
        Align::Centre,
    );
}

/// The notice a cheat puts over the field: one window along the top of the picture, with its line of text.
#[must_use]
pub fn notice(text: &str) -> DrawList {
    let mut list = DrawList::new();
    let height = 2.0 * FRAME + 2.0 * PADDING + LARGE_TEXT;
    let window = Rect::new((WIDTH - NOTICE_WIDTH) / 2.0, MARGIN, NOTICE_WIDTH, height);
    list.push(Op::Window { rect: window });
    list.text(WIDTH / 2.0, window.middle_y(), text, LARGE_TEXT, palette::BLACK, Align::Centre);
    list
}

/// The width of a notice window.
const NOTICE_WIDTH: f32 = 640.0;

/// The original's save-slot lists, which mark a slot saved with a cheat on (task 3): the title's "Which slot?" list
/// (Continue) and the list of the Save, at its slot step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotList {
    Continue,
    Save,
}

/// The original's picture is its 256 x 239 pixels, centred in the 372-pixel-wide frame of the 16:9 view (with 58
/// pixels of the map at each side), so a pixel of the original lies at `(x + SNES_LEFT, y)` in that frame.
const SNES_LEFT: f32 = 58.0;
const FRAME_WIDTH: f32 = 372.0;
const FRAME_HEIGHT: f32 = 239.0;

/// Where the badge of `slot` goes on `list`, in the original's pixels (left, top, right, bottom): on the Continue list
/// right of the slot's tab (the tabs' tops are 48, 104 and 160); on the Save list right of the slot's row (the rows
/// are centred at 186, 202 and 218, beside the window's right edge).
fn badge_rect(list: SlotList, slot: usize) -> (f32, f32, f32, f32) {
    match list {
        SlotList::Continue => {
            let top = [48.0, 104.0, 160.0][slot] + 15.0;
            (207.0, top, 236.0, top + 14.0)
        }
        SlotList::Save => {
            let middle = [186.0, 202.0, 218.0][slot];
            (206.0, middle - 7.0, 236.0, middle + 7.0)
        }
    }
}

/// The CHEATS badges of a slot list: a tag beside each slot whose save was made with a cheat on (`marked`).
#[must_use]
pub fn slot_badges(list: SlotList, marked: [bool; 3]) -> DrawList {
    let mut out = DrawList::new();
    for (slot, &on) in marked.iter().enumerate() {
        if !on {
            continue;
        }
        let (left, top, right, bottom) = badge_rect(list, slot);
        let scale_x = WIDTH / FRAME_WIDTH;
        let scale_y = HEIGHT / FRAME_HEIGHT;
        let rect = Rect::new(
            (left + SNES_LEFT) * scale_x,
            top * scale_y,
            (right - left) * scale_x,
            (bottom - top) * scale_y,
        );
        out.fill(rect, palette::CREAM);
        out.push(Op::Outline { rect, width: LINE, color: palette::ORANGE });
        out.text(
            rect.x + rect.w / 2.0,
            rect.middle_y(),
            "CHEATS",
            SMALL_TEXT,
            palette::DARK_RED,
            Align::Centre,
        );
    }
    out
}

/// The help of the row under the cursor, along the foot, with the key that goes back.
fn help_strip(list: &mut DrawList, row: Row) {
    let window =
        Rect::new(MARGIN, HEIGHT - CHEATS_EDGE - CHEATS_HELP, WIDTH - 2.0 * MARGIN, CHEATS_HELP);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + 20.0;
    list.text(left, window.middle_y(), row.help(), ROW_TEXT, palette::BLACK, Align::Left);
    let right = window.right() - FRAME - 20.0;
    list.text(right, window.middle_y(), "B  Back", SMALL_TEXT, palette::DARK_RED, Align::Right);
}

/// The Options screen's grid, in logical pixels, as the approved design sets it: the category window on the
/// left, the rows window beside it, the help strip at the foot and the question over a veil.
const OPT_TOP: f32 = 32.0;
const OPT_HEIGHT: f32 = 522.0;
const OPT_CATEGORIES: f32 = 300.0;
const OPT_ROWS_X: f32 = 362.0;
const OPT_ROWS_WIDTH: f32 = 878.0;
const OPT_HELP_TOP: f32 = 576.0;
const OPT_HELP_HEIGHT: f32 = 112.0;
/// A window's padding inside its frame: 18 px at the sides, 12 px above and below.
const OPT_SIDE: f32 = 18.0;
const OPT_PAD_TOP: f32 = 12.0;
/// The title above a list: 26 px high in the category window, 28 px in the rows window.
const OPT_TITLE: f32 = 26.0;
const OPT_GROUP: f32 = 28.0;
/// A category row is 58 px high; a value row 44 px; the rows are 2 px apart.
const OPT_CATEGORY_ROW: f32 = 58.0;
const OPT_CATEGORY_ROW_TIGHT: f32 = 48.0;
const OPT_ROW: f32 = 44.0;
const OPT_GAP: f32 = 2.0;
/// A button of the category window (Reset this page, Reset all): 44 px high, 6 px apart.
const OPT_BUTTON: f32 = 44.0;
/// The width of an option's cycler value and of the Open button.
const OPT_CYCLER_VALUE: f32 = 160.0;
const OPT_OPEN_WIDTH: f32 = 220.0;
/// A note row under a group is 22 px high (4 px above its 18 px line).
const OPT_NOTE_ROW: f32 = 22.0;
/// The text sizes of the screen.
const OPT_CATEGORY_TEXT: f32 = 28.0;
const OPT_SUB_TEXT: f32 = 14.0;
const OPT_LABEL_TEXT: f32 = 20.0;
const OPT_HELP_TEXT: f32 = 20.0;
/// The text of a read-only value.
const OPT_VALUE_TEXT: f32 = 18.0;
/// The tint of the row or the category under the cursor: the orange at 14 %.
const OPT_TINT: Color = Color([0xFF, 0x52, 0x00, 36]);
/// The sample window of the Accessibility page: 420 px wide, three rows of 40 px in its frame and padding.
const SAMPLE_WIDTH: f32 = 420.0;
const SAMPLE_ROW: f32 = 40.0;
const SAMPLE_HEIGHT: f32 = 2.0 * FRAME + 16.0 + 3.0 * SAMPLE_ROW;
/// The veil over the screen while a question is open: black at 55 %.
const OPT_VEIL: Color = Color([0, 0, 0, 140]);

/// The Options screen: the categories with where each is kept, the rows of the chosen category with the cursor on
/// one of them, Reset this page and Reset all, and at the foot the help of the row and the key hints. While Reset
/// all asks, the question is over a veil. Everything lies inside the space.
#[must_use]
pub fn options_screen(view: &OptionsView) -> DrawList {
    let mut list = DrawList::new();
    backdrop(&mut list);
    options_categories(&mut list, view);
    options_rows(&mut list, view);
    options_help(&mut list, view);
    if let Some(choice) = view.confirm {
        options_question(&mut list, choice);
    }
    if let Some(ask) = &view.video {
        options_video_question(&mut list, ask);
    }
    list
}

/// The window of the categories: the list with the scope label of each, and the two buttons at its foot.
fn options_categories(list: &mut DrawList, view: &OptionsView) {
    let window = Rect::new(MARGIN, OPT_TOP, OPT_CATEGORIES, OPT_HEIGHT);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + OPT_SIDE;
    let width = window.w - 2.0 * (FRAME + OPT_SIDE);
    let top = window.y + FRAME + OPT_PAD_TOP;
    list.text(
        left + 26.0,
        top + OPT_TITLE - 9.0,
        "-- Options --",
        GROUP_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    let mut y = top + OPT_TITLE + 6.0;
    let shown = Category::shown();
    // Seven pages (the Game page is on) are packed a little closer so that the list keeps its room.
    let step = if shown.len() > 6 { OPT_CATEGORY_ROW_TIGHT } else { OPT_CATEGORY_ROW };
    let squeeze = step / OPT_CATEGORY_ROW;
    for &category in shown {
        let row = Rect::new(left, y, width, step);
        if category == view.category {
            list.fill(row, OPT_TINT);
            list.push(Op::Cursor { x: left, y: row.middle_y() - CURSOR_HEIGHT / 2.0 });
        }
        list.text(
            left + 26.0,
            y + (20.0 * squeeze),
            category.name(),
            OPT_CATEGORY_TEXT,
            palette::BLACK,
            Align::Left,
        );
        list.text(
            left + 26.0,
            y + (44.0 * squeeze),
            category.scope_label(),
            OPT_SUB_TEXT,
            palette::DARK_RED,
            Align::Left,
        );
        y += step;
    }
    let bottom = window.bottom() - FRAME - OPT_PAD_TOP;
    for (index, text) in ["Reset this page", "Reset all"].into_iter().enumerate() {
        let from_bottom = (1 - index) as f32 * (OPT_BUTTON + 6.0) + OPT_BUTTON;
        let rect = Rect::new(left, bottom - from_bottom, width, OPT_BUTTON);
        list.push(Op::Outline { rect, width: LINE, color: palette::DARK_RED });
        list.text(
            rect.x + rect.w / 2.0,
            rect.middle_y(),
            text,
            ROW_TEXT,
            palette::BLACK,
            Align::Centre,
        );
    }
}

/// The window of the rows: each group's title, and its rows with the value at the right end.
fn options_rows(list: &mut DrawList, view: &OptionsView) {
    if view.category == Category::Controls {
        crate::controls_page::rows(list, view);
        return;
    }
    let window = Rect::new(OPT_ROWS_X, OPT_TOP, OPT_ROWS_WIDTH, OPT_HEIGHT);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + OPT_SIDE;
    let right = window.right() - FRAME - OPT_SIDE;
    let y = window.y + FRAME + OPT_PAD_TOP;
    let count = view.category.rows().len();
    let y = option_row_list(list, view, 0..count, left, right, y);
    if let Some(note) = view.category.note() {
        list.text(
            left + 26.0,
            y + 4.0 + CAPTION_TEXT / 2.0,
            note,
            CAPTION_TEXT,
            palette::DARK_RED,
            Align::Left,
        );
    }
    if view.category == Category::Accessibility {
        access_sample(list, window);
    }
}

/// The rows `range` of the category as a list from `y` down (each group's title, each row with its value at the
/// right end), between `left` and `right`: returns where the next line would start.
pub(crate) fn option_row_list(
    list: &mut DrawList,
    view: &OptionsView,
    range: std::ops::Range<usize>,
    left: f32,
    right: f32,
    mut y: f32,
) -> f32 {
    let mut group = None;
    for index in range {
        let Some(row) = view.category.rows().get(index) else { break };
        if group != Some(row.group) {
            group = Some(row.group);
            let title = format!("-- {} --", row.group);
            list.text(
                left + 26.0,
                y + OPT_GROUP - 9.0,
                &title,
                GROUP_TEXT,
                palette::DARK_RED,
                Align::Left,
            );
            y += OPT_GROUP + OPT_GAP;
        }
        if let Kind::Note(text) = row.kind {
            // A line of small text under its group: no cursor, no value.
            list.text(
                left + 26.0,
                y + 4.0 + CAPTION_TEXT / 2.0,
                text,
                CAPTION_TEXT,
                palette::DARK_RED,
                Align::Left,
            );
            y += OPT_NOTE_ROW + OPT_GAP;
            continue;
        }
        let middle = y + OPT_ROW / 2.0;
        if index == view.row {
            list.fill(Rect::new(left, y, right - left, OPT_ROW), OPT_TINT);
            list.push(Op::Cursor { x: left, y: middle - CURSOR_HEIGHT / 2.0 });
        }
        list.text(left + 26.0, middle, row.label, OPT_LABEL_TEXT, palette::BLACK, Align::Left);
        let chosen = view.values.get(index).copied().unwrap_or(0);
        match row.kind {
            Kind::Choice(labels) => {
                if chosen >= labels.len() {
                    // No choice is the value: the page says so (the Difficulty preset changed on the Cheats screen).
                    let x = left + 26.0 + text_width(row.label, OPT_LABEL_TEXT) + 16.0;
                    list.text(x, middle, "Custom", OPT_SUB_TEXT, palette::DARK_RED, Align::Left);
                }
                let texts: Vec<String> = labels.iter().map(|&text| text.to_owned()).collect();
                choices(list, &texts, chosen, right, middle, palette::BLACK);
            }
            Kind::Cycle(_) => cycler(list, row.text(chosen), right, middle, OPT_CYCLER_VALUE),

            Kind::Slider { steps } => slider(list, chosen, steps, right, middle),

            // The Controls page draws its own rows (see `controls_page`).
            Kind::Note(_) | Kind::Key => {}

            Kind::Info => {
                let text = crate::options::info_text(view.category, row.key, &view.values);
                let width = (text_width(&text, OPT_VALUE_TEXT) + 16.0).max(72.0);
                let rect = Rect::new(right - width, middle - 18.0, width, 36.0);
                list.fill(rect, palette::ORANGE);
                list.push(Op::Outline { rect, width: LINE, color: palette::DARK_RED });
                list.text(
                    rect.x + width / 2.0,
                    middle,
                    &text,
                    OPT_VALUE_TEXT,
                    palette::BLACK,
                    Align::Centre,
                );
                let caption = crate::options::info_caption(view.category, row.key);
                list.text(
                    rect.x - 12.0,
                    middle,
                    caption,
                    OPT_SUB_TEXT,
                    palette::DARK_RED,
                    Align::Right,
                );
            }
            Kind::Button(text) => {
                let width = (text_width(text, ROW_TEXT) + 24.0).max(OPT_OPEN_WIDTH);
                let rect =
                    Rect::new(right - width, middle - CHOICE_HEIGHT / 2.0, width, CHOICE_HEIGHT);
                list.push(Op::Outline { rect, width: LINE, color: palette::DARK_RED });
                list.text(
                    rect.x + width / 2.0,
                    middle,
                    text,
                    ROW_TEXT,
                    palette::BLACK,
                    Align::Centre,
                );
            }
        }
        y += OPT_ROW + OPT_GAP;
    }
    y
}

/// The "Sample" of the Accessibility page: a small menu window at the foot of the rows window, drawn in the
/// design like any menu (the chosen style is applied to the whole screen, so it shows the choices at once).
fn access_sample(list: &mut DrawList, rows: Rect) {
    let left = rows.x + FRAME + OPT_SIDE;
    let window = Rect::new(
        left,
        rows.bottom() - FRAME - OPT_PAD_TOP - SAMPLE_HEIGHT,
        SAMPLE_WIDTH,
        SAMPLE_HEIGHT,
    );
    list.text(
        left + 26.0,
        window.y - 6.0 - 11.0,
        "-- Sample --",
        GROUP_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    list.push(Op::Window { rect: window });
    let text = window.x + FRAME + 16.0 + 34.0;
    let top = window.y + FRAME + 8.0;
    for (row, label) in ["Items", "Status", "Options"].into_iter().enumerate() {
        let middle = top + SAMPLE_ROW * row as f32 + SAMPLE_ROW / 2.0;
        if row == 1 {
            list.push(Op::Cursor { x: text - 34.0 + 9.0, y: middle - CURSOR_HEIGHT / 2.0 });
        }
        list.text(text, middle, label, LARGE_TEXT, palette::BLACK, Align::Left);
    }
}

/// The help strip: the help of the row under the cursor (wrapped to three lines at most) and the key hints.
fn options_help(list: &mut DrawList, view: &OptionsView) {
    let window = Rect::new(MARGIN, OPT_HELP_TOP, WIDTH - 2.0 * MARGIN, OPT_HELP_HEIGHT);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + 22.0;
    let row = view.category.rows().get(view.row);
    let refusal = view.controls.as_ref().and_then(|state| state.refused.as_ref());
    let help =
        refusal.map_or_else(|| row.map_or("", |row| row.help), |refusal| refusal.message.as_str());
    let chars = ((window.w - 2.0 * (FRAME + 22.0)) / (ADVANCE * OPT_HELP_TEXT)).floor() as usize;
    let top = window.y + FRAME + 10.0;
    for (line, text) in wrap(help, chars).iter().take(3).enumerate() {
        list.text(
            left,
            top + 13.0 + 26.0 * line as f32,
            text,
            OPT_HELP_TEXT,
            palette::BLACK,
            Align::Left,
        );
    }
    let hints = if view.controls.as_ref().is_some_and(|state| state.listening) {
        let shortcut = view.row > 0
            && (view.row - 1) % crate::options::controls::ACTIONS
                >= crate::options::controls::BUTTONS;
        let mut hints = vec!["Press the new key"];
        hints.push("Esc  Cancel");
        if shortcut {
            hints.push("Backspace  Clear");
        }
        hints.join("   ")
    } else {
        view.category.hints(row).join("   ")
    };
    let y = window.bottom() - FRAME - 10.0 - 9.0;
    list.text(
        window.right() - FRAME - 22.0,
        y,
        &hints,
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Right,
    );
}

/// Reset all's question over a veil: the words, Yes and No (the cursor on `choice`, 0 Yes and 1 No), and the hint.
fn options_question(list: &mut DrawList, choice: u8) {
    list.fill(Rect::new(0.0, 0.0, WIDTH, HEIGHT), OPT_VEIL);
    let window = Rect::new(340.0, 220.0, 600.0, 270.0);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + 28.0;
    let top = window.y + FRAME + OPT_PAD_TOP;
    list.text(
        left,
        top + OPT_TITLE - 9.0,
        "-- Options --",
        GROUP_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    list.text(
        left,
        top + 49.0,
        "Reset all options to their defaults?",
        26.0,
        palette::BLACK,
        Align::Left,
    );
    // The second line wraps inside the window (the design's text is wider than its box), and the buttons move down.
    let chars = ((window.w - 2.0 * (FRAME + 28.0)) / (ADVANCE * ROW_TEXT)).floor() as usize;
    let lines = wrap("Difficulty, Gameplay, Video, Audio, Controls and Accessibility.", chars);
    for (line, text) in lines.iter().enumerate() {
        let y = top + 81.0 + 22.0 * line as f32;
        list.text(left, y, text, ROW_TEXT, palette::BLACK, Align::Left);
    }
    let buttons = top + 110.0 + 22.0 * (lines.len() as f32 - 1.0);
    for (index, text) in ["Yes", "No"].into_iter().enumerate() {
        let rect = Rect::new(left + index as f32 * 224.0, buttons, 200.0, 52.0);
        if usize::from(choice) == index {
            list.fill(rect, OPT_TINT);
            list.push(Op::Cursor { x: rect.x, y: rect.middle_y() - CURSOR_HEIGHT / 2.0 });
        }
        list.text(rect.x + 26.0, rect.middle_y(), text, 30.0, palette::BLACK, Align::Left);
    }
    list.text(
        window.right() - FRAME - 28.0,
        window.bottom() - FRAME - OPT_PAD_TOP - 9.0,
        "A  OK   B  No",
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Right,
    );
}

/// "Apply video changes?" over a veil: what the video would be, Keep and Revert (the cursor on `ask.choice`), the
/// countdown with its bar, and the hint.
fn options_video_question(list: &mut DrawList, ask: &VideoAsk) {
    list.fill(Rect::new(0.0, 0.0, WIDTH, HEIGHT), OPT_VEIL);
    let window = Rect::new(340.0, 220.0, 600.0, 280.0);
    list.push(Op::Window { rect: window });
    let left = window.x + FRAME + 28.0;
    let top = window.y + FRAME + OPT_PAD_TOP;
    list.text(
        left,
        top + OPT_TITLE - 9.0,
        "-- Video --",
        GROUP_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    list.text(left, top + 49.0, "Apply video changes?", 26.0, palette::BLACK, Align::Left);
    let chars = ((window.w - 2.0 * (FRAME + 28.0)) / (ADVANCE * ROW_TEXT)).floor() as usize;
    let lines = wrap(&ask.summary, chars);
    for (line, text) in lines.iter().enumerate() {
        let y = top + 81.0 + 22.0 * line as f32;
        list.text(left, y, text, ROW_TEXT, palette::BLACK, Align::Left);
    }
    let extra = 22.0 * (lines.len() as f32 - 1.0).max(0.0);
    let buttons = top + 110.0 + extra;
    for (index, text) in ["Keep", "Revert"].into_iter().enumerate() {
        let rect = Rect::new(left + index as f32 * 224.0, buttons, 200.0, 52.0);
        if usize::from(ask.choice) == index {
            list.fill(rect, OPT_TINT);
            list.push(Op::Cursor { x: rect.x, y: rect.middle_y() - CURSOR_HEIGHT / 2.0 });
        }
        list.text(rect.x + 26.0, rect.middle_y(), text, 30.0, palette::BLACK, Align::Left);
    }
    let seconds = ask.tenths.div_ceil(10);
    let middle = buttons + 52.0 + 14.0 + 7.0;
    list.text(
        left,
        middle,
        &format!("Reverting in {seconds} s"),
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    let bar = Rect::new(left + 202.0, middle - 7.0, 300.0, 14.0);
    list.push(Op::Bar {
        rect: bar,
        fraction: ask.tenths.min(100) as f32 / 100.0,
        color: palette::ORANGE,
    });
    list.text(
        window.right() - FRAME - 28.0,
        window.bottom() - FRAME - OPT_PAD_TOP - 9.0,
        "A  Keep   B  Revert",
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Right,
    );
}

/// The words of `text` in lines of at most `width` characters; a word longer than a line has a line of its own.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.chars().count() + 1 + word.chars().count() <= width => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(word.to_owned()),
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cheats::{Cheats, Setting, ZMode, ZTarget};
    use crate::options::Category;
    use crate::view::{Member, Slot, SpawnerRow};

    fn field_view(cursor: u8) -> MenuView {
        MenuView {
            screen: Screen::Command,
            bag: None,
            town: false,
            flying: false,
            cursor,
            page: 0,
            zeni: 40,
            party: vec![
                Member {
                    name: "Goku".into(),
                    level: 10,
                    hp: 80,
                    max_hp: 100,
                    ki: 12,
                    max_ki: 12,
                    bp: 416,
                    face: Some(0x00),
                },
                Member {
                    name: "Piccolo".into(),
                    level: 2,
                    hp: 77,
                    max_hp: 77,
                    ki: 9,
                    max_ki: 12,
                    bp: 416,
                    face: None,
                },
            ],
            items: Vec::new(),
            selected: None,
        }
    }

    fn items_view(count: usize, page: u8, cursor: u8) -> MenuView {
        let items = (0..count)
            .map(|i| Slot { object: i as u8, attribute: 0xC0, name: format!("Item{i}") })
            .collect();
        MenuView {
            screen: Screen::Items,
            items,
            page,
            cursor,
            selected: Some(Selected {
                object: 2,
                name: "Bulma".into(),
                price: Some(3),
                help: vec!["Restores 25".into(), "HP.".into()],
                battle: true,
                field: true,
            }),
            ..field_view(0)
        }
    }

    fn texts(list: &DrawList) -> Vec<&str> {
        list.texts().map(|(_, _, text)| text).collect()
    }

    fn cursors(list: &DrawList) -> Vec<(f32, f32)> {
        list.ops()
            .iter()
            .filter_map(|op| match op {
                Op::Cursor { x, y } => Some((*x, *y)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_command_window_lists_the_field_commands_with_the_cursor_on_one() {
        let list = command(&field_view(2));
        for label in ["Talk", "Look", "Fly", "Item", "Menu"] {
            assert!(texts(&list).contains(&label), "{label}");
        }
        let cursor = cursors(&list);
        assert_eq!(cursor.len(), 1, "one cursor");
        // The rows are 44 px apart and the cursor is on the third one.
        let first = cursors(&command(&field_view(0)))[0].1;
        assert!((cursor[0].1 - first - 2.0 * COMMAND_ROW).abs() < 1e-3);
    }

    #[test]
    fn flying_and_town_have_their_own_commands() {
        let mut view = field_view(0);
        view.flying = true;
        let names: Vec<&str> = commands(&view).iter().map(|c| c.label()).collect();
        assert_eq!(names, ["Land", "Item", "Menu"]);
        view.flying = false;
        view.town = true;
        let names: Vec<&str> = commands(&view).iter().map(|c| c.label()).collect();
        assert_eq!(names, ["Talk", "Look", "Item", "Menu"]);
    }

    #[test]
    fn the_menu_list_shows_its_seven_entries_and_greys_none() {
        let mut view = field_view(6);
        view.screen = Screen::Menu;
        let list = scene(&view);
        let text = texts(&list);
        for label in ["Cards", "Status", "Order", "Text", "Save", "Cheats", "Options"] {
            assert!(text.contains(&label), "{label} in {text:?}");
        }
        assert!(!text.contains(&"coming soon"), "the options exist now");
        let grey = list
            .ops()
            .iter()
            .filter(|op| matches!(op, Op::Text { color, .. } if *color == palette::GREY))
            .count();
        assert_eq!(grey, 0);
        assert!(
            text.contains(&"Game, picture, sound and keys."),
            "the help of Options under the cursor"
        );
        assert_eq!(cursors(&list).len(), 1, "the cursor is on Options");
        view.cursor = 5;
        assert!(texts(&scene(&view)).contains(&"Set the cheats of this run."));
    }

    #[test]
    fn the_backdrop_is_the_first_step_and_the_money_and_the_party_are_drawn() {
        let list = command(&field_view(0));
        assert_eq!(list.ops().first(), Some(&Op::Backdrop));
        let text = texts(&list);
        assert!(text.contains(&"Zeni 040P"), "{text:?}");
        let portraits: Vec<Option<Picture>> = list
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Portrait { picture, .. } => Some(*picture),
                _ => None,
            })
            .collect();
        assert_eq!(
            portraits,
            [Some(Picture::Face(0x00)), None],
            "one frame per corner member, empty without a face"
        );
    }

    #[test]
    fn the_item_page_shows_its_eight_entries_and_the_cursor_on_its_row() {
        let list = items(&items_view(12, 1, 2));
        let text = texts(&list);
        assert!(text.contains(&"Item8") && text.contains(&"Item11"), "{text:?}");
        assert!(!text.contains(&"Item7") && !text.contains(&"Item12"), "{text:?}");
        assert!(text.contains(&"Page 2/2"), "{text:?}");
        assert_eq!(cursors(&list).len(), 1);
    }

    #[test]
    fn the_header_counts_against_the_capacity_of_24() {
        let list = items(&items_view(5, 0, 0));
        assert!(texts(&list).contains(&"05/24"));
    }

    #[test]
    fn the_price_box_shows_the_price_and_its_unit_or_nothing_and_the_help_follows_the_name() {
        let mut view = items_view(1, 0, 0);
        let list = items(&view);
        let text = texts(&list);
        assert!(text.contains(&"3") && text.contains(&"Bulma"), "{text:?}");
        assert!(text.contains(&"Restores 25") && text.contains(&"HP."), "{text:?}");
        assert!(text.contains(&"Pt"), "{text:?}");
        // An object with no price: no figure, no dash, no unit
        view.selected.as_mut().unwrap().price = None;
        let list = items(&view);
        let text = texts(&list);
        assert!(!text.contains(&"-") && !text.contains(&"Pt") && !text.contains(&"3"), "{text:?}");
        assert!(text.contains(&"Bulma") && text.contains(&"Restores 25"), "{text:?}");
    }

    #[test]
    fn every_window_lies_inside_the_space() {
        let mut menu = field_view(6);
        menu.screen = Screen::Menu;
        for list in [command(&field_view(0)), menu_list(&menu), items(&items_view(24, 2, 7))] {
            for op in list.ops() {
                if let Op::Window { rect } = op {
                    assert!(rect.x >= 0.0 && rect.y >= 0.0, "{rect:?}");
                    assert!(rect.right() <= WIDTH && rect.bottom() <= HEIGHT, "{rect:?}");
                }
            }
        }
    }

    #[test]
    fn the_drop_sheet_asks_one_unit_with_yes_and_no_and_no_arrows() {
        let bag = BagView {
            tab: Tab::All,
            stacks: Vec::new(),
            used: 0,
            slots: 60,
            full: false,
            usable_here: true,
            droppable_here: true,
            sheet: Sheet::Confirm,
            choice: 1,
            notice: None,
        };
        let window = sheet_window(&bag).expect("the Drop sheet is a sheet");
        let mut list = DrawList::new();
        sheet_draw(&mut list, &window);
        let texts: Vec<&str> = list.texts().map(|(_, _, text)| text).collect();
        assert!(texts.contains(&"Drop one?"), "the question: {texts:?}");
        assert!(texts.contains(&"Yes") && texts.contains(&"No"), "the answers: {texts:?}");
        assert!(list.ops().iter().all(|op| !matches!(op, Op::Arrow { .. })), "no quantity arrows");
    }

    #[test]
    fn the_gauge_fraction_is_clamped_and_zero_without_a_maximum() {
        let mut list = DrawList::new();
        let row = GaugeRow { x: 0.0, y: 0.0, width: 104.0, label: "HP", fill: palette::HP };
        gauge_row(&mut list, row, (150, 100));
        gauge_row(
            &mut list,
            GaugeRow { x: 0.0, y: 0.0, width: 104.0, label: "KI", fill: palette::KI },
            (0, 0),
        );
        let fractions: Vec<f32> = list
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Bar { fraction, .. } => Some(*fraction),
                _ => None,
            })
            .collect();
        assert_eq!(fractions, [1.0, 0.0]);
        assert!((BAR_HEIGHT - 12.0).abs() < 1e-3);
    }

    fn cheats_view(row: Row, cheats: Cheats) -> CheatsView {
        CheatsView {
            cheats,
            row,
            names: (0..12).map(|id| format!("Fighter {id}")).collect(),
            spawner: None,
        }
    }

    fn has_text(list: &DrawList, text: &str, colour: Color) -> bool {
        list.ops().iter().any(
            |op| matches!(op, Op::Text { text: t, color, .. } if t == text && *color == colour),
        )
    }

    #[test]
    fn every_window_and_text_of_the_cheats_screen_lies_inside_the_space() {
        let mut busy = Cheats::neutral();
        for setting in Setting::ALL {
            busy.set_index(setting, setting.steps().len() - 1);
        }
        busy.set_random_battles_off(true);
        busy.set_z_mode(ZMode::Both);
        busy.set_z_target(ZTarget::Fighter(11));
        busy.set_zeni(true);
        busy.set_endless(true);
        for row in Row::ALL {
            for cheats in [Cheats::neutral(), busy] {
                let list = cheats_screen(&cheats_view(row, cheats));
                for op in list.ops() {
                    if let Op::Window { rect } = op {
                        assert!(rect.x >= MARGIN - 1e-3 && rect.right() <= WIDTH - MARGIN + 1e-3);
                        assert!(rect.y >= 0.0 && rect.bottom() <= HEIGHT, "{rect:?}");
                    }
                }
                for (x, y, text) in list.texts() {
                    assert!((0.0..=WIDTH).contains(&x) && (0.0..=HEIGHT).contains(&y), "{text}");
                }
            }
        }
    }

    #[test]
    fn the_accessibility_page_has_three_rows_and_a_sample_window_at_its_foot() {
        let list = options_screen(&options_view(Category::Accessibility, 2, None));
        let texts: Vec<&str> = list.texts().map(|(_, _, text)| text).collect();
        for wanted in ["Text size", "High contrast", "Cursor", "-- Sample --", "Items", "Status"] {
            assert!(texts.contains(&wanted), "{wanted}");
        }
        assert!(!texts.contains(&"Coming in a later version."));
        // Two windows more than the other pages: the sample's.
        let windows = |category| {
            let list = options_screen(&options_view(category, 0, None));
            list.ops().iter().filter(|op| matches!(op, Op::Window { .. })).count()
        };
        assert_eq!(windows(Category::Accessibility), windows(Category::Video) + 1);
        // The sample window lies inside the rows window.
        let rows = list
            .ops()
            .iter()
            .filter_map(|op| if let Op::Window { rect } = op { Some(*rect) } else { None })
            .nth(1)
            .unwrap();
        let sample = list
            .ops()
            .iter()
            .filter_map(|op| if let Op::Window { rect } = op { Some(*rect) } else { None })
            .nth(2)
            .unwrap();
        assert!(
            sample.x >= rows.x
                && sample.right() <= rows.right()
                && sample.bottom() <= rows.bottom()
        );
    }

    fn options_view(category: Category, row: usize, confirm: Option<u8>) -> OptionsView {
        let values = category.rows().iter().map(crate::options::RowDef::default_index).collect();
        OptionsView { category, row, values, confirm, video: None, controls: None }
    }

    fn controls_view(
        row: usize,
        device: usize,
        state: Option<crate::view::ControlsView>,
    ) -> OptionsView {
        use crate::options::controls::{self, Device, KEYBOARD_DEFAULTS};
        let mut view = options_view(Category::Controls, row, None);
        view.values[0] = device;
        for (action, &code) in KEYBOARD_DEFAULTS.iter().enumerate() {
            view.values[controls::row_of(Device::Keyboard, action)] = usize::from(code);
        }
        view.controls = state;
        view
    }

    #[test]
    fn the_controls_page_shows_the_device_the_groups_and_a_key_box_per_action_two_to_a_line() {
        use crate::view::ControlsView;
        let list = options_screen(&controls_view(1, 0, Some(ControlsView::default())));
        let text = texts(&list);
        for label in [
            "Device",
            "Keyboard",
            "Gamepad",
            "-- D-pad and shoulders --",
            "-- Buttons --",
            "-- Shortcuts --",
            "A (confirm)",
            "B (back)",
            "Speed x1/x2/x4/x8",
            "Save state",
            "Load state",
            // The keys of today.
            "Up",
            "X",
            "Z",
            "S",
            "Enter",
            "R-Shift",
            "Tab",
            "P",
            "F5",
            "F9",
        ] {
            assert!(text.contains(&label), "{label} in {text:?}");
        }
        let hints = text.iter().find(|t| t.contains("Select  Reset all")).copied().unwrap_or("");
        assert!(hints.contains("A  Remap"), "{hints}");
        // The cursor of the rows is on the first key box (Up), under the device row.
        let device_row_top = 32.0 + 6.0 + 12.0;
        let cursor_ys: Vec<f32> = cursors(&list).iter().map(|c| c.1).collect();
        assert_eq!(cursor_ys.len(), 2, "the category and the key box");
        assert!(cursor_ys[1] > device_row_top + 44.0 + 28.0, "{cursor_ys:?}");
    }

    #[test]
    fn a_key_box_that_waits_says_press_a_key_and_a_refused_one_says_key_in_use_and_the_strip_names_the_action()
     {
        use crate::view::{ControlsView, RefusedKey};
        let waiting = ControlsView { listening: true, refused: None };
        let text_of = |view: &OptionsView| {
            texts(&options_screen(view)).iter().map(|t| (*t).to_owned()).collect::<Vec<_>>()
        };
        let keyboard = text_of(&controls_view(4, 0, Some(waiting.clone())));
        assert!(keyboard.contains(&"Press a key...".to_owned()), "{keyboard:?}");
        assert!(keyboard.iter().any(|t| t.contains("Esc  Cancel")));
        assert!(!keyboard.iter().any(|t| t.contains("Backspace")), "a button cannot be cleared");
        let pad = text_of(&controls_view(4, 1, Some(waiting.clone())));
        assert!(pad.contains(&"Press a button...".to_owned()), "{pad:?}");
        // A shortcut can be cleared.
        let shortcut = text_of(&controls_view(13, 0, Some(waiting)));
        assert!(shortcut.iter().any(|t| t.contains("Backspace  Clear")), "{shortcut:?}");
        let refused = ControlsView {
            listening: false,
            refused: Some(RefusedKey {
                position: 2,
                message: "Key already used by A (confirm). Choose another key.".to_owned(),
                blink: true,
            }),
        };
        let shown = text_of(&controls_view(2, 0, Some(refused)));
        assert!(shown.contains(&"Key in use".to_owned()), "{shown:?}");
        assert!(shown.contains(&"Key already used by A (confirm). Choose another key.".to_owned()));
        assert!(!shown.contains(&"Press a key...".to_owned()));
    }

    #[test]
    fn every_window_and_text_of_the_options_screen_lies_inside_the_space() {
        for category in Category::ALL {
            for row in 0..category.rows().len().max(1) {
                for confirm in [None, Some(0), Some(1)] {
                    let list = options_screen(&options_view(category, row, confirm));
                    for op in list.ops() {
                        if let Op::Window { rect } = op {
                            assert!(
                                rect.x >= MARGIN - 1e-3 && rect.right() <= WIDTH - MARGIN + 1e-3
                            );
                            assert!(rect.y >= 0.0 && rect.bottom() <= HEIGHT, "{rect:?}");
                        }
                        if let Op::Text { x, y, text, size, align, .. } = op {
                            let width = text_width(text, *size);
                            let (from, to) = match align {
                                Align::Left => (*x, *x + width),
                                Align::Centre => (*x - width / 2.0, *x + width / 2.0),
                                Align::Right => (*x - width, *x),
                            };
                            assert!(
                                from >= MARGIN && to <= WIDTH - MARGIN,
                                "{text}: {from} to {to}"
                            );
                            assert!((0.0..=HEIGHT).contains(y), "{text}");
                        }
                    }
                }
            }
        }
    }

    fn video_ask(choice: u8, tenths: u32) -> VideoAsk {
        VideoAsk {
            choice,
            tenths,
            summary: "Fullscreen, 6x, 4:3 (1536 x 1344), Scanlines Light, CRT Full".into(),
        }
    }

    #[test]
    fn the_video_page_shows_its_groups_the_size_and_the_note_and_the_question_the_countdown() {
        let mut view = options_view(Category::Video, 4, None);
        view.values[1] = 4;
        let text = texts(&options_screen(&view)).join("|");
        for want in [
            "-- Window --",
            "-- Picture --",
            "-- Filters --",
            "V-Sync",
            "Picture size",
            "pixels at this scale",
            "2232 x 1344",
            "CRT filter",
            "revert after 10 s",
        ] {
            assert!(text.contains(want), "{want} in {text}");
        }
        for (choice, tenths) in [(1, 100), (0, 73), (1, 0)] {
            view.video = Some(video_ask(choice, tenths));
            let list = options_screen(&view);
            let shown = texts(&list);
            assert!(shown.contains(&"Apply video changes?"));
            assert!(shown.contains(&"A  Keep   B  Revert"));
            let seconds = format!("Reverting in {} s", tenths.div_ceil(10));
            assert!(shown.contains(&seconds.as_str()), "{seconds}");
            let bars: Vec<f32> = list
                .ops()
                .iter()
                .filter_map(|op| match op {
                    Op::Bar { fraction, .. } => Some(*fraction),
                    _ => None,
                })
                .collect();
            assert_eq!(bars, [tenths as f32 / 100.0]);
            for op in list.ops() {
                if let Op::Text { x, text, size, align, .. } = op {
                    let width = text_width(text, *size);
                    let (from, to) = match align {
                        Align::Left => (*x, *x + width),
                        Align::Centre => (*x - width / 2.0, *x + width / 2.0),
                        Align::Right => (*x - width, *x),
                    };
                    assert!(
                        from >= 340.0 && to <= 940.0 || from >= MARGIN,
                        "{text}: {from} to {to}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_game_page_shows_with_its_rows_when_the_platform_turns_it_on_and_fits_the_list() {
        let _guard =
            crate::options::GAME_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        crate::options::show_game_page(true);
        crate::options::set_game_info("Enhanced", "Installed, 0.9 GB");
        let list = options_screen(&options_view(Category::Game, 1, None));
        let text = texts(&list);
        crate::options::show_game_page(false);
        for label in [
            "Game",
            "This device",
            "Change mode",
            "Install the HD pack",
            "Enhanced",
            "Installed, 0.9 GB",
        ] {
            assert!(text.contains(&label), "{label} in {text:?}");
        }
        assert_eq!(
            Category::Accessibility.next(),
            Category::Difficulty,
            "hidden again: the old ring"
        );
    }

    #[test]
    fn the_options_screen_lists_the_categories_with_where_each_is_kept_and_the_two_buttons() {
        let _guard =
            crate::options::GAME_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let list = options_screen(&options_view(Category::Gameplay, 0, None));
        let text = texts(&list);
        assert!(!text.contains(&"Game"), "the Game page is hidden unless the platform turns it on");
        for &category in Category::shown() {
            assert!(text.contains(&category.name()), "{}", category.name());
        }
        assert_eq!(text.iter().filter(|t| **t == "Saved with the game").count(), 2);
        assert_eq!(text.iter().filter(|t| **t == "This PC").count(), 4);
        for label in
            ["-- Options --", "Reset this page", "Reset all", "Text speed", "Random encounters"]
        {
            assert!(text.contains(&label), "{label} in {text:?}");
        }
        assert!(
            text.contains(&"-- Messages --") && text.contains(&"-- Exploring --"),
            "the groups"
        );
        let cursor_ys: Vec<f32> = cursors(&list).iter().map(|c| c.1).collect();
        assert_eq!(cursor_ys.len(), 2, "one for the category, one for the row");
        // The cursor of the category sits on the second category row (Gameplay), 58 px below the first.
        let first_row_top = 32.0 + 6.0 + 12.0 + 26.0 + 6.0;
        let want = first_row_top + 58.0 + 29.0 - CURSOR_HEIGHT / 2.0;
        assert!((cursor_ys[0] - want).abs() < 1e-3, "{cursor_ys:?}");
        let hints = text.iter().find(|t| t.contains("Select  Reset all")).copied().unwrap_or("");
        for hint in
            ["Left/Right  Change", "B  Back", "Y  Reset page", "Select  Reset all", "L/R  Page"]
        {
            assert!(hints.contains(hint), "{hint} in {hints}");
        }
    }

    #[test]
    fn the_chosen_option_is_orange_with_black_text_and_a_cycler_shows_its_value_between_arrows() {
        let mut view = options_view(Category::Gameplay, 0, None);
        view.values = vec![3, 0, 1, 0, 0];
        let list = options_screen(&view);
        assert!(has_text(&list, "Fast", palette::BLACK));
        let orange = list
            .ops()
            .iter()
            .any(|op| matches!(op, Op::Fill { color, .. } if *color == palette::ORANGE));
        assert!(orange, "the chosen text speed is on an orange button");
        assert!(texts(&list).contains(&"Off"), "the cycler shows its value");
        let arrows = list.ops().iter().filter(|op| matches!(op, Op::Arrow { .. })).count();
        assert_eq!(arrows, 4, "two cyclers: the animation speed and the encounters");
        let video = options_screen(&options_view(Category::Video, 0, None));
        assert!(texts(&video).contains(&"4x"));
        let difficulty = options_screen(&options_view(Category::Difficulty, 0, None));
        assert!(texts(&difficulty).contains(&"Open Cheats"));
        assert!(texts(&difficulty).contains(&"Cheats screen"));
    }

    #[test]
    fn the_difficulty_page_shows_the_preset_what_it_sets_and_the_notes() {
        let mut cheats = Cheats::neutral();
        crate::options::difficulty::apply(&mut cheats, 0);
        let view = |cheats: &Cheats, row| OptionsView {
            category: Category::Difficulty,
            row,
            values: crate::options::Options::default().indices(Category::Difficulty, cheats, 0),
            confirm: None,
            video: None,
            controls: None,
        };
        let easy = options_screen(&view(&cheats, 0));
        let text = texts(&easy);
        for want in [
            "-- Preset --",
            "Difficulty",
            "Easy",
            "Normal",
            "Hard",
            "Easy and Hard mark the save with the CHEATS badge. Only Normal is clean.",
            "-- The preset sets --",
            "Enemy power",
            "EXP per battle",
            "Money from events",
            "Cheats: Enemy power",
            "-- Fine control --",
            "Open Cheats",
            "The preset and the Cheats screen write the same numbers: the last change wins.",
        ] {
            assert!(text.contains(&want), "{want}");
        }
        assert!(!text.contains(&"Custom"), "Easy is a preset");
        // Easy: power x1/2, EXP x2, money x2 in the three orange boxes; Easy is the chosen choice.
        let boxes = easy
            .ops()
            .iter()
            .filter(|op| matches!(op, Op::Fill { rect, color } if *color == palette::ORANGE && (rect.h - 36.0).abs() < 1e-3))
            .count();
        assert_eq!(boxes, 3);
        assert_eq!(
            cursors(&easy).len(),
            2,
            "the category and the preset row; the info rows have no cursor"
        );
        // Another value on the Cheats screen: Custom, no choice chosen.
        cheats.set_index(Setting::Exp, 3);
        let custom = options_screen(&view(&cheats, 0));
        assert!(texts(&custom).contains(&"Custom"));
        let chosen = custom
            .ops()
            .iter()
            .filter(|op| matches!(op, Op::Fill { rect, color } if *color == palette::ORANGE && (rect.h - CHOICE_HEIGHT).abs() < 1e-3))
            .count();
        assert_eq!(chosen, 0, "none of Easy, Normal, Hard is highlighted");
    }

    #[test]
    fn a_category_with_no_rows_yet_shows_its_note_and_an_empty_help() {
        for category in Category::ALL.into_iter().filter(|c| c.rows().is_empty()) {
            let list = options_screen(&options_view(category, 0, None));
            assert!(texts(&list).contains(&"Coming in a later version."));
            assert_eq!(cursors(&list).len(), 1, "only the cursor of the category");
        }
    }

    #[test]
    fn the_audio_page_draws_the_volume_bar_with_its_percent_and_the_output_choices() {
        let list = options_screen(&options_view(Category::Audio, 0, None));
        let text = texts(&list);
        assert!(text.contains(&"Volume") && text.contains(&"Sound"));
        assert!(text.contains(&"100"), "full volume by default");
        assert!(text.contains(&"Stereo") && text.contains(&"Mono"));
        let arrows = list.ops().iter().filter(|op| matches!(op, Op::Arrow { .. })).count();
        assert_eq!(arrows, 2, "one arrow each side of the bar");
        let mut view = options_view(Category::Audio, 0, None);
        view.values[0] = 3;
        let list = options_screen(&view);
        assert!(texts(&list).contains(&"30"));
        view.values[0] = 0;
        let list = options_screen(&view);
        assert!(texts(&list).contains(&"0"));
        let filled = |list: &DrawList| {
            list.ops()
                .iter()
                .filter(|op| matches!(op, Op::Fill { color, .. } if *color == palette::HP))
                .count()
        };
        assert_eq!(filled(&list), 0, "silence: an empty bar");
    }

    #[test]
    fn the_help_strip_shows_the_help_of_the_row_wrapped_inside_the_window() {
        let list = options_screen(&options_view(Category::Gameplay, 1, None));
        let help = Category::Gameplay.rows()[1].help;
        let shown: Vec<&str> =
            texts(&list).into_iter().filter(|t| help.contains(*t) && t.len() > 20).collect();
        assert_eq!(shown.join(" "), help, "the whole help, wrapped");
    }

    #[test]
    fn the_reset_all_question_has_a_veil_yes_and_no_and_the_cursor_on_the_choice() {
        let ask = |choice| options_screen(&options_view(Category::Video, 0, Some(choice)));
        let list = ask(1);
        let text = texts(&list);
        assert!(text.contains(&"Reset all options to their defaults?"));
        assert!(text.contains(&"Yes") && text.contains(&"No") && text.contains(&"A  OK   B  No"));
        let veil = list.ops().iter().any(
            |op| matches!(op, Op::Fill { rect, color } if (rect.w - WIDTH).abs() < 1e-3 && color.0[3] == 140),
        );
        assert!(veil);
        let at = |list: &DrawList| *cursors(list).last().unwrap();
        let (no, yes) = (at(&list), at(&ask(0)));
        assert!(no.0 > yes.0 && (no.1 - yes.1).abs() < 1e-3, "the cursor moves from Yes to No");
        let closed = options_screen(&options_view(Category::Video, 0, None));
        assert!(texts(&closed).iter().all(|t| *t != "Yes"));
    }
    #[test]
    fn every_row_of_the_cheats_screen_is_drawn_and_the_cursor_shows_on_each() {
        for row in Row::ALL {
            let list = cheats_screen(&cheats_view(row, Cheats::neutral()));
            assert!(texts(&list).contains(&row.label()), "{row:?} is drawn");
            // Reset all is the On window's button, highlighted instead of pointed at.
            if row != Row::ResetAll {
                assert_eq!(cursors(&list).len(), 1, "the cursor shows on {row:?}");
            }
        }
    }

    #[test]
    fn the_item_spawner_shows_its_page_the_story_tag_and_the_warning() {
        let rows = (0..11)
            .map(|object| SpawnerRow {
                object,
                name: format!("Object {object}"),
                story: object == 4,
                in_bag: u32::from(object),
            })
            .collect();
        let mut spawner = SpawnerView {
            rows,
            object: 4,
            name: "Object 4".to_owned(),
            help: vec!["What it does.".to_owned()],
            story: true,
            count: 1,
            confirm: None,
        };
        let mut view = cheats_view(Row::ItemSpawner, Cheats::neutral());
        view.spawner = Some(spawner.clone());
        let list = cheats_screen(&view);
        let text = texts(&list);
        assert!(text.contains(&"-- Item spawner --") && text.contains(&"In bag"));
        assert!(has_text(&list, "Object 4", palette::STORY), "a story object is red");
        assert!(has_text(&list, "STORY", palette::WHITE));
        assert!(text.contains(&"one at a time") && text.contains(&"What it does."));
        assert!(!text.contains(&"! STORY ITEM"), "no warning before A");
        spawner.confirm = Some(1);
        view.spawner = Some(spawner);
        let asking = cheats_screen(&view);
        let text = texts(&asking);
        assert!(text.contains(&"! STORY ITEM") && text.contains(&"Save the game first."));
        assert!(has_text(&asking, "Cancel", palette::WHITE), "the cursor is on Cancel");
    }

    #[test]
    fn the_chosen_choice_is_orange_with_white_text_and_the_others_are_black() {
        let mut cheats = Cheats::neutral();
        cheats.set_index(Setting::Damage, 2);
        let list = cheats_screen(&cheats_view(Row::Damage, cheats));
        assert!(has_text(&list, "×2", palette::WHITE), "the chosen Damage is white");
        assert!(has_text(&list, "×1", palette::BLACK), "Defence's neutral choice is black");
        let highlights = list
            .ops()
            .iter()
            .filter(|op| matches!(op, Op::Fill { rect, color } if *color == palette::ORANGE && rect.x < 960.0))
            .count();
        assert_eq!(highlights, 13, "one highlight in each row of choices, the For cycler has none");
    }

    #[test]
    fn the_for_row_cycles_through_the_party_with_the_name_of_its_choice() {
        let mut cheats = Cheats::neutral();
        cheats.set_z_target(ZTarget::Fighter(1));
        let list = cheats_screen(&cheats_view(Row::ZFor, cheats));
        assert!(texts(&list).contains(&"Fighter 1"), "{:?}", texts(&list));
        let arrows: Vec<bool> = list
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Arrow { left, .. } => Some(*left),
                _ => None,
            })
            .collect();
        assert_eq!(arrows, [true, false], "a left and a right arrow");
        assert_eq!(cursors(&list).len(), 1, "the cursor is on the For row");
        let whole = cheats_screen(&cheats_view(Row::ZFor, Cheats::neutral()));
        assert!(texts(&whole).contains(&"Whole party"));
    }

    #[test]
    fn the_card_shows_a_z_in_each_corner_that_is_z_and_its_caption_says_so() {
        let card = |mode: ZMode| {
            let mut cheats = Cheats::neutral();
            cheats.set_z_mode(mode);
            cheats_screen(&cheats_view(Row::Damage, cheats))
        };
        // The HD card is one step that says which corners are Z
        let corners = |list: &DrawList| {
            let cards: Vec<(bool, bool)> = list
                .ops()
                .iter()
                .filter_map(|op| match op {
                    Op::Card { attack_z, defence_z, .. } => Some((*attack_z, *defence_z)),
                    _ => None,
                })
                .collect();
            assert_eq!(cards.len(), 1, "one card");
            cards[0]
        };
        assert_eq!(corners(&card(ZMode::Off)), (false, false));
        assert_eq!(corners(&card(ZMode::Attack)), (true, false));
        assert_eq!(corners(&card(ZMode::Defence)), (false, true));
        assert_eq!(corners(&card(ZMode::Both)), (true, true));
        // The pixel-style card (no card art) has a Z of three strokes in each Z corner and the
        // kanji seven in a defence that is not Z
        let strokes =
            |list: &DrawList| list.ops().iter().filter(|op| matches!(op, Op::Line { .. })).count();
        let rect = Rect::new(10.0, 20.0, CARD_WIDTH, CARD_HEIGHT);
        assert_eq!(strokes(&pixel_card(rect, false, false)), 0);
        assert_eq!(strokes(&pixel_card(rect, true, false)), 3, "a Z is three strokes");
        assert_eq!(strokes(&pixel_card(rect, false, true)), 3);
        assert_eq!(strokes(&pixel_card(rect, true, true)), 6);
        assert!(
            texts(&pixel_card(rect, false, false)).contains(&"七"),
            "the defence of a card that is not Z"
        );
        assert!(
            texts(&pixel_card(rect, true, false)).contains(&"七"),
            "the defence stays a number"
        );
        assert!(
            !texts(&pixel_card(rect, false, true)).contains(&"七"),
            "a Z defence has no number"
        );
        let off = card(ZMode::Off);
        assert!(texts(&off).contains(&"A card as the"), "{:?}", texts(&off));
        let attack = card(ZMode::Attack);
        assert!(texts(&attack).contains(&"Z in attack"), "{:?}", texts(&attack));
        assert_eq!(wrap("Z in attack and defence", CAPTION_CHARS), ["Z in attack and", "defence"]);
    }

    #[test]
    fn the_on_window_counts_what_is_on_and_says_none_when_nothing_is() {
        let off = cheats_screen(&cheats_view(Row::ResetAll, Cheats::neutral()));
        assert!(texts(&off).contains(&"-- On: 0 --"));
        assert!(texts(&off).contains(&"None. The game"), "{:?}", texts(&off));
        assert!(has_text(&off, "Reset all", palette::WHITE), "the cursor is on Reset all");
        let mut on = Cheats::neutral();
        on.set_index(Setting::Damage, 2);
        on.set_zeni(true);
        let list = cheats_screen(&cheats_view(Row::Damage, on));
        assert!(texts(&list).contains(&"-- On: 2 --"));
        assert!(texts(&list).contains(&"Zeni") && texts(&list).contains(&"max"));
        assert!(has_text(&list, "Reset all", palette::BLACK), "not focused: black on cream");
    }

    #[test]
    fn the_help_line_is_the_help_of_the_row_under_the_cursor() {
        for row in Row::ALL {
            let list = cheats_screen(&cheats_view(row, Cheats::neutral()));
            assert!(texts(&list).contains(&row.help()), "{row:?}");
        }
    }

    #[test]
    fn a_notice_is_one_window_along_the_top_with_its_line_centred() {
        let list = notice("Cheats off for this fight");
        let windows: Vec<Rect> = list
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Window { rect } => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(windows.len(), 1, "one window");
        let window = windows[0];
        assert!((window.x + window.w / 2.0 - WIDTH / 2.0).abs() < 1e-3, "centred: {window:?}");
        assert!(window.y >= MARGIN - 1e-3 && window.bottom() < HEIGHT / 4.0, "{window:?}");
        assert!(texts(&list).contains(&"Cheats off for this fight"));
    }

    #[test]
    fn the_cheats_badges_mark_each_slot_saved_with_a_cheat_inside_the_space() {
        for list in [SlotList::Continue, SlotList::Save] {
            assert!(slot_badges(list, [false; 3]).ops().is_empty(), "no badge without a cheat");
            let all = slot_badges(list, [true; 3]);
            let badges = all.texts().filter(|(_, _, text)| *text == "CHEATS").count();
            assert_eq!(badges, 3, "{list:?}: one badge a slot");
            for op in all.ops() {
                if let Op::Fill { rect, .. } = op {
                    assert!(rect.x >= 0.0 && rect.right() <= WIDTH, "{rect:?}");
                    assert!(rect.y >= 0.0 && rect.bottom() <= HEIGHT, "{rect:?}");
                }
            }
            let one = slot_badges(list, [false, true, false]);
            assert_eq!(one.texts().count(), 1, "only the marked slot");
        }
        // The Continue list's first tag is below the first slot's tab, the Save list's second one at its second row.
        let continue_first = slot_badges(SlotList::Continue, [true, false, false]);
        let (_, first_y, _) = continue_first.texts().next().expect("the badge text");
        assert!(
            first_y > 63.0 * HEIGHT / FRAME_HEIGHT - 1.0 && first_y < HEIGHT / 2.0,
            "{first_y}"
        );
        let save_second = slot_badges(SlotList::Save, [false, true, false]);
        let (_, second_y, _) = save_second.texts().next().expect("the badge text");
        assert!((second_y - 202.0 * HEIGHT / FRAME_HEIGHT).abs() < 1.0, "{second_y}");
    }

    #[test]
    fn the_words_of_a_caption_wrap_at_the_width_given() {
        assert_eq!(wrap("A card as the game deals it", 15), ["A card as the", "game deals it"]);
        assert_eq!(wrap("", 15), Vec::<String>::new());
        assert_eq!(wrap("averyveryverylongword", 5), ["averyveryverylongword"]);
    }

    fn pack_line(name: &str, state: PackState, reason: Option<&str>) -> crate::view::PackLine {
        crate::view::PackLine {
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            author: "The project".to_owned(),
            description: "Goku can use the Kaioken while he is a Super Saiyan, up to x10."
                .to_owned(),
            needs: "nothing".to_owned(),
            size: 412 * 1024,
            state,
            reason: reason.map(str::to_owned),
        }
    }

    fn mods_view(cursor: usize, ask: Option<(ModsAsk, u8)>) -> ModsView {
        ModsView {
            packs: vec![
                pack_line("Free progression", PackState::On, None),
                pack_line(
                    "Old map test",
                    PackState::Refused,
                    Some(
                        "This pack was made for another ROM (CRC 1A2B3C4D). Your game is A8CF4F2A.",
                    ),
                ),
            ],
            chapters: vec![
                crate::view::ChapterLine {
                    name: "The Androids".to_owned(),
                    description: "Goku's family.".to_owned(),
                    locked: true,
                    main: false,
                },
                crate::view::ChapterLine {
                    name: "Training".to_owned(),
                    description: "A room with a Kaioken master.".to_owned(),
                    locked: false,
                    main: false,
                },
            ],
            cursor,
            moving: false,
            ask,
            notice: None,
        }
    }

    #[test]
    fn the_data_packs_list_shows_the_boxes_versions_chapters_and_the_keys() {
        let list = mods_screen(&mods_view(0, None));
        let shown = texts(&list);
        for text in [
            "-- Data packs --",
            "Version",
            "Free progression",
            "1.0.0",
            "Old map test",
            "-- Chapters --",
            "The Androids",
            "locked",
            "Training",
            "Classic is never changed by packs.",
            "Apply",
            "A on/off   Y + Up/Down move (order)   X Apply",
            "B Back",
        ] {
            assert!(shown.contains(&text), "{text} is shown: {shown:?}");
        }
        assert!(!shown.contains(&"Start"), "the first line is a pack: no Start");
        assert!(shown.contains(&"Free progression  1.0.0"), "the details name the pack");
        assert!(shown.contains(&"by The project"), "and its author");
        assert!(shown.contains(&"Size: 412 KB"), "and its size");
        assert!(shown.contains(&"Needs: nothing"), "and what it needs");
        assert!(!shown.contains(&"REFUSED:"));
    }

    #[test]
    fn a_refused_pack_says_why_it_stays_off_and_a_chapter_shows_its_start() {
        let refused_list = mods_screen(&mods_view(1, None));
        let refused = texts(&refused_list);
        assert!(refused.contains(&"REFUSED:"), "{refused:?}");
        assert!(refused.iter().any(|line| line.starts_with("This pack was made for another ROM")));
        assert!(refused.iter().any(|line| line.starts_with("It stays off.")));
        let install_list = mods_screen(&mods_view(2, None));
        let install = texts(&install_list);
        assert!(install.contains(&"+ Install a pack..."), "{install:?}");
        assert!(install.contains(&"Install a pack"), "the details of the install line");
        assert!(!install.contains(&"Start"));
        let chapter_list = mods_screen(&mods_view(4, None));
        let chapter = texts(&chapter_list);
        assert!(chapter.contains(&"A room with a Kaioken master."), "{chapter:?}");
        assert!(chapter.contains(&"Start"), "an open chapter's details have the Start button");
        let locked_list = mods_screen(&mods_view(3, None));
        let locked = texts(&locked_list);
        assert!(locked.contains(&"Locked: the game must be finished first."), "{locked:?}");
        assert!(!locked.contains(&"Start"));
    }

    #[test]
    fn a_question_is_drawn_over_the_screen_with_its_two_buttons() {
        let apply_list = mods_screen(&mods_view(0, Some((ModsAsk::Apply, 1))));
        let apply = texts(&apply_list);
        for text in [
            "Apply the packs?",
            "The game goes back to the title screen.",
            "Save first: unsaved progress is lost.",
            "Apply",
            "Cancel",
        ] {
            assert!(apply.contains(&text), "{text}: {apply:?}");
        }
        let start_list = mods_screen(&mods_view(3, Some((ModsAsk::Chapter(3), 0))));
        let start = texts(&start_list);
        assert!(start.contains(&"Start this chapter?"));
        assert!(start.contains(&"The game goes to the chapter's own saves."));
        assert!(start.contains(&"Start"));
        let main_list = mods_screen(&mods_view(0, Some((ModsAsk::Main, 1))));
        let main = texts(&main_list);
        assert!(main.contains(&"Back to the main game?") && main.contains(&"Go"));
        // The cursor is on the choice: Cancel (choice 1) has the orange cursor beside it, Apply (choice 0) another place.
        let on_cancel = mods_screen(&mods_view(0, Some((ModsAsk::Apply, 1))));
        let on_apply = mods_screen(&mods_view(0, Some((ModsAsk::Apply, 0))));
        let cursor_at = |list: &DrawList| {
            list.ops()
                .iter()
                .filter_map(|op| match op {
                    Op::Cursor { x, y } if *x > 300.0 && *x < 950.0 => Some((*x, *y)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let (cancel, apply) = (cursor_at(&on_cancel), cursor_at(&on_apply));
        assert_eq!(cancel.len(), 1, "one cursor in the question");
        assert!(cancel[0].0 > apply[0].0, "Cancel is to the right of Apply: {cancel:?} {apply:?}");
    }
}
