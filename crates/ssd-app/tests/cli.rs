use std::path::Path;
use std::process::{Command, Output};

/// The app with a private data directory, so no test touches the real one.
fn app(data_dir: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ssd-recompiled"));
    command.env("SSD_DATA_DIR", data_dir);
    command
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn first_launch_without_rom_exits_with_code_2_in_headless_mode() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ssd-recompiled"))
        .args(["--headless-frames", "1"])
        .env("SSD_DATA_DIR", dir.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    // clap usage errors also exit with 2, so the message proves it is the missing-ROM path.
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("No ROM yet"), "stderr: {stderr}");
}

#[test]
fn rejected_rom_exits_with_code_3_and_caches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let bogus = dir.path().join("bogus.sfc");
    std::fs::write(&bogus, vec![0x42_u8; 0x8000]).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_ssd-recompiled"))
        .arg("--rom")
        .arg(&bogus)
        .args(["--headless-frames", "1"])
        .env("SSD_DATA_DIR", dir.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(3), "stderr: {stderr}");
    assert!(stderr.contains("unknown ROM"), "stderr: {stderr}");
    assert!(!dir.path().join("cache/rom/canonical.sfc").exists());
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn headless_run_caches_rom_and_writes_screenshot() {
    let dir = tempfile::tempdir().unwrap();
    let shot = dir.path().join("shot.bmp");
    let out = Command::new(env!("CARGO_BIN_EXE_ssd-recompiled"))
        .args([
            "--rom",
            &std::env::var("SSD_ROM_PATH").unwrap(),
            "--headless-frames",
            "600",
            "--screenshot",
        ])
        .arg(&shot)
        .env("SSD_DATA_DIR", dir.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "stderr: {stderr}");
    assert!(dir.path().join("cache/rom/canonical.sfc").exists());
    assert!(std::fs::metadata(&shot).unwrap().len() > 0);
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn ui_snapshot_run_writes_a_png_of_the_panel_at_the_requested_size() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("ui.txt");
    std::fs::write(&script, "panel open\ntab memory\nselect side_record_a\n").unwrap();
    let panel = dir.path().join("panel.png");
    let out = app(dir.path())
        .args(["--rom", &std::env::var("SSD_ROM_PATH").unwrap(), "--headless-frames", "120"])
        .arg("--ui-script")
        .arg(&script)
        .arg("--ui-snapshot")
        .arg(&panel)
        .args(["--ui-size", "800x600"])
        .output()
        .unwrap();
    let stderr = stderr(&out);
    assert_eq!(out.status.code(), Some(0), "stderr: {stderr}");
    // A PNG: the signature, then the IHDR chunk with the width and height.
    let bytes = std::fs::read(&panel).unwrap();
    assert_eq!(bytes[..8], [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n']);
    assert_eq!(u32::from_be_bytes(bytes[16..20].try_into().unwrap()), 800);
    assert_eq!(u32::from_be_bytes(bytes[20..24].try_into().unwrap()), 600);
}

#[test]
fn bad_symbols_file_exits_with_code_4_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad-symbols.toml");
    std::fs::write(
        &bad,
        "[[routine]]
name = \"x\"
bogus = 1
",
    )
    .unwrap();
    // No ROM in the data dir: the symbols are checked first, so the exit is 4, not 2.
    let out = app(dir.path())
        .args(["--hooks", "verify", "--headless-frames", "1", "--symbols"])
        .arg(&bad)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("bad-symbols.toml"), "stderr: {}", stderr(&out));
}

#[test]
fn missing_symbols_file_exits_with_code_4_naming_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let out = app(dir.path())
        .args(["--hooks", "rust", "--headless-frames", "1", "--symbols"])
        .arg(dir.path().join("no-such-symbols.toml"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("no-such-symbols.toml"), "stderr: {}", stderr(&out));
}

