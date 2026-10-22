//! The line encoder: the inverse of the English font's tile table, for the printable set of the message
//! engine (`docs/re/data/text-messages.md`, sections 0.5, 0.7 and 4). A line of text becomes the pieces the
//! engine writes: a tile for each character and a token for each `{...}`.
//!
//! The printable set: the space, `a` to `z`, `A` to `Z`, `0` to `9`, `! ? . , ' -`, the ellipsis `…`
//! and the quote marks. A quote mark is the opening one (`7E`) when it is the first character of the line
//! or follows a space, else the closing one (`7F`). The tokens are `{name:1}` to `{name:3}`, `{number:1}`
//! to `{number:3}` and `{portrait N}` (N from 0 to 255).

/// One thing a line is made of: a tile (one cell), a name or a number slot, or a portrait (no cell).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece {
    /// A tile code: a character, or the space `01`.
    Tile(u8),
    /// The name slot `{name:N}` (N from 1 to 3): `FB` for 1, `FA` for 2, `F9` for 3.
    Name(u8),
    /// The number slot `{number:N}` (N from 1 to 3): `F8` for 1, `F7` for 2, `F6` for 3.
    Number(u8),
    /// The portrait `{portrait N}`: `F4` and its number.
    Portrait(u8),
}

/// The cells a name slot counts for: the most a name's tiles can be (the copy buffer's limit).
pub const NAME_CELLS: usize = 15;
/// The cells a number slot counts for, the worst case the compiler budgets.
pub const NUMBER_CELLS: usize = 8;
/// The space: the engine moves on a cell and writes nothing.
pub const SPACE: Piece = Piece::Tile(0x01);

impl Piece {
    /// The cells the piece takes in the window.
    #[must_use]
    pub fn cells(self) -> usize {
        match self {
            Self::Tile(_) => 1,
            Self::Name(_) => NAME_CELLS,
            Self::Number(_) => NUMBER_CELLS,
            Self::Portrait(_) => 0,
        }
    }

    /// Appends the bytes of the piece to `out`.
    pub fn write(self, out: &mut Vec<u8>) {
        match self {
            Self::Tile(code) => out.push(code),
            Self::Name(slot) => out.push(match slot {
                1 => 0xFB,
                2 => 0xFA,
                _ => 0xF9,
            }),
            Self::Number(slot) => out.push(match slot {
                1 => 0xF8,
                2 => 0xF7,
                _ => 0xF6,
            }),
            Self::Portrait(number) => out.extend([0xF4, number]),
        }
    }
}

/// A line that does not encode: what is wrong with it, and where in the line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineError {
    /// A character the English font has no tile for, at `column` (1-based, in characters).
    Character { ch: char, column: usize },
    /// A `{` with no `}` after it.
    Unclosed,
    /// A `{...}` that is not one of the tokens (the text between the braces, braces included).
    Token(String),
}

/// The pieces of one line of text.
///
/// # Errors
///
/// [`LineError::Character`] for a character outside the printable set, [`LineError::Unclosed`] for a `{`
/// without its `}`, [`LineError::Token`] for a `{...}` that is not a token.
pub fn line(text: &str) -> Result<Vec<Piece>, LineError> {
    let chars: Vec<char> = text.chars().collect();
    let mut pieces = Vec::with_capacity(chars.len());
    // The source character before the one being encoded: `None` at the start of the line.
    let mut previous: Option<char> = None;
    let mut at = 0;
    while at < chars.len() {
        let ch = chars[at];
        if ch == '{' {
            let close = chars[at..].iter().position(|&c| c == '}').ok_or(LineError::Unclosed)?;
            let inner: String = chars[at + 1..at + close].iter().collect();
            let piece = token(&inner).ok_or_else(|| LineError::Token(format!("{{{inner}}}")))?;
            pieces.push(piece);
            previous = Some('}');
            at += close + 1;
            continue;
        }
        let code = if ch == '"' {
            if previous.is_none_or(|before| before == ' ') { 0x7E } else { 0x7F }
        } else {
            tile(ch).ok_or(LineError::Character { ch, column: at + 1 })?
        };
        pieces.push(Piece::Tile(code));
        previous = Some(ch);
        at += 1;
    }
    Ok(pieces)
}

