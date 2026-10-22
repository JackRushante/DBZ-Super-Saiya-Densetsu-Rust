//! The scene-script language (M10, Task 3): the text form of the story scripts' bytecode. A pack
//! assembles its scenes with it, and the tools decompile the ROM's scripts into it (for the tests and
//! for research, never tracked). The commands are in `docs/re/data/scenes.md`, the language below and
//! in `docs/mods/triggers.md`.
//!
//! One table of the commands ([`ops`]) serves both directions, so the assembler and the decompiler
//! cannot disagree. A scene file holds, in this order: the `flag` and `const` declarations, then one
//! `scene <name>:` line, then labels (`<name>:` on a line of their own) and commands (one to a line);
//! `#` starts a comment. The modules: [`ops`] (the commands), `names` (the names of a scene), `lexer`
//! and `parser` (the text), `asm` ([`assemble`] and [`relocate`]) and `disasm` ([`disassemble`]).

mod asm;
mod ast;
mod disasm;
mod lexer;
mod names;
pub mod ops;
mod parser;

pub use asm::{Assembled, assemble, relocate};
pub use disasm::disassemble;

use std::fmt;

/// A scene that does not assemble, or a script that does not decompile: where the problem is (a
/// 1-based line and column; line 0 for the text as a whole) and what it is, in plain words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneError {
    /// The line, 1-based; 0 for the text as a whole.
    pub line: usize,
    /// The column, 1-based; 0 when the error is not about a column.
    pub column: usize,
    /// What is wrong.
    pub message: String,
}

impl SceneError {
    /// An error at `line` and `column` (both 1-based).
    #[must_use]
    pub fn at(line: usize, column: usize, message: impl Into<String>) -> Self {
        Self { line, column, message: message.into() }
    }

    /// An error about the text as a whole, with no line.
    #[must_use]
    pub fn whole(message: impl Into<String>) -> Self {
        Self { line: 0, column: 0, message: message.into() }
    }
}

impl fmt::Display for SceneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.line == 0 {
            write!(f, "{}", self.message)
        } else {
            write!(f, "line {}, column {}: {}", self.line, self.column, self.message)
        }
    }
}

impl std::error::Error for SceneError {}