#[test]
fn a_symbols_file_given_explicitly_is_checked_even_without_hooks() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad-symbols.toml");
    std::fs::write(&bad, "this is not toml").unwrap();
    let out =
        app(dir.path()).args(["--headless-frames", "1", "--symbols"]).arg(&bad).output().unwrap();
    assert_eq!(out.status.code(), Some(4), "stderr: {}", stderr(&out));
}

#[test]
fn a_routine_without_a_rust_function_exits_with_code_4_naming_it() {
    let dir = tempfile::tempdir().unwrap();
    let symbols = dir.path().join("symbols.toml");
    std::fs::write(
        &symbols,
        r#"[[routine]]
name = "no_such_routine"
address = 0x018000
returns = "rtl"
timing = "pure"
mode = "verify"
"#,
    )
    .unwrap();
    let out = app(dir.path())
        .args(["--hooks", "verify", "--headless-frames", "1", "--symbols"])
        .arg(&symbols)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(4), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("no_such_routine"), "stderr: {}", stderr(&out));
}

#[test]
fn the_workspace_symbols_are_found_by_default() {
    // Cargo runs integration tests inside the workspace, so the default path resolves to
    // re/symbols.toml. Without a ROM the run then stops with 2; a missing file would be 4.
    let dir = tempfile::tempdir().unwrap();
    let out =
        app(dir.path()).args(["--hooks", "verify", "--headless-frames", "1"]).output().unwrap();
    assert_eq!(out.status.code(), Some(2), "stderr: {}", stderr(&out));
    assert!(stderr(&out).contains("No ROM yet"), "stderr: {}", stderr(&out));
}

#[test]
fn screenshot_without_headless_frames_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let out =
        app(dir.path()).arg("--screenshot").arg(dir.path().join("shot.bmp")).output().unwrap();
    let stderr = stderr(&out);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("required"), "stderr: {stderr}");
    assert!(stderr.contains("--headless-frames"), "stderr: {stderr}");
    assert!(!stderr.contains("No ROM yet"), "the ROM check must not run: {stderr}");
}

#[test]
fn record_with_hooks_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = app(dir.path())
        .args(["--hooks", "rust", "--headless-frames", "1", "--record"])
        .arg(dir.path().join("run.ssdreplay"))
        .output()
        .unwrap();
    let stderr = stderr(&out);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("--hooks original"), "stderr: {stderr}");
    assert!(!stderr.contains("No ROM yet"), "the ROM check must not run: {stderr}");
}

#[test]
fn a_divergence_dir_is_ignored_with_a_warning_unless_the_hooks_are_verified() {
    let dir = tempfile::tempdir().unwrap();
    let reports = dir.path().join("reports");
    for (mode, warned) in [("original", true), ("rust", true), ("verify", false)] {
        let out = app(dir.path())
            .args(["--hooks", mode, "--headless-frames", "1", "--divergence-dir"])
            .arg(&reports)
            .output()
            .unwrap();
        let stderr = stderr(&out);
        // No ROM yet: the run stops there, after the symbols and the hooks are set up.
        assert_eq!(out.status.code(), Some(2), "{mode}: stderr: {stderr}");
        assert!(stderr.contains("No ROM yet"), "{mode}: stderr: {stderr}");
        let warning =
            format!("--divergence-dir {} is ignored with --hooks {mode}", reports.display());
        assert_eq!(stderr.contains(&warning), warned, "{mode}: stderr: {stderr}");
    }
    assert!(!reports.exists(), "nothing was saved");
}

#[test]
fn ips_without_rom_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let out = app(dir.path()).arg("--ips").arg(dir.path().join("patch.ips")).output().unwrap();
    let stderr = stderr(&out);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("required"), "stderr: {stderr}");
    assert!(stderr.contains("--rom"), "stderr: {stderr}");
}

#[test]
fn an_unknown_hooks_mode_is_a_usage_error_listing_the_modes() {
    let dir = tempfile::tempdir().unwrap();
    let out = app(dir.path()).args(["--hooks", "turbo"]).output().unwrap();
    let stderr = stderr(&out);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    for mode in ["original", "rust", "verify"] {
        assert!(stderr.contains(mode), "stderr: {stderr}");
    }
}