/// The tile of a printable character, except the quote marks (see [`line`]).
fn tile(ch: char) -> Option<u8> {
    Some(match ch {
        ' ' => 0x01,
        'a' => 0x05,
        'b' => 0x07,
        'c' => 0x09,
        'd' => 0x0A,
        'e' => 0x0C,
        'f' => 0x0D,
        'g' => 0x0E,
        'h' => 0x11,
        'i' => 0x15,
        'j' => 0x16,
        'k' => 0x17,
        'l' => 0x18,
        'm' => 0x19,
        'n' => 0x1A,
        'o' => 0x1B,
        'p' => 0x1D,
        'q' => 0x51,
        'r' => 0x52,
        's' => 0x53,
        't' => 0x54,
        'u' => 0x55,
        'v' => 0x56,
        'w' => 0x57,
        'x' => 0x58,
        'y' => 0x5B,
        'z' => 0x5C,
        'A' => 0xE8,
        'B' => 0x8C,
        'C' => 0xE9,
        'D' => 0xEA,
        'E' => 0xEB,
        'F' => 0xEC,
        'G' => 0xED,
        'H' => 0x8A,
        'I' => 0x8E,
        'J' => 0x04,
        'K' => 0x8D,
        'L' => 0xEE,
        'M' => 0x0F,
        'N' => 0x50,
        'O' => 0x5A,
        'P' => 0x8B,
        'Q' => 0x12,
        'R' => 0x06,
        'S' => 0x14,
        'T' => 0x47,
        'U' => 0x0B,
        'V' => 0xEF,
        'W' => 0x10,
        'X' => 0x08,
        'Y' => 0x45,
        'Z' => 0x13,
        '0'..='9' => 0x80 + (ch as u8 - b'0'),
        '!' => 0x3F,
        '?' => 0x3E,
        '.' => 0x7C,
        ',' => 0x7B,
        '\'' => 0x42,
        '-' => 0x1C,
        '…' => 0x40,
        _ => return None,
    })
}

/// The piece of the text between a `{` and its `}`, `None` when it is not a token.
fn token(inner: &str) -> Option<Piece> {
    if let Some(slot) = inner.strip_prefix("name:") {
        return slot_of(slot).map(Piece::Name);
    }
    if let Some(slot) = inner.strip_prefix("number:") {
        return slot_of(slot).map(Piece::Number);
    }
    let number = inner.strip_prefix("portrait ")?;
    if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    number.parse::<u8>().ok().map(Piece::Portrait)
}

