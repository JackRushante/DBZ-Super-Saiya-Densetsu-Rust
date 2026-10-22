//! The messages of a pack, from `text/messages.toml` to the bytes the message engine reads (`docs/re/data/text-messages.md`).
//!
//! A file holds `[[message]]` tables: `id` (the message number), `window` (`"dialogue"`, which is the string table 2,
//! or `"narration"`, which is table 3), an optional `portrait` (a `F4 nn` at the start of each page) and `pages`, an
//! array of pages, each an array of lines:
//!
//! ```toml
//! [[message]]
//! id = 400
//! window = "dialogue"
//! portrait = 5
//! pages = [
//!   ["\"You came far.", "Dangerous ahead.\""],
//!   ["\"Take care.\""],
//! ]
//! ```
//!
//! A message compiles to one string: the pages, each its lines (the first line of each dialogue page is written as
//! it is, the others start with a space tile), a line ends with `FF`, a page with `FE FF` and the string with `FF FF`.
//! Its list is three words: `D0nn` (the table), the string's index in that table, and `FFFF`. The index is not known
//! until the set is built: the image builder gives the strings of each table the indices after the original's last one
//! (see [`FIRST_DIALOGUE`] and [`FIRST_NARRATION`]). A line is at most 20 cells in a dialogue and 25 in a narration
//! (a name slot counts 15 cells, a number slot 8), a page at most 5 lines in a dialogue and 4 in a narration.

use super::encode::{self, LineError, Piece, SPACE};
use serde::Deserialize;
use std::fmt;

/// The highest message id. The engine reads a message's list at `07:8000 + 2 x id` in 16 bits, so above this the
/// address wraps into the registers.
pub const MAX_ID: u16 = 0x3FFF;
/// The first string index free in table 2 (the dialogue): the original's table holds 366 strings, 0 to 365.
pub const FIRST_DIALOGUE: u16 = 366;
/// The first string index free in table 3 (the narration): the original's table holds 167 strings, 0 to 166.
pub const FIRST_NARRATION: u16 = 167;
/// The highest string index: the engine's index word has a zero high nibble.
pub const MAX_INDEX: u16 = 0x0FFF;

/// The window of a message: the string table it writes in (`D002` or `D003`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Window {
    /// The dialogue box with a portrait: string table 2.
    Dialogue,
    /// The narration box: string table 3.
    Narration,
}

impl Window {
    /// The string table the window writes in, 2 or 3.
    #[must_use]
    pub fn table(self) -> u8 {
        match self {
            Self::Dialogue => 2,
            Self::Narration => 3,
        }
    }

    /// The most cells a line holds (a continuation line of a dialogue counts its leading space).
    #[must_use]
    pub fn cells(self) -> usize {
        match self {
            Self::Dialogue => 20,
            Self::Narration => 25,
        }
    }

    /// The most lines a page holds.
    #[must_use]
    pub fn lines(self) -> usize {
        match self {
            Self::Dialogue => 5,
            Self::Narration => 4,
        }
    }

    /// The first string index the image builder may give this window's strings.
    #[must_use]
    pub fn first_index(self) -> u16 {
        match self {
            Self::Dialogue => FIRST_DIALOGUE,
            Self::Narration => FIRST_NARRATION,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Dialogue => "dialogue",
            Self::Narration => "narration",
        }
    }
}

/// A message that does not compile, or a file that is not a messages file. Every message error names the message,
/// the page and the line (1-based) where there is one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageError {
    /// The file is not a messages file: its TOML does not parse, a field is wrong or missing, or it is not text.
    File(String),
    /// A message id above [`MAX_ID`].
    Id { id: u16 },
    /// A message with no page.
    NoPages { id: u16 },
    /// A page with no line.
    EmptyPage { id: u16, page: usize },
    /// A page with more lines than its window holds.
    TooManyLines { id: u16, page: usize, lines: usize, window: Window },
    /// A character the English font has no tile for.
    Character { id: u16, page: usize, line: usize, ch: char, column: usize },
    /// A `{` with no `}` on its line.
    Unclosed { id: u16, page: usize, line: usize },
    /// A `{...}` that is not a token.
    Token { id: u16, page: usize, line: usize, token: String },
    /// A line wider than its window.
    TooWide { id: u16, page: usize, line: usize, cells: usize, window: Window },
}