#[test]
fn ui_snapshot_without_headless_frames_is_a_usage_error() {
    let dir = tempfile::tempdir().unwrap();
    let out =
        app(dir.path()).arg("--ui-snapshot").arg(dir.path().join("panel.png")).output().unwrap();
    let stderr = stderr(&out);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("--headless-frames"), "stderr: {stderr}");
    assert!(!stderr.contains("No ROM yet"), "the ROM check must not run: {stderr}");
}

#[test]
fn ui_script_and_ui_size_need_a_ui_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    for flags in [&["--ui-script", "ui.txt"][..], &["--ui-size", "800x600"][..]] {
        let out = app(dir.path()).args(["--headless-frames", "1"]).args(flags).output().unwrap();
        let stderr = stderr(&out);
        assert_eq!(out.status.code(), Some(2), "{flags:?}: stderr: {stderr}");
        assert!(stderr.contains("--ui-snapshot"), "{flags:?}: stderr: {stderr}");
    }
}

#[test]
fn a_bad_ui_script_is_a_usage_error_naming_its_line() {
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("ui.txt");
    std::fs::write(&script, "panel open\n# a comment\ntab inventory\n").unwrap();
    let panel = dir.path().join("panel.png");
    let out = app(dir.path())
        .args(["--headless-frames", "1", "--ui-snapshot"])
        .arg(&panel)
        .arg("--ui-script")
        .arg(&script)
        .output()
        .unwrap();
    let stderr = stderr(&out);
    assert_eq!(out.status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("line 3"), "stderr: {stderr}");
    assert!(stderr.contains("unknown tab `inventory`"), "stderr: {stderr}");
    assert!(!stderr.contains("No ROM yet"), "the script is checked before the ROM: {stderr}");
    assert!(!panel.exists(), "no picture for a script that is not read");
}

#[test]
fn ui_size_is_width_x_height_within_the_limits() {
    let dir = tempfile::tempdir().unwrap();
    for size in ["1440", "wide", "0x900", "1440x0", "1440x9000"] {
        let out = app(dir.path())
            .args(["--headless-frames", "1", "--ui-snapshot"])
            .arg(dir.path().join("panel.png"))
            .args(["--ui-size", size])
            .output()
            .unwrap();
        let stderr = stderr(&out);
        assert_eq!(out.status.code(), Some(2), "{size}: stderr: {stderr}");
        assert!(
            stderr.contains("WIDTHxHEIGHT") || stderr.contains("each side"),
            "{size}: stderr: {stderr}"
        );
    }
}

/// Needs `SSD_ROM_PATH`; run with `cargo test -p ssd-app --release -- --ignored`.
#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn verify_run_prints_the_stats_table_and_exits_0() {
    let dir = tempfile::tempdir().unwrap();
    let out = app(dir.path())
        .args(["--rom", &std::env::var("SSD_ROM_PATH").unwrap()])
        .args(["--headless-frames", "1200", "--hooks", "verify"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let stdout = String::from_utf8_lossy(&out.stdout);
    for column in [
        "routine",
        "calls",
        "verified_ok",
        "divergences",
        "interrupted",
        "hdma_crossed",
        "sampled_out",
        "contract_mismatch",
        "stack_switched",
        "no_return",
        "mean_cpu_cycles",
        "mean_instructions",
    ] {
        assert!(
            stdout.contains(column),
            "no `{column}` column in:
{stdout}"
        );
    }
    for routine in ["advance_x_by_6", "rng_next_a", "bin24_to_packed_bcd"] {
        assert!(
            stdout.contains(routine),
            "no `{routine}` row in:
{stdout}"
        );
    }
}

#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn rust_run_exits_0_and_screenshots_a_picture() {
    let dir = tempfile::tempdir().unwrap();
    let shot = dir.path().join("rust.bmp");
    let out = app(dir.path())
        .args(["--rom", &std::env::var("SSD_ROM_PATH").unwrap()])
        .args(["--headless-frames", "1200", "--hooks", "rust", "--screenshot"])
        .arg(&shot)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "stderr: {}", stderr(&out));
    let bmp = std::fs::read(&shot).unwrap();
    let pixels = &bmp[54..];
    assert!(pixels.iter().any(|&b| b != pixels[0]), "the frame is one flat colour");
}

