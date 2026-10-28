//! The HD Cards screen: the hand of five cards drawn big, a cursor over it, and under it what the card under the cursor
//! is: its kanji with the reading and the meaning the battle's tooltip gives, its attack and defence numbers, and the
//! members of the party it favours (+25 percent battle power, `docs/re/data/cards.md`). The positions are those of the
//! approved design.
//!
//! The original's own Cards screen deals the five cards one at a time (a face-down card until its turn) and waits for A.
//! The HD screen reads the same hand: a card the original has not dealt yet shows its back, and the cards' faces are drawn
//! from the tiles the original wrote for them, so every kanji, ball and numeral is the game's own.

use crate::draw::{Align, CardPart, Color, DrawList, Op, Rect};
use crate::layout::{FRAME, LARGE_TEXT, LINE, SMALL_TEXT, fit, palette, text_width};
use crate::screens::pixel_card;
use crate::view::Picture;

/// Cards in the hand.
pub const HAND: usize = 5;
/// The value that is a Z ball (the card's corner shows a Z) and what it counts as in battle.
const Z_VALUE: u8 = 8;
const Z_COUNTS: u8 = 10;
/// The symbol that favours every fighter (`docs/re/data/cards.md`: the battle's test accepts 7 as a wildcard).
const WILD: u8 = 7;
/// The most fighters of the party the screen shows (the width of the row of portraits).
pub const FIGHTERS: usize = 5;

/// The reading and the meaning of each symbol, the same words as the battle's tooltip (`ssd_hd::tooltip`): symbol 0 to 6
/// of `docs/re/data/cards.md`, and the wildcard 7.
const WORDS: [(&str, &str); 8] = [
    ("Kai", "World"),
    ("Kami", "God"),
    ("Ma", "Demon"),
    ("Waku", "Doubt"),
    ("Ginyu", "Force"),
    ("Hitsu", "Sure"),
    ("Chou", "Super"),
    ("Any", "Wild"),
];

/// One card of the hand as the game holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Card {
    /// The first value (1 to 8): the ball in the top-left corner, the attack number.
    pub attack: u8,
    /// The second value (1 to 8): the ball in the bottom-right corner, the defence number.
    pub defence: u8,
    /// The symbol (0 to 6; 7 is the wildcard): the kanji in the middle.
    pub symbol: u8,
    /// The 4 x 6 tile words the game wrote for the card, row by row, for the backend to draw it from.
    pub tiles: Vec<u16>,
}

impl Card {
    /// What a value counts as in battle: a Z ball counts as 10.
    #[must_use]
    pub const fn counts(value: u8) -> u8 {
        if value == Z_VALUE { Z_COUNTS } else { value }
    }

    /// The reading and the meaning of the card's symbol.
    #[must_use]
    pub fn words(&self) -> (&'static str, &'static str) {
        WORDS[usize::from(self.symbol).min(WORDS.len() - 1)]
    }
}

/// A place of the hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Place {
    /// The original has not dealt the card yet: its back shows.
    Back,
    Card(Card),
}

/// A fighter of the party, for the row of the favoured.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fighter {
    pub name: String,
    /// The face index of the corner picture.
    pub face: Option<u8>,
    /// The fighter's class (`unit_stats` byte 0): a card whose symbol is it favours the fighter.
    pub class: Option<u8>,
}

/// The Cards screen as it shows: the hand, the card under the cursor and the party.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CardsView {
    pub hand: Vec<Place>,
    pub cursor: usize,
    pub party: Vec<Fighter>,
}

impl CardsView {
    /// The cards the original has dealt so far.
    #[must_use]
    pub fn dealt(&self) -> usize {
        self.hand.iter().filter(|place| matches!(place, Place::Card(_))).count()
    }

    /// The card under the cursor, once it is dealt.
    #[must_use]
    pub fn card(&self) -> Option<&Card> {
        match self.hand.get(self.cursor)? {
            Place::Card(card) => Some(card),
            Place::Back => None,
        }
    }
}

/// Whether `card` favours `fighter` (+25 percent battle power for the fighter: the battle's test, `05:B6F2`): its symbol is
/// the fighter's class, or the wildcard.
#[must_use]
pub fn favours(card: &Card, fighter: &Fighter) -> bool {
    card.symbol == WILD || fighter.class == Some(card.symbol)
}

/// The names of the fighters of `party` the card favours, in party order.
fn favoured<'a>(card: &Card, party: &'a [Fighter]) -> Vec<&'a str> {
    party.iter().take(FIGHTERS).filter(|f| favours(card, f)).map(|f| f.name.as_str()).collect()
}

