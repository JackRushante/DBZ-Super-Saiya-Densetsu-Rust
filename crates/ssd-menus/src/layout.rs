//! The layout and palette of the HD menus: the approved design (spec §8, the canvas of the field
//! menu and of the items). Every size is in logical pixels of a [`WIDTH`] x [`HEIGHT`] space; the
//! backend scales that space to the window.

/// Width of the logical space.
pub const WIDTH: f32 = 1280.0;
/// Height of the logical space.
pub const HEIGHT: f32 = 720.0;

/// Distance of the windows from the edges of the space (the command window's left and bottom).
pub const MARGIN: f32 = 40.0;
/// Width of the orange frame of a window.
pub const FRAME: f32 = 6.0;
/// Width of the dark-red lines just outside and just inside the frame.
pub const LINE: f32 = 2.0;
/// Side of a corner cap (a small square at each corner of a window).
pub const CAP: f32 = 12.0;
/// How far a corner cap sticks out of the window's corner.
pub const CAP_OVERHANG: f32 = 9.0;
/// Padding inside a window, from its frame to its content.
pub const PADDING: f32 = 16.0;
/// Size of the orange cursor (a right-pointing triangle).
pub const CURSOR_WIDTH: f32 = 16.0;
pub const CURSOR_HEIGHT: f32 = 18.0;
/// Height of one row of the command window.
pub const COMMAND_ROW: f32 = 44.0;
/// Height of one row of the item list.
pub const ITEM_ROW: f32 = 46.0;
/// Height of one line of help text.
pub const HELP_LINE: f32 = 48.0;
/// Font size of the names, labels and help text, in Share Tech Mono (the game's HD text face): 10 rows of the
/// original's em, at this space's 720 pixels for the 239 rows of the 16:9 view, which is 30 pixels with caps
/// 21 pixels high, the size of the game's own text beside the menus.
pub const LARGE_TEXT: f32 = 30.0;
/// Font size of the small labels (HP, KI, page numbers, hints).
pub const SMALL_TEXT: f32 = 16.0;
/// The advance of every character of Share Tech Mono, in ems: 540 of its 1000 units, the same for all of them
/// (the face is monospaced), so a line's width is its characters times the size times this.
pub const ADVANCE: f32 = 0.54;
/// Side of a corner portrait of the field menu, frame included.
pub const CORNER_PORTRAIT: f32 = 104.0;
/// Width of the item portrait, frame included.
pub const ITEM_PORTRAIT: f32 = 150.0;
/// Frame of a portrait: black, with a white line just outside.
pub const PORTRAIT_FRAME: f32 = 3.0;
/// Width and height of the two HP and KI bars, border included.
pub const BAR_HEIGHT: f32 = 12.0;
/// Border of a bar: white, around the black well.
pub const BAR_BORDER: f32 = 2.0;
/// Side of a picture of an object: 32 x 32 pixels, drawn at an integer scale.
pub const PICTURE: f32 = 32.0;

/// The colours of the design.
pub mod palette {
    use crate::draw::Color;

    /// The paper of the windows.
    pub const CREAM: Color = Color::rgb(0xFF, 0xEF, 0xCE);
    /// The frame of the windows and the cursor.
    pub const ORANGE: Color = Color::rgb(0xFF, 0x52, 0x00);
    /// The lines around the frame, the outline of the cursor, the hints and the footers.
    pub const DARK_RED: Color = Color::rgb(0x84, 0x00, 0x00);
    /// The corner caps of the windows.
    pub const CAP: Color = Color::rgb(0xFF, 0xB5, 0x73);
    /// Black: the text, the outlines of the portraits and the bars' wells.
    pub const BLACK: Color = Color::rgb(0x00, 0x00, 0x00);
    /// White: the text on the field, the lines around the portraits and the bars.
    pub const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
    /// The orange of a card's frame and balls (the card preview of the cheats screen).
    pub const CARD: Color = Color::rgb(0xFF, 0x8C, 0x00);
    /// The purple price box.
    pub const PRICE: Color = Color::rgb(0x9C, 0x6B, 0xFF);
    /// The digits of the price box.
    pub const PRICE_DIGITS: Color = Color::rgb(0xFF, 0xFF, 0x63);
    /// The HP bar.
    pub const HP: Color = Color::rgb(0xFF, 0x8C, 0x00);
    /// The KI bar.
    pub const KI: Color = Color::rgb(0x52, 0x4A, 0xFF);
    /// The text of the entries that are greyed out (their label and their "coming soon").
    pub const GREY: Color = Color::rgb(0x8E, 0x82, 0x72);
    /// The story objects of the item spawner and its warning.
    pub const STORY: Color = Color::rgb(0xD0, 0x10, 0x10);
    /// The warning's background.
    pub const WARNING: Color = Color::rgb(0xFF, 0xE0, 0xE0);
}

/// The width in pixels of `text` set in `size` pixels of Share Tech Mono.
#[must_use]
pub fn text_width(text: &str, size: f32) -> f32 {
    text.chars().count() as f32 * ADVANCE * size
}

/// `text`, or as much of its start as fits `max_width` pixels at `size`, ending in an ellipsis when it is cut.
#[must_use]
pub fn fit(text: &str, size: f32, max_width: f32) -> String {
    if text_width(text, size) <= max_width {
        return text.to_owned();
    }
    let room = (max_width / (ADVANCE * size)).floor().max(0.0) as usize;
    text.chars().take(room.saturating_sub(1)).chain(std::iter::once('\u{2026}')).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether two design sizes are the same, to a thousandth of a pixel.
    fn same(size: f32, design: f32) -> bool {
        (size - design).abs() < 1e-3
    }

    #[test]
    fn the_space_and_the_margins_are_the_design_ones() {
        assert!(same(WIDTH, 1280.0) && same(HEIGHT, 720.0));
        assert!(same(FRAME, 6.0), "the frame is 6 px");
        assert!(same(CAP, 12.0), "corner caps are 12 px squares");
        // The large text is the game's HD text size: 10 rows of its em over the 239 rows of the 16:9 view.
        assert!((LARGE_TEXT - 10.0 * HEIGHT / 239.0).abs() < 0.2, "{LARGE_TEXT}");
    }

    #[test]
    fn a_line_is_as_wide_as_its_characters_in_share_tech_mono() {
        assert!(same(text_width("Items", 30.0), 5.0 * 0.54 * 30.0));
        assert!(same(text_width("", 30.0), 0.0));
        assert!(same(text_width("\u{2026}", 16.0), 0.54 * 16.0));
    }

    #[test]
    fn a_text_that_does_not_fit_is_cut_with_an_ellipsis_and_one_that_fits_is_kept() {
        assert_eq!(fit("Turtle", 30.0, 500.0), "Turtle");
        // Six characters are 97.2 pixels at 30 px (16.2 each): in 90, five slots fit, four letters and the ellipsis.
        assert_eq!(fit("Dragon", 30.0, 90.0), "Drag\u{2026}");
        assert_eq!(fit("Dragon", 30.0, 16.0), "\u{2026}", "only the ellipsis fits");
        assert_eq!(fit("Dragon", 30.0, 0.0), "\u{2026}");
    }
}
