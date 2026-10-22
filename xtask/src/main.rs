mod art_derive;
mod bench;
mod bench_hd;
mod catalogue;
#[cfg(feature = "histogram")]
mod causes;
mod cheats_test;
mod coverage;
mod disasm;
mod export_mlb;
mod export_public;
#[cfg(test)]
mod fixtures;
mod framecheck;
#[cfg(feature = "recomp")]
mod fuzz;
mod gfx_log;
mod hd_art_run;
mod hd_bg;
mod hd_coverage;
mod hd_figures;
mod hd_items;
mod hd_maps;
mod hd_margins;
mod hd_mode7;
mod hd_pack;
mod hd_pack_public;
mod hd_pack_to_store;
mod hd_parity;
mod hd_wide;
mod kaioken_art;
mod make_replays;
mod menu_test;
mod mod_tileset;
mod probe;
mod publish_check;
mod rebase_replays;
mod recompile;
mod repro;
mod routines;
mod save;
mod scene_dump;
mod scene_flicker;
mod scene_mixed;
mod scenes;
mod shared;
mod shots;
mod standalone_diff;
mod text_dump;
mod tour;
mod tour_account;
mod tour_audit;
mod tour_battle;
mod tour_run;
mod tour_sheet;
mod tour_visit;
mod trace;
mod verify_replays;

use anyhow::Context;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// The directory of the user's data: `SSD_DATA_DIR`, or `%LOCALAPPDATA%\dbz-ssd-recompiled`.
fn data_dir() -> std::path::PathBuf {
    std::env::var_os("SSD_DATA_DIR").map(std::path::PathBuf::from).unwrap_or_else(|| {
        std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default())
            .join("dbz-ssd-recompiled")
    })
}

fn rom_cache() -> std::path::PathBuf {
    data_dir().join("cache").join("rom").join("canonical.sfc")
}

fn repo_root() -> anyhow::Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .context("running git rev-parse --show-toplevel")?;
    if !out.status.success() {
        anyhow::bail!(
            "git rev-parse --show-toplevel failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let root =
        String::from_utf8(out.stdout).context("git output is not valid UTF-8")?.trim().to_string();
    Ok(PathBuf::from(root))
}

fn tracked_files(root: &Path) -> anyhow::Result<Vec<String>> {
    let out = Command::new("git")
        .current_dir(root)
        .args(["ls-files", "-z"])
        .output()
        .context("running git ls-files")?;

    if !out.status.success() {
        anyhow::bail!("git ls-files failed: {}", String::from_utf8_lossy(&out.stderr));
    }

    let paths: Vec<String> = out
        .stdout
        .split(|&b| b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).replace('\\', "/"))
        .collect();

    if paths.is_empty() {
        anyhow::bail!("git ls-files returned no tracked files");
    }

    Ok(paths)
}

/// Why `publish-check` cannot say anything about ROM bytes, and what to do about it.
fn rom_scan_off_message(cache: &Path) -> String {
    format!(
        "publish-check: ROM scan is off: no usable ROM cache at {} (missing, unreadable, or without anything to look for).\n\
         Without the ROM the check cannot find ROM bytes, raw or pasted as literals, so a clean \
         result would prove nothing.\n\
         Fill the cache by running `ssd-recompiled --rom <path>` once (SSD_DATA_DIR relocates it), \
         or pass --allow-no-rom to accept a check by file type and path only.",
        cache.display()
    )
}

fn publish_check(allow_no_rom: bool) -> anyhow::Result<ExitCode> {
    let root = repo_root()?;
    let paths = tracked_files(&root)?;

    let mut findings = publish_check::check_paths(&paths);
    let cache = rom_cache();
    // A cache that yields no windows (empty, tiny, low-entropy) would let every file pass.
    let index = std::fs::read(&cache)
        .ok()
        .map(|rom| publish_check::RomIndex::new(&rom))
        .filter(|index| !index.is_empty());
    if let Some(index) = &index {
        for p in &paths {
            let full_path = root.join(p);
            match std::fs::read(&full_path) {
                Ok(data) if publish_check::is_vetted(p, &data) => {}
                Ok(data) => {
                    if let Some(off) = index.find_run(&data) {
                        findings.push(publish_check::Finding {
                            path: p.clone(),
                            reason: format!("contains ROM bytes at offset {off}"),
                        });
                    } else if publish_check::is_literal_scanned(p)
                        && let Some(line) = index.find_literal(&data)
                    {
                        findings.push(publish_check::Finding {
                            path: p.clone(),
                            reason: format!("contains ROM bytes as literals (run at line {line})"),
                        });
                    }
                }
                Err(e) => {
                    findings.push(publish_check::Finding {
                        path: p.clone(),
                        reason: format!("unreadable tracked file: {e}"),
                    });
                }
            }
        }
    }
    for f in &findings {
        println!("{}: {}", f.path, f.reason);
    }
    if index.is_none() && !allow_no_rom {
        eprintln!("{}", rom_scan_off_message(&cache));
        return Ok(ExitCode::from(1));
    }
    if findings.is_empty() {
        println!(
            "publish-check: clean ({} files, ROM scan: {})",
            paths.len(),
            if index.is_some() { "on" } else { "off" }
        );
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::from(1))
    }
}

/// The canonical ROM from the cache.
fn cached_rom() -> anyhow::Result<Vec<u8>> {
    let cache = rom_cache();
    let rom = std::fs::read(&cache).with_context(|| {
        format!(
            "reading the ROM cache {}: fill it by running `ssd-recompiled --rom <path>` once \
             (SSD_DATA_DIR relocates it)",
            cache.display()
        )
    })?;
    anyhow::ensure!(
        ssd_rom::crc32(&rom) == ssd_rom::CANONICAL_CRC32,
        "the ROM cache {} is not the canonical ROM: run `ssd-recompiled --rom <path>` again",
        cache.display()
    );
    Ok(rom)
}