/// The slot a token names: 1, 2 or 3.
fn slot_of(text: &str) -> Option<u8> {
    match text {
        "1" => Some(1),
        "2" => Some(2),
        "3" => Some(3),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bytes of a line: a piece per tile, a token's bytes in place.
    fn bytes(text: &str) -> Vec<u8> {
        let mut out = Vec::new();
        for piece in line(text).expect("the line encodes") {
            piece.write(&mut out);
        }
        out
    }

    #[test]
    fn every_printable_character_has_its_tile() {
        // The codes of `docs/re/data/text-messages.md`, section 4, one per character.
        let table: [(char, u8); 26 + 26 + 10 + 8] = [
            ('a', 0x05),
            ('b', 0x07),
            ('c', 0x09),
            ('d', 0x0A),
            ('e', 0x0C),
            ('f', 0x0D),
            ('g', 0x0E),
            ('h', 0x11),
            ('i', 0x15),
            ('j', 0x16),
            ('k', 0x17),
            ('l', 0x18),
            ('m', 0x19),
            ('n', 0x1A),
            ('o', 0x1B),
            ('p', 0x1D),
            ('q', 0x51),
            ('r', 0x52),
            ('s', 0x53),
            ('t', 0x54),
            ('u', 0x55),
            ('v', 0x56),
            ('w', 0x57),
            ('x', 0x58),
            ('y', 0x5B),
            ('z', 0x5C),
            ('A', 0xE8),
            ('B', 0x8C),
            ('C', 0xE9),
            ('D', 0xEA),
            ('E', 0xEB),
            ('F', 0xEC),
            ('G', 0xED),
            ('H', 0x8A),
            ('I', 0x8E),
            ('J', 0x04),
            ('K', 0x8D),
            ('L', 0xEE),
            ('M', 0x0F),
            ('N', 0x50),
            ('O', 0x5A),
            ('P', 0x8B),
            ('Q', 0x12),
            ('R', 0x06),
            ('S', 0x14),
            ('T', 0x47),
            ('U', 0x0B),
            ('V', 0xEF),
            ('W', 0x10),
            ('X', 0x08),
            ('Y', 0x45),
            ('Z', 0x13),
            ('0', 0x80),
            ('1', 0x81),
            ('2', 0x82),
            ('3', 0x83),
            ('4', 0x84),
            ('5', 0x85),
            ('6', 0x86),
            ('7', 0x87),
            ('8', 0x88),
            ('9', 0x89),
            ('!', 0x3F),
            ('?', 0x3E),
            ('.', 0x7C),
            (',', 0x7B),
            ('\'', 0x42),
            ('-', 0x1C),
            ('…', 0x40),
            (' ', 0x01),
        ];
        for (ch, code) in table {
            assert_eq!(bytes(&ch.to_string()), [code], "{ch:?}");
        }
    }

    #[test]
    fn every_token_is_its_slot_or_portrait_code() {
        assert_eq!(bytes("{name:1}"), [0xFB]);
        assert_eq!(bytes("{name:2}"), [0xFA]);
        assert_eq!(bytes("{name:3}"), [0xF9]);
        assert_eq!(bytes("{number:1}"), [0xF8]);
        assert_eq!(bytes("{number:2}"), [0xF7]);
        assert_eq!(bytes("{number:3}"), [0xF6]);
        assert_eq!(bytes("{portrait 5}"), [0xF4, 0x05]);
        assert_eq!(bytes("{portrait 0}"), [0xF4, 0x00]);
        assert_eq!(bytes("{portrait 255}"), [0xF4, 0xFF]);
        assert_eq!(bytes("a{name:1}!"), [0x05, 0xFB, 0x3F], "a token among the tiles");
    }

    #[test]
    fn a_token_has_its_cells_and_a_portrait_none() {
        assert_eq!(Piece::Name(1).cells(), NAME_CELLS);
        assert_eq!(Piece::Number(3).cells(), NUMBER_CELLS);
        assert_eq!(Piece::Portrait(9).cells(), 0);
        assert_eq!(SPACE.cells(), 1);
    }

    #[test]
    fn a_quote_opens_at_the_start_of_a_line_or_after_a_space_and_closes_otherwise() {
        assert_eq!(bytes("\"Hi\""), [0x7E, 0x8A, 0x15, 0x7F], "at the start, then after a letter");
        assert_eq!(bytes("a \"b\""), [0x05, 0x01, 0x7E, 0x07, 0x7F], "after a space: opening");
        assert_eq!(bytes("ok.\""), [0x1B, 0x17, 0x7C, 0x7F], "after a full stop: closing");
        assert_eq!(bytes("{name:1}\""), [0xFB, 0x7F], "after a token: closing");
    }

    #[test]
    fn a_character_outside_the_set_is_named_with_its_column() {
        assert_eq!(line("a;b"), Err(LineError::Character { ch: ';', column: 2 }));
        assert_eq!(line("x(y"), Err(LineError::Character { ch: '(', column: 2 }));
        assert_eq!(line("A\tB"), Err(LineError::Character { ch: '\t', column: 2 }));
        assert_eq!(line("a}"), Err(LineError::Character { ch: '}', column: 2 }));
    }

    #[test]
    fn a_brace_that_is_no_token_or_is_not_closed_is_an_error() {
        assert_eq!(line("{name:1"), Err(LineError::Unclosed));
        assert_eq!(line("{name:4}"), Err(LineError::Token("{name:4}".into())));
        assert_eq!(line("{portrait 256}"), Err(LineError::Token("{portrait 256}".into())));
        assert_eq!(line("{portrait}"), Err(LineError::Token("{portrait}".into())));
        assert_eq!(line("{ name:1}"), Err(LineError::Token("{ name:1}".into())));
        assert_eq!(line("{}"), Err(LineError::Token("{}".into())));
    }
}
