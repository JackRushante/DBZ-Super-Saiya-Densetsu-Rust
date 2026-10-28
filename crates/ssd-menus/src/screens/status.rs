//! The Status screen: the party at the left, and the member under the cursor at the right. The original has two pages (every
//! member, then one member, which A opens and B leaves); the HD shows both at once, and the right panel follows the cursor.
//! The positions are those of the approved design (the canvas's `Status`), in the 1280 x 720 space; both panels lie inside the
//! 960-pixel 4:3 core. The figures are the game's own, read by the probe; the screen only draws them.

use crate::draw::{Align, Color, DrawList, Op, Rect};
use crate::layout::{CURSOR_HEIGHT, fit, palette};
use crate::view::Picture;

/// The slots of the party's list: the game keeps nine records.
pub const SLOTS: usize = 9;

/// The kanji of a card symbol, which is a fighter's class, by symbol number 0 to 6: its reading and its meaning (the card
/// tooltip's words, `docs/re/data/cards.md`).
pub const CLASSES: [(&str, &str); 7] = [
    ("Kai", "World"),
    ("Kami", "God"),
    ("Ma", "Demon"),
    ("Waku", "Doubt"),
    ("Ginyu", "Force"),
    ("Hitsu", "Sure"),
    ("Chou", "Super"),
];

/// One technique of a member with its KI cost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Technique {
    pub name: String,
    pub cost: u8,
}

/// A member of the party as Status shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusMember {
    pub name: String,
    pub level: u8,
    pub hp: u16,
    pub max_hp: u16,
    pub ki: u16,
    pub max_ki: u16,
    /// The battle power.
    pub bp: u32,
    /// The figure the original labels "PLA." (the record's byte `+0E`): what it counts is not decoded yet.
    pub pla: u8,
    pub exp: u32,
    /// The EXP still needed for the next level (the original's "EX LV UP.").
    pub to_next: u32,
    /// The class: the symbol number of the kanji under the portrait (see [`CLASSES`]).
    pub class: u8,
    /// The face index of the portrait.
    pub face: Option<u8>,
    pub techniques: Vec<Technique>,
}

/// The Status screen as it shows: the party in party order, and the member under the cursor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusView {
    pub members: Vec<StatusMember>,
    /// The member under the cursor: an index of `members`.
    pub cursor: usize,
}

/// The tint of the row under the cursor (the Options screen's orange at 16 percent).
const TINT: Color = Color([0xFF, 0x52, 0x00, 40]);
/// The left panel and the right panel.
const LEFT: Rect = Rect::new(176.0, 30.0, 500.0, 620.0);
const RIGHT: Rect = Rect::new(696.0, 30.0, 408.0, 620.0);
/// The distance between two members of the list, and the side of a portrait of it.
const ROW: f32 = 56.0;
const ROW_PORTRAIT: f32 = 46.0;
/// Where the right panel's figures end.
const FIGURES_RIGHT: f32 = 1078.0;

/// The draw list of the Status screen.
#[must_use]
pub fn status_screen(view: &StatusView) -> DrawList {
    let mut list = DrawList::new();
    list.push(Op::Backdrop);
    list.push(Op::Window { rect: LEFT });
    list.push(Op::Window { rect: RIGHT });
    party_panel(&mut list, view);
    if let Some(member) = view.members.get(view.cursor) {
        member_panel(&mut list, member);
    }
    list
}

/// The left panel: the header with the count, the nine slots, and the hints.
fn party_panel(list: &mut DrawList, view: &StatusView) {
    list.text(206.0, 69.0, "Status", 30.0, palette::BLACK, Align::Left);
    let count = format!("{}/{SLOTS}", view.members.len().min(SLOTS));
    list.text(646.0, 69.0, &count, 16.0, palette::DARK_RED, Align::Right);
    list.fill(Rect::new(206.0, 96.0, 440.0, 2.0), palette::DARK_RED);
    for slot in 0..SLOTS {
        let top = 113.0 + ROW * slot as f32;
        let portrait = Rect::new(216.0, top, ROW_PORTRAIT, ROW_PORTRAIT);
        let middle = top + ROW_PORTRAIT / 2.0;
        let Some(member) = view.members.get(slot) else {
            dashed(list, portrait, palette::GREY);
            list.text(278.0, middle, "\u{2014}", 20.0, palette::GREY, Align::Left);
            continue;
        };
        if slot == view.cursor {
            list.fill(Rect::new(190.0, top - 5.0, 472.0, ROW), TINT);
            list.push(Op::Cursor { x: 194.0, y: middle - CURSOR_HEIGHT / 2.0 });
            // The portrait's rings: orange, then dark red, outside its white line.
            list.fill(portrait.grow(7.0), palette::DARK_RED);
            list.fill(portrait.grow(5.0), palette::ORANGE);
        }
        list.push(Op::Portrait { rect: portrait, picture: member.face.map(Picture::Face) });
        let name = fit(&member.name, 22.0, 140.0);
        list.text(278.0, top + 13.0, &name, 22.0, palette::BLACK, Align::Left);
        let level = format!("LV {}", member.level);
        list.text(278.0, top + 38.0, &level, 16.0, palette::DARK_RED, Align::Left);
        let gauges = [
            ("HP", member.hp, member.max_hp, palette::HP, top + 6.0),
            ("KI", member.ki, member.max_ki, palette::KI, top + 28.0),
        ];
        for (label, value, max, colour, y) in gauges {
            list.text(428.0, y, label, 16.0, palette::BLACK, Align::Left);
            bar(list, Rect::new(456.0, y - 7.0, 150.0, 14.0), value, max, colour);
            list.text(616.0, y, &value.to_string(), 16.0, palette::BLACK, Align::Left);
        }
    }
    for (x, hint) in [(206.0, "Up/Down  Member"), (410.0, "B  Back"), (520.0, "Start  Close")] {
        list.text(x, 628.0, hint, 16.0, palette::DARK_RED, Align::Left);
    }
}

