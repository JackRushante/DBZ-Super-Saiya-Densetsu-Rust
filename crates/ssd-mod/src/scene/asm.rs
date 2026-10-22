//! The assembler: a scene file into the bytes of its script, at base 0, with the offset of each label word
//! for [`relocate`] to add the script's base to. A label is its offset from the start of the scene, and a
//! number in a label operand is an offset as well, so the two mean the same thing.

use super::SceneError;
use super::ast::{DeclarationKind, Item, Operand, Source};
use super::names::{Names, Symbol};
use super::ops::Kind;
use super::parser;

/// An assembled script: its bytes, at base 0, and where its label words are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assembled {
    /// The opcodes and operands, in order.
    pub bytes: Vec<u8>,
    /// The offset into `bytes` of the low byte of each label word. [`relocate`] adds the base to them.
    pub relocs: Vec<usize>,
}

/// Assembles the scene file `text` into its script (see the module documentation).
///
/// # Errors
///
/// The first error of the text, with its line and column: a line that is not valid (see the parser), a name
/// that is defined twice or is not known, a name used for the wrong kind of operand, a number that does not fit
/// its operand, or a label past the 65 535th byte.
pub fn assemble(text: &str) -> Result<Assembled, SceneError> {
    let source = parser::parse(text)?;
    let names = declare(&source)?;
    let mut bytes = Vec::new();
    let mut relocs = Vec::new();
    for item in &source.items {
        let Item::Command { op, operands, line, .. } = item else { continue };
        bytes.push(op.code);
        for (&kind, operand) in op.operands.iter().zip(operands) {
            let value = value(kind, operand, &names, *line)?;
            if kind == Kind::Label {
                relocs.push(bytes.len());
            }
            if kind.size() == 1 {
                bytes.push(value as u8);
            } else {
                bytes.extend_from_slice(&(value as u16).to_le_bytes());
            }
        }
    }
    Ok(Assembled { bytes, relocs })
}

/// Adds `base` to each label word of a script: `relocs` are the offsets [`assemble`] gives, and the words
/// become absolute addresses in the bank that `base` names (its low 16 bits).
///
/// # Panics
///
/// If an offset in `relocs` is not the start of a word inside `bytes`.
pub fn relocate(bytes: &mut [u8], relocs: &[usize], base: u16) {
    for &at in relocs {
        let value = u16::from_le_bytes([bytes[at], bytes[at + 1]]).wrapping_add(base);
        bytes[at..at + 2].copy_from_slice(&value.to_le_bytes());
    }
}

/// The names of the file: its declarations, then its labels, each at its offset in the script.
fn declare(source: &Source) -> Result<Names, SceneError> {
    let mut names = Names::default();
    for declaration in &source.declarations {
        let symbol = match declaration.kind {
            DeclarationKind::Flag => Symbol::Flag(declaration.value),
            DeclarationKind::Const => Symbol::Const(declaration.value),
        };
        names.define(&declaration.name, symbol, declaration.line, declaration.column)?;
    }
    let mut offset = 0_usize;
    for item in &source.items {
        match item {
            Item::Label { name, line, column } => {
                if offset > 0xFFFF {
                    return Err(SceneError::at(
                        *line,
                        *column,
                        "the label is past the 65535th byte of the scene",
                    ));
                }
                names.define(name, Symbol::Label(offset), *line, *column)?;
            }
            Item::Command { op, .. } => offset += op.length(),
        }
    }
    Ok(names)
}

/// The value of an operand of `kind`: a number as written, or what a name stands for, checked to fit.
fn value(kind: Kind, operand: &Operand, names: &Names, line: usize) -> Result<u32, SceneError> {
    match operand {
        Operand::Number { value, column } => fits(kind, *value, &value.to_string(), line, *column),
        Operand::Name { name, column } => {
            let column = *column;
            match names.get(name) {
                None => Err(SceneError::at(
                    line,
                    column,
                    format!(
                        "unknown name `{name}`: it is not a label, a flag or a constant of this file"
                    ),
                )),
                Some(Symbol::Label(offset)) if kind == Kind::Label => {
                    fits(kind, offset as u32, &format!("`{name}`"), line, column)
                }
                Some(Symbol::Label(_)) => Err(SceneError::at(
                    line,
                    column,
                    format!("`{name}` is a label: only a label operand takes a label"),
                )),
                Some(Symbol::Flag(value)) if matches!(kind, Kind::Flag | Kind::ExtFlag) => {
                    fits(kind, value, &format!("`{name}` ({value})"), line, column)
                }
                Some(Symbol::Flag(_)) => Err(SceneError::at(
                    line,
                    column,
                    format!("`{name}` is a flag: only a flag operand takes a flag"),
                )),
                Some(Symbol::Const(value)) => {
                    fits(kind, value, &format!("`{name}` ({value})"), line, column)
                }
            }
        }
    }
}

