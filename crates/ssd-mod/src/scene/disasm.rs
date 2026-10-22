//! The decompiler: the script that starts at an entry point, as text that [`assemble`](super::assemble)
//! turns back into the same bytes, relocated to the entry.
//!
//! The walk starts at the entry and follows the commands as the engine runs them: a jump, a call, a flag
//! test and a battle go to their label, and a command that does not end the script goes on to the next one.
//! The bytes the walk never reaches are not in the text. A target inside the bytes is a label, named
//! `l_<address>` from its address; a target outside them (a subroutine of another script, for one) is written
//! as a number, the offset from the entry, with its address in a comment.

use super::SceneError;
use super::ops::{self, Kind, Op};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

/// The opcodes after which the script does not go on: `end`, `jump`, `return` and `return2`, `scene` (the
/// scene goes on in another script), and `start` and `start2` (the scene starts again).
const ENDS: [u8; 7] = [0x00, 0x01, 0x12, 0x20, 0x26, 0x2E, 0x32];

/// A command the walk reached: its keyword and the values of its operands (a label's value is an address).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Reached {
    op: &'static Op,
    values: Vec<u32>,
}

impl Reached {
    /// The addresses the command can go to, besides the next command: its label operands.
    fn targets(&self) -> impl Iterator<Item = u32> + '_ {
        self.op
            .operands
            .iter()
            .zip(&self.values)
            .filter(|(kind, _)| **kind == Kind::Label)
            .map(|(_, &value)| value)
    }
}

/// The text of the script that starts at `entry`, read from `bytes`, the bytes of the bank from `base` on.
/// The walk stays in `bytes`, and the text names its scene `at_<entry>`. A script whose last command goes on
/// past `bytes` ends with that command: the text has no `end` for it.
///
/// # Errors
///
/// The entry is not inside `bytes`; the walk reaches a byte that is not a command of the original engine,
/// or a command whose operands run past the end of `bytes`; the walk reaches code before the entry; or the
/// reached commands are not one run from the entry (a gap or an overlap, which the text cannot give back).
pub fn disassemble(bytes: &[u8], base: u16, entry: u16) -> Result<String, SceneError> {
    let start = u32::from(base);
    let end = start + bytes.len() as u32;
    let origin = u32::from(entry);
    if origin < start || origin >= end {
        return Err(SceneError::whole(format!("the entry ${origin:04x} is outside its bytes")));
    }
    let found = reach(bytes, start, end, origin)?;
    check_one_run(&found, origin)?;
    let labels: BTreeSet<u32> = found
        .values()
        .flat_map(Reached::targets)
        .filter(|&target| target >= start && target < end)
        .collect();
    let mut text = String::new();
    let _ = writeln!(text, "scene at_{origin:04x}:");
    for (&address, reached) in &found {
        if labels.contains(&address) {
            let _ = writeln!(text, "l_{address:04x}:");
        }
        text.push_str(&line(reached, origin, &labels, start, end));
    }
    Ok(text)
}

/// The commands the walk from `origin` reaches, by address: each is decoded, its targets are followed, and the
/// next command follows unless the command ends the script.
fn reach(
    bytes: &[u8],
    start: u32,
    end: u32,
    origin: u32,
) -> Result<BTreeMap<u32, Reached>, SceneError> {
    let mut found = BTreeMap::new();
    let mut todo = vec![origin];
    while let Some(address) = todo.pop() {
        if found.contains_key(&address) {
            continue;
        }
        let code = bytes[(address - start) as usize];
        let op = match ops::by_code(code) {
            Some(op) if !op.extension => op,
            _ => {
                return Err(SceneError::whole(format!(
                    "the byte ${code:02x} at ${address:04x} is not a command of the original engine"
                )));
            }
        };
        let length = op.length() as u32;
        if address + length > end {
            return Err(SceneError::whole(format!(
                "the command at ${address:04x} runs past the end of its bytes"
            )));
        }
        let reached = Reached { op, values: values(op, bytes, start, address) };
        for target in reached.targets() {
            if target >= start && target < end {
                todo.push(target);
            }
        }
        // A command that goes on to the next one, when that is past the bytes, ends the script there: the
        // scripts of the ROM end on an `effect` that the walk cannot follow into the next script.
        let next = address + length;
        if !ENDS.contains(&code) && next < end {
            todo.push(next);
        }
        found.insert(address, reached);
    }
    Ok(found)
}

/// The values of the operands of the command at `address` (a byte or a word each, little endian).
fn values(op: &Op, bytes: &[u8], start: u32, address: u32) -> Vec<u32> {
    let mut at = (address - start) as usize + 1;
    op.operands
        .iter()
        .map(|kind| {
            let value = match kind.size() {
                1 => u32::from(bytes[at]),
                _ => u32::from(bytes[at]) | u32::from(bytes[at + 1]) << 8,
            };
            at += kind.size();
            value
        })
        .collect()
}