/// The right panel: the member's portrait and name, the figures in the original's order, and every technique with its cost.
fn member_panel(list: &mut DrawList, member: &StatusMember) {
    list.push(Op::Portrait {
        rect: Rect::new(724.0, 58.0, 140.0, 140.0),
        picture: member.face.map(Picture::Face),
    });
    let name = fit(&member.name, 30.0, 190.0);
    list.text(886.0, 84.0, &name, 30.0, palette::BLACK, Align::Left);
    let level = format!("LV {}", member.level);
    list.text(886.0, 122.0, &level, 24.0, palette::BLACK, Align::Left);
    if let Some(&(reading, meaning)) = CLASSES.get(usize::from(member.class)) {
        let badge = Rect::new(886.0, 146.0, 52.0, 52.0);
        list.fill(badge, palette::DARK_RED);
        list.fill(badge.grow(-2.0), palette::ORANGE);
        list.fill(badge.grow(-5.0), palette::CREAM);
        list.push(Op::Symbol { rect: badge.grow(-7.0), symbol: member.class });
        list.text(950.0, 162.0, reading, 20.0, palette::BLACK, Align::Left);
        list.text(950.0, 186.0, meaning, 16.0, palette::DARK_RED, Align::Left);
    }
    let rows: [(&str, String); 6] = [
        ("HP", format!("{} / {}", member.hp, member.max_hp)),
        ("KI", format!("{} / {}", member.ki, member.max_ki)),
        ("BP", member.bp.to_string()),
        ("PLA.", format!("{:02}", member.pla)),
        ("EXP", member.exp.to_string()),
        ("To next LV", member.to_next.to_string()),
    ];
    for (i, (label, figure)) in rows.iter().enumerate() {
        let y = 244.0 + 38.0 * i as f32;
        list.text(724.0, y, label, 20.0, palette::BLACK, Align::Left);
        list.text(FIGURES_RIGHT, y, figure, 24.0, palette::BLACK, Align::Right);
    }
    bar(list, Rect::new(772.0, 237.0, 140.0, 14.0), member.hp, member.max_hp, palette::HP);
    bar(list, Rect::new(772.0, 275.0, 140.0, 14.0), member.ki, member.max_ki, palette::KI);
    list.fill(Rect::new(724.0, 464.0, 354.0, 2.0), palette::DARK_RED);
    list.text(724.0, 488.0, "Techniques", 16.0, palette::DARK_RED, Align::Left);
    list.text(FIGURES_RIGHT, 488.0, "KI-USE", 16.0, palette::DARK_RED, Align::Right);
    techniques(list, &member.techniques);
}

/// The techniques, 42 px apart as in the design; closer, and smaller, when there are too many to end above the panel's foot.
fn techniques(list: &mut DrawList, techniques: &[Technique]) {
    const FIRST: f32 = 526.0;
    const LAST: f32 = 618.0;
    let steps = techniques.len().saturating_sub(1).max(1) as f32;
    let pitch = ((LAST - FIRST) / steps).min(42.0);
    let size = if pitch >= 30.0 {
        24.0
    } else if pitch >= 22.0 {
        20.0
    } else {
        16.0
    };
    for (i, technique) in techniques.iter().enumerate() {
        let y = FIRST + pitch * i as f32;
        let name = fit(&technique.name, size, 280.0);
        list.text(724.0, y, &name, size, palette::BLACK, Align::Left);
        let cost = technique.cost.to_string();
        list.text(FIGURES_RIGHT, y, &cost, size, palette::BLACK, Align::Right);
    }
}

/// A gauge with the dark-red line the design puts around it.
fn bar(list: &mut DrawList, rect: Rect, value: u16, max: u16, colour: Color) {
    let fraction = if max == 0 { 0.0 } else { (f32::from(value) / f32::from(max)).min(1.0) };
    list.fill(rect.grow(1.0), palette::DARK_RED);
    list.push(Op::Bar { rect, fraction, color: colour });
}