/// Records the reference replays into `.work/replays/` (see [`make_replays`]).
fn make_replays() -> anyhow::Result<ExitCode> {
    let rom = cached_rom()?;
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    let dir = root.join(".work").join("replays");
    for made in make_replays::make(&rom, &symbols, &dir)? {
        println!(
            "{}: {} frames, {} bytes, {} sync events; the playback ends in the recorded state",
            made.path.display(),
            made.frames,
            made.bytes,
            made.sync_events
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Plays the `secret-*` replays of `dir` in Classic and with the extreme battle cheats (see
/// [`cheats_test::spike`]): the group spike of M5 task 1.
fn cheat_spike(dir: &str) -> anyhow::Result<ExitCode> {
    let rom = cached_rom()?;
    let mut out = String::new();
    cheats_test::spike(&rom, Path::new(dir), &mut out)?;
    print!("{out}");
    Ok(ExitCode::SUCCESS)
}

/// Plays the HD menus headless, scenario by scenario (see [`menu_test`]).
fn menu_test(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match menu_test::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("menu-test: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let code = menu_test::run(&rom, &args, &mut std::io::stdout())?;
    Ok(ExitCode::from(code))
}

/// Plays the replays in lockstep (see [`verify_replays`]).
fn verify_replays(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match verify_replays::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("verify-replays: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    let mods = args.mods_identity.then(|| ssd_mod::build(&rom, &[])).transpose()?;
    let config = verify_replays::config(&symbols, args.hooks, args.cheats_identity, mods.as_ref())?;
    let dir = args.dir.unwrap_or_else(|| root.join(".work").join("replays"));
    let sync_points: Vec<&str> = symbols.sync_points().iter().map(|p| p.name.as_str()).collect();
    let hooks = match args.hooks {
        verify_replays::Hooks::Rust => "rust",
        verify_replays::Hooks::Original => "original",
    };
    println!(
        "verify-replays: {}, hooks {hooks} ({} routines), sync points {}, budget {} frames",
        dir.display(),
        config.table.as_ref().map_or(0, ssd_oracle::HookTable::len),
        sync_points.join(", "),
        config.budget_frames
    );
    let code =
        verify_replays::verify(&rom, &symbols, &config, args.hooks, &dir, &mut std::io::stdout())?;
    Ok(ExitCode::from(code))
}

/// Records replay files again for this build's emulator core (see [`rebase_replays`]).
fn rebase_replays(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match rebase_replays::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("rebase-replays: {message}");
            return Ok(usage());
        }
    };
    let code = rebase_replays::run(&args, &mut std::io::stdout())?;
    Ok(ExitCode::from(code))
}

/// Times the drawing of the Enhanced picture over the replays (see [`bench_hd`]).
fn bench_hd(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match bench_hd::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("bench-hd: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let dir = repo_root()?.join(".work").join("replays");
    bench_hd::run(&rom, &dir, &hd_pack_dir(&rom), &args, &mut std::io::stdout())?;
    Ok(ExitCode::SUCCESS)
}

/// Checks that the frames a fast-forward skips do not change the picture of the one it shows (see
/// [`bench_hd::skip_check`]).
fn hd_skip_check(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match bench_hd::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("hd-skip-check: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let dir = repo_root()?.join(".work").join("replays");
    let ok = bench_hd::skip_check(&rom, &dir, &hd_pack_dir(&rom), &args, &mut std::io::stdout())?;
    Ok(if ok { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// Times the app's frame loop headless: runs the app's own `--bench-app` (the binary built next to
/// this one) over the replays at each speed, with the routines in Rust (`--hooks rust`) as the shipped
/// game runs them. `args`: `[--frames N] [--speeds A,B] [--scale N] [--hooks MODE] [--panel-closed]
/// [replay...]`.
fn bench_app(args: &[&str]) -> anyhow::Result<ExitCode> {
    let dir = repo_root()?.join(".work").join("replays");
    let mut replays = Vec::new();
    let mut passed: Vec<String> = Vec::new();
    let mut hooks = "rust".to_owned();
    let mut args = args.iter();
    while let Some(&arg) = args.next() {
        match arg {
            "--frames" | "--speeds" | "--scale" => {
                let Some(value) = args.next() else {
                    eprintln!("bench-app: {arg} needs a value");
                    return Ok(usage());
                };
                let flag = match arg {
                    "--frames" => "--bench-frames",
                    "--speeds" => "--bench-speeds",
                    _ => "--hd-scale",
                };
                passed.extend([flag.to_owned(), (*value).to_owned()]);
            }
            "--hooks" => {
                let Some(value) = args.next() else {
                    eprintln!("bench-app: --hooks needs a mode");
                    return Ok(usage());
                };
                (*value).clone_into(&mut hooks);
            }
            "--panel-closed" => passed.push("--bench-panel-closed".to_owned()),
            option if option.starts_with('-') => {
                eprintln!("bench-app: unknown option `{option}`");
                return Ok(usage());
            }
            path => replays.push(PathBuf::from(path)),
        }
    }
    if replays.is_empty() {
        replays = ["battle", "wander", "shop"]
            .iter()
            .map(|name| dir.join(format!("{name}.ssdreplay")))
            .collect();
    }
    let exe = std::env::current_exe()?
        .with_file_name(format!("ssd-recompiled{}", std::env::consts::EXE_SUFFIX));
    anyhow::ensure!(
        exe.exists(),
        "{} is not built: cargo build --release -p ssd-app [--features recomp]",
        exe.display()
    );
    let status = Command::new(&exe)
        .args(["--mode", "enhanced", "--hooks", &hooks, "--bench-app"])
        .args(&replays)
        .args(&passed)
        .status()
        .with_context(|| format!("running {}", exe.display()))?;
    Ok(if status.success() { ExitCode::SUCCESS } else { ExitCode::FAILURE })
}

/// Re-renders every frame of the replays with `ssd-hd` and compares it with the PPU's (see
/// [`hd_parity`]).
fn hd_parity(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match hd_parity::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("hd-parity: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let dir = repo_root()?.join(".work").join("replays");
    let code = hd_parity::run(&rom, &dir, &args, &mut std::io::stdout())?;
    Ok(ExitCode::from(code))
}

/// Lists the graphics streams the replays load, where they go and in which scene (see
/// [`gfx_log`]).
fn gfx_log(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match gfx_log::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("gfx-log: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let dir = repo_root()?.join(".work").join("replays");
    let files = shared::replay_files(&args.paths, &dir)?;
    anyhow::ensure!(!files.is_empty(), "gfx-log: no replay to play");
    gfx_log::run(&rom, &files, &mut std::io::stdout())?;
    Ok(ExitCode::SUCCESS)
}

/// The directory of the HD pack the converter writes for the cached ROM.
fn hd_pack_dir(rom: &[u8]) -> PathBuf {
    ssd_hdpack::convert::pack_dir(&data_dir().join("cache"), ssd_rom::crc32(rom))
}

/// Builds the HD pack from the cached ROM, under the cache directory (see [`ssd_hdpack::convert`]).
fn hd_convert() -> anyhow::Result<ExitCode> {
    let rom = cached_rom()?;
    let dir = hd_pack_dir(&rom);
    ssd_hdpack::convert::run(&rom, &dir, &mut std::io::stdout())?;
    println!("hd-convert: wrote {}", dir.display());
    Ok(ExitCode::SUCCESS)
}

/// Puts the approved art into the pack (see [`ssd_hdpack::import`]).
fn hd_import(args: &[&str]) -> anyhow::Result<ExitCode> {
    let mut art = None;
    let mut sources = None;
    let mut pack = None;
    let mut masks = None;
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        match arg {
            "--sources" => sources = rest.next().map(|&dir| Path::new(dir)),
            "--pack" => pack = rest.next().map(|&dir| PathBuf::from(dir)),
            "--masks" => masks = rest.next().map(|&dir| Path::new(dir)),
            other if art.is_none() && !other.starts_with("--") => art = Some(Path::new(other)),
            _ => {
                art = None;
                break;
            }
        }
    }
    let Some(art) = art else {
        eprintln!(
            "hd-import: <art dir> [--sources <folder>] [--masks <folder>] [--pack <pack folder>]"
        );
        return Ok(usage());
    };
    let rom = cached_rom()?;
    let dir = pack.unwrap_or_else(|| hd_pack_dir(&rom));
    ssd_hdpack::import::run(&rom, art, sources, masks, &dir, &mut std::io::stdout())?;
    Ok(ExitCode::SUCCESS)
}

/// Assembles the public repository's tree from a commit and scans it (see [`export_public`]).
fn export_public(args: &[&str]) -> anyhow::Result<ExitCode> {
    let root = repo_root()?;
    let rom = std::fs::read(rom_cache()).ok();
    let code = export_public::run(&root, rom.as_deref(), args, &mut std::io::stdout())?;
    if code == ExitCode::from(EXIT_USAGE) {
        return Ok(usage());
    }
    Ok(code)
}

/// Builds the shared HD art pack from the art store (see [`hd_pack_public`]).
fn hd_pack_public(args: &[&str]) -> anyhow::Result<ExitCode> {
    let rom = cached_rom()?;
    let root = repo_root()?;
    let code = hd_pack_public::run(&rom, &root, args, &mut std::io::stdout())?;
    if code == ExitCode::from(EXIT_USAGE) {
        return Ok(usage());
    }
    Ok(code)
}

/// Puts the art of a shared pack back into the art store (see [`hd_pack_to_store`]).
fn hd_pack_to_store(args: &[&str]) -> anyhow::Result<ExitCode> {
    let mut pack = None;
    let mut store = None;
    let mut masks = None;
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        match arg {
            "--masks" => masks = rest.next().map(|&dir| std::path::PathBuf::from(dir)),
            other if !other.starts_with("--") => {
                if pack.is_none() {
                    pack = Some(std::path::PathBuf::from(other));
                } else if store.is_none() {
                    store = Some(std::path::PathBuf::from(other));
                }
            }
            _ => {}
        }
    }
    let (Some(pack), Some(store)) = (pack, store) else {
        eprintln!("hd-pack-to-store: <pack.zip|pack dir> <art store> [--masks <dir>]");
        return Ok(usage());
    };
    hd_pack_to_store::run(&pack, &store, masks.as_deref(), &mut std::io::stdout())?;
    Ok(ExitCode::SUCCESS)
}

/// What the HD art does in the replays, or its frames (see [`hd_art_run`]).
fn hd_art(args: &[&str]) -> anyhow::Result<ExitCode> {
    if let Some((&"scenes", &["merge", out, ref dirs @ ..])) = args.split_first() {
        let dirs: Vec<std::path::PathBuf> = dirs.iter().map(std::path::PathBuf::from).collect();
        let code =
            scenes::merge(&repo_root()?, std::path::Path::new(out), &dirs, &mut std::io::stdout())?;
        return Ok(ExitCode::from(code));
    }
    if let Some((&"scenes", rest)) = args.split_first() {
        let args = match tour::parse_args(rest) {
            Ok(args) => args,
            Err(message) => {
                eprintln!("hd-art scenes: {message}");
                return Ok(usage());
            }
        };
        let rom = cached_rom()?;
        let dir = hd_pack_dir(&rom);
        let code = scenes::run(&rom, &dir, &repo_root()?, &args, &mut std::io::stdout())?;
        return Ok(ExitCode::from(code));
    }
    let args = match hd_art_run::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("hd-art: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let dir = hd_pack_dir(&rom);
    let replays = repo_root()?.join(".work").join("replays");
    let code = match args {
        hd_art_run::Args::Report { paths, scale, frames, jobs, pack, art } => {
            let dir = pack.unwrap_or(dir);
            let files = shared::replay_files(&paths, &replays)?;
            anyhow::ensure!(!files.is_empty(), "hd-art: no replay to play");
            hd_art_run::run_report(
                &rom,
                &dir,
                &files,
                hd_art_run::ReportOptions { scale, limit: frames, jobs, art },
                &mut std::io::stdout(),
            )?
        }
        hd_art_run::Args::Colours { paths, scale, frames, jobs, pack } => {
            let dir = pack.unwrap_or(dir);
            let files = shared::replay_files(&paths, &replays)?;
            anyhow::ensure!(!files.is_empty(), "hd-art: no replay to play");
            hd_art_run::run_colours(
                &rom,
                &dir,
                &files,
                scale,
                frames,
                jobs,
                &mut std::io::stdout(),
            )?
        }
        hd_art_run::Args::BgScan { paths, out } => {
            let files = shared::replay_files(&paths, &replays)?;
            hd_bg::run(&rom, &files, out.as_deref(), &mut std::io::stdout())?
        }
        hd_art_run::Args::BgMaps { frames, jobs, out, maps } => hd_bg::run_maps(
            &rom,
            &repo_root()?,
            frames,
            jobs,
            &maps,
            out.as_deref(),
            &mut std::io::stdout(),
        )?,
        hd_art_run::Args::Diff { paths, packs, every } => {
            let files = shared::replay_files(&paths, &replays)?;
            hd_art_run::run_diff(
                &rom,
                [&packs[0], &packs[1], &packs[2]],
                &files,
                every,
                &mut std::io::stdout(),
            )?
        }
        hd_art_run::Args::RefPal { paths, pack, maps_dump, out } => {
            let files = shared::replay_files(&paths, &replays)?;
            let dir = pack.unwrap_or_else(|| hd_pack_dir(&rom));
            let pack = ssd_hd::pack::Pack::open(&dir)
                .map_err(|e| anyhow::anyhow!("the pack {}: {e}", dir.display()))?;
            let (bg, maps) =
                hd_bg::refpal(&rom, &pack, &files, maps_dump.as_deref(), &mut std::io::stdout())?;
            let target = out.unwrap_or_else(|| PathBuf::from(".work/ref-palettes.json"));
            let text = serde_json::to_string_pretty(&ssd_hdpack::import::RefPalettes { bg, maps })?;
            std::fs::write(&target, text)?;
            println!("refpal: wrote {}", target.display());
            0
        }
        hd_art_run::Args::M7Scan { paths } => {
            let files = shared::replay_files(&paths, &replays)?;
            hd_mode7::scan(&rom, &files, &mut std::io::stdout())?
        }
        hd_art_run::Args::M7Dump { replay, frame, out } => {
            hd_mode7::dump(&rom, &replay, frame, &out, &mut std::io::stdout())?
        }
        hd_art_run::Args::PicScan { out, paths } => {
            let files = shared::replay_files(&paths, &replays)?;
            hd_bg::picscan(&rom, &files, &out)?
        }
        hd_art_run::Args::BgDump { replay, frames, out } => {
            hd_bg::dump(&rom, &replay, &frames, &out, &mut std::io::stdout())?
        }
        hd_art_run::Args::Scan { replay, every, pack } => {
            let dir = pack.unwrap_or(dir);
            hd_art_run::run_scan(&rom, &dir, &replay, every, &mut std::io::stdout())?
        }
        hd_art_run::Args::Figures { paths, jobs, pack, walks, frames, dump, gaps } => {
            let dir = pack.unwrap_or(dir);
            if walks.is_empty() {
                let files = shared::replay_files(&paths, &replays)?;
                hd_figures::run(&rom, &dir, &files, jobs, &mut std::io::stdout())?
            } else {
                let pack = std::sync::Arc::new(
                    ssd_hd::pack::Pack::open(&dir).map_err(|e| anyhow::anyhow!("the pack: {e}"))?,
                );
                let mut total = 0;
                for (map, party) in &walks {
                    total += hd_figures::walk(
                        &rom,
                        &repo_root()?,
                        &pack,
                        (*map, party),
                        (frames, dump),
                        gaps.as_deref(),
                        &mut std::io::stdout(),
                    )?;
                }
                u8::from(total > 0)
            }
        }
        hd_art_run::Args::Items { out, pack } => {
            let dir = pack.unwrap_or(dir);
            hd_items::run(&rom, &dir, &out, &mut std::io::stdout())?
        }
        hd_art_run::Args::Margins(args) => {
            let dir = args.pack.clone().unwrap_or(dir);
            hd_margins::run(&rom, &repo_root()?, &dir, &args, &mut std::io::stdout())?
        }
        hd_art_run::Args::MapShots { out, scale, shots, pack, font } => {
            let dir = pack.unwrap_or(dir);
            let out = out.unwrap_or_else(|| replays.join("..").join("m6-art"));
            hd_art_run::run_map_shots(
                &rom,
                &repo_root()?,
                (&dir, &out),
                scale,
                &shots,
                font,
                &mut std::io::stdout(),
            )?
        }
        hd_art_run::Args::Shots { out, scale, shots, pack, font } => {
            let dir = pack.unwrap_or(dir);
            let out = out.unwrap_or_else(|| replays.join("..").join("m6-art"));
            hd_art_run::run_shots(&rom, &dir, &out, scale, &shots, font, &mut std::io::stdout())?
        }
    };
    Ok(ExitCode::from(code))
}

/// Checks the HD pack's coverage of the replays (see [`hd_coverage`]).
fn hd_coverage(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match hd_coverage::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("hd-coverage: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let dir = hd_pack_dir(&rom);
    let files = shared::replay_files(&args.paths, &repo_root()?.join(".work").join("replays"))?;
    anyhow::ensure!(!files.is_empty(), "hd-coverage: no replay to play");
    hd_coverage::run(&rom, &dir, &files, &args, &mut std::io::stdout())?;
    Ok(ExitCode::SUCCESS)
}

/// The automatic coverage tour of the HD art (see [`tour`]).
fn hd_tour(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match tour::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("hd-tour: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let dir = hd_pack_dir(&rom);
    let code = tour::run(&rom, &dir, &repo_root()?, &args, &mut std::io::stdout())?;
    Ok(ExitCode::from(code))
}

/// The field maps as pictures (see [`hd_maps`]).
fn hd_maps(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match hd_maps::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("hd-maps: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let root = repo_root()?;
    let code = match args {
        hd_maps::Args::Dump { out, maps, frames, jobs, base } => hd_maps::run_dump(
            &rom,
            &root,
            out.as_deref(),
            &maps,
            frames,
            jobs,
            base.as_deref(),
            &mut std::io::stdout(),
        )?,
        hd_maps::Args::Shots { out, pack, scale, shots, base } => hd_maps::run_shots(
            &rom,
            &root,
            &pack.unwrap_or_else(|| hd_pack_dir(&rom)),
            &out.unwrap_or_else(|| root.join(".work").join("maps").join("shots")),
            scale,
            &shots,
            base.as_deref(),
            &mut std::io::stdout(),
        )?,
        hd_maps::Args::Scan { paths, every } => {
            let files = shared::replay_files(&paths, &root.join(".work").join("replays"))?;
            hd_maps::run_scan(&rom, &files, every, &mut std::io::stdout())?
        }
    };
    Ok(ExitCode::from(code))
}

/// Merges the approved art of `assets/hd/` into the HD pack, or checks it (see [`hd_pack`]).
fn hd_pack(args: &[&str]) -> anyhow::Result<ExitCode> {
    let check = match hd_pack::parse_args(args) {
        Ok(check) => check,
        Err(message) => {
            eprintln!("hd-pack: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let code =
        hd_pack::run(&rom, &repo_root()?, &hd_pack_dir(&rom), check, &mut std::io::stdout())?;
    Ok(ExitCode::from(code))
}

/// Checks the widescreen field view against the game, or writes its pictures (see [`hd_wide`]).
fn hd_wide(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match hd_wide::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("hd-wide: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let root = repo_root()?;
    let dir = root.join(".work").join("replays");
    let code = match args {
        hd_wide::Args::Check { paths, frames, jobs, widths } => {
            hd_wide::run_check(&rom, &dir, &paths, frames, jobs, &widths, &mut std::io::stdout())?
        }
        hd_wide::Args::Shots { widths, shots } => hd_wide::run_shots(
            &rom,
            &dir,
            &root.join(".work").join("hd-wide"),
            &widths,
            &shots,
            &mut std::io::stdout(),
        )?,
    };
    Ok(ExitCode::from(code))
}

/// Measures the share of a replay's instructions that run as Rust (see [`coverage`]).
fn coverage(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match coverage::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("coverage: {message}");
            return Ok(usage());
        }
    };
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    if let Err(message) = coverage::check_excluded(&symbols, &args.exclude) {
        eprintln!("coverage: {message}");
        return Ok(usage());
    }
    let rom = cached_rom()?;
    let replay =
        args.replay.unwrap_or_else(|| root.join(".work").join("replays").join("battle.ssdreplay"));
    coverage::coverage(
        &rom,
        &symbols,
        &replay,
        &args.exclude,
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )?;
    Ok(ExitCode::SUCCESS)
}

/// Times the real ROM headless, unhooked and with every routine in Rust (see [`bench`]).
fn bench() -> anyhow::Result<ExitCode> {
    let rom = cached_rom()?;
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    let table = ssd_game::hook_table(&symbols, Some(ssd_oracle::HookMode::Rust))?;
    let (start, frames, repeats) = (bench::START_FRAME, bench::FRAMES, bench::REPEATS);
    bench::bench(&rom, &table, start, frames, repeats, &mut std::io::stdout())?;
    Ok(ExitCode::SUCCESS)
}

/// Runs the call of a divergence report again (see [`repro`]).
fn repro(args: &[&str]) -> anyhow::Result<ExitCode> {
    let state = match repro::parse_args(args) {
        Ok(state) => state,
        Err(message) => {
            eprintln!("repro: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    // Every routine of the symbols file, whatever its mode: a routine kept on the original
    // because it diverges is the one to fix.
    let table = ssd_game::hook_table(&symbols, None)?;
    let resolve = |name: &str| table.entries().find(|entry| entry.name == name).cloned();
    let code = repro::repro(
        &rom,
        &state,
        &resolve,
        Some(std::sync::Arc::new(symbols)),
        &mut std::io::stdout(),
        &mut std::io::stderr(),
    )?;
    Ok(ExitCode::from(code))
}

/// Writes the Mesen labels and the tracer symbols (see [`export_mlb`]).
fn export_mlb(args: &[&str]) -> anyhow::Result<ExitCode> {
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    let dir = match args {
        [] => root.join(".work").join("re"),
        [dir] => PathBuf::from(dir),
        _ => {
            eprintln!("export-mlb: takes at most one argument, the output directory");
            return Ok(usage());
        }
    };
    for path in export_mlb::export(&symbols, &dir)? {
        println!("export-mlb: wrote {}", path.display());
    }
    Ok(ExitCode::SUCCESS)
}

/// The trace runs of `.work/re/` (every folder with an `exec.csv`), or the ones given.
fn load_runs(root: &Path, dirs: &[PathBuf]) -> anyhow::Result<Vec<trace::Run>> {
    let dirs = if dirs.is_empty() {
        trace::discover(&root.join(".work").join("re"))
    } else {
        dirs.to_vec()
    };
    dirs.iter().map(|dir| trace::load_run(dir)).collect()
}

/// Disassembles a routine of the cached ROM (see [`disasm`]).
fn disasm(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match disasm::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("disasm: {message}");
            return Ok(usage());
        }
    };
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    let rom = cached_rom()?;
    let image = disasm::RomImage(&rom);
    let address = disasm::resolve_target(&symbols, &args.target)?;
    let runs = if args.trace { load_runs(&root, &[]).unwrap_or_default() } else { Vec::new() };
    let observed = trace::observed_widths(&runs);
    let options =
        disasm::Options { m8: args.m8, x8: args.x8, len: args.len, dbr: args.dbr, dp: args.dp };
    let known = symbols.routines().iter().find(|r| r.address == trace::phys(address));
    let (entry, source) = disasm::entry_widths(
        &options,
        known.map(|r| (r.entry_m8, r.entry_x8)),
        observed.get(&trace::phys(address)).copied(),
    );
    let get = |pc: u32| observed.get(&trace::phys(pc)).copied();
    let mut listing = disasm::disassemble(&image, address, entry, &options, &get);
    listing.entry_source = source;
    let namer = disasm::Namer { symbols: Some(&symbols) };
    let name = namer.code(address).map(|n| format!(" {n}")).unwrap_or_default();
    let title = format!("disasm {address:06X}{name} ({} trace run(s))", runs.len());
    print!("{}", disasm::render(&listing, &image, &namer, &options, &title));
    Ok(ExitCode::SUCCESS)
}

/// Recompiles the ROM's code into Rust under `.work/recomp/` (see [`recompile`]).
fn recompile() -> anyhow::Result<ExitCode> {
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    let rom = cached_rom()?;
    let runs = load_runs(&root, &[])?;
    recompile::run(&rom, &symbols, &runs, &root, &mut std::io::stdout())?;
    Ok(ExitCode::SUCCESS)
}

/// Plays a replay frame by frame with and without the routines (see [`framecheck`]).
fn framecheck(args: &[&str]) -> anyhow::Result<ExitCode> {
    let [replay] = args else {
        eprintln!("framecheck: takes one argument, the replay");
        return Ok(usage());
    };
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    let rom = cached_rom()?;
    framecheck::run(&rom, &symbols, Path::new(replay), &mut std::io::stdout())?;
    Ok(ExitCode::SUCCESS)
}

/// Plays replays on a free-running original and a free-running standalone machine (see
/// [`standalone_diff`]); exit 1 when one differs.
fn standalone_diff(args: &[&str]) -> anyhow::Result<ExitCode> {
    let verbose = args.contains(&"--verbose");
    let cheats = args.contains(&"--cheats-identity");
    let mods_identity = args.contains(&"--mods-identity");
    let replays: Vec<&str> = args.iter().copied().filter(|a| !a.starts_with("--")).collect();
    if replays.is_empty()
        || args.iter().any(|a| {
            a.starts_with("--")
                && !["--verbose", "--cheats-identity", "--mods-identity"].contains(a)
        })
    {
        eprintln!(
            "standalone-diff: takes [--verbose] [--cheats-identity] [--mods-identity] and one or more replays"
        );
        return Ok(usage());
    }
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    let rom = cached_rom()?;
    let mods = mods_identity.then(|| ssd_mod::build(&rom, &[])).transpose()?;
    let mut differing = 0_u32;
    for replay in replays {
        let outcome = standalone_diff::run(
            &rom,
            &symbols,
            Path::new(replay),
            verbose,
            cheats,
            mods.as_ref(),
            &mut std::io::stdout(),
        )?;
        differing += u32::from(outcome.first_difference.is_some());
    }
    Ok(ExitCode::from(u8::from(differing > 0)))
}

/// Writes the original's blocks as a Tiled tileset for painting pack maps, outside the repository or under `.work/`
/// (see [`mod_tileset`]).
fn mod_tileset_command(args: &[&str]) -> anyhow::Result<ExitCode> {
    let [out] = args else {
        eprintln!("mod-tileset: takes <out dir> (under .work/)");
        return Ok(usage());
    };
    let out = std::path::absolute(out)?;
    if let Some(reason) = text_dump::refused(&repo_root()?, &out) {
        eprintln!("mod-tileset: {reason}");
        return Ok(usage());
    }
    let rom = cached_rom()?;
    mod_tileset::run(&rom, &out)?;
    Ok(ExitCode::SUCCESS)
}

/// Packs a data pack folder into a `.ssdpack` zip, after its checks against the cached ROM (see
/// [`ssd_mod`]). `--local` lets the files hold runs of the ROM's bytes, for a build that is not tracked.
fn mod_build(args: &[&str]) -> anyhow::Result<ExitCode> {
    let local = args.contains(&"--local");
    let paths: Vec<&str> = args.iter().copied().filter(|a| *a != "--local").collect();
    let [dir, out] = paths.as_slice() else {
        eprintln!("mod-build: takes <dir> <out.ssdpack> [--local]");
        return Ok(usage());
    };
    let pack = ssd_mod::Pack::from_dir(Path::new(dir))?;
    let rom = cached_rom()?;
    let options = ssd_mod::CheckOptions { allow_rom_bytes: local };
    if let Err(errors) = ssd_mod::check_set(&rom, std::slice::from_ref(&pack), options) {
        for error in &errors {
            eprintln!("mod-build: refused: {error}");
        }
        return Ok(ExitCode::from(1));
    }
    pack.write_zip(Path::new(out))?;
    println!("mod-build: {out}: pack `{}` {}", pack.manifest.pack.id, pack.manifest.pack.version);
    Ok(ExitCode::SUCCESS)
}

/// Checks data packs against the cached ROM and prints the placements of their data, the image's size, its
/// CRC32 and the set id (see [`ssd_mod`]). `--local` as for [`mod_build`].
fn mod_check(args: &[&str]) -> anyhow::Result<ExitCode> {
    let local = args.contains(&"--local");
    let paths: Vec<&str> = args.iter().copied().filter(|a| *a != "--local").collect();
    if paths.is_empty() || args.iter().any(|a| a.starts_with("--") && *a != "--local") {
        eprintln!(
            "mod-check: takes <pack>... [--local], each a folder with pack.toml or a .ssdpack zip"
        );
        return Ok(usage());
    }
    let rom = cached_rom()?;
    let packs = paths
        .iter()
        .map(|path| ssd_mod::Pack::open(Path::new(path)))
        .collect::<Result<Vec<_>, _>>()?;
    let options = ssd_mod::CheckOptions { allow_rom_bytes: local };
    if let Err(errors) = ssd_mod::check_set(&rom, &packs, options) {
        for error in &errors {
            println!("refused: {error}");
        }
        return Ok(ExitCode::from(1));
    }
    println!("checks: ok, {} pack(s)", packs.len());
    let image = ssd_mod::build(&rom, &packs)?;
    for placement in &image.placements {
        println!(
            "{} {:>5}  ${:02X}:{:04X}  {} bytes",
            placement.family.name(),
            placement.id,
            placement.address >> 16,
            placement.address & 0xFFFF,
            placement.len
        );
    }
    println!("image: {} bytes, CRC32 {:08X}", image.bytes.len(), image.crc32);
    println!("set id: {:016X}", image.set_id);
    Ok(ExitCode::SUCCESS)
}

/// Builds the routine table from the traces (see [`routines`]).
fn routines(args: &[&str]) -> anyhow::Result<ExitCode> {
    let mut dirs = Vec::new();
    let mut top = 50_usize;
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--runs" => {
                for dir in it.by_ref().copied().take_while(|d| !d.starts_with("--")) {
                    dirs.push(PathBuf::from(dir));
                }
            }
            "--top" => match it.next().and_then(|n| n.parse().ok()) {
                Some(n) => top = n,
                None => {
                    eprintln!("routines: --top needs a number");
                    return Ok(usage());
                }
            },
            other => {
                eprintln!("routines: unknown argument `{other}`");
                return Ok(usage());
            }
        }
    }
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    let rom = cached_rom()?;
    let runs = load_runs(&root, &dirs)?;
    anyhow::ensure!(
        !runs.is_empty(),
        "no trace run in .work/re/: make some with tools/re/run_trace.sh (tools/re/README.md)"
    );
    let table = routines::build(&runs, &disasm::RomImage(&rom), &symbols);
    let out = root.join(".work").join("re");
    std::fs::create_dir_all(&out)?;
    std::fs::write(out.join("routines.csv"), routines::csv(&table))?;
    let text = routines::summary(&table, &runs, top);
    std::fs::write(out.join("routines.txt"), &text)?;
    print!("{text}");
    println!(
        "
wrote {} and routines.txt",
        out.join("routines.csv").display()
    );
    Ok(ExitCode::SUCCESS)
}

/// Counts a routine's calls in the replays, or checks its Rust function (see [`probe`]).
fn probe(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match probe::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("probe: {message}");
            return Ok(usage());
        }
    };
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    let rom = cached_rom()?;
    let dir = args.dir.clone().unwrap_or_else(|| root.join(".work").join("replays"));
    let subject = probe::subject(&args, &symbols)?;
    if args.verify {
        let code =
            probe::verify(&rom, &symbols, &subject.name, &dir, args.quick, &mut std::io::stdout())?;
        return Ok(ExitCode::from(code));
    }
    let runs = load_runs(&root, &[]).unwrap_or_default();
    let observed = trace::observed_widths(&runs);
    let get = |pc: u32| observed.get(&trace::phys(pc)).copied();
    print!("{}", probe::run_probe(&rom, &symbols, &subject, &args, &dir, &get)?);
    Ok(ExitCode::SUCCESS)
}

/// Plays a script headless and writes screenshots (see [`shots`]).
fn shots(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match shots::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("shots: {message}");
            return Ok(usage());
        }
    };
    let root = repo_root()?;
    let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
    let rom = cached_rom()?;
    let hd_pack = args.hd_pack.clone().unwrap_or_else(|| hd_pack_dir(&rom));
    shots::run(&rom, &symbols, &args, &root, &hd_pack, &mut std::io::stdout())?;
    Ok(ExitCode::SUCCESS)
}

/// Edits a save file (see [`save`]).
/// `chapter-save`: writes the save slot a chapter of the packs starts from (see [`save::chapter_save`]).
fn chapter_save(args: &[&str]) -> anyhow::Result<ExitCode> {
    match save::chapter_save(args, &cached_rom()?) {
        Ok(text) => {
            print!("{text}");
            Ok(ExitCode::SUCCESS)
        }
        Err(error) => {
            eprintln!("chapter-save: {error:#}");
            Ok(usage())
        }
    }
}

fn save_edit(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match save::parse_edit_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("save-edit: {message}");
            return Ok(usage());
        }
    };
    let game = if args.needs_game() { Some(save::Game::from_rom(&cached_rom()?)?) } else { None };
    save::edit(&args, game.as_ref(), &mut std::io::stdout())?;
    Ok(ExitCode::SUCCESS)
}

/// Writes a replay that starts from a save file (see [`save`]).
fn replay_from_save(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match save::parse_replay_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("replay-from-save: {message}");
            return Ok(usage());
        }
    };
    save::replay_from_save(&cached_rom()?, &args, &mut std::io::stdout())?;
    Ok(ExitCode::SUCCESS)
}

/// Prints the typed game catalogue read from the cached ROM (see [`catalogue`]).
fn catalogue(args: &[&str]) -> anyhow::Result<ExitCode> {
    let kinds = match catalogue::parse_args(args) {
        Ok(kinds) => kinds,
        Err(message) => {
            eprintln!("catalogue: {message}");
            return Ok(usage());
        }
    };
    let rom = cached_rom()?;
    let data = ssd_catalogue::Catalogue::from_rom(&rom).context("reading the catalogue")?;
    print!("{}", catalogue::render(&data, &kinds));
    Ok(ExitCode::SUCCESS)
}

/// Writes the text tables of the cached ROM as a TSV outside the repository (see [`text_dump`]).
fn text_dump(args: &[&str]) -> anyhow::Result<ExitCode> {
    let args = match text_dump::parse_args(args) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("text-dump: {message}");
            return Ok(usage());
        }
    };
    if let Some(reason) = text_dump::refused(&repo_root()?, &std::path::absolute(&args.out)?) {
        eprintln!("text-dump: {reason}");
        return Ok(usage());
    }
    let rom = cached_rom()?;
    text_dump::run(&rom, &args, &mut std::io::stdout())?;
    Ok(ExitCode::SUCCESS)
}

/// The exit code of a command line the commands do not take (`EX_USAGE`): apart from the codes
/// the commands give their results (`verify-replays`: 2 = unverified; `repro`: 3 = cannot be
/// reproduced).
const EXIT_USAGE: u8 = 64;

fn usage() -> ExitCode {
    eprintln!("usage: cargo xtask publish-check [--allow-no-rom]");
    eprintln!("       cargo run --release -p xtask -- make-replays");
    eprintln!(
        "       cargo run --release -p xtask -- verify-replays [--hooks rust|original] [--cheats-identity] [--mods-identity] [dir]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- coverage [--exclude name[,name...]] [replay]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- hd-parity [--frames N] [--jobs N] [replay|dir...]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- hd-wide check [--frames N] [--jobs N] [--widths A,B] [replay|dir...]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- hd-wide shots [--widths A,B] [label=replay:frame...]"
    );
    eprintln!("       cargo run --release -p xtask -- rebase-replays <dir|replay>... [--check]");
    eprintln!("       cargo run --release -p xtask -- gfx-log [replay|dir...]");
    eprintln!("       cargo run --release -p xtask -- hd-convert");
    eprintln!(
        "       cargo run --release -p xtask -- hd-art report [--scale N] [--frames N] [--jobs N] [--pack DIR] [--no-art] [replay|dir...]"
    );
    eprintln!("       cargo run --release -p xtask -- hd-art bgscan [replay|dir...]");
    eprintln!("       cargo run --release -p xtask -- hd-art bgmaps [--frames N] [--jobs N]");
    eprintln!("       cargo run --release -p xtask -- hd-art picscan <out dir> [replay|dir...]");
    eprintln!(
        "       cargo run --release -p xtask -- hd-art bgdump <replay> <frame>[,<frame>...] <out dir>"
    );
    eprintln!("       cargo run --release -p xtask -- hd-art items [--out DIR] [--pack DIR]");
    eprintln!("       cargo run --release -p xtask -- hd-art m7scan [replay|dir...]");
    eprintln!("       cargo run --release -p xtask -- hd-art m7dump <replay> <frame> <out dir>");
    eprintln!(
        "       cargo run --release -p xtask -- hd-art colours [--scale N] [--frames N] [--jobs N] [--pack DIR] [replay|dir...]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- hd-art shots [--scale N] [--out DIR] [--pack DIR] [--no-font] label=replay:frame..."
    );
    eprintln!(
        "       cargo run --release -p xtask -- hd-art margins [--maps N,..] [--scale N] [--jobs N] [--list] [--find ID] [--objects] [--base SAVE] [--pack DIR] [label=MAP:X,Y[:DX]...]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- hd-import <art dir> [--sources <dir>] [--masks <dir>] [--pack <dir>]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- hd-coverage [--jobs N] [--assets] [replay|dir...]"
    );
    eprintln!("       cargo run --release -p xtask -- hd-pack [--check]");
    eprintln!(
        "       cargo run --release -p xtask -- export-public <commit> <out-dir> [--allow-no-rom]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- hd-pack-public <art store> <out.zip> [--masks <dir>]... [--sources <dir>] [--maps-quality N] [--revision TEXT] [--jobs N] [--without-masks] [--no-verify] [--keep-reference <dir>]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- hd-pack-to-store <pack.zip|pack dir> <art store> [--masks <dir>]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- hd-maps dump [--out DIR] [--maps N,..] [--frames N] [--jobs N] [--base SAVE]"
    );
    eprintln!("       cargo run --release -p xtask -- hd-maps scan [--every N] [replay|dir...]");
    eprintln!(
        "       cargo run --release -p xtask -- hd-maps shots [--out DIR] [--pack DIR] [--scale N] [--base SAVE] label=map:x,y..."
    );
    eprintln!(
        "       cargo run --release -p xtask -- hd-tour [--class a,b] [--maps [N,..]] [--groups [N,..]] [--replays [name,..]] [--menus] [--title] [--only part,..] [--out DIR] [--fresh] [--jobs N] [--stride N] [--frames N] [--seed N] [--config FILE] [--base SAVE] [--dump DIR[:EVERY]] [--gaps DIR] [--no-font] [--list] [--report]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- bench-hd [--frames N] [--pack DIR] [replay|dir...]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- hd-skip-check [--frames N] [--pack DIR] [replay...]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- bench-app [--frames N] [--speeds A,B] [--scale N] [--hooks MODE] [--panel-closed] [replay...]"
    );
    eprintln!("       cargo run --release -p xtask -- repro <path.ssdstate>");
    eprintln!(
        "       cargo run --release -p xtask -- menu-test field|town|diff|items --save <file.sav> [--slot N] [name...] | --replay <file.ssdreplay> --frames N [name...]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- menu-test cheats [--save <file.sav>] [--replays <dir>] [name...]"
    );
    eprintln!("       cargo run --release -p xtask -- cheat-spike <dir>");
    eprintln!("       cargo run --release -p xtask -- recompile");
    eprintln!("       cargo run --release -p xtask -- framecheck <replay>");
    eprintln!(
        "       cargo run --release -p xtask -- standalone-diff [--verbose] [--cheats-identity] [--mods-identity] <replay>..."
    );
    eprintln!("       cargo run --release -p xtask -- mod-build <dir> <out.ssdpack> [--local]");
    eprintln!("       cargo run --release -p xtask -- mod-check <pack>... [--local]");
    eprintln!("       cargo run --release -p xtask -- mod-tileset <out dir>");
    eprintln!(
        "       cargo run --release -p xtask -- art-derive <hd.png> <face|tiles> <out dir> [--palette <file>]"
    );
    eprintln!("       cargo run --release -p xtask -- kaioken-art <pack dir>");
    eprintln!(
        "       cargo run --release --features recomp -p xtask -- recomp-cover <out.cov> <replay>..."
    );
    eprintln!(
        "       cargo run --release --features recomp -p xtask -- fuzz-recomp harvest [--seeds N] [--only A,B] <dir> <replay>...\n       cargo run --release --features recomp -p xtask -- fuzz-recomp run [--seed N] [--iters N] [--seeds N] [--seconds S] [--budget F] [--only A,B] [--resume] [--part I/N] [--no-hdma] [--no-nmi] [--at-frame F] [--repro KEY:TAG:IT] [--verbose] <dir> <out.tsv>"
    );
    eprintln!(
        "       cargo run --release --features recomp -p xtask -- fuzz-report [--cover f.cov]... [--fuzz f.tsv]..."
    );
    eprintln!("       cargo run --release -p xtask -- bench");
    eprintln!("       cargo run --release -p xtask -- export-mlb [out-dir]");
    eprintln!(
        "       cargo run --release -p xtask -- disasm <address|symbol> [--len N] [--m8|--m16] [--x8|--x16] [--dbr XX] [--dp XXXX] [--no-trace]"
    );
    eprintln!("       cargo run --release -p xtask -- routines [--runs dir...] [--top N]");
    eprintln!(
        "       cargo run --release -p xtask -- probe <symbol> | --address A --returns rts|rtl [--name n] [--entry-m8 b] [--entry-x8 b] [--cycles N] [--samples N] [--dir D] [--verify [--quick]]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- shots <script|timeline.txt> [--frames a,b|--every N] [--upto N] [--sheet COLS] [--peek v,a] [--state-in f] [--state-out f] [--wram-out f] [--mesen-inputs f] [--hd-scale N] [--pack DIR] [--gaps DIR] [--mods PACK]... [--out dir] [--no-png]"
    );
    eprintln!(
        "       cargo run --release -p xtask -- catalogue [--kind characters|cards|enemies|items|attacks|encounters]"
    );
    eprintln!("       cargo run --release -p xtask -- text-dump [out.tsv]");
    eprintln!("       cargo run --release -p xtask -- scene-dump <scene id hex|all> [--out <dir>]");
    eprintln!("       cargo run --release -p xtask -- scene-check <file.scene>...");
    eprintln!(
        "       cargo run --release -p xtask -- save-edit <in.sav> <out.sav> [--slot N] [--to-slot N] [--set field=value ...]"
    );
    eprintln!("       cargo run --release -p xtask -- save-dump <file.sav>");
    eprintln!(
        "       cargo run --release -p xtask -- chapter-save <pack>... --chapter <id> --out <folder>"
    );
    eprintln!(
        "       cargo run --release -p xtask -- replay-from-save <save.sav> <script.txt> <out.ssdreplay> [--slot N] [--title] [--cheats]"
    );
    eprintln!(
        "verify-replays plays every *.ssdreplay of dir (default .work/replays) in lockstep: exit 0 \
         all green, 1 a divergence or an error (a missing directory, a replay that cannot be \
         read), 2 unverified (a replay without sync events, or no replay in the directory)."
    );
    eprintln!(
        "coverage plays the replay (default .work/replays/battle.ssdreplay) twice, every routine \
         in Verify to measure the instructions of the original per call, then in Rust to count \
         the instructions left original, and prints the share that runs as Rust. It counts \
         instructions, and routines called from the idle wait dominate it: --exclude \
         rng_next_a,rng_next_b gives a second, conservative figure without them. Exit 0, or 1 \
         on an error (a replay that cannot be read, a failed run)."
    );
    eprintln!(
        "hd-parity plays every replay (default .work/replays; --jobs at a time, 4 by default) with          the PPU's render log on, re-renders each frame with ssd-hd at 1x and compares it with          the PPU's frame pixel for pixel, then plays it again with the log off: the machine's          state and the frames must come out the same. Reports per replay the frames checked and          differing, the first differing pixel, the render time and the PPU features used. Exit 0          when every frame is equal and the machine is unchanged, 1 otherwise."
    );
    eprintln!(
        "hd-wide check plays every replay (default .work/replays) with the PPU's render log on and, for the frames of the field (game_mode 00 and 06), checks the ported map lookup against the game's own BG2 tile map words and the ported object builder against the OAM, and renders the wide view at each --widths (256,372,488 by default): at 256 the frame must equal the Classic frame, wider the original window inside it must equal it except under the objects the original does not draw, and no tile from outside the map may be drawn. hd-wide shots writes wide frames of chosen frames (label=replay:frame; the default set when none) to .work/hd-wide/. Exit 0 when green, 1 otherwise."
    );
    eprintln!(
        "rebase-replays rewrites the replays (a directory's *.ssdreplay, or the files named) as made \
         with this build's emulator core: the core version field and the checksum change, the ROM, \
         WRAM seed, initial SRAM and inputs are kept, and a file that does not read back intact is \
         refused. For a change of the vendored core that does not change what it emulates. With \
         --check nothing is written and the exit code is 1 when a replay is for another core."
    );
    eprintln!(
        "repro reloads the state of a divergence report (the .txt next to it names the routine) \
         and runs that routine's call again in Verify with the Rust function of this build: exit \
         0 it now matches the original, 1 it still diverges (or an error: the ROM cache, a file \
         that cannot be read, the symbols), 3 it cannot be reproduced (the state is for another \
         ROM or core, the report has no header, the routine moved, the state is not at the \
         entry of its call, the call is not compared; the reason is on stderr). A match whose \
         declared cost is wrong or missing exits 0 with a warning on stderr."
    );
    eprintln!(
        "bench plays the battle script on the cached ROM unhooked to frame {}, saves the state \
         there, then times {} frames from it, headless, {} times for each of two modes (no \
         hooks, every routine of re/symbols.toml in Rust; the order alternates), and prints the \
         median run of each with the frames per second of all of them (no assertion; the Rust \
         line also gives the hooked calls per second, the Rust runs and the timing fallbacks). \
         Exit 0, or 1 on an error (the ROM cache, the symbols, a failed run).",
        bench::START_FRAME,
        bench::FRAMES,
        bench::REPEATS
    );
    eprintln!(
        "export-mlb writes dbz.mlb (Mesen2 labels: import them in the debugger) and symbols.lua \n         (for the tracer) from re/symbols.toml into out-dir (default .work/re). Exit 0, or 1 on an \n         error."
    );
    eprintln!(
        "disasm lists a routine of the cached ROM: 65816 with the M/X widths followed across          REP/SEP/PLP (`?` marks assumed ones), local branches followed to the end of the routine          (--len N: exactly N bytes), names from re/symbols.toml, widths from the trace runs of          .work/re/. Stdout only."
    );
    eprintln!(
        "routines builds the routine table from the trace runs of .work/re/ (or --runs dirs; see          tools/re/README.md): .work/re/routines.csv, and the top candidates (leaf, no MMIO,          small, hot, not yet in symbols.toml) as text."
    );
    eprintln!(
        "probe plays the reference replays with a counting hook at the routine and reports its          call sites, registers, cost, writes and a listing; --verify runs every call of every          replay in Verify, then the routine alone in Rust in lockstep (--quick skips that): exit          0 matched, 1 divergence or cost problem, 2 never called."
    );
    eprintln!(
        "shots plays a script of ssd_game::scripts or a timeline file headless and writes PNGs          (or one contact sheet) of the screen at chosen frames with WRAM bytes to find where the          game is; nothing is shown in a window."
    );
    eprintln!(
        "catalogue prints the typed game data (characters, cards, enemies, items, attacks,          encounters) read from the cached ROM as text tables, all kinds or the one of --kind.          Stdout only."
    );
    eprintln!(
        "text-dump writes every string of the text tables of the cached ROM (dialogue, help, \
         battle messages, names, menu blocks, the ending) decoded, one row per page keyed by its \
         ROM coordinates, to out.tsv (default {}) and the messages' lists to messages.tsv beside \
         it, and prints a summary per table. The files are ROM-derived: keep them outside the \
         repository (docs/re/data/text-engine.md).",
        text_dump::DEFAULT_OUT
    );
    eprintln!(
        "save-edit copies a battery save, changes the fields named by --set in slot N (default 1;          --to-slot writes the result to another slot) and rewrites its checksum and backup; save-dump prints what the slots hold;          replay-from-save writes a replay that powers on with the save in the cartridge, goes          through the title screen and Continue and then plays the script (a timeline file). The          fields and the format are described in docs/re/data/save-format.md."
    );
    eprintln!("A command line not listed here exits {EXIT_USAGE}.");
    ExitCode::from(EXIT_USAGE)
}

fn main() -> anyhow::Result<ExitCode> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        ["publish-check"] => publish_check(false),
        ["publish-check", "--allow-no-rom"] => publish_check(true),
        ["make-replays"] => make_replays(),
        ["verify-replays", rest @ ..] => verify_replays(rest),
        ["hd-parity", rest @ ..] => hd_parity(rest),
        ["hd-wide", rest @ ..] => hd_wide(rest),
        ["gfx-log", rest @ ..] => gfx_log(rest),
        ["hd-convert"] => hd_convert(),
        ["hd-art", rest @ ..] => hd_art(rest),
        ["hd-import", rest @ ..] => hd_import(rest),
        ["export-public", rest @ ..] => export_public(rest),
        ["hd-pack-public", rest @ ..] => hd_pack_public(rest),
        ["hd-pack-to-store", rest @ ..] => hd_pack_to_store(rest),
        ["hd-coverage", rest @ ..] => hd_coverage(rest),
        ["hd-tour", rest @ ..] => hd_tour(rest),
        ["hd-maps", rest @ ..] => hd_maps(rest),
        ["bench-hd", rest @ ..] => bench_hd(rest),
        ["bench-app", rest @ ..] => bench_app(rest),
        ["hd-skip-check", rest @ ..] => hd_skip_check(rest),
        ["hd-pack", rest @ ..] => hd_pack(rest),
        ["rebase-replays", rest @ ..] => rebase_replays(rest),
        ["menu-test", rest @ ..] => menu_test(rest),
        ["cheat-spike", dir] => cheat_spike(dir),
        ["coverage", rest @ ..] => coverage(rest),
        ["repro", rest @ ..] => repro(rest),
        ["export-mlb", rest @ ..] => export_mlb(rest),
        ["disasm", rest @ ..] => disasm(rest),
        ["routines", rest @ ..] => routines(rest),
        ["probe", rest @ ..] => probe(rest),
        ["shots", rest @ ..] => shots(rest),
        ["catalogue", rest @ ..] => catalogue(rest),
        ["text-dump", rest @ ..] => text_dump(rest),
        ["scene-dump", rest @ ..] => scene_dump::dump(rest),
        ["scene-check", rest @ ..] => scene_dump::check(rest),
        ["save-edit", rest @ ..] => save_edit(rest),
        ["chapter-save", rest @ ..] => chapter_save(rest),
        ["save-dump", path] => {
            let game = cached_rom().ok().and_then(|rom| save::Game::from_rom(&rom).ok());
            save::dump(Path::new(path), game.as_ref(), &mut std::io::stdout())?;
            Ok(ExitCode::SUCCESS)
        }
        ["replay-from-save", rest @ ..] => replay_from_save(rest),
        ["recompile"] => recompile(),
        #[cfg(feature = "histogram")]
        ["causes", replay, rest @ ..] => {
            let root = repo_root()?;
            let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
            let rom = cached_rom()?;
            causes::causes(
                &rom,
                &symbols,
                &root,
                Path::new(replay),
                rest.contains(&"--entries"),
                &mut std::io::stdout(),
            )?;
            Ok(ExitCode::SUCCESS)
        }
        ["framecheck", rest @ ..] => framecheck(rest),
        ["standalone-diff", rest @ ..] => standalone_diff(rest),
        ["mod-build", rest @ ..] => mod_build(rest),
        ["mod-check", rest @ ..] => mod_check(rest),
        ["mod-tileset", rest @ ..] => mod_tileset_command(rest),
        ["art-derive", rest @ ..] => art_derive::command(rest),
        ["kaioken-art", rest @ ..] => kaioken_art::command(rest),
        #[cfg(feature = "recomp")]
        ["recomp-cover", out, replays @ ..] if !replays.is_empty() => {
            let root = repo_root()?;
            let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
            let rom = cached_rom()?;
            let replays: Vec<PathBuf> = replays.iter().map(PathBuf::from).collect();
            fuzz::cover(&rom, &symbols, Path::new(out), &replays, &mut std::io::stdout())?;
            Ok(ExitCode::SUCCESS)
        }
        #[cfg(feature = "recomp")]
        ["fuzz-recomp", "pervcall", dir, replay] => {
            let root = repo_root()?;
            let symbols = ssd_oracle::Symbols::load(root.join("re").join("symbols.toml"))?;
            let rom = cached_rom()?;
            fuzz::pervcall(
                &rom,
                &symbols,
                Path::new(dir),
                Path::new(replay),
                &mut std::io::stdout(),
            )?;
            Ok(ExitCode::SUCCESS)
        }
        #[cfg(feature = "recomp")]
        ["fuzz-recomp", mode @ ("harvest" | "run"), rest @ ..] => match fuzz::parse_options(rest) {
            Ok((options, paths)) => {
                let rom = cached_rom()?;
                let out = &mut std::io::stdout();
                match (*mode, paths.as_slice()) {
                    ("harvest", [dir, replays @ ..]) if !replays.is_empty() => {
                        fuzz::harvest(&rom, &options, dir, replays, out)?;
                    }
                    ("run", [dir, tsv]) => fuzz::run(&rom, &options, dir, tsv, out)?,
                    _ => {
                        eprintln!("fuzz-recomp: harvest <dir> <replay>... | run <dir> <out.tsv>");
                        return Ok(usage());
                    }
                }
                Ok(ExitCode::SUCCESS)
            }
            Err(message) => {
                eprintln!("fuzz-recomp: {message}");
                Ok(usage())
            }
        },
        #[cfg(feature = "recomp")]
        ["fuzz-report", rest @ ..] => {
            fuzz::report(rest, &mut std::io::stdout())?;
            Ok(ExitCode::SUCCESS)
        }
        ["bench"] => bench(),
        ["bench", ..] => {
            eprintln!("bench: takes no arguments");
            Ok(usage())
        }
        _ => Ok(usage()),
    }
}
