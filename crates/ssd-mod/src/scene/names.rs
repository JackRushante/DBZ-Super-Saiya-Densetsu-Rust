//! The names of a scene: the `flag` and `const` declarations of its file and its labels. A name is
//! lowercase letters, digits and `_`, and does not start with a digit; each name stands for one thing.

use super::SceneError;
use std::collections::BTreeMap;

/// Whether `name` is a valid name: lowercase letters, digits and `_`, not starting with a digit.
#[must_use]
pub fn valid(name: &str) -> bool {
    name.bytes().next().is_some_and(|b| b.is_ascii_lowercase() || b == b'_')
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// What a name stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Symbol {
    /// A flag (`flag <name> = <number>`): an operand that takes a flag accepts it.
    Flag(u32),
    /// A constant (`const <name> = <number>`): any numeric operand accepts it.
    Const(u32),
    /// A label: its offset from the start of the scene, which only a label operand takes.
    Label(usize),
}

/// The names of one scene, each with the line it was defined on (for the error of a second definition).
#[derive(Debug, Default)]
pub struct Names {
    symbols: BTreeMap<String, (Symbol, usize)>,
}

impl Names {
    /// Defines `name` as `symbol`, defined on `line` and `column`.
    ///
    /// # Errors
    ///
    /// The name is not valid, or it is defined already (the error names the line of the first definition).
    pub fn define(
        &mut self,
        name: &str,
        symbol: Symbol,
        line: usize,
        column: usize,
    ) -> Result<(), SceneError> {
        if !valid(name) {
            return Err(SceneError::at(
                line,
                column,
                format!(
                    "`{name}` is not a name: use lowercase letters, digits and `_`, not starting with a digit"
                ),
            ));
        }
        if let Some((_, first)) = self.symbols.get(name) {
            return Err(SceneError::at(
                line,
                column,
                format!("`{name}` is already defined on line {first}"),
            ));
        }
        self.symbols.insert(name.to_owned(), (symbol, line));
        Ok(())
    }

    /// What `name` stands for, `None` when the scene has no such name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<Symbol> {
        self.symbols.get(name).map(|&(symbol, _)| symbol)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_lowercase_and_do_not_start_with_a_digit() {
        assert!(valid("met_elder"));
        assert!(valid("_x2"));
        assert!(!valid("2x"));
        assert!(!valid("Elder"));
        assert!(!valid("a.b"));
        assert!(!valid(""));
    }

    #[test]
    fn a_second_definition_names_the_line_of_the_first() {
        let mut names = Names::default();
        names.define("here", Symbol::Label(0), 3, 1).unwrap();
        let again = names.define("here", Symbol::Const(7), 9, 1).unwrap_err();
        assert_eq!((again.line, again.column), (9, 1));
        assert!(again.message.contains("already defined on line 3"), "{again}");
        assert_eq!(names.get("here"), Some(Symbol::Label(0)));
        assert_eq!(names.get("there"), None);
    }
}
