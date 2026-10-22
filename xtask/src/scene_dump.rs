//! `scene-dump` and `scene-check`: the scene language on the cached ROM (M10, Task 3; `docs/re/data/scenes.md`).
//!
//! `scene-dump <scene id hex|all> [--out <dir>]` decompiles the story scripts of the ROM into the language of
//! `ssd_mod::scene`. The scenes are the 82 words of the pointer table at `06:D109`; a script is written once per
//! distinct start, with the ids of the scenes that share it. Each script is the commands the walk from its start
//! reaches before the next start (see `ssd_mod::scene::disassemble`): the bytes the walk does not reach are not
//! written. The text goes to stdout, or with `--out` to a folder under `.work/` or outside the repository, since it
//! is derived from the game. Nothing it prints or writes is tracked.
//!
//! `scene-check <file.scene>...` assembles scene files and prints, for each, its size and the offsets of its label
//! words, or its error with the line and column (exit 1 when one fails). It reads no ROM.
//!
//! The round trip of the 82 scenes is the ROM-gated test at the end of this module (`SSD_ROM_PATH`).

use crate::text_dump;
use anyhow::Context as _;
use ssd_mod::scene::{assemble, disassemble};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The pointer table of the scenes: one word per scene, at `06:D109`.
const TABLE: u16 = 0xD109;
/// The scenes of the table.
pub const SCENES: usize = 0x52;
/// The first byte after the last script (`06:DFCB`).
const END: u16 = 0xDFCB;
/// The bank the scripts are in.
const BANK: u8 = 6;

/// A script of the ROM: its start (a bank 6 address), the scenes that start there, and where its bytes end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Script {
    /// The start, which is the entry of every scene that shares the script.
    pub start: u16,
    /// The scenes that start here, in order.
    pub scenes: Vec<u8>,
    /// The address after the script's last byte: the next distinct start, or the end of the scripts.
    pub end: u16,
}

/// The scripts of `rom`, one per distinct start, in address order.
#[must_use]
pub fn scripts(rom: &[u8]) -> Vec<Script> {
    let mut by_start: BTreeMap<u16, Vec<u8>> = BTreeMap::new();
    for scene in 0..SCENES {
        let pointer = TABLE + 2 * scene as u16;
        let start = u16::from_le_bytes([byte(rom, pointer), byte(rom, pointer + 1)]);
        by_start.entry(start).or_default().push(scene as u8);
    }
    let starts: Vec<u16> = by_start.keys().copied().collect();
    starts
        .iter()
        .enumerate()
        .map(|(index, &start)| Script {
            start,
            scenes: by_start.get(&start).cloned().unwrap_or_default(),
            end: starts.get(index + 1).copied().unwrap_or(END),
        })
        .collect()
}

/// The byte at bank 6 address `address` (`LoROM`: the bank's window is `$8000-$FFFF`).
fn byte(rom: &[u8], address: u16) -> u8 {
    rom[usize::from(BANK) * 0x8000 + usize::from(address) - 0x8000]
}

/// The bytes of `script`, from its start to its end.
fn region<'a>(rom: &'a [u8], script: &Script) -> &'a [u8] {
    let at = |address: u16| usize::from(BANK) * 0x8000 + usize::from(address) - 0x8000;
    &rom[at(script.start)..at(script.end)]
}

/// The ids of `scenes` as the text writes them: two hexadecimal digits each, separated by commas.
fn ids(scenes: &[u8]) -> String {
    scenes.iter().map(|id| format!("{id:02x}")).collect::<Vec<_>>().join(", ")
}

/// The text of `script`: a comment that says what it is, then the scene (see `ssd_mod::scene::disassemble`).
fn script_text(rom: &[u8], script: &Script) -> anyhow::Result<String> {
    let text = disassemble(region(rom, script), script.start, script.start)
        .with_context(|| format!("scenes {}", ids(&script.scenes)))?;
    Ok(format!(
        "# Scenes {}: bank {BANK}, ${:04x} to ${:04x} of the cached ROM (derived from the game: not tracked)\n{text}",
        ids(&script.scenes),
        script.start,
        script.end
    ))
}