/// The help line of a card: what its kanji does for the party.
#[must_use]
pub fn help(card: &Card, party: &[Fighter]) -> String {
    let (_, meaning) = card.words();
    let names = favoured(card, party);
    let party = party.len().min(FIGHTERS);
    let line = match names.as_slice() {
        [] => format!("A {meaning} card: nobody in the party hits harder with it."),
        [one] => format!("A {meaning} card: {one} hits 25% harder with it."),
        _ if names.len() == party && card.symbol == WILD => {
            format!("A {meaning} card: everyone hits 25% harder with it.")
        }
        [first @ .., last] => {
            format!("A {meaning} card: {} and {last} hit 25% harder with it.", first.join(", "))
        }
    };
    // Five names do not fit the strip at its size: count them instead
    if text_width(&line, HELP_TEXT) <= STRIP_TEXT_WIDTH {
        line
    } else {
        format!("A {meaning} card: {} of the party hit 25% harder with it.", names.len())
    }
}

// The design's positions, in the 1280 x 720 space.
const WINDOW_X: f32 = 176.0;
const WINDOW_W: f32 = 928.0;
const HAND_WINDOW: Rect = Rect::new(WINDOW_X, 30.0, WINDOW_W, 357.0);
const DETAIL_WINDOW: Rect = Rect::new(WINDOW_X, 410.0, WINDOW_W, 160.0);
const STRIP_WINDOW: Rect = Rect::new(WINDOW_X, 592.0, WINDOW_W, 88.0);
const CARD_W: f32 = 160.0;
const CARD_H: f32 = 240.0;
const CARD_X: f32 = 200.0;
const CARD_Y: f32 = 124.0;
const CARD_PITCH: f32 = 180.0;
/// The size of the text of the help line, and the width it has in the strip.
const HELP_TEXT: f32 = 22.0;
const STRIP_TEXT_WIDTH: f32 = 868.0;
const TITLE_TEXT: f32 = LARGE_TEXT;
/// How much of a fighter who is not favoured is left (the design's 38 percent), as the alpha of the paper laid over.
const DIM_ALPHA: u8 = 158;
/// The side of a portrait of the row of the favoured, and the pitch.
const FACE: f32 = 60.0;
const FACE_PITCH: f32 = 72.0;

/// The card cursor: the card's frame, a ring of orange and dark red outside it.
fn selection(list: &mut DrawList, card: Rect) {
    list.push(Op::Outline { rect: card.grow(6.0), width: 2.0, color: palette::DARK_RED });
    list.push(Op::Outline { rect: card.grow(4.0), width: 4.0, color: palette::ORANGE });
}

/// The back of a card the original has not dealt yet: the pink and yellow card of the game with a Z, since the dragon the
/// game draws there is not part of the card art.
fn back(list: &mut DrawList, card: Rect) {
    list.fill(card, Color::rgb(0xF9, 0xF6, 0xE7));
    list.fill(card.grow(-6.0), Color::rgb(0xFF, 0x63, 0xBD));
    list.fill(card.grow(-18.0), Color::rgb(0xFF, 0xFF, 0x63));
    let disc = Rect::new(card.x + 30.0, card.y + 70.0, card.w - 60.0, card.w - 60.0);
    list.push(Op::Disc { rect: disc, color: Color::rgb(0x52, 0x4A, 0xFF) });
    list.text(disc.x + disc.w / 2.0, disc.middle_y(), "Z", 60.0, palette::CARD, Align::Centre);
}

/// The Cards screen of `view`.
#[must_use]
pub fn cards_screen(view: &CardsView) -> DrawList {
    let mut list = DrawList::new();
    list.push(Op::Backdrop);
    hand_window(&mut list, view);
    detail_window(&mut list, view);
    strip_window(&mut list, view);
    list
}

fn hand_window(list: &mut DrawList, view: &CardsView) {
    let window = HAND_WINDOW;
    list.push(Op::Window { rect: window });
    list.text(window.x + 30.0, window.y + 39.0, "Cards", TITLE_TEXT, palette::BLACK, Align::Left);
    let count = format!("Hand {}/{HAND}", view.dealt());
    list.text(
        window.x + window.w - 30.0,
        window.y + 39.0,
        &count,
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Right,
    );
    list.fill(
        Rect::new(window.x + 30.0, window.y + 66.0, window.w - 60.0, LINE),
        palette::DARK_RED,
    );
    for (at, place) in view.hand.iter().enumerate().take(HAND) {
        let rect = Rect::new(CARD_X + CARD_PITCH * at as f32, CARD_Y, CARD_W, CARD_H);
        match place {
            Place::Back => back(list, rect),
            Place::Card(card) => {
                list.push(Op::HandCard { rect, tiles: card.tiles.clone(), part: CardPart::Whole });
            }
        }
        if at == view.cursor && matches!(place, Place::Card(_)) {
            selection(list, rect);
            list.push(Op::CursorDown { x: rect.x + rect.w / 2.0 - 9.0, y: rect.y - 21.0 });
        }
    }
}