/// A symbols file that hooks the first RNG's address with the second RNG's Rust function: the
/// original updates one seed, the Rust code the other, so every checked call diverges.
#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn verify_divergences_exit_with_code_5_and_leave_reports() {
    let dir = tempfile::tempdir().unwrap();
    let symbols = dir.path().join("wrong-symbols.toml");
    std::fs::write(
        &symbols,
        r#"[[routine]]
name = "rng_next_b"
address = 0x0082A3
returns = "rtl"
timing = "pure"
mode = "verify"
entry_m8 = true
flags = "NVZC"

[[variable]]
name = "rng_seed_a"
address = 0x7E002F
size = 2

[[variable]]
name = "rng_seed_b"
address = 0x7E0031
size = 2
"#,
    )
    .unwrap();
    let reports = dir.path().join("reports");
    let out = app(dir.path())
        .args(["--rom", &std::env::var("SSD_ROM_PATH").unwrap()])
        .args(["--headless-frames", "1200", "--hooks", "verify", "--symbols"])
        .arg(&symbols)
        .arg("--divergence-dir")
        .arg(&reports)
        .output()
        .unwrap();
    let stderr = stderr(&out);
    assert_eq!(out.status.code(), Some(5), "stderr: {stderr}");
    assert!(stderr.contains("divergence"), "stderr: {stderr}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("rng_next_b"), "stdout: {stdout}");
    let saved: Vec<_> =
        std::fs::read_dir(&reports).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert!(
        saved.iter().any(|name| name.to_string_lossy().ends_with(".txt")),
        "no divergence report in {saved:?}"
    );
    assert!(
        saved.iter().any(|name| name.to_string_lossy().ends_with(".ssdstate")),
        "no divergence state in {saved:?}"
    );
}

/// The files of `dir` and their contents (none if it does not exist).
fn files(dir: &Path) -> Vec<(std::ffi::OsString, Vec<u8>)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<_> = entries
        .map(|entry| {
            let entry = entry.unwrap();
            (entry.file_name(), std::fs::read(entry.path()).unwrap())
        })
        .collect();
    files.sort();
    files
}

/// Runs the app headless for `frames` frames with `args`, writing a screenshot; returns it.
fn screenshot(dir: &Path, name: &str, frames: u64, args: &[&std::ffi::OsStr]) -> Vec<u8> {
    let shot = dir.join(format!("{name}.bmp"));
    let out = app(dir)
        .args(["--headless-frames", &frames.to_string(), "--screenshot"])
        .arg(&shot)
        .args(args)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{name}: stderr: {}", stderr(&out));
    std::fs::read(&shot).unwrap()
}

