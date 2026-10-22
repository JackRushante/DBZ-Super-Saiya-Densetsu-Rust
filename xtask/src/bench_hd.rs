//! `bench-hd`: how long the Enhanced picture takes to draw a frame. Each replay (default `battle`
//! and `wander`) is played with the PPU's render log on, and every frame is drawn with
//! `ssd_hd::Renderer::render_hd` at each configuration; only the drawing is timed, not the
//! emulation. Reports the mean, the 99th percentile and the worst frame per replay and
//! configuration, and over all the replays together. The last two configurations are the 16:9
//! view at 4x with the look alone and with the pack's art (`ssd_hd::world::WideRun::note_hd_shared`),
//! the picture of the Enhanced mode at 16:9. Those share their pictures as the window does: a frame whose
//! inputs are those of the picture before is not drawn again, and `reused` counts such frames. `--check`
//! compares every shared picture with one drawn fresh (by a run that draws every frame).

use crate::shared::{read_replay, render_table};
use anyhow::Context;
use ssd_hd::pack::Pack;
use ssd_hd::world::{SharedHd, WideRun};
use ssd_hd::{HdOptions, Look, Renderer};
use ssd_oracle::NullAudio;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What a configuration draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// The 4:3 picture of `Renderer::render_hd`.
    Square,
    /// The 16:9 view, with the look alone.
    Wide,
    /// The 16:9 view with the pack's art.
    WideArt,
}

/// The configurations timed: the 1x render (the frame of `hd-parity`), then 2x and 4x with the
/// crisp look (the default), 4x with the smooth one, and the 16:9 view at 4x without and with art.
const CONFIGURATIONS: [(Kind, u32, Look); 6] = [
    (Kind::Square, 1, Look::Crisp),
    (Kind::Square, 2, Look::Crisp),
    (Kind::Square, 4, Look::Crisp),
    (Kind::Square, 4, Look::Smooth),
    (Kind::Wide, 4, Look::Crisp),
    (Kind::WideArt, 4, Look::Crisp),
];

/// The command line of `bench-hd`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    /// Replay files, or directories of them; empty: `battle` and `wander` of `.work/replays`.
    pub paths: Vec<PathBuf>,
    /// Stop each replay after this many frames.
    pub frames: Option<u64>,
    /// The pack folder of the art (default: the converter's under the cache directory).
    pub pack: Option<PathBuf>,
    /// Compare every picture of the 16:9 configurations that is shared (not drawn again) with a fresh one.
    pub check: bool,
}

/// Parses the arguments after `bench-hd`: `[--frames N] [--pack DIR] [replay|dir...]`.
///
/// # Errors
///
/// An unknown option, or `--frames` without a number.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut parsed = Args { paths: Vec::new(), frames: None, pack: None, check: false };
    let mut args = args.iter();
    while let Some(&arg) = args.next() {
        match arg {
            "--frames" => {
                parsed.frames = Some(
                    args.next()
                        .and_then(|value| value.parse::<u64>().ok())
                        .filter(|&n| n > 0)
                        .ok_or_else(|| "--frames needs a number".to_owned())?,
                );
            }
            "--check" => parsed.check = true,
            "--pack" => {
                parsed.pack = Some(PathBuf::from(
                    args.next().ok_or_else(|| "--pack takes a folder".to_owned())?,
                ));
            }
            option if option.starts_with('-') => return Err(format!("unknown option `{option}`")),
            path => parsed.paths.push(PathBuf::from(path)),
        }
    }
    Ok(parsed)
}

/// The draw times of one replay, per configuration, and for the 16:9 ones how many frames showed the picture
/// of the frame before (not drawn again), and with `--check` how many shared pictures were compared with a
/// fresh draw and how many differed.
struct Timings {
    name: String,
    frames: Vec<Vec<Duration>>,
    reused: Vec<u64>,
    checked: Vec<u64>,
    differing: Vec<u64>,
}

/// The mean, the 99th percentile (nearest rank) and the worst of `times`, in milliseconds.
fn summary(times: &[Duration]) -> (f64, f64, f64) {
    if times.is_empty() {
        return (0.0, 0.0, 0.0);
    }
    let mut sorted: Vec<f64> = times.iter().map(|t| t.as_secs_f64() * 1000.0).collect();
    sorted.sort_by(f64::total_cmp);
    let mean = sorted.iter().sum::<f64>() / sorted.len() as f64;
    let rank = (sorted.len() * 99).div_ceil(100).max(1);
    (mean, sorted[rank - 1], sorted[sorted.len() - 1])
}

