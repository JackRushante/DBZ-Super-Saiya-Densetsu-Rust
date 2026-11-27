//! Which pose numbers the game can ask for, found statically in its animation scripts.
//!
//! A pose number is set in the side record's byte `+$21` (bit 7: load it) by a few places in
//! bank 5; `05:BECB` then loads the pose of the unit's poseset (docs/re/data/battle-poses.md,
//! "Which poses are used"). Two script interpreters feed it:
//!
//! * the battle script (`05:CA8D`): instructions of an opcode byte and its arguments, dispatched
//!   through the table at `05:CAFA`; the scripts are in bank `$1E` (two tables of 125 pointers, one
//!   per side, and three blocks the follow modes use). Opcodes 3, 49 and 51 set a pose from one of
//!   their arguments. Argument counts are read from the handlers' own code (the `LDA #n` before the
//!   jump that advances the script position); branch instructions are followed to every forward
//!   offset in their arguments, so the set is an upper bound;
//! * the effect script (`05:DEC3`): bank `$1D`, pointers in two tables; opcode 3 sets a pose from
//!   its argument, 1 and `$22` take one argument, 0 ends the script and any other opcode advances
//!   by its own value.
//!
//! Beside them, a handful of routines set fixed poses (the idle pair, the reset to 0, the
//! command screen's 23, the pose of an opening) which are listed below with where they are.
//!
//! The answer is a set of pose numbers: scripts are not tied to a unit, so every unit of a
//! poseset can show every pose its poseset has. Replay-observed loads (`--observed`) are added as
//! evidence for the (unit, pose) pairs and checked against the static set.

use super::decode::{self, Rom};
use crate::disasm::{self, Mode, Options, RomImage, Widths};
use std::collections::{BTreeMap, BTreeSet};

/// The jump table of the battle script's opcodes.
const OP_TABLE: u32 = 0x05_CAFA;
/// The most opcodes the jump table can hold (it is read until a word is not a handler).
const OP_LIMIT: usize = 160;
/// The battle script pointer tables (bank `$1E`, 125 words each) and the blocks of the follow modes.
const SCRIPT_TABLES: [u32; 2] = [0x1E_8000, 0x1E_80FA];
const SCRIPT_BLOCKS: [u32; 3] = [0x1E_81F7, 0x1E_821D, 0x1E_8240];
/// The effect script pointer tables (bank `$1D`).
const EFFECT_TABLES: [u32; 2] = [0x1D_908C, 0x1D_9183];
/// Opcodes that set a pose from one argument byte: (opcode, byte after the opcode).
const POSE_OPS: [(usize, u32); 3] = [(3, 1), (49, 2), (51, 2)];
/// How far a script is followed from its start.
const SCRIPT_SPAN: u32 = 0x600;
/// Poses set by fixed routines, with what sets them. The idle pair is picked at random by the
/// battle idle routine (`05:CA3B`), 0 and 2 are the resets of the field and battle set-ups
/// (`05:F2DA`, `05:F326`), 5 is the pose the fight opens in (`05:B04B`), 23 is the command screen's.
const FIXED: [(u8, &str); 6] = [
    (0, "fixed: reset by the battle set-up"),
    (2, "fixed: set by the battle set-up"),
    (3, "fixed: the idle pair"),
    (4, "fixed: the idle pair"),
    (5, "fixed: the pose a fight opens in"),
    (23, "fixed: the command screen's pose"),
];