fn detail_window(list: &mut DrawList, view: &CardsView) {
    let window = DETAIL_WINDOW;
    list.push(Op::Window { rect: window });
    let Some(card) = view.card() else { return };
    let (x, y) = (window.x, window.y);
    // The kanji, with its reading and its meaning
    list.push(Op::HandCard {
        rect: Rect::new(x + 30.0, y + 31.0, 88.0, 88.0),
        tiles: card.tiles.clone(),
        part: CardPart::Symbol,
    });
    let (reading, meaning) = card.words();
    list.text(x + 136.0, y + 56.0, reading, TITLE_TEXT, palette::BLACK, Align::Left);
    list.text(x + 136.0, y + 92.0, meaning, SMALL_TEXT, palette::DARK_RED, Align::Left);
    // The two numbers
    for (row, (label, value, part)) in
        [("Attack", card.attack, CardPart::Attack), ("Defence", card.defence, CardPart::Defence)]
            .into_iter()
            .enumerate()
    {
        let top = y + 28.0 + 56.0 * row as f32;
        list.push(Op::HandCard {
            rect: Rect::new(x + 262.0, top, 44.0, 44.0),
            tiles: card.tiles.clone(),
            part,
        });
        list.text(x + 318.0, top + 22.0, label, 20.0, palette::BLACK, Align::Left);
        let counts = Card::counts(value).to_string();
        list.text(x + 470.0, top + 22.0, &counts, TITLE_TEXT, palette::BLACK, Align::Right);
    }
    list.fill(Rect::new(x + 506.0, y + 26.0, 2.0, 108.0), palette::DARK_RED);
    // Who the card favours
    list.text(
        x + 530.0,
        y + 34.0,
        "Favoured by (+25% power)",
        SMALL_TEXT,
        palette::DARK_RED,
        Align::Left,
    );
    for (at, fighter) in view.party.iter().take(FIGHTERS).enumerate() {
        let face = Rect::new(x + 530.0 + FACE_PITCH * at as f32, y + 54.0, FACE, FACE);
        list.push(Op::Portrait { rect: face, picture: fighter.face.map(Picture::Face) });
        if favours(card, fighter) {
            list.push(Op::Outline { rect: face.grow(7.0), width: 2.0, color: palette::DARK_RED });
            list.push(Op::Outline { rect: face.grow(5.0), width: 3.0, color: palette::ORANGE });
            let name = fit(&fighter.name, SMALL_TEXT, FACE_PITCH - 4.0);
            list.text(
                face.x + FACE / 2.0,
                y + 132.0,
                &name,
                SMALL_TEXT,
                palette::DARK_RED,
                Align::Centre,
            );
        } else {
            list.fill(face.grow(2.0), Color([0xFF, 0xEF, 0xCE, DIM_ALPHA]));
        }
    }
}

fn strip_window(list: &mut DrawList, view: &CardsView) {
    let window = STRIP_WINDOW;
    list.push(Op::Window { rect: window });
    let line = match view.card() {
        Some(card) => help(card, &view.party),
        None => "Dealing the hand...".to_owned(),
    };
    list.text(window.x + 30.0, window.y + 30.0, &line, HELP_TEXT, palette::BLACK, Align::Left);
    for (text, at) in [("Left/Right  Browse", 30.0), ("B  Back", 250.0), ("Start  Close", 370.0)] {
        list.text(window.x + at, window.y + 64.0, text, SMALL_TEXT, palette::DARK_RED, Align::Left);
    }
}