/// The outline of an empty slot: a dashed frame, as the design draws it.
fn dashed(list: &mut DrawList, rect: Rect, colour: Color) {
    const DASH: f32 = 6.0;
    const GAP: f32 = 4.0;
    const WIDTH: f32 = 2.0;
    let mut at = 0.0;
    while at < rect.w {
        let length = DASH.min(rect.w - at);
        list.fill(Rect::new(rect.x + at, rect.y, length, WIDTH), colour);
        list.fill(Rect::new(rect.x + at, rect.bottom() - WIDTH, length, WIDTH), colour);
        let length = DASH.min(rect.h - at);
        list.fill(Rect::new(rect.x, rect.y + at, WIDTH, length), colour);
        list.fill(Rect::new(rect.right() - WIDTH, rect.y + at, WIDTH, length), colour);
        at += DASH + GAP;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(name: &str, techniques: usize) -> StatusMember {
        StatusMember {
            name: name.to_owned(),
            level: 40,
            hp: 560,
            max_hp: 560,
            ki: 142,
            max_ki: 142,
            bp: 42673,
            pla: 6,
            exp: 65400,
            to_next: 7750,
            class: 2,
            face: Some(9),
            techniques: (0..techniques)
                .map(|i| Technique { name: format!("Technique {i}"), cost: 4 })
                .collect(),
        }
    }

    fn view(count: usize, cursor: usize) -> StatusView {
        let members = (0..count).map(|i| member(&format!("Fighter {i}"), 2)).collect();
        StatusView { members, cursor }
    }

    fn texts(list: &DrawList) -> Vec<String> {
        list.texts().map(|(_, _, text)| text.to_owned()).collect()
    }

    #[test]
    fn the_header_counts_the_party_over_the_nine_slots() {
        for (count, shown) in [(5, "5/9"), (1, "1/9"), (9, "9/9")] {
            assert!(texts(&status_screen(&view(count, 0))).contains(&shown.to_owned()), "{shown}");
        }
    }

    #[test]
    fn the_panel_shows_the_member_under_the_cursor_with_the_figures_in_the_originals_order() {
        let list = status_screen(&view(3, 1));
        let texts = texts(&list);
        let at =
            |text: &str| texts.iter().rposition(|t| t == text).unwrap_or_else(|| panic!("{text}"));
        let order = ["HP", "KI", "BP", "PLA.", "EXP", "To next LV"].map(at);
        assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");
        for figure in ["560 / 560", "142 / 142", "42673", "06", "65400", "7750", "Ma", "Demon"] {
            assert!(texts.contains(&figure.to_owned()), "{figure}");
        }
        let both = ["Technique 0", "Technique 1"];
        assert!(both.iter().all(|name| texts.contains(&(*name).to_owned())));
    }

    #[test]
    fn the_panel_follows_the_cursor() {
        let mut view = view(3, 0);
        view.members[2].name = "Piccolo".into();
        let first = texts(&status_screen(&view));
        view.cursor = 2;
        let third = texts(&status_screen(&view));
        // The name is in the list and, once under the cursor, in the panel as well.
        let count = |texts: &[String], name: &str| texts.iter().filter(|t| *t == name).count();
        assert_eq!(count(&first, "Piccolo"), 1);
        assert_eq!(count(&third, "Piccolo"), 2);
    }

    #[test]
    fn unused_slots_are_empty_frames_with_a_dash_and_a_full_party_has_none() {
        let list = status_screen(&view(2, 0));
        let dashes = texts(&list).iter().filter(|t| *t == "\u{2014}").count();
        assert_eq!(dashes, SLOTS - 2);
        let portraits = list.ops().iter().filter(|op| matches!(op, Op::Portrait { .. })).count();
        assert_eq!(portraits, 2 + 1, "two members in the list and the big portrait");
        let full = status_screen(&view(SLOTS, 8));
        assert_eq!(texts(&full).iter().filter(|t| *t == "\u{2014}").count(), 0);
    }

    #[test]
    fn nothing_leaves_the_four_to_three_core() {
        let list = status_screen(&view(SLOTS, 4));
        for (x, _, text) in list.texts() {
            assert!((160.0..=1120.0).contains(&x), "{text} at {x}");
        }
        for op in list.ops() {
            if let Op::Window { rect } = op {
                assert!(rect.x >= 160.0 && rect.right() <= 1120.0, "{rect:?}");
            }
        }
    }

    #[test]
    fn many_techniques_end_above_the_panels_foot() {
        for count in [1, 3, 4, 6, 8] {
            let mut view = view(1, 0);
            view.members[0] = member("Goku", count);
            let list = status_screen(&view);
            let lowest = list
                .texts()
                .filter(|(x, y, _)| *x > 700.0 && *y > 500.0)
                .map(|(_, y, _)| y)
                .fold(0.0_f32, f32::max);
            assert!(lowest <= 619.0, "{count} techniques end at {lowest}");
        }
    }

    #[test]
    fn a_class_the_game_has_no_kanji_for_shows_no_badge() {
        let mut view = view(1, 0);
        view.members[0].class = 9;
        let list = status_screen(&view);
        assert!(!list.ops().iter().any(|op| matches!(op, Op::Symbol { .. })));
        view.members[0].class = 1;
        let texts = texts(&status_screen(&view));
        assert!(texts.contains(&"Kami".to_owned()) && texts.contains(&"God".to_owned()));
    }
}