fn label((kind, scale, look): (Kind, u32, Look)) -> (String, &'static str) {
    let look = match (kind, look) {
        (Kind::Wide, _) => "16:9",
        (Kind::WideArt, _) => "16:9 art",
        (_, Look::Crisp) => "crisp",
        (_, Look::Smooth) => "smooth",
    };
    (format!("{scale}x"), look)
}

/// Plays the replays and writes the report.
///
/// # Errors
///
/// A replay or the pack cannot be read or played, or a frame is refused by the renderer.
pub fn run(
    rom: &[u8],
    default_dir: &Path,
    pack_dir: &Path,
    args: &Args,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let paths = if args.paths.is_empty() {
        vec![default_dir.join("battle.ssdreplay"), default_dir.join("wander.ssdreplay")]
    } else {
        args.paths.clone()
    };
    let art = {
        let dir = args.pack.clone().unwrap_or_else(|| pack_dir.to_path_buf());
        Arc::new(Pack::open(&dir).with_context(|| format!("opening the pack {}", dir.display()))?)
    };
    let mut replays = Vec::new();
    for path in &paths {
        let replay = read_replay(path, rom)?;
        let frames = replay.frames().min(args.frames.unwrap_or(u64::MAX));
        let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let mut machine = replay.power_on(rom.to_vec())?;
        machine.set_line_log(true);
        let mut renderer = Renderer::new();
        // The wide runs share their pictures as the window does (`note_hd_shared`): a frame whose inputs are
        // those of the picture before is not drawn again
        let mut plain = WideRun::new(rom);
        let mut with_art = WideRun::new(rom);
        with_art.set_art(Some(Arc::clone(&art)));
        // With `--check`: runs that draw every frame, the reference the shared pictures are compared with
        let mut fresh_plain = WideRun::new(rom);
        let mut fresh_art = WideRun::new(rom);
        fresh_art.set_art(Some(Arc::clone(&art)));
        // ... and transfer each colour as it is drawn, the way the art was drawn before the transfer was kept
        fresh_art.set_transfer_cache(false);
        let configurations = CONFIGURATIONS.len();
        let mut timings = Timings {
            name,
            frames: vec![Vec::new(); configurations],
            reused: vec![0; configurations],
            checked: vec![0; configurations],
            differing: vec![0; configurations],
        };
        let mut last: Vec<Option<u64>> = vec![None; configurations];
        while machine.frame_count() < frames {
            machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
            let log = machine.frame_log();
            let mut shown: Vec<Option<SharedHd>> = vec![None; configurations];
            for (slot, &(kind, scale, look)) in CONFIGURATIONS.iter().enumerate() {
                let started = Instant::now();
                match kind {
                    Kind::Square => {
                        let Some(log) = log else { continue };
                        let picture = renderer.render_hd(log, HdOptions { scale, look })?;
                        std::hint::black_box(&picture);
                    }
                    Kind::Wide | Kind::WideArt => {
                        // The wide runs follow the work RAM frame by frame, logged or not
                        let run = if kind == Kind::Wide { &mut plain } else { &mut with_art };
                        let picture =
                            run.note_hd_shared(machine.wram(), log, HdOptions { scale, look });
                        if log.is_none() {
                            continue;
                        }
                        std::hint::black_box(&picture);
                        shown[slot] = picture;
                    }
                }
                timings.frames[slot].push(started.elapsed());
                if let Some(picture) = &shown[slot] {
                    timings.reused[slot] += u64::from(last[slot] == Some(picture.serial));
                    last[slot] = Some(picture.serial);
                }
            }
            // Outside the timing: the fresh runs follow every frame too (a frame with no log is no picture)
            if args.check {
                for (slot, &(kind, scale, look)) in CONFIGURATIONS.iter().enumerate() {
                    if !matches!(kind, Kind::Wide | Kind::WideArt) {
                        continue;
                    }
                    let run = if kind == Kind::Wide { &mut fresh_plain } else { &mut fresh_art };
                    let fresh = run.note_hd(machine.wram(), log, HdOptions { scale, look });
                    let shared = shown[slot].as_ref().map(|picture| picture.picture.as_ref());
                    timings.checked[slot] += 1;
                    timings.differing[slot] += u64::from(fresh.as_ref() != shared);
                }
            }
        }
        replays.push(timings);
    }

    writeln!(
        out,
        "bench-hd: {} replays ({}), each frame drawn at every configuration; only the drawing is timed",
        replays.len(),
        replays.iter().map(|r| r.name.as_str()).collect::<Vec<_>>().join(", ")
    )?;
    // The frames that showed the picture of the frame before: the 16:9 configurations only ("-" for the others)
    let header = ["replay", "scale", "look", "frames", "reused", "mean ms", "p99 ms", "worst ms"];
    let mut rows = Vec::new();
    for timings in &replays {
        for (slot, &configuration) in CONFIGURATIONS.iter().enumerate() {
            let (mean, p99, worst) = summary(&timings.frames[slot]);
            let (scale, look) = label(configuration);
            rows.push(vec![
                timings.name.clone(),
                scale,
                look.to_owned(),
                timings.frames[slot].len().to_string(),
                reused_text(configuration.0, timings.reused[slot]),
                format!("{mean:.2}"),
                format!("{p99:.2}"),
                format!("{worst:.2}"),
            ]);
        }
    }
    for (slot, &configuration) in CONFIGURATIONS.iter().enumerate() {
        let all: Vec<Duration> =
            replays.iter().flat_map(|timings| timings.frames[slot].iter().copied()).collect();
        let reused: u64 = replays.iter().map(|timings| timings.reused[slot]).sum();
        let (mean, p99, worst) = summary(&all);
        let (scale, look) = label(configuration);
        rows.push(vec![
            "all".to_owned(),
            scale,
            look.to_owned(),
            all.len().to_string(),
            reused_text(configuration.0, reused),
            format!("{mean:.2}"),
            format!("{p99:.2}"),
            format!("{worst:.2}"),
        ]);
    }
    write!(out, "{}", render_table(&header, &rows))?;
    if args.check {
        let mut differing = 0;
        for timings in &replays {
            for (slot, &(kind, scale, look)) in CONFIGURATIONS.iter().enumerate() {
                if matches!(kind, Kind::Wide | Kind::WideArt) {
                    let (scale, look) = label((kind, scale, look));
                    writeln!(
                        out,
                        "check {} {scale} {look}: {} shared pictures compared with a fresh draw, {} differ",
                        timings.name, timings.checked[slot], timings.differing[slot]
                    )?;
                    differing += timings.differing[slot];
                }
            }
        }
        anyhow::ensure!(differing == 0, "{differing} shared pictures differ from a fresh draw");
        writeln!(out, "check: every shared picture equals a fresh draw")?;
    }
    Ok(())
}

