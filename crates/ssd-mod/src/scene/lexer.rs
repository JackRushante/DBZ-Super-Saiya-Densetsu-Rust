//! The tokens of one line of a scene file: words (keywords and names), numbers (`123` in decimal,
//! `$7f` in hexadecimal), and the punctuation `:`, `,` and `=`. A `#` starts a comment that runs to the
//! end of the line. Columns are 1-based.

use super::SceneError;

/// One token of a line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// A keyword or a name: letters, digits, `_` and `.` (the extension keywords have a dot).
    Word(String),
    /// A number, already read.
    Number(u32),
    /// `:`
    Colon,
    /// `,`
    Comma,
    /// `=`
    Equals,
}

/// A token and the column where it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spanned {
    /// The token.
    pub token: Token,
    /// The column of its first character, 1-based.
    pub column: usize,
}

/// The tokens of `line`, which is line `number` of the file (for the errors).
///
/// # Errors
///
/// A character that is not part of a token, or a number that is not a number (`12ab`, `$`) or is too
/// large to be read (more than 4 294 967 295).
pub fn tokens(line: &str, number: usize) -> Result<Vec<Spanned>, SceneError> {
    let bytes = line.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let column = at + 1;
        let punctuation = match bytes[at] {
            b' ' | b'\t' | b'\r' => {
                at += 1;
                continue;
            }
            b'#' => break,
            b':' => Token::Colon,
            b',' => Token::Comma,
            b'=' => Token::Equals,
            byte if is_word_byte(byte) => {
                let end = bytes[at..]
                    .iter()
                    .position(|&b| !is_word_byte(b))
                    .map_or(bytes.len(), |n| at + n);
                out.push(Spanned {
                    token: word_or_number(&line[at..end], number, column)?,
                    column,
                });
                at = end;
                continue;
            }
            _ => {
                let character = line[at..].chars().next().unwrap_or('?');
                return Err(SceneError::at(
                    number,
                    column,
                    format!("`{character}` is not part of a command, a name or a number"),
                ));
            }
        };
        out.push(Spanned { token: punctuation, column });
        at += 1;
    }
    Ok(out)
}

/// Whether `byte` can be part of a word or a number.
fn is_word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'$')
}

/// The token of `text`, a run of word bytes: a number when it starts with a digit or `$`, else a word.
fn word_or_number(text: &str, number: usize, column: usize) -> Result<Token, SceneError> {
    if let Some(hex) = text.strip_prefix('$') {
        if hex.is_empty() || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(SceneError::at(
                number,
                column,
                format!(
                    "`{text}` is not a number: a hexadecimal number is `$` and its digits, as `$7f`"
                ),
            ));
        }
        return read(hex, 16, text, number, column);
    }
    if text.starts_with(|c: char| c.is_ascii_digit()) {
        if !text.bytes().all(|b| b.is_ascii_digit()) {
            return Err(SceneError::at(
                number,
                column,
                format!(
                    "`{text}` is not a number: write it in decimal (`12`) or in hexadecimal (`$0c`)"
                ),
            ));
        }
        return read(text, 10, text, number, column);
    }
    Ok(Token::Word(text.to_owned()))
}

/// The number `digits` in `radix`, or an error that it is too large to read.
fn read(
    digits: &str,
    radix: u32,
    text: &str,
    number: usize,
    column: usize,
) -> Result<Token, SceneError> {
    u32::from_str_radix(digits, radix).map(Token::Number).map_err(|_| {
        SceneError::at(
            number,
            column,
            format!("`{text}` is too large: the largest number is 4294967295"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(line: &str) -> Vec<Token> {
        tokens(line, 1).unwrap().into_iter().map(|s| s.token).collect()
    }

    #[test]
    fn words_numbers_and_punctuation_come_out_with_their_columns() {
        let spanned = tokens("  jump $1f, label_2   # a comment", 1).unwrap();
        let columns: Vec<usize> = spanned.iter().map(|s| s.column).collect();
        assert_eq!(columns, [3, 8, 11, 13]);
        assert_eq!(
            kinds("flagx.set 3: = , $0a 12"),
            [
                Token::Word("flagx.set".into()),
                Token::Number(3),
                Token::Colon,
                Token::Equals,
                Token::Comma,
                Token::Number(10),
                Token::Number(12),
            ]
        );
        assert_eq!(kinds("   # only a comment"), Vec::<Token>::new());
    }

    #[test]
    fn a_bad_or_too_large_number_names_its_column() {
        let bad = tokens("wait 12ab", 4).unwrap_err();
        assert_eq!((bad.line, bad.column), (4, 6));
        assert!(bad.message.contains("`12ab` is not a number"), "{bad}");
        let empty = tokens("wait $", 2).unwrap_err();
        assert_eq!((empty.line, empty.column), (2, 6));
        let huge = tokens("wait 99999999999", 3).unwrap_err();
        assert!(huge.message.contains("too large"), "{huge}");
        let stray = tokens("wait @", 5).unwrap_err();
        assert_eq!((stray.line, stray.column), (5, 6));
    }
}
