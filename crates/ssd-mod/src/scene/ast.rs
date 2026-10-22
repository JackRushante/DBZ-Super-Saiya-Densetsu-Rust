//! What the parser builds from a scene file: its declarations and its scene, each line with its line and
//! column, so that the assembler can name the place of each error.

use super::ops::Op;

/// Which declaration a line is: a story or extended flag (`flag`), or a constant (`const`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    /// `flag <name> = <number>`: a flag, for the operands that take a flag.
    Flag,
    /// `const <name> = <number>`: a number, for any numeric operand.
    Const,
}

/// A `flag` or `const` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    /// Which declaration it is.
    pub kind: DeclarationKind,
    /// The name it gives the number.
    pub name: String,
    /// The number.
    pub value: u32,
    /// The line it is on (1-based).
    pub line: usize,
    /// The column of the name (1-based).
    pub column: usize,
}

/// An operand as written: a number, or a name, and the column it starts at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Operand {
    /// A number, already read.
    Number { value: u32, column: usize },
    /// A name: a label, a flag or a constant.
    Name { name: String, column: usize },
}

/// One line of a scene's body: a label, or a command with its operands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    /// `<name>:` on a line of its own.
    Label { name: String, line: usize, column: usize },
    /// A keyword and its operands.
    Command { op: &'static Op, operands: Vec<Operand>, line: usize, column: usize },
}

/// A parsed scene file: its declarations, the name of its scene, and the scene's labels and commands in
/// the order they are written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The `flag` and `const` lines, in order.
    pub declarations: Vec<Declaration>,
    /// The name of the scene (`scene <name>:`).
    pub scene: String,
    /// The labels and commands of the scene.
    pub items: Vec<Item>,
}