/// `value` when it fits an operand of `kind`, else the error that says so (`subject` names the value).
fn fits(
    kind: Kind,
    value: u32,
    subject: &str,
    line: usize,
    column: usize,
) -> Result<u32, SceneError> {
    if value > kind.max() {
        Err(SceneError::at(
            line,
            column,
            format!("{subject} does not fit {} (0 to {})", kind.describe(), kind.max()),
        ))
    } else {
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::ops::OPS;

    fn error(text: &str) -> SceneError {
        assemble(text).expect_err("the text should not assemble")
    }

    fn at(error: &SceneError, line: usize, column: usize) {
        assert_eq!((error.line, error.column), (line, column), "{error}");
    }

    #[test]
    fn every_opcode_assembles_to_its_length() {
        for op in OPS {
            let operands: Vec<&str> = op.operands.iter().map(|_| "1").collect();
            let line = if operands.is_empty() {
                format!("    {}", op.keyword)
            } else {
                format!("    {} {}", op.keyword, operands.join(", "))
            };
            let text = format!("scene every:\n{line}\n");
            let assembled = assemble(&text).unwrap_or_else(|e| panic!("{}: {e}", op.keyword));
            assert_eq!(assembled.bytes.len(), op.length(), "{}", op.keyword);
            assert_eq!(assembled.bytes[0], op.code, "{}", op.keyword);
            let labels = op.operands.iter().filter(|&&kind| kind == Kind::Label).count();
            assert_eq!(assembled.relocs.len(), labels, "{}", op.keyword);
        }
    }

    #[test]
    fn forward_and_backward_labels_are_offsets_from_the_start() {
        let text = "scene loop_test:\n\
                    start_here:\n\
                    \x20   wait $02\n\
                    \x20   jump later\n\
                    back:\n\
                    \x20   ifflag 3, start_here\n\
                    \x20   end\n\
                    later:\n\
                    \x20   jump back\n";
        let assembled = assemble(text).unwrap();
        assert_eq!(
            assembled.bytes,
            [0x10, 0x02, 0x12, 0x0A, 0x00, 0x0B, 0x03, 0x00, 0x00, 0x01, 0x12, 0x05, 0x00]
        );
        assert_eq!(assembled.relocs, [3, 7, 11]);
        let mut bytes = assembled.bytes;
        relocate(&mut bytes, &assembled.relocs, 0x8000);
        assert_eq!(&bytes[3..5], &[0x0A, 0x80], "later is at $800A");
        assert_eq!(&bytes[7..9], &[0x00, 0x80], "start_here is at $8000");
        assert_eq!(&bytes[11..13], &[0x05, 0x80], "back is at $8005");
    }

    #[test]
    fn constants_and_flags_stand_for_their_numbers() {
        let text = "flag met = 3\nconst amount = $10\nscene constants:\n    setflag met\n    wait amount\n    poke 0, amount\n";
        assert_eq!(assemble(text).unwrap().bytes, [0x0C, 3, 0x10, 0x10, 0x13, 0x00, 0x00, 0x10]);
    }

    #[test]
    fn a_line_that_is_not_valid_names_its_line_and_column() {
        let unknown = error("scene x:\n    jmp 3\n");
        at(&unknown, 2, 5);
        assert!(unknown.message.contains("`jmp` is not a command"), "{unknown}");
        at(&error("scene x:\n    wait 12ab\n"), 2, 10);
        let large = error("scene x:\n    wait 300\n");
        at(&large, 2, 10);
        assert!(large.message.contains("300 does not fit a byte (0 to 255)"), "{large}");
        at(&error("scene x:\n    flagx.set 2048\n"), 2, 15);
        assert!(
            error("scene x:\n    flagx.set 2048\n")
                .message
                .contains("an extended flag (0 to 2047)")
        );
        at(&error("scene x:\n    3 wait\n"), 2, 5);
        at(&error("wait 1\nscene x:\n"), 1, 1);
        let none = error("flag a = 1\n");
        at(&none, 0, 0);
        assert!(none.message.contains("no `scene <name>:` line"), "{none}");
        let twice = error("scene a:\n    end\nscene b:\n    end\n");
        at(&twice, 3, 1);
        assert!(twice.message.contains("one scene"), "{twice}");
        let late = error("scene a:\nflag b = 1\n");
        at(&late, 2, 1);
        let count = error("scene x:\n    jump\n");
        at(&count, 2, 5);
        assert!(count.message.contains("`jump` takes 1 operand, found 0"), "{count}");
    }

    #[test]
    fn an_unknown_or_duplicate_label_names_its_line_and_column() {
        let unknown = error("scene x:\n    jump nowhere\n");
        at(&unknown, 2, 10);
        assert!(unknown.message.contains("unknown name `nowhere`"), "{unknown}");
        let duplicate = error("scene x:\na:\n    end\na:\n    end\n");
        at(&duplicate, 4, 1);
        assert!(duplicate.message.contains("already defined on line 2"), "{duplicate}");
    }

    #[test]
    fn a_name_of_the_wrong_kind_is_refused_where_it_is_used() {
        let flag_as_byte = error("flag f = 2\nscene x:\n    wait f\n");
        at(&flag_as_byte, 3, 10);
        assert!(
            flag_as_byte.message.contains("only a flag operand takes a flag"),
            "{flag_as_byte}"
        );
        let label_as_flag = error("scene x:\nhere:\n    setflag here\n");
        at(&label_as_flag, 3, 13);
        assert!(
            label_as_flag.message.contains("only a label operand takes a label"),
            "{label_as_flag}"
        );
        let big_const = error("const big = 300\nscene x:\n    wait big\n");
        at(&big_const, 3, 10);
        assert!(big_const.message.contains("`big` (300) does not fit a byte"), "{big_const}");
    }
}