impl fmt::Display for MessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(text) => write!(f, "{text}"),
            Self::Id { id } => write!(
                f,
                "message {id}: the id must be 0 to {MAX_ID} (above it the address of the message's list wraps)"
            ),
            Self::NoPages { id } => write!(f, "message {id}: it has no page"),
            Self::EmptyPage { id, page } => {
                write!(f, "message {id}, page {page}: the page has no line")
            }
            Self::TooManyLines { id, page, lines, window } => write!(
                f,
                "message {id}, page {page}: {lines} lines, a {} page holds {}",
                window.name(),
                window.lines()
            ),
            Self::Character { id, page, line, ch, column } => write!(
                f,
                "message {id}, page {page}, line {line}, column {column}: the character `{ch}` has no tile in the English font"
            ),
            Self::Unclosed { id, page, line } => {
                write!(f, "message {id}, page {page}, line {line}: a `{{` has no `}}` after it")
            }
            Self::Token { id, page, line, token } => write!(
                f,
                "message {id}, page {page}, line {line}: `{token}` is not a token (use `{{name:1}}` to `{{name:3}}`, `{{number:1}}` to `{{number:3}}` or `{{portrait N}}`, N from 0 to 255)"
            ),
            Self::TooWide { id, page, line, cells, window } => write!(
                f,
                "message {id}, page {page}, line {line}: {cells} cells, a {} line holds {} (a name slot counts 15 cells, a number slot 8)",
                window.name(),
                window.cells()
            ),
        }
    }
}

impl std::error::Error for MessageError {}

/// A message as the file gives it: the fields of a `[[message]]` table.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    id: u16,
    window: Window,
    #[serde(default)]
    portrait: Option<u8>,
    pages: Vec<Vec<String>>,
}

/// The whole of a messages file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    message: Vec<Spec>,
}

/// A message compiled to its string. Its index in the table is not known yet: [`Compiled::list`] takes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    /// The message number.
    pub id: u16,
    /// The window, which picks the string table.
    pub window: Window,
    /// The string: the pages, as the engine reads them.
    pub string: Vec<u8>,
}

impl Compiled {
    /// The message's list for the string at `index` of its table: `D0nn` (the table), the index, `FFFF`.
    #[must_use]
    pub fn list(&self, index: u16) -> [u8; 6] {
        let [low, high] = index.to_le_bytes();
        [self.window.table(), 0xD0, low, high, 0xFF, 0xFF]
    }
}

/// The messages of a messages file, compiled in the order the file declares them.
///
/// # Errors
///
/// [`MessageError::File`] when the TOML does not parse, and the error of the first message that does not compile.
pub fn compile(text: &str) -> Result<Vec<Compiled>, MessageError> {
    let file: File = toml::from_str(text).map_err(|e| MessageError::File(e.to_string()))?;
    file.message.iter().map(compile_message).collect()
}

