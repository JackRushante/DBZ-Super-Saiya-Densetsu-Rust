//! The parser: a scene file, line by line, into a [`Source`]. A line is a `flag` or `const` declaration (before
//! the scene), the `scene <name>:` line, a label (`<name>:`), or a command with its operands. Anything else is
//! an error that names its line and column.

use super::SceneError;
use super::ast::{Declaration, DeclarationKind, Item, Operand, Source};
use super::lexer::{Spanned, Token, tokens};
use super::names::valid;
use super::ops::{self, Op};

/// Parses the scene file `text`.
///
/// # Errors
///
/// The first line that is not valid: a bad token, an unknown command, a declaration after the scene, a
/// second scene, a missing scene, a command or label before the scene, or the wrong number of operands.
pub fn parse(text: &str) -> Result<Source, SceneError> {
    let mut declarations = Vec::new();
    let mut scene: Option<String> = None;
    let mut items = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let number = index + 1;
        let spanned = tokens(line, number)?;
        let Some(first) = spanned.first() else { continue };
        let Token::Word(word) = &first.token else {
            return Err(SceneError::at(
                number,
                first.column,
                "a line starts with a command, a label, `flag`, `const` or `scene`",
            ));
        };
        if is_label(&spanned) {
            require_scene(scene.is_some(), number, first.column)?;
            if !valid(word) {
                return Err(not_a_name(word, number, first.column));
            }
            items.push(Item::Label { name: word.clone(), line: number, column: first.column });
            continue;
        }
        match word.as_str() {
            "flag" | "const" => {
                if scene.is_some() {
                    return Err(SceneError::at(
                        number,
                        first.column,
                        "a declaration comes before the `scene` line",
                    ));
                }
                let kind =
                    if word == "flag" { DeclarationKind::Flag } else { DeclarationKind::Const };
                declarations.push(declaration(kind, &spanned, number, line)?);
            }
            // `scene <name>:` is the header; `scene <number>` is the command of that keyword (`$2E`).
            "scene" if is_scene_header(&spanned) => {
                if scene.is_some() {
                    return Err(SceneError::at(number, first.column, "a file holds one scene"));
                }
                scene = Some(header(&spanned, number, line)?);
            }
            _ => {
                require_scene(scene.is_some(), number, first.column)?;
                let Some(op) = ops::by_keyword(word) else {
                    return Err(SceneError::at(
                        number,
                        first.column,
                        format!("`{word}` is not a command, a declaration or a label"),
                    ));
                };
                items.push(command(op, &spanned, number, line)?);
            }
        }
    }
    let Some(scene) = scene else {
        return Err(SceneError::whole("the file has no `scene <name>:` line"));
    };
    Ok(Source { declarations, scene, items })
}

/// Whether the line is `<word>:` and nothing more.
fn is_label(spanned: &[Spanned]) -> bool {
    matches!(spanned, [Spanned { token: Token::Word(_), .. }, Spanned { token: Token::Colon, .. }])
}

/// Whether a line that starts with `scene` is the scene header: a colon follows the keyword, or a name and
/// then a colon. Otherwise it is the `scene` command (`scene 3`).
fn is_scene_header(spanned: &[Spanned]) -> bool {
    matches!(spanned, [_, Spanned { token: Token::Colon, .. }, ..])
        || matches!(
            spanned,
            [_, Spanned { token: Token::Word(_), .. }, Spanned { token: Token::Colon, .. }, ..]
        )
}

/// The error for a name that is not a valid name (see [`valid`]).
fn not_a_name(name: &str, line: usize, column: usize) -> SceneError {
    SceneError::at(
        line,
        column,
        format!(
            "`{name}` is not a name: use lowercase letters, digits and `_`, not starting with a digit"
        ),
    )
}

/// The error for a command or a label that comes before the `scene` line (`seen`: whether it has come).
fn require_scene(seen: bool, line: usize, column: usize) -> Result<(), SceneError> {
    if seen {
        Ok(())
    } else {
        Err(SceneError::at(
            line,
            column,
            "a command or a label comes after the `scene <name>:` line",
        ))
    }
}

/// The column just after the line's last character: where a line ends too soon.
fn end_column(line: &str) -> usize {
    line.len() + 1
}

/// `flag <name> = <number>` or `const <name> = <number>`.
fn declaration(
    kind: DeclarationKind,
    spanned: &[Spanned],
    line: usize,
    text: &str,
) -> Result<Declaration, SceneError> {
    let keyword = if kind == DeclarationKind::Flag { "flag" } else { "const" };
    let [_, name, equals, value, rest @ ..] = spanned else {
        return Err(SceneError::at(
            line,
            end_column(text),
            format!("`{keyword}` takes a name, `=` and a number: `{keyword} <name> = <number>`"),
        ));
    };
    let Token::Word(name_text) = &name.token else {
        return Err(SceneError::at(
            line,
            name.column,
            format!("expected a name after `{keyword}`"),
        ));
    };
    if !valid(name_text) {
        return Err(not_a_name(name_text, line, name.column));
    }
    if equals.token != Token::Equals {
        return Err(SceneError::at(line, equals.column, "expected `=` after the name"));
    }
    let Token::Number(number) = value.token else {
        return Err(SceneError::at(line, value.column, "expected a number after `=`"));
    };
    if let Some(extra) = rest.first() {
        return Err(SceneError::at(line, extra.column, "unexpected text after the number"));
    }
    Ok(Declaration { kind, name: name_text.clone(), value: number, line, column: name.column })
}

/// `scene <name>:`, the scene's name.
fn header(spanned: &[Spanned], line: usize, text: &str) -> Result<String, SceneError> {
    match spanned {
        [_, Spanned { token: Token::Word(name), column }, Spanned { token: Token::Colon, .. }] => {
            if valid(name) {
                Ok(name.clone())
            } else {
                Err(not_a_name(name, line, *column))
            }
        }
        _ => Err(SceneError::at(
            line,
            end_column(text),
            "the scene line is `scene <name>:`, with a name of lowercase letters, digits and `_`",
        )),
    }
}

/// A command and its operands: each operand a number or a name, separated by commas, and as many as the
/// command takes.
fn command(
    op: &'static Op,
    spanned: &[Spanned],
    line: usize,
    text: &str,
) -> Result<Item, SceneError> {
    let first = &spanned[0];
    let mut operands = Vec::new();
    let mut expecting_operand = true;
    for item in &spanned[1..] {
        match (&item.token, expecting_operand) {
            (Token::Number(value), true) => {
                operands.push(Operand::Number { value: *value, column: item.column });
                expecting_operand = false;
            }
            (Token::Word(name), true) => {
                operands.push(Operand::Name { name: name.clone(), column: item.column });
                expecting_operand = false;
            }
            (Token::Comma, false) => expecting_operand = true,
            (_, true) => return Err(SceneError::at(line, item.column, "expected an operand here")),
            (_, false) => {
                return Err(SceneError::at(
                    line,
                    item.column,
                    "expected a comma before the next operand",
                ));
            }
        }
    }
    if expecting_operand && !operands.is_empty() {
        return Err(SceneError::at(line, end_column(text), "a comma is followed by no operand"));
    }
    if operands.len() != op.operands.len() {
        let expected = op.operands.len();
        let noun = if expected == 1 { "operand" } else { "operands" };
        return Err(SceneError::at(
            line,
            first.column,
            format!("`{}` takes {expected} {noun}, found {}", op.keyword, operands.len()),
        ));
    }
    Ok(Item::Command { op, operands, line, column: first.column })
}