/// A part of a card drawn flat, for a backend that has no card art: the whole card as [`pixel_card`], a ball as an orange
/// disc, the kanji as a cream square.
#[must_use]
pub fn flat_part(rect: Rect, part: CardPart) -> DrawList {
    let mut list = DrawList::new();
    match part {
        CardPart::Whole => list.append(pixel_card(rect, false, false)),
        CardPart::Attack | CardPart::Defence => {
            list.push(Op::Disc { rect, color: palette::BLACK });
            list.push(Op::Disc { rect: rect.grow(-2.0), color: palette::CARD });
        }
        CardPart::Symbol => {
            list.fill(rect, palette::ORANGE);
            list.fill(rect.grow(-FRAME.min(rect.w / 4.0)), palette::CREAM);
        }
    }
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(attack: u8, defence: u8, symbol: u8) -> Card {
        Card { attack, defence, symbol, tiles: vec![0; 24] }
    }

    fn fighter(name: &str, class: u8) -> Fighter {
        Fighter { name: name.to_owned(), face: None, class: Some(class) }
    }

    fn party() -> Vec<Fighter> {
        vec![
            fighter("Goku", 1),
            fighter("Gohan", 2),
            fighter("Piccolo", 2),
            fighter("Krillin", 1),
            fighter("Tien", 1),
        ]
    }

    fn view(hand: Vec<Place>, cursor: usize) -> CardsView {
        CardsView { hand, cursor, party: party() }
    }

    fn five(symbol: u8) -> Vec<Place> {
        vec![Place::Card(card(5, 3, symbol)); HAND]
    }

    fn texts(list: &DrawList) -> Vec<String> {
        list.texts().map(|(_, _, text)| text.to_owned()).collect()
    }

    #[test]
    fn a_card_favours_the_fighters_whose_class_is_its_symbol() {
        let party = party();
        let demon = card(5, 3, 2);
        let who: Vec<&str> =
            party.iter().filter(|f| favours(&demon, f)).map(|f| &*f.name).collect();
        assert_eq!(
            who,
            ["Gohan", "Piccolo"],
            "the Demon card is the half of the party that is class 2"
        );
        let god = card(5, 3, 1);
        let who: Vec<&str> = party.iter().filter(|f| favours(&god, f)).map(|f| &*f.name).collect();
        assert_eq!(who, ["Goku", "Krillin", "Tien"]);
        assert!(party.iter().all(|f| !favours(&card(5, 3, 6), f)), "Super favours no one here");
        assert!(party.iter().all(|f| favours(&card(5, 3, 7), f)), "the wildcard favours all");
        assert!(!favours(&demon, &Fighter { name: "X".into(), face: None, class: None }));
    }

    #[test]
    fn a_z_ball_counts_as_ten() {
        assert_eq!([1, 7, 8].map(Card::counts), [1, 7, 10]);
    }

    #[test]
    fn the_help_line_says_in_plain_words_who_hits_harder() {
        let party = party();
        assert_eq!(
            help(&card(5, 3, 2), &party),
            "A Demon card: Gohan and Piccolo hit 25% harder with it."
        );
        assert_eq!(
            help(&card(5, 3, 1), &party),
            "A God card: Goku, Krillin and Tien hit 25% harder with it."
        );
        assert_eq!(
            help(&card(5, 3, 6), &party),
            "A Super card: nobody in the party hits harder with it."
        );
        assert_eq!(
            help(&card(5, 3, 2), &party[..2]),
            "A Demon card: Gohan hits 25% harder with it."
        );
        assert_eq!(help(&card(5, 3, 7), &party), "A Wild card: everyone hits 25% harder with it.");
    }

    #[test]
    fn a_help_line_that_does_not_fit_the_strip_counts_the_party_instead() {
        let long = ["Goku", "Gohan", "Krillin", "Yamcha", "Chiaotzu"].map(|name| fighter(name, 1));
        let line = help(&card(5, 3, 1), &long);
        assert_eq!(line, "A God card: 5 of the party hit 25% harder with it.");
        assert!(text_width(&line, HELP_TEXT) <= STRIP_TEXT_WIDTH);
    }

    #[test]
    fn every_help_line_of_every_symbol_fits_the_strip() {
        let names = ["Goku", "Piccolo", "Krillin", "Chiaotzu", "Yamcha"];
        for symbol in 0..=7 {
            let party: Vec<Fighter> = names.map(|name| fighter(name, symbol)).to_vec();
            let line = help(&card(1, 1, symbol), &party);
            assert!(text_width(&line, HELP_TEXT) <= STRIP_TEXT_WIDTH, "{line}");
        }
    }

    #[test]
    fn the_screen_has_the_design_windows_texts_and_keys() {
        let list = cards_screen(&view(five(2), 4));
        let windows: Vec<Rect> = list
            .ops()
            .iter()
            .filter_map(|op| if let Op::Window { rect } = op { Some(*rect) } else { None })
            .collect();
        assert_eq!(windows, [HAND_WINDOW, DETAIL_WINDOW, STRIP_WINDOW]);
        let all = texts(&list);
        for expected in [
            "Cards",
            "Hand 5/5",
            "Ma",
            "Demon",
            "Attack",
            "Defence",
            "Favoured by (+25% power)",
            "Gohan",
            "Piccolo",
            "Left/Right  Browse",
            "B  Back",
            "Start  Close",
        ] {
            assert!(all.iter().any(|text| text == expected), "{expected}: {all:?}");
        }
        assert!(all.iter().any(|text| text.starts_with("A Demon card:")));
    }

    #[test]
    fn the_hand_is_five_cards_across_with_the_cursor_ring_and_arrow_on_the_chosen_one() {
        let list = cards_screen(&view(five(2), 4));
        let cards: Vec<Rect> = list
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::HandCard { rect, part: CardPart::Whole, .. } => Some(*rect),
                _ => None,
            })
            .collect();
        assert_eq!(cards.len(), HAND);
        assert_eq!((cards[0].x, cards[4].x), (200.0, 920.0), "the design's positions");
        for (at, card) in cards.iter().enumerate() {
            assert_eq!(*card, Rect::new(200.0 + 180.0 * at as f32, 124.0, 160.0, 240.0));
        }
        assert!(
            list.ops()
                .iter()
                .any(|op| matches!(op, Op::Outline { rect, .. } if *rect == cards[4].grow(4.0))),
            "the ring is round the card under the cursor"
        );
        let arrows: Vec<(f32, f32)> = list
            .ops()
            .iter()
            .filter_map(|op| if let Op::CursorDown { x, y } = op { Some((*x, *y)) } else { None })
            .collect();
        assert_eq!(arrows, [(991.0, 103.0)], "the arrow over the fifth card, as in the design");
    }

    #[test]
    fn the_numbers_are_drawn_with_a_z_as_ten() {
        let mut hand = five(1);
        hand[0] = Place::Card(card(8, 2, 1));
        let list = cards_screen(&view(hand, 0));
        let numbers: Vec<String> = list
            .ops()
            .iter()
            .filter_map(|op| match op {
                Op::Text { text, size, align: Align::Right, .. }
                    if (*size - TITLE_TEXT).abs() < f32::EPSILON =>
                {
                    Some(text.clone())
                }
                _ => None,
            })
            .collect();
        assert_eq!(numbers, ["10", "2"]);
    }

    #[test]
    fn the_favoured_are_ringed_with_their_names_and_the_others_dimmed() {
        let list = cards_screen(&view(five(2), 0));
        let faces: Vec<Rect> = list
            .ops()
            .iter()
            .filter_map(|op| if let Op::Portrait { rect, .. } = op { Some(*rect) } else { None })
            .collect();
        assert_eq!(faces.len(), 5, "the party's five");
        assert_eq!((faces[0].x, faces[0].y, faces[0].w), (706.0, 464.0, 60.0));
        let ringed = |face: Rect| {
            list.ops()
                .iter()
                .any(|op| matches!(op, Op::Outline { rect, .. } if *rect == face.grow(5.0)))
        };
        let dimmed = |face: Rect| {
            list.ops().iter().any(|op| {
                matches!(op, Op::Fill { rect, color } if *rect == face.grow(2.0) && color.0[3] < 255)
            })
        };
        let order = [false, true, true, false, false];
        for (face, favoured) in faces.iter().zip(order) {
            assert_eq!(ringed(*face), favoured);
            assert_eq!(dimmed(*face), !favoured);
        }
    }

    #[test]
    fn a_card_not_dealt_yet_shows_its_back_and_no_detail() {
        let mut hand = five(2);
        hand[3] = Place::Back;
        hand[4] = Place::Back;
        let list = cards_screen(&view(hand.clone(), 3));
        let faces = list
            .ops()
            .iter()
            .filter(|op| matches!(op, Op::HandCard { part: CardPart::Whole, .. }))
            .count();
        assert_eq!(faces, 3, "three faces and two backs");
        let all = texts(&list);
        assert!(all.iter().any(|text| text == "Hand 3/5"));
        assert!(all.iter().any(|text| text == "Dealing the hand..."));
        assert!(!all.iter().any(|text| text == "Attack"), "no detail for a card not dealt");
        assert!(
            !list.ops().iter().any(|op| matches!(op, Op::CursorDown { .. })),
            "no cursor on a card not dealt"
        );
    }

    #[test]
    fn the_symbol_words_are_the_tooltips() {
        let words: Vec<_> = (0..=7).map(|symbol| card(1, 1, symbol).words()).collect();
        assert_eq!(
            words[..7],
            [
                ("Kai", "World"),
                ("Kami", "God"),
                ("Ma", "Demon"),
                ("Waku", "Doubt"),
                ("Ginyu", "Force"),
                ("Hitsu", "Sure"),
                ("Chou", "Super"),
            ]
        );
    }
}