/// One message: its pages, each with its portrait and its lines, and the terminators between them.
fn compile_message(spec: &Spec) -> Result<Compiled, MessageError> {
    let id = spec.id;
    if id > MAX_ID {
        return Err(MessageError::Id { id });
    }
    if spec.pages.is_empty() {
        return Err(MessageError::NoPages { id });
    }
    let window = spec.window;
    let mut string = Vec::new();
    for (index, lines) in spec.pages.iter().enumerate() {
        let page = index + 1;
        if lines.is_empty() {
            return Err(MessageError::EmptyPage { id, page });
        }
        if lines.len() > window.lines() {
            return Err(MessageError::TooManyLines { id, page, lines: lines.len(), window });
        }
        if let Some(portrait) = spec.portrait {
            Piece::Portrait(portrait).write(&mut string);
        }
        for (at, text) in lines.iter().enumerate() {
            let line = at + 1;
            let mut pieces =
                encode::line(text).map_err(|error| line_error(id, page, line, error))?;
            // The first line of a dialogue page starts where the portrait puts it; the others start with a space.
            if window == Window::Dialogue && at > 0 {
                pieces.insert(0, SPACE);
            }
            let cells: usize = pieces.iter().map(|piece| piece.cells()).sum();
            if cells > window.cells() {
                return Err(MessageError::TooWide { id, page, line, cells, window });
            }
            for piece in pieces {
                piece.write(&mut string);
            }
            if at + 1 < lines.len() {
                string.extend([0xFF]);
            } else if index + 1 < spec.pages.len() {
                string.extend([0xFE, 0xFF]);
            } else {
                string.extend([0xFF, 0xFF]);
            }
        }
    }
    Ok(Compiled { id, window, string })
}