/// The frames of a configuration that showed the picture before: a count for the 16:9 ones, `-` otherwise.
fn reused_text(kind: Kind, reused: u64) -> String {
    match kind {
        Kind::Wide | Kind::WideArt => reused.to_string(),
        Kind::Square => "-".to_owned(),
    }
}

/// The batch sizes `hd-skip-check` tries.
const BATCHES: [u64; 5] = [2, 4, 7, 8, 16];

/// `hd-skip-check`: the picture of a fast-forward's last frame is the same drawn after the frames
/// before it were skipped as drawn with every frame drawn, the way the app runs them above speed 1.
/// Each replay (default `battle` and `wander`) is played once as the reference, with the render log
/// on and every frame drawn ([`WideRun::note_hd`]). For each batch size a second machine plays it
/// as the app's batches do: the frames before the last of a batch run without drawing the PPU's
/// pixels, without the render log and without resampling their sound, and their 16:9 view is only
/// followed ([`WideRun::skip`]). At the end of every batch the picture of that run
/// ([`WideRun::redraw_hd`], from its own log) must equal the reference's pixel for pixel, so must
/// the PPU's own frame, and the work RAM, the CPU's registers and the master cycles of the two
/// machines must be the same. Returns whether everything was equal.
///
/// # Errors
///
/// A replay or the pack cannot be read or played.
pub fn skip_check(
    rom: &[u8],
    default_dir: &Path,
    pack_dir: &Path,
    args: &Args,
    out: &mut dyn Write,
) -> anyhow::Result<bool> {
    let paths = if args.paths.is_empty() {
        vec![default_dir.join("battle.ssdreplay"), default_dir.join("wander.ssdreplay")]
    } else {
        args.paths.clone()
    };
    let dir = args.pack.clone().unwrap_or_else(|| pack_dir.to_path_buf());
    let art =
        Arc::new(Pack::open(&dir).with_context(|| format!("opening the pack {}", dir.display()))?);
    let options = HdOptions { scale: 4, look: Look::Crisp };
    let new_run = || {
        let mut run = WideRun::new(rom);
        run.set_art(Some(Arc::clone(&art)));
        run
    };
    let mut ok = true;
    for path in &paths {
        let replay = read_replay(path, rom)?;
        let frames = replay.frames().min(args.frames.unwrap_or(u64::MAX));
        let name = path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        let mut machine = replay.power_on(rom.to_vec())?;
        machine.set_line_log(true);
        let mut reference = new_run();
        let mut skipping: Vec<WideRun> = BATCHES.iter().map(|_| new_run()).collect();
        let mut machines = Vec::new();
        for _ in BATCHES {
            let mut other = replay.power_on(rom.to_vec())?;
            other.set_line_log(true);
            machines.push(other);
        }
        let mut compared = vec![0_u64; BATCHES.len()];
        let mut differing = vec![0_u64; BATCHES.len()];
        let mut frames_differing = vec![0_u64; BATCHES.len()];
        let mut machines_differing = vec![0_u64; BATCHES.len()];
        while machine.frame_count() < frames {
            machine.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
            let log = machine.frame_log();
            let wanted = reference.note_hd(machine.wram(), log, options);
            let frame = machine.frame_count();
            for (slot, &batch) in BATCHES.iter().enumerate() {
                let other = &mut machines[slot];
                let shown = frame % batch == 0;
                other.set_compose_pixels(shown);
                other.set_line_log(shown);
                other.set_audio_resampling(frame % batch == 1 || batch == 1);
                other.run_frame_with(&mut |f| replay.input(f), &mut NullAudio)?;
                // The app follows the last two frames of a batch only
                if frame % batch == 0 || frame % batch == batch - 1 {
                    skipping[slot].skip(other.wram());
                }
                if shown {
                    let drawn = skipping[slot].redraw_hd(other.frame_log(), options);
                    compared[slot] += 1;
                    differing[slot] += u64::from(drawn != wanted);
                    frames_differing[slot] +=
                        u64::from(other.frame().pixels != machine.frame().pixels);
                    machines_differing[slot] += u64::from(
                        other.wram() != machine.wram()
                            || format!("{:?}", other.cpu_registers())
                                != format!("{:?}", machine.cpu_registers())
                            || other.total_master_cycles() != machine.total_master_cycles(),
                    );
                }
            }
        }
        for (slot, &batch) in BATCHES.iter().enumerate() {
            writeln!(
                out,
                "{name}: batches of {batch}: {} pictures compared, {} differ, {} PPU frames differ, {} machines differ",
                compared[slot], differing[slot], frames_differing[slot], machines_differing[slot]
            )?;
            ok &= differing[slot] == 0
                && frames_differing[slot] == 0
                && machines_differing[slot] == 0;
        }
    }
    writeln!(out, "hd-skip-check: {}", if ok { "everything equal" } else { "DIFFERENCES" })?;
    Ok(ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_takes_a_frame_limit_a_pack_and_replays() {
        let args = parse_args(&["--frames", "300", "a.ssdreplay"]).unwrap();
        assert_eq!(args.frames, Some(300));
        assert_eq!(args.paths, [PathBuf::from("a.ssdreplay")]);
        let plain = Args { paths: Vec::new(), frames: None, pack: None, check: false };
        assert_eq!(parse_args(&[]).unwrap(), plain);
        assert!(parse_args(&["--check"]).unwrap().check);
        assert_eq!(parse_args(&["--pack", "p"]).unwrap().pack, Some(PathBuf::from("p")));
        assert!(parse_args(&["--frames"]).unwrap_err().contains("needs a number"));
        assert!(parse_args(&["--fast"]).unwrap_err().contains("unknown option"));
    }

    #[test]
    fn the_percentile_is_the_nearest_rank_and_the_mean_is_the_average() {
        let times: Vec<Duration> = (1..=100).map(Duration::from_millis).collect();
        let (mean, p99, worst) = summary(&times);
        assert!((mean - 50.5).abs() < 1e-9);
        assert!((p99 - 99.0).abs() < 1e-9);
        assert!((worst - 100.0).abs() < 1e-9);
        assert_eq!(summary(&[]), (0.0, 0.0, 0.0));
    }
}