/// Checks that the reached commands are one run from `origin`: the first is the entry, and each starts where the
/// one before it ends.
fn check_one_run(found: &BTreeMap<u32, Reached>, origin: u32) -> Result<(), SceneError> {
    let mut expected = origin;
    for (&address, reached) in found {
        if address < expected {
            return Err(SceneError::whole(format!(
                "the command at ${address:04x} is reached before the entry ${origin:04x} or overlaps the one before it"
            )));
        }
        if address > expected {
            return Err(SceneError::whole(format!(
                "the code from ${origin:04x} is not one run: the bytes from ${expected:04x} to ${address:04x} are not reached"
            )));
        }
        expected = address + reached.op.length() as u32;
    }
    Ok(())
}

/// The text of one command: its keyword and its operands. A label inside the script is its name; a target outside
/// it is the offset from `origin`, and the line carries its address in a comment.
fn line(reached: &Reached, origin: u32, labels: &BTreeSet<u32>, start: u32, end: u32) -> String {
    let mut comments = Vec::new();
    let operands: Vec<String> = reached
        .op
        .operands
        .iter()
        .zip(&reached.values)
        .map(|(&kind, &value)| match kind {
            Kind::Label if labels.contains(&value) && value >= start && value < end => {
                format!("l_{value:04x}")
            }
            Kind::Label => {
                comments.push(format!("${value:04x}"));
                format!("${:04x}", (value + 0x1_0000 - origin) & 0xFFFF)
            }
            _ if kind.size() == 1 => format!("${value:02x}"),
            _ => format!("${value:04x}"),
        })
        .collect();
    let mut text = format!("    {}", reached.op.keyword);
    if !operands.is_empty() {
        let _ = write!(text, " {}", operands.join(", "));
    }
    if !comments.is_empty() {
        let _ = write!(text, "  # {}", comments.join(" "));
    }
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::{assemble, relocate};

    /// Decompiles `bytes` placed at `base`, then assembles the text and relocates it to `base` again.
    fn round_trip(bytes: &[u8], base: u16) -> Vec<u8> {
        let text = disassemble(bytes, base, base).unwrap();
        let assembled = assemble(&text).unwrap_or_else(|e| panic!("{e}\n{text}"));
        let mut again = assembled.bytes;
        relocate(&mut again, &assembled.relocs, base);
        again
    }

    #[test]
    fn a_script_with_labels_a_battle_and_a_call_out_of_it_comes_back_as_the_same_bytes() {
        let source = "scene sample:\n\
                      \x20   ifflag 4, after\n\
                      \x20   battle 2, 0, after\n\
                      \x20   wait $05\n\
                      \x20   call $0100\n\
                      after:\n\
                      \x20   setflag 4\n\
                      \x20   jump after\n";
        let assembled = assemble(source).unwrap();
        let mut bytes = assembled.bytes.clone();
        relocate(&mut bytes, &assembled.relocs, 0x8000);
        let text = disassemble(&bytes, 0x8000, 0x8000).unwrap();
        assert!(text.contains("l_800e:\n"), "the walk names the label {text}");
        assert!(text.contains("call $0100  # $8100"), "the call out of the script {text}");
        assert_eq!(round_trip(&bytes, 0x8000), bytes);
    }

    #[test]
    fn code_that_is_not_one_run_is_refused_rather_than_misplaced() {
        // A jump over one byte nothing reaches: the reached commands have a gap between them.
        let gap = disassemble(&[0x12, 0x04, 0x80, 0xFF, 0x01], 0x8000, 0x8000).unwrap_err();
        assert!(gap.message.contains("not one run"), "{gap}");
    }

    #[test]
    fn a_byte_that_is_not_an_original_command_is_refused() {
        let extension = disassemble(&[0x33, 0x00, 0x00], 0x8000, 0x8000).unwrap_err();
        assert!(extension.message.contains("$33"), "{extension}");
        let unknown = disassemble(&[0x40], 0x8000, 0x8000).unwrap_err();
        assert!(unknown.message.contains("not a command"), "{unknown}");
    }

    #[test]
    fn a_command_that_runs_past_its_bytes_and_an_entry_outside_them_are_refused() {
        let past = disassemble(&[0x0C], 0x8000, 0x8000).unwrap_err();
        assert!(past.message.contains("runs past the end"), "{past}");
        let outside = disassemble(&[0x01], 0x8000, 0x8001).unwrap_err();
        assert!(outside.message.contains("outside its bytes"), "{outside}");
    }

    #[test]
    fn a_script_that_goes_on_past_its_bytes_ends_there_without_an_end_command() {
        assert_eq!(
            disassemble(&[0x10, 0x05], 0x8000, 0x8000).unwrap(),
            "scene at_8000:\n    wait $05\n"
        );
        assert_eq!(round_trip(&[0x10, 0x05], 0x8000), [0x10, 0x05]);
    }

    #[test]
    fn a_walk_that_stops_at_its_end_leaves_the_bytes_after_it_out_of_the_text() {
        // `end` at $8000 and bytes that no command reaches after it: only the end is in the text.
        let text = disassemble(&[0x01, 0xCE, 0xD5], 0x8000, 0x8000).unwrap();
        assert_eq!(text, "scene at_8000:\n    end\n");
        assert_eq!(round_trip(&[0x01, 0xCE, 0xD5], 0x8000), [0x01]);
    }
}