/// `scene-dump <scene id hex|all> [--out <dir>]`: the scripts of the ROM, to stdout or to a folder.
///
/// # Errors
///
/// The cached ROM cannot be read, a scene id is not in the table, or a script does not decompile.
pub fn dump(args: &[&str]) -> anyhow::Result<ExitCode> {
    let (selected, out) = match args {
        [selected] => (*selected, None),
        [selected, "--out", dir] => (*selected, Some(PathBuf::from(dir))),
        _ => {
            eprintln!("scene-dump: takes <scene id hex|all> and, optionally, --out <dir>");
            return Ok(crate::usage());
        }
    };
    let rom = crate::cached_rom()?;
    let all = scripts(&rom);
    let chosen: Vec<&Script> = if selected == "all" {
        all.iter().collect()
    } else {
        let id = usize::from_str_radix(selected, 16).ok().filter(|&id| id < SCENES).with_context(
            || format!("`{selected}` is not a scene id: the ids are 00 to {:02x}", SCENES - 1),
        )?;
        let script = all
            .iter()
            .find(|script| script.scenes.contains(&(id as u8)))
            .with_context(|| format!("no script holds scene {selected}"))?;
        vec![script]
    };
    match out {
        None => {
            for script in chosen {
                println!("{}", script_text(&rom, script)?);
            }
        }
        Some(dir) => {
            if let Some(reason) =
                text_dump::refused(&crate::repo_root()?, &std::path::absolute(&dir)?)
            {
                eprintln!("scene-dump: {reason}");
                return Ok(crate::usage());
            }
            std::fs::create_dir_all(&dir)?;
            for script in chosen {
                let name = script.scenes.iter().map(|id| format!("{id:02x}")).collect::<Vec<_>>();
                let path = dir.join(format!("scene-{}.scene", name.join("-")));
                std::fs::write(&path, script_text(&rom, script)?)
                    .with_context(|| format!("writing {}", path.display()))?;
                println!("{}", path.display());
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// `scene-check <file.scene>...`: assembles each file and says what it holds, or why it does not assemble.
///
/// # Errors
///
/// A file cannot be read.
pub fn check(args: &[&str]) -> anyhow::Result<ExitCode> {
    if args.is_empty() {
        eprintln!("scene-check: takes one or more scene files");
        return Ok(crate::usage());
    }
    let mut failed = false;
    for path in args {
        let text =
            std::fs::read_to_string(Path::new(path)).with_context(|| format!("reading {path}"))?;
        match assemble(&text) {
            Ok(assembled) => println!(
                "{path}: {} bytes, label words at offsets {:?}",
                assembled.bytes.len(),
                assembled.relocs
            ),
            Err(error) => {
                eprintln!("{path}: {error}");
                failed = true;
            }
        }
    }
    Ok(if failed { ExitCode::from(1) } else { ExitCode::SUCCESS })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_mod::scene::relocate;

    /// The bytes that the script's text assembles back to, at its own address, and how many bytes that is; the
    /// error that says why they are not the ROM's.
    fn round_trip(rom: &[u8], script: &Script) -> Result<usize, String> {
        let bytes = region(rom, script);
        let text = disassemble(bytes, script.start, script.start).map_err(|e| e.to_string())?;
        let assembled = assemble(&text).map_err(|e| format!("the text does not assemble: {e}"))?;
        let mut placed = assembled.bytes.clone();
        relocate(&mut placed, &assembled.relocs, script.start);
        if placed.len() > bytes.len() {
            return Err(format!("it is {} bytes, the script has {}", placed.len(), bytes.len()));
        }
        match placed.iter().zip(bytes).position(|(a, b)| a != b) {
            None => Ok(placed.len()),
            Some(at) => {
                Err(format!("the bytes differ from the ROM at offset {at:#x} of the script"))
            }
        }
    }

    /// With the ROM (`SSD_ROM_PATH`): each of the 82 scenes decompiles, and the text assembles at the script's own
    /// address to the ROM's bytes over the commands the walk reaches. Every script that does not is listed with the
    /// reason, and the test fails.
    #[test]
    #[ignore = "needs SSD_ROM_PATH"]
    fn every_scene_round_trips_through_the_language() {
        let raw = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let rom = ssd_rom::normalize(&raw, None).unwrap().bytes;
        let scripts = scripts(&rom);
        assert_eq!(scripts.iter().map(|script| script.scenes.len()).sum::<usize>(), SCENES);
        let mut passed = 0;
        let mut failures = Vec::new();
        for script in &scripts {
            match round_trip(&rom, script) {
                Ok(_) => passed += script.scenes.len(),
                Err(why) => failures.push(format!("scenes {}: {why}", ids(&script.scenes))),
            }
        }
        eprintln!("round trip: {passed} of {SCENES} scenes");
        assert!(
            failures.is_empty(),
            "{passed} of {SCENES} scenes round-trip:\n{}",
            failures.join("\n")
        );
    }
}
