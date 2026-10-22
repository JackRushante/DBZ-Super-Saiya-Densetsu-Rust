//! `rebase-replays`: records replay files again as made with this build's emulator core
//! (`ssd_oracle::CORE_VERSION`), for the case where the core changed without changing what it
//! emulates (an output-only patch of the vendored sources): the ROM, WRAM seed, initial SRAM and
//! inputs are kept, only the core version field and the checksum change. A replay that does not
//! read back intact is refused and left alone. That the new core plays the replay as the old one
//! did is not checked here; `verify-replays` and the game's own checks are what prove it.

use anyhow::Context;
use ssd_oracle::{CORE_VERSION, Replay};
use std::io::Write;
use std::path::{Path, PathBuf};

/// The command line of `rebase-replays`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Replay files, or directories of `*.ssdreplay`.
    pub paths: Vec<PathBuf>,
    /// Change nothing; exit 1 when a replay is for another core.
    pub check: bool,
}

/// Parses the arguments after `rebase-replays`: `<dir|replay>... [--check]`.
///
/// # Errors
///
/// An unknown option, or no path.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut parsed = Args { paths: Vec::new(), check: false };
    for &arg in args {
        match arg {
            "--check" => parsed.check = true,
            option if option.starts_with('-') => return Err(format!("unknown option `{option}`")),
            path => parsed.paths.push(PathBuf::from(path)),
        }
    }
    if parsed.paths.is_empty() {
        return Err("needs a directory or a replay".to_owned());
    }
    Ok(parsed)
}

/// What happened to the replays: how many were for this core already, how many were (or, with
/// `check`, would be) rebased.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Summary {
    pub current: usize,
    pub rebased: usize,
}

/// Rebases the replays named by `paths` to `core_version`, one line per file (`old -> new`).
///
/// # Errors
///
/// A path cannot be read, a file is not an intact replay (it and the ones after it are left as
/// they are), or a file cannot be written.
pub fn rebase(
    paths: &[PathBuf],
    core_version: &str,
    check: bool,
    out: &mut dyn Write,
) -> anyhow::Result<Summary> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            files.extend(crate::verify_replays::replay_paths(path)?);
        } else {
            files.push(path.clone());
        }
    }
    anyhow::ensure!(!files.is_empty(), "no replay to rebase");
    let mut summary = Summary::default();
    for file in files {
        let name = file.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let bytes = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
        let (old, new) = Replay::rebase(&bytes, core_version)
            .with_context(|| format!("{}: not rebased", file.display()))?;
        if old == core_version {
            writeln!(out, "{name}: already {core_version}")?;
            summary.current += 1;
            continue;
        }
        let note = if check { " (not written)" } else { "" };
        writeln!(out, "{name}: {old} -> {core_version}{note}")?;
        if !check {
            write_atomically(&file, &new)?;
        }
        summary.rebased += 1;
    }
    Ok(summary)
}

/// `bytes` to `path` through a temporary file next to it and a rename.
fn write_atomically(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    let tmp = path.with_extension("ssdreplay.tmp");
    let written = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written.with_context(|| format!("writing {}", path.display()))
}

/// The command: rebases to this build's core; exit 0, or 1 with `--check` when a replay is for
/// another core.
///
/// # Errors
///
/// As [`rebase`].
pub fn run(args: &Args, out: &mut dyn Write) -> anyhow::Result<u8> {
    let summary = rebase(&args.paths, CORE_VERSION, args.check, out)?;
    let verb = if args.check { "to rebase" } else { "rebased" };
    writeln!(
        out,
        "rebase-replays: {} {verb}, {} already current",
        summary.rebased, summary.current
    )?;
    Ok(u8::from(args.check && summary.rebased > 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "jgenesis 000000000000+00000000 oracle 1";
    const NEW: &str = "jgenesis 000000000000+11111111 oracle 1";

    fn replay() -> Replay {
        Replay {
            rom_crc32: 0xA8CF_4F2A,
            wram_seed: 7,
            initial_sram: Some(vec![1, 2, 3, 4]),
            inputs: (0..20).map(|f| ssd_oracle::joypad_from_bits((f * 37) & 0x0FFF)).collect(),
            mode: ssd_oracle::MODE_CLASSIC,
            events: Vec::new(),
            packs: None,
        }
    }

    #[test]
    fn the_command_line_takes_paths_and_check() {
        let args = parse_args(&["a", "--check", "b"]).unwrap();
        assert_eq!(args.paths, [PathBuf::from("a"), PathBuf::from("b")]);
        assert!(args.check);
        assert!(parse_args(&[]).unwrap_err().contains("needs"));
        assert!(parse_args(&["--force", "a"]).unwrap_err().contains("unknown option"));
    }

    #[test]
    fn a_directory_is_rebased_keeping_inputs_and_sram_and_a_second_run_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let stale = dir.path().join("a.ssdreplay");
        let current = dir.path().join("b.ssdreplay");
        replay().write_to(&stale, OLD).unwrap();
        replay().write_to(&current, NEW).unwrap();
        let before = std::fs::read(&current).unwrap();

        // --check reports and writes nothing
        let mut out = Vec::new();
        let summary = rebase(&[dir.path().to_path_buf()], NEW, true, &mut out).unwrap();
        assert_eq!(summary, Summary { current: 1, rebased: 1 });
        assert!(String::from_utf8(out).unwrap().contains(&format!("a.ssdreplay: {OLD} -> {NEW}")));
        assert_eq!(std::fs::read(&stale).unwrap(), replay().to_bytes(OLD));

        let summary = rebase(&[dir.path().to_path_buf()], NEW, false, &mut Vec::new()).unwrap();
        assert_eq!(summary, Summary { current: 1, rebased: 1 });
        assert_eq!(std::fs::read(&stale).unwrap(), replay().to_bytes(NEW));
        assert_eq!(std::fs::read(&current).unwrap(), before);
        assert!(!dir.path().join("a.ssdreplay.tmp").exists());

        let summary = rebase(&[dir.path().to_path_buf()], NEW, false, &mut Vec::new()).unwrap();
        assert_eq!(summary, Summary { current: 2, rebased: 0 });
    }

    #[test]
    fn a_replay_that_is_not_intact_is_refused_and_left_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.ssdreplay");
        let mut bytes = replay().to_bytes(OLD);
        *bytes.last_mut().unwrap() ^= 1;
        std::fs::write(&path, &bytes).unwrap();
        let error = rebase(std::slice::from_ref(&path), NEW, false, &mut Vec::new()).unwrap_err();
        assert!(format!("{error:#}").contains("corrupted"), "{error:#}");
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