/// The message error of a line that does not encode.
fn line_error(id: u16, page: usize, line: usize, error: LineError) -> MessageError {
    match error {
        LineError::Character { ch, column } => {
            MessageError::Character { id, page, line, ch, column }
        }
        LineError::Unclosed => MessageError::Unclosed { id, page, line },
        LineError::Token(token) => MessageError::Token { id, page, line, token },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The message of the example: a portrait and two pages of our own words.
    const TWO_PAGES: &str = r#"
[[message]]
id = 400
window = "dialogue"
portrait = 5
pages = [
  ["\"You came far.", "Dangerous ahead.\""],
  ["\"Take care.\""],
]
"#;

    fn one(text: &str) -> Compiled {
        let mut all = compile(text).expect("the file compiles");
        assert_eq!(all.len(), 1, "one message");
        all.remove(0)
    }

    fn refused(text: &str) -> MessageError {
        compile(text).expect_err("the file is refused")
    }

    #[test]
    fn a_two_page_dialogue_message_is_its_bytes_written_out() {
        let message = one(TWO_PAGES);
        assert_eq!((message.id, message.window), (400, Window::Dialogue));
        let expected: Vec<u8> = [
            // Page 1: the portrait 5, the opening quote, "You came far.", then FF ends the line.
            &[
                0xF4, 0x05, 0x7E, 0x45, 0x1B, 0x55, 0x01, 0x09, 0x05, 0x19, 0x0C, 0x01, 0x0D, 0x05,
                0x52, 0x7C, 0xFF,
            ][..],
            // The second line starts with the space tile; FE FF ends the page.
            &[
                0x01, 0xEA, 0x05, 0x1A, 0x0E, 0x0C, 0x52, 0x1B, 0x55, 0x53, 0x01, 0x05, 0x11, 0x0C,
                0x05, 0x0A, 0x7C, 0x7F, 0xFE, 0xFF,
            ],
            // Page 2: the portrait again, "Take care.", then FF FF ends the string.
            &[
                0xF4, 0x05, 0x7E, 0x47, 0x05, 0x17, 0x0C, 0x01, 0x09, 0x05, 0x52, 0x0C, 0x7C, 0x7F,
                0xFF, 0xFF,
            ],
        ]
        .concat();
        assert_eq!(message.string, expected);
    }

    #[test]
    fn the_list_is_the_table_the_index_and_the_end() {
        let message = one(TWO_PAGES);
        assert_eq!(message.list(366), [0x02, 0xD0, 0x6E, 0x01, 0xFF, 0xFF]);
        let narration = Compiled { id: 401, window: Window::Narration, string: Vec::new() };
        assert_eq!(narration.list(0x7A), [0x03, 0xD0, 0x7A, 0x00, 0xFF, 0xFF]);
    }

    #[test]
    fn the_file_is_refused_with_its_message_for_each_fault() {
        let budget =
            "[[message]]\nid = 1\nwindow = \"dialogue\"\npages = [[\"ABCDEFGHIJKLMNOPQRSTU\"]]\n";
        assert!(matches!(
            refused(budget),
            MessageError::TooWide { id: 1, page: 1, line: 1, cells: 21, .. }
        ));
        let narration = "[[message]]\nid = 2\nwindow = \"narration\"\npages = [[\"ABCDEFGHIJKLMNOPQRSTUVWXYZ\"]]\n";
        assert!(matches!(refused(narration), MessageError::TooWide { id: 2, cells: 26, .. }));
        let lines = "[[message]]\nid = 3\nwindow = \"narration\"\npages = [[\"a\", \"b\", \"c\", \"d\", \"e\"]]\n";
        assert!(matches!(
            refused(lines),
            MessageError::TooManyLines { id: 3, page: 1, lines: 5, .. }
        ));
        let letter = "[[message]]\nid = 4\nwindow = \"dialogue\"\npages = [[\"ok\"], [\"a;b\"]]\n";
        assert!(matches!(
            refused(letter),
            MessageError::Character { id: 4, page: 2, line: 1, ch: ';', column: 2 }
        ));
        let token = "[[message]]\nid = 5\nwindow = \"dialogue\"\npages = [[\"{name:4}\"]]\n";
        assert!(matches!(refused(token), MessageError::Token { id: 5, page: 1, line: 1, .. }));
        let empty = "[[message]]\nid = 6\nwindow = \"dialogue\"\npages = [[]]\n";
        assert!(matches!(refused(empty), MessageError::EmptyPage { id: 6, page: 1 }));
        let none = "[[message]]\nid = 7\nwindow = \"dialogue\"\npages = []\n";
        assert!(matches!(refused(none), MessageError::NoPages { id: 7 }));
        let high = "[[message]]\nid = 16384\nwindow = \"dialogue\"\npages = [[\"ok\"]]\n";
        assert!(matches!(refused(high), MessageError::Id { id: 16384 }));
        let window = "[[message]]\nid = 8\nwindow = \"battle\"\npages = [[\"ok\"]]\n";
        assert!(
            matches!(refused(window), MessageError::File(_)),
            "an unknown window is a file error"
        );
        let field = "[[message]]\nid = 9\nwindow = \"dialogue\"\npage = [[\"ok\"]]\n";
        assert!(
            matches!(refused(field), MessageError::File(_)),
            "a misspelt field is a file error"
        );
    }

    #[test]
    fn a_continuation_line_of_a_dialogue_counts_its_leading_space() {
        // 19 characters after the leading space make 20 cells: the most a continuation line holds.
        let fits = "[[message]]\nid = 1\nwindow = \"dialogue\"\npages = [[\"ok\", \"ABCDEFGHIJKLMNOPQRS\"]]\n";
        assert!(compile(fits).is_ok(), "19 + 1 cells");
        let over = "[[message]]\nid = 1\nwindow = \"dialogue\"\npages = [[\"ok\", \"ABCDEFGHIJKLMNOPQRST\"]]\n";
        assert!(matches!(refused(over), MessageError::TooWide { cells: 21, line: 2, .. }));
        let first =
            "[[message]]\nid = 1\nwindow = \"dialogue\"\npages = [[\"ABCDEFGHIJKLMNOPQRST\"]]\n";
        assert!(compile(first).is_ok(), "the first line holds 20 cells");
    }

    #[test]
    fn a_name_slot_counts_its_worst_case_of_fifteen_cells() {
        let fits =
            "[[message]]\nid = 1\nwindow = \"narration\"\npages = [[\"{name:1}ABCDEFGHI\"]]\n";
        assert!(compile(fits).is_ok(), "15 + 9 cells is 24");
        let over =
            "[[message]]\nid = 1\nwindow = \"narration\"\npages = [[\"{name:1}ABCDEFGHIJK\"]]\n";
        assert!(matches!(refused(over), MessageError::TooWide { cells: 26, .. }));
    }
}