/// Recording and playback through the command line: a recorded run replays to the same picture
/// without touching the save files; a replay with input, played while recording, records the
/// same bytes, and plays to the same picture with every routine in `Verify` (no divergence).
#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn recorded_runs_replay_to_the_same_picture_and_record_the_same_bytes() {
    use ssd_oracle::{CORE_VERSION, Replay, joypad_from_bits};
    use std::ffi::OsStr;

    let dir = tempfile::tempdir().unwrap();
    let data = dir.path();
    let rom = std::env::var("SSD_ROM_PATH").unwrap();
    let recorded = data.join("recorded.ssdreplay");
    let recorded_shot = screenshot(
        data,
        "a",
        300,
        &[OsStr::new("--rom"), OsStr::new(&rom), OsStr::new("--record"), recorded.as_os_str()],
    );
    let replay = Replay::from_bytes(
        &std::fs::read(&recorded).unwrap(),
        ssd_rom::CANONICAL_CRC32,
        CORE_VERSION,
    )
    .unwrap();
    assert_eq!(replay.frames(), 300);

    let saves = files(&data.join("saves"));
    let replayed_shot = screenshot(data, "b", 300, &[OsStr::new("--replay"), recorded.as_os_str()]);
    assert!(recorded_shot == replayed_shot, "the replay shows the recorded run's last frame");
    assert_eq!(files(&data.join("saves")), saves, "a playback touches no save file");

    // Buttons pressed from frame 120 on, every frame a different combination.
    let pressed = data.join("pressed.ssdreplay");
    let inputs = (0..400_u16)
        .map(|f| {
            if f < 120 {
                joypad_from_bits(0)
            } else {
                joypad_from_bits(f.wrapping_mul(0x9E37) & 0x0FFF)
            }
        })
        .collect();
    std::fs::write(&pressed, Replay { inputs, ..replay }.to_bytes(CORE_VERSION)).unwrap();
    let again = data.join("again.ssdreplay");
    // More frames than the replay has: the run ends with it, and exits 0.
    let played_shot = screenshot(
        data,
        "c",
        1000,
        &[OsStr::new("--replay"), pressed.as_os_str(), OsStr::new("--record"), again.as_os_str()],
    );
    assert!(std::fs::read(&pressed).unwrap() == std::fs::read(&again).unwrap(), "byte-identical");
    let verified_shot = screenshot(
        data,
        "e",
        400,
        &[OsStr::new("--replay"), pressed.as_os_str(), OsStr::new("--hooks"), OsStr::new("verify")],
    );
    assert!(played_shot == verified_shot, "the verified playback shows the same last frame");
}

/// A replay the run cannot play exits with code 6: not a replay, an initial SRAM the cartridge
/// does not take, or `--record` naming the replay itself.
#[test]
#[ignore = "needs SSD_ROM_PATH"]
fn a_rejected_replay_exits_with_code_6() {
    use ssd_oracle::{CORE_VERSION, DEFAULT_WRAM_SEED, Replay};

    let dir = tempfile::tempdir().unwrap();
    let rom = std::env::var("SSD_ROM_PATH").unwrap();
    let run = |args: &[&std::ffi::OsStr]| {
        let out = app(dir.path())
            .args(["--rom", &rom, "--headless-frames", "10"])
            .args(args)
            .output()
            .unwrap();
        (out.status.code(), stderr(&out))
    };

    let garbage = dir.path().join("garbage.ssdreplay");
    std::fs::write(&garbage, b"not a replay").unwrap();
    let (code, stderr) = run(&["--replay".as_ref(), garbage.as_os_str()]);
    assert_eq!(code, Some(6), "stderr: {stderr}");
    assert!(stderr.contains("replay rejected") && stderr.contains("not a replay"), "{stderr}");

    let small_sram = dir.path().join("small-sram.ssdreplay");
    let replay = Replay {
        rom_crc32: ssd_rom::CANONICAL_CRC32,
        wram_seed: DEFAULT_WRAM_SEED,
        initial_sram: Some(vec![0; 16]),
        mode: 0,
        events: Vec::new(),
        packs: None,
        inputs: Vec::new(),
    };
    replay.write_to(&small_sram, CORE_VERSION).unwrap();
    let (code, stderr) = run(&["--replay".as_ref(), small_sram.as_os_str()]);
    assert_eq!(code, Some(6), "stderr: {stderr}");
    assert!(stderr.contains("replay rejected") && stderr.contains("SRAM"), "{stderr}");

    let before = std::fs::read(&small_sram).unwrap();
    let (code, stderr) = run(&[
        "--replay".as_ref(),
        small_sram.as_os_str(),
        "--record".as_ref(),
        small_sram.as_os_str(),
    ]);
    assert_eq!(code, Some(6), "stderr: {stderr}");
    assert!(stderr.contains("overwrite"), "{stderr}");
    assert_eq!(std::fs::read(&small_sram).unwrap(), before, "the replay is untouched");
}
