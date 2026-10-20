//! A translation of the game's text, read at run time from a tab-separated file (`zh-hk.tsv`): the Enhanced mode
//! draws it over the original's windows. Nothing of the game's text is held here: the file is the player's, beside
//! the game or in its data folder, and this module only reads it.
//!
//! The file is a header line naming the columns (`key`, `zh`, then any others, such as `note`), and one row per page
//! of a string, name or block. The keys are those of `xtask text-dump` ([`string_key`], [`page_key`]: `str3:0x3E`,
//! `str3:0x3E/p2`, `battle:149`, `unit:0x02`, `object:7`, `help:12`...). In the text a literal `\n` (a backslash and
//! an `n`) is a line break, and the dump's tokens stand for what the game fills in while it runs:
//!
//! | Token | What it is |
//! |---|---|
//! | `{name:1}` to `{name:3}` | the name in the slot of the engine's code `FB`, `FA`, `F9` (a unit, object or attack) |
//! | `{num:1}` to `{num:3}` | the number in the slot of the code `F8`, `F7`, `F6` |
//! | `{phrase:n}` | the inline phrase `n` (code `E0 + n`) |
//! | `{same:KEY}` | the whole text is `KEY`'s (the string is at the address of an earlier one) |
//! | `{auto}`, `{portrait:..}`, `{FC}`, `{tile:..}` | what the engine does by itself: nothing to draw |
//!
//! A row with an empty text, and a key the file does not have, are not translated: the game's own text stays.

use std::collections::HashMap;
use std::path::Path;

/// How many `{same:KEY}` links are followed before a text is taken as missing (a loop in the file).
const SAME_HOPS: usize = 8;

/// The entries of `help` (table 0) before table 1 starts on the same words.
const HELP_ENTRIES: u32 = 42;

/// Why a translation file could not be read.
#[derive(Debug, thiserror::Error)]
pub enum TranslationError {
    #[error("reading the translation: {0}")]
    Io(#[from] std::io::Error),
    /// The first line does not name a `key` and a `zh` column.
    #[error("the first line of the translation must name its columns, `key` and `zh` among them")]
    Header,
}

/// The translated texts, by key.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Translation {
    rows: HashMap<String, String>,
}

impl Translation {
    /// Reads the text of a translation file: its header, then its rows. A row without the `zh` column, or with an
    /// empty text, is left out (the game's own text stays there); a repeated key keeps its last row.
    ///
    /// # Errors
    ///
    /// The header does not name the `key` and `zh` columns.
    pub fn parse(text: &str) -> Result<Self, TranslationError> {
        let text = text.strip_prefix('\u{FEFF}').unwrap_or(text);
        let mut lines = text.lines();
        let header: Vec<&str> = lines.next().unwrap_or("").split('\t').map(str::trim).collect();
        let column = |name: &str| header.iter().position(|&column| column == name);
        let (Some(key), Some(zh)) = (column("key"), column("zh")) else {
            return Err(TranslationError::Header);
        };
        let mut rows = HashMap::new();
        for line in lines {
            let fields: Vec<&str> = line.split('\t').collect();
            let (Some(&name), Some(&text)) = (fields.get(key), fields.get(zh)) else { continue };
            let (name, text) = (name.trim(), text.trim_end_matches('\r'));
            if !name.is_empty() && !text.trim().is_empty() {
                rows.insert(name.to_owned(), text.to_owned());
            }
        }
        Ok(Self { rows })
    }

    /// Reads the translation file at `path`.
    ///
    /// # Errors
    ///
    /// The file cannot be read, or is not a translation ([`Translation::parse`]).
    pub fn read(path: &Path) -> Result<Self, TranslationError> {
        Self::parse(&std::fs::read_to_string(path)?)
    }

    /// The number of keys with a text.
    #[must_use]
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether no key has a text.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The text of `key` as the file has it: tokens and literal `\n` included, a `{same:KEY}` row followed to the
    /// row it names. `None` when the file does not translate it.
    #[must_use]
    pub fn text(&self, key: &str) -> Option<&str> {
        let mut text = self.rows.get(key)?.as_str();
        for _ in 0..SAME_HOPS {
            let Some(other) = same_key(text) else { return Some(text) };
            text = self.rows.get(other)?.as_str();
        }
        None
    }
}

/// The key a `{same:KEY}` text names.
fn same_key(text: &str) -> Option<&str> {
    text.trim().strip_prefix("{same:")?.strip_suffix('}')
}

/// The key of string `index` of the `message_table` value `table` (the engine's coordinates while it writes it:
/// `message_table`, `message_string`), as `xtask text-dump` writes it. Table 0 from index 42 is table 1.
#[must_use]
pub fn string_key(table: u8, index: u32) -> String {
    match table {
        0 if index < HELP_ENTRIES => format!("help:{index}"),
        0 => format!("battle:{}", index - HELP_ENTRIES),
        1 => format!("battle:{index}"),
        2 => format!("str2:0x{index:03X}"),
        3..=7 => format!("str{table}:0x{index:02X}"),
        _ => format!("table{table}:{index}"),
    }
}

/// The key of page `page` (0 the first) of the string of `key`: the first page has the string's key, the next ones
/// `/p2`, `/p3`...
#[must_use]
pub fn page_key(key: &str, page: usize) -> String {
    if page == 0 { key.to_owned() } else { format!("{key}/p{}", page + 1) }
}