/// What the walk of one battle opcode needs.
struct Op {
    /// Arguments of the instruction, when the handler advances by a fixed count.
    args: Option<u8>,
    /// The handler can branch (several counts, a computed one, or a jump out).
    branchy: bool,
    /// The handler stores a pose number (`STA $0021,Y`).
    stores_pose: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Count {
    Fixed(u8),
    Computed,
}

fn analyse(image: &RomImage<'_>, handler: u16) -> Op {
    let widths = Widths { m8: true, x8: false, m_known: true, x_known: true };
    let options = Options { m8: Some(true), x8: Some(false), ..Options::default() };
    let listing =
        disasm::disassemble(image, 0x05_0000 | u32::from(handler), widths, &options, &|_| None);
    let lines = &listing.lines;
    let mut counts: Vec<Count> = Vec::new();
    let mut jumps_out = false;
    let mut stores_pose = false;
    for (k, line) in lines.iter().enumerate() {
        let i = &line.instr;
        let advance = i.mode == Mode::Abs
            && matches!(i.mnemonic, "JMP" | "JSR")
            && matches!(i.operand & 0xFFFF, 0xCA8D | 0xDD51);
        if advance {
            for before in lines[k.saturating_sub(5)..k].iter().rev().map(|l| &l.instr) {
                if before.mnemonic == "LDA" && before.mode == Mode::ImmM {
                    counts.push(Count::Fixed(before.operand as u8));
                    break;
                }
                if matches!(
                    before.mnemonic,
                    "LDA" | "TXA" | "TYA" | "PLA" | "ADC" | "SBC" | "DEC" | "INC"
                ) {
                    counts.push(Count::Computed);
                    break;
                }
            }
        } else if i.mnemonic == "JMP"
            && i.mode == Mode::Abs
            && !listing.labels.contains(&(i.target().unwrap_or(0)))
        {
            jumps_out = true;
        }
        stores_pose |= i.mnemonic == "STA" && i.mode == Mode::AbsY && i.operand == 0x21;
    }
    let first = counts.iter().find_map(|c| match c {
        Count::Fixed(n) if *n != 0xFF => Some(*n),
        _ => None,
    });
    let distinct: BTreeSet<u8> = counts
        .iter()
        .filter_map(|c| if let Count::Fixed(n) = c { Some(*n) } else { None })
        .filter(|&n| n != 0xFF)
        .collect();
    let branchy = counts.iter().any(|c| *c == Count::Computed || *c == Count::Fixed(0xFF))
        || distinct.len() > 1
        || jumps_out;
    Op { args: first, branchy, stores_pose }
}

/// What the static walk found: for each pose number the places that set it.
#[derive(Debug, Default)]
pub struct PoseUse {
    /// Pose number to evidence (a few examples, then a count).
    pub poses: BTreeMap<u8, Vec<String>>,
}

impl PoseUse {
    fn add(&mut self, pose: u8, evidence: String) {
        let list = self.poses.entry(pose).or_default();
        if list.len() < 3 && !list.contains(&evidence) {
            list.push(evidence);
        }
    }
}

fn byte(rom: Rom<'_>, address: u32) -> Option<u8> {
    rom.view().u8(address).ok()
}

/// The next ROM address of bank `bank` for script position `pos` (a 16-bit offset).
fn at(bank: u32, pos: u32) -> u32 {
    (bank << 16) | (pos & 0xFFFF)
}

/// The pose numbers the game can ask for.
///
/// # Errors
///
/// The opcode table or a handler is not what this reads (another ROM).
pub fn static_poses(rom: Rom<'_>) -> anyhow::Result<PoseUse> {
    let image = RomImage(rom.bytes());
    let view = rom.view();
    let limit = (0..decode::POSESETS).map(|k| decode::pose_count(rom, k)).max().unwrap_or(0) as u8;
    let mut found = PoseUse::default();
    for &(pose, what) in &FIXED {
        found.add(pose, what.to_owned());
    }

    // The battle script's opcodes
    let handlers: Vec<u16> = (0..OP_LIMIT as u32)
        .map_while(|n| view.u16(OP_TABLE + 2 * n).ok().filter(|h| (0xCB00..0xE000).contains(h)))
        .collect();
    anyhow::ensure!(
        handlers.len() > 100,
        "the battle script's opcode table has {} entries",
        handlers.len()
    );
    let ops: Vec<Op> = handlers.iter().map(|&h| analyse(&image, h)).collect();
    for &(op, _) in &POSE_OPS {
        anyhow::ensure!(ops[op].stores_pose, "opcode {op} does not store a pose number");
    }
    let mut starts: BTreeSet<u32> = SCRIPT_BLOCKS.iter().map(|&b| b & 0xFFFF).collect();
    for table in SCRIPT_TABLES {
        for n in 0..125 {
            if let Ok(pointer) = view.u16(table + 2 * n).map(u32::from)
                && pointer >= 0x8000
            {
                starts.insert(pointer);
            }
        }
    }
    for &start in &starts {
        // (position, whether a branch was taken to get here)
        let mut stack = vec![(start, false)];
        let mut seen = BTreeSet::new();
        while let Some((pos, branched)) = stack.pop() {
            if pos >= start + SCRIPT_SPAN || pos >= 0xFFF0 || !seen.insert(pos) {
                continue;
            }
            let Some(op) = byte(rom, at(0x1E, pos)).map(usize::from).filter(|&op| op < ops.len())
            else {
                continue;
            };
            if let Some(&(_, offset)) = POSE_OPS.iter().find(|(o, _)| *o == op)
                && let Some(arg) = byte(rom, at(0x1E, pos + offset))
                && (arg & 0x7F) < limit
            {
                let how = if branched { "reached by a branch from" } else { "straight from" };
                found.add(
                    arg & 0x7F,
                    format!(
                        "battle script op {op} at 1E:{pos:04X}, {how} the script at 1E:{start:04X}"
                    ),
                );
            }
            let info = &ops[op];
            match (info.args, info.branchy) {
                (None, false) => {}
                (args, branchy) => {
                    if let Some(args) = args {
                        stack.push((pos + 1 + u32::from(args), branched));
                    }
                    if branchy || args.is_none() {
                        for k in 1..=4 {
                            if let Some(b) =
                                byte(rom, at(0x1E, pos + k)).filter(|&b| (1..0x60).contains(&b))
                            {
                                stack.push((pos + u32::from(b), true));
                            }
                        }
                    }
                }
            }
        }
    }

    // The effect scripts
    let mut effect_starts: BTreeSet<u32> = BTreeSet::new();
    for table in EFFECT_TABLES {
        for n in 0..124 {
            if let Ok(pointer) = view.u16(table + 2 * n).map(u32::from)
                && pointer >= 0x8000
            {
                effect_starts.insert(pointer);
            }
        }
    }
    for &start in &effect_starts {
        let mut pos = start;
        for _ in 0..60 {
            let Some(op) = byte(rom, at(0x1D, pos)) else { break };
            match op {
                0 => break,
                3 => {
                    if let Some(arg) = byte(rom, at(0x1D, pos + 1)).filter(|a| (a & 0x7F) < limit) {
                        found.add(arg & 0x7F, format!("effect script op 3 at 1D:{pos:04X}, in the script at 1D:{start:04X}"));
                    }
                    pos += 2;
                }
                1 | 0x22 => pos += 2,
                other if other < 8 => pos += u32::from(other),
                _ => pos += 1,
            }
        }
    }
    Ok(found)
}

/// The (unit, pose) pairs a log of loads names, one line each: `<replay> f<frame> B <unit> <pose> ...`
/// (`B`: a battle animation load; the pose has bit 7 set). Returns the pairs with the replays that loaded them.
#[must_use]
pub fn observed(text: &str) -> BTreeMap<(u8, u8), BTreeSet<String>> {
    let mut seen: BTreeMap<(u8, u8), BTreeSet<String>> = BTreeMap::new();
    for line in text.lines() {
        let words: Vec<&str> = line.split_whitespace().collect();
        if words.len() < 5 || words[2] != "B" {
            continue;
        }
        let (Ok(unit), Ok(pose)) =
            (u8::from_str_radix(words[3], 16), u8::from_str_radix(words[4], 16))
        else {
            continue;
        };
        seen.entry((unit & 0x7F, pose & 0x7F)).or_default().insert(words[0].to_owned());
    }
    seen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_log_of_loads_gives_the_unit_and_pose_without_the_load_bit() {
        let log = "battle f1 B 00 85 00 slot14=00\nbattle f2 C 00 slot=0\nsecret-x f9 B 04 89 05 s\nbattle f3 B 00 85 00 s\n";
        let seen = observed(log);
        assert_eq!(seen.keys().copied().collect::<Vec<_>>(), [(0, 5), (4, 9)]);
        assert_eq!(seen[&(0, 5)].len(), 1);
    }

    /// With the ROM (`SSD_ROM_PATH`): the walk finds the poses the replays loaded, and not a pose past the table.
    #[test]
    #[ignore = "needs the ROM (SSD_ROM_PATH)"]
    fn the_scripts_reach_the_poses_a_battle_shows() {
        let bytes = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let rom = Rom::new(&bytes).unwrap();
        let found = static_poses(rom).unwrap();
        for pose in [0_u8, 2, 3, 4, 5, 6, 8, 13, 17, 18, 23, 24, 26, 27, 37, 39] {
            assert!(found.poses.contains_key(&pose), "pose {pose} is not reached");
        }
        assert!(found.poses.keys().all(|&pose| pose < 40));
    }
}