/// What the game fills in at a token of a text: the slot of a name, of a number, or an inline phrase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    /// `{name:n}`, 1 to 3.
    Name(u8),
    /// `{num:n}`, 1 to 3.
    Number(u8),
    /// `{phrase:n}`.
    Phrase(u8),
}

/// A piece of a text: words, a line break, or a slot the game fills in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Piece<'a> {
    Text(&'a str),
    Break,
    Slot(Slot),
}

/// The pieces of a translated text: the words between the tokens, a [`Piece::Break`] for each `\n` (literal or not)
/// and the slots. The tokens of what the engine does by itself (`{auto}`, `{portrait:..}`, `{FC}`...) give nothing;
/// a `{` that opens no token is a letter.
#[must_use]
pub fn pieces(text: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let next = rest.find(['{', '\\', '\n']).unwrap_or(rest.len());
        if next > 0 {
            out.push(Piece::Text(&rest[..next]));
            rest = &rest[next..];
            continue;
        }
        if let Some(after) = rest.strip_prefix("\\n").or_else(|| rest.strip_prefix('\n')) {
            out.push(Piece::Break);
            rest = after;
        } else if let Some((token, after)) = rest.strip_prefix('{').and_then(|r| r.split_once('}'))
            && !token.contains('{')
        {
            out.extend(slot(token).map(Piece::Slot));
            rest = after;
        } else {
            // A lone `{` or `\`: a letter
            let width = rest.chars().next().map_or(1, char::len_utf8);
            out.push(Piece::Text(&rest[..width]));
            rest = &rest[width..];
        }
    }
    out
}

/// The slot a token names (`name:1`, `num:2`, `phrase:0`); `None` for the others.
fn slot(token: &str) -> Option<Slot> {
    let (kind, number) = token.split_once(':')?;
    let number = number.parse::<u8>().ok()?;
    match kind {
        "name" => Some(Slot::Name(number)),
        "num" => Some(Slot::Number(number)),
        "phrase" => Some(Slot::Phrase(number)),
        _ => None,
    }
}

/// The text with its slots filled in by `fill` and its line breaks as `\n` characters, the other tokens left out.
#[must_use]
pub fn render(text: &str, mut fill: impl FnMut(Slot) -> String) -> String {
    let mut out = String::with_capacity(text.len());
    for piece in pieces(text) {
        match piece {
            Piece::Text(words) => out.push_str(words),
            Piece::Break => out.push('\n'),
            Piece::Slot(slot) => out.push_str(&fill(slot)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_keys_follow_the_engine_tables_as_the_dump_writes_them() {
        assert_eq!(string_key(0, 12), "help:12");
        assert_eq!(string_key(0, 42 + 78), "battle:78", "table 0 goes on as table 1");
        assert_eq!(string_key(1, 78), "battle:78");
        assert_eq!(string_key(2, 0x41), "str2:0x041");
        assert_eq!(string_key(3, 0x3E), "str3:0x3E");
        assert_eq!(string_key(7, 6), "str7:0x06");
        assert_eq!(page_key("str3:0x3E", 0), "str3:0x3E");
        assert_eq!(page_key("str3:0x3E", 1), "str3:0x3E/p2");
    }

    #[test]
    fn the_header_names_the_columns_in_any_order() {
        let table = Translation::parse("note\tzh\tkey\nx\t你好\ta:1\n").unwrap();
        assert_eq!(table.text("a:1"), Some("你好"));
        assert!(matches!(Translation::parse("key\tenglish\n"), Err(TranslationError::Header)));
        assert!(matches!(Translation::parse(""), Err(TranslationError::Header)));
    }

    #[test]
    fn a_same_row_is_followed_and_a_loop_or_a_gap_is_no_text() {
        let text =
            "key\tzh\na\t甲\nb\t{same:a}\nc\t{same:b}\nd\t{same:e}\ne\t{same:d}\nf\t{same:none}\n";
        let table = Translation::parse(text).unwrap();
        assert_eq!(table.text("c"), Some("甲"));
        assert_eq!(table.text("d"), None, "a loop");
        assert_eq!(table.text("f"), None, "the row it names is not translated");
    }

    #[test]
    fn tokens_become_slots_and_the_engines_own_codes_vanish() {
        let parts = pieces("{portrait:01}「{name:1}！」\\n{num:2}點{auto}");
        assert_eq!(
            parts,
            [
                Piece::Text("「"),
                Piece::Slot(Slot::Name(1)),
                Piece::Text("！」"),
                Piece::Break,
                Piece::Slot(Slot::Number(2)),
                Piece::Text("點"),
            ]
        );
        assert_eq!(pieces("a{b"), [Piece::Text("a"), Piece::Text("{"), Piece::Text("b")]);
        assert_eq!(pieces("C:\\x"), [Piece::Text("C:"), Piece::Text("\\"), Piece::Text("x")]);
    }

    #[test]
    fn render_fills_the_slots_and_breaks_the_lines() {
        let filled = render("{name:1}受到{num:1}點傷害！\\n{phrase:0}", |slot| match slot {
            Slot::Name(1) => "悟空".to_owned(),
            Slot::Number(1) => "120".to_owned(),
            Slot::Phrase(0) => "（短語）".to_owned(),
            _ => String::new(),
        });
        assert_eq!(filled, "悟空受到120點傷害！\n（短語）");
    }
}
