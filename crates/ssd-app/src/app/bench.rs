//! `--bench-app`: the window's frame loop without the window. Each frame of the loop is the window's:
//! the emulated frames of the speed (the machine, the HD menu's driver, the Enhanced 16:9 picture),
//! the pictures, and the interface painted over them (the debug panel open, as the speed buttons
//! are in it) into an offscreen texture on the GPU, which is waited for as a swap chain with one
//! frame of latency would. It runs flat out, with no clock, and reports the emulated frames per
//! second at each speed, where the time of a window frame went (the mean, and the 50th, 95th and 99th percentile and the
//! worst of each stage), and how many cores were busy. `--bench-report FILE` also writes the report to a file (Android
//! has no stdout: its launch adds the file in the data folder).

use super::{
    Enhanced, Env, Pacer, Stages, WindowState, enhanced_session, fallback_picture, frame_layer,
    game_views, hd_layer, header_status, new_machine, note_wram, window_frames,
};
use crate::egui_input::InputAdapter;
use crate::hd::Hd;
use crate::hook_setup::{HookSetup, HookSwitch, HooksArg};
use crate::hotkeys::Speed;
use crate::panel::{Controls, HooksSwitch};
use crate::replay_io::Session;
use crate::snapshot::{Offscreen, PresentTimes};
use crate::ui::{self, View};
use crate::{audio, hd_art};
use anyhow::Context;
use ssd_game::Ruleset;
use ssd_hd::HdOptions;
use ssd_oracle::Joypad;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The default size of the window the bench draws: the one the window opens with.
pub const SIZE: (u32, u32) = (1440, 900);

/// The window frame that is on time at 60 Hz, in milliseconds.
const BUDGET_MS: f32 = 1000.0 / 60.0;

/// The stages of a window frame the percentiles are taken over, in the order of [`Sample`].
const STAGES: [&str; 10] =
    ["wall", "driver", "emulate", "hist", "hd", "status", "views", "ui", "paint", "wait"];

/// What `--bench-app` is asked for.
#[derive(Debug, Clone)]
pub struct Request {
    /// The replays played, each from power-on.
    pub replays: Vec<PathBuf>,
    /// The speeds run (1, 2, 4 or 8).
    pub speeds: Vec<u32>,
    /// The emulated frames timed at each speed (fewer when the replay ends first).
    pub frames: u64,
    /// The scale and look of the Enhanced picture (`--hd-scale`).
    pub enhanced: HdOptions,
    /// Whether the debug panel is open.
    pub panel: bool,
    /// The frames run before the timing starts: the art is loaded and the buffers are sized.
    pub warm_up: u64,
    /// Exactly `speed` frames in every window frame, as a window on a steady 60 Hz clock runs them, instead of the
    /// flat-out batches (which catch up and run up to 64 frames): the percentiles then are those of a window frame.
    pub lockstep: bool,
    /// Latency Low (the picture of each frame drawn and shown at once) instead of Smooth (drawn while the next frame
    /// runs).
    pub low_latency: bool,
    /// The size of the offscreen target.
    pub size: (u32, u32),
    /// A file the report is also written to (a line at a time, so a run that is killed leaves what it printed).
    pub report: Option<PathBuf>,
}

/// One window frame, in milliseconds per stage ([`STAGES`]).
type Sample = [f32; STAGES.len()];

/// The sums of one run's window frames.
#[derive(Debug, Default, Clone)]
struct Sums {
    /// Every window frame timed, for the percentiles.
    samples: Vec<Sample>,
    /// The emulated frames each window frame ran.
    ran: Vec<u32>,
    window_frames: u64,
    emulated: u64,
    wall: Duration,
    stages: Stages,
    status: Duration,
    views: Duration,
    present: PresentTimes,
}

/// The CPU time this process has used, user and kernel, over all its threads.
#[cfg(windows)]
fn process_cpu() -> Duration {
    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn GetProcessTimes(
            process: isize,
            creation: *mut FileTime,
            exit: *mut FileTime,
            kernel: *mut FileTime,
            user: *mut FileTime,
        ) -> i32;
    }
    let ticks = |time: FileTime| (u64::from(time.high) << 32) | u64::from(time.low);
    let (mut creation, mut exit, mut kernel, mut user) = Default::default();
    // SAFETY: the pseudo-handle of the current process is always valid, and the four times are
    // plain structs the call fills.
    let ok = unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &raw mut creation,
            &raw mut exit,
            &raw mut kernel,
            &raw mut user,
        )
    };
    if ok == 0 {
        return Duration::ZERO;
    }
    // 100 ns units
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

/// The CPU time this process has used, user and kernel, over all its threads (`/proc/self/stat`, in ticks of 10 ms).
#[cfg(any(target_os = "linux", target_os = "android"))]
fn process_cpu() -> Duration {
    let text = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    Duration::from_millis(stat_ticks(&text).unwrap_or(0) * 10)
}

/// The user and kernel ticks (fields 14 and 15) of a `/proc/<pid>/stat` line. The name (field 2) is in parentheses and
/// may hold spaces, so the fields are counted from the last parenthesis.
#[cfg(any(target_os = "linux", target_os = "android", test))]
fn stat_ticks(text: &str) -> Option<u64> {
    let rest = text.get(text.rfind(')')? + 1..)?;
    let mut fields = rest.split_whitespace().skip(11);
    let user: u64 = fields.next()?.parse().ok()?;
    let kernel: u64 = fields.next()?.parse().ok()?;
    Some(user + kernel)
}

#[cfg(not(any(windows, target_os = "linux", target_os = "android")))]
fn process_cpu() -> Duration {
    Duration::ZERO
}

/// Where the report goes: standard output and, when asked, a file.
struct Report {
    file: Option<std::fs::File>,
}

impl Report {
    fn new(path: Option<&Path>) -> Self {
        let file = path.and_then(|path| {
            std::fs::File::create(path)
                .inspect_err(|e| eprintln!("bench-app: cannot write {}: {e}", path.display()))
                .ok()
        });
        Self { file }
    }

    fn line(&mut self, text: &str) {
        println!("{text}");
        if let Some(file) = self.file.as_mut() {
            let _ = writeln!(file, "{text}");
            let _ = file.flush();
        }
    }
}

/// The value at quantile `q` (0 to 1) of `sorted`, ascending: the nearest rank.
fn percentile(sorted: &[f32], q: f64) -> f32 {
    match sorted.len() {
        0 => 0.0,
        n => sorted[(((n - 1) as f64) * q).round() as usize],
    }
}

fn millis(total: Duration, count: u64) -> f64 {
    total.as_secs_f64() * 1000.0 / count.max(1) as f64
}

/// Plays `replay` at `speed` through the window's frame loop and sums what it costs.
fn run_one(
    rom: &[u8],
    hooks: Option<&HookSetup>,
    switch: Option<&HookSwitch>,
    replay: &Path,
    speed: u32,
    request: &Request,
) -> anyhow::Result<(Sums, Duration)> {
    let ruleset = Ruleset::enhanced();
    let mut session = Session::new(Some(replay), None, rom)?;
    enhanced_session(&mut session, ruleset)?;
    let mut machine = new_machine(rom.to_vec(), hooks, &mut session)?;
    let mut state =
        WindowState::new(Controls::new(HooksArg::Original, HooksSwitch::Allowed, false, false));
    state.controls.speed = Speed::ALL
        .into_iter()
        .find(|s| s.multiplier() == speed)
        .with_context(|| format!("{speed} is not a speed: 1, 2, 4 or 8"))?;
    state.hd = hd_layer(rom, ruleset)?;
    if let Some(hd) = state.hd.as_mut() {
        hd.set_replaying();
    }
    state.panel.open = request.panel;
    state.scaled = Some(request.enhanced);
    state.video.smooth = !request.low_latency;
    machine.set_line_log(true);
    let mut enhanced = Enhanced::new(request.enhanced, rom);
    let env = Env { rom, switch, verify: None };
    let mut offscreen = Offscreen::new(request.size.0, request.size.1)?;
    let mut input = InputAdapter::default();
    let mut audio = audio::Audio::muted();
    let clock = Instant::now();
    // Flat out: the clock stays at the start, so the window is always behind and runs as many frames
    // as it can hold
    let mut pacer = Pacer::new(Duration::from_secs_f64(1.0 / machine.target_fps()));
    let mut sums = Sums::default();
    let mut cpu_start = None;
    loop {
        if session.playback_over(&machine) {
            break;
        }
        let frame_started = Instant::now();
        state.stages = Stages::default();
        let frames = if request.lockstep { speed } else { pacer.frames(speed, Instant::now()) };
        let ran = window_frames(
            &mut state,
            &mut machine,
            &mut session,
            &env,
            Joypad::default(),
            &mut audio,
            frames,
        )?;
        pacer.note_cost(ran, state.stages.frames());
        let status_started = Instant::now();
        note_wram(&mut state, &machine);
        let status = header_status(&machine, &state.history, 60.0 * speed as f32);
        let views_started = Instant::now();
        let status_time = views_started - status_started;
        let picture = fallback_picture(&state, &mut machine, Some(&mut enhanced), false);
        let registers = machine.cpu_registers();
        let controls = state.controls;
        let views = game_views(
            state.hd_view.as_ref(),
            state.view.as_ref(),
            &machine,
            picture.as_ref(),
            None,
            matches!(state.video.look, ssd_hd::Look::Smooth),
        );
        let scene = state.hd.as_ref().and_then(|hd| hd.scene(machine.wram()));
        let overlay = state.hd.as_ref().and_then(|hd| hd.overlay(machine.wram(), machine.sram()));
        let symbols = env.symbols();
        let history = &state.history;
        let panel = &mut state.panel;
        let mut hd_art = state.hd.as_mut().map(Hd::art);
        let mut actions = Vec::new();
        let shown: &ssd_oracle::Machine = &machine;
        let views_time = views_started.elapsed();
        let present =
            offscreen.present(&mut input, clock.elapsed().as_secs_f64(), &views, |ui, games| {
                let view = View {
                    status,
                    controls: &controls,
                    registers: &registers,
                    recording: false,
                    verify_labels: None,
                    machine: shown,
                    symbols,
                    history,
                    touch: None,
                };
                let layer = frame_layer(scene.as_ref(), overlay.as_ref(), hd_art.as_deref_mut());
                ui::draw(ui, &view, games, panel, &mut actions, layer);
            })?;
        let wall = frame_started.elapsed();
        log::debug!("window frame: {ran} frames run, at frame {}", machine.frame_count());
        if machine.frame_count() >= request.warm_up {
            cpu_start.get_or_insert_with(process_cpu);
            sums.window_frames += 1;
            let ms = |time: Duration| time.as_secs_f32() * 1000.0;
            sums.samples.push([
                ms(wall),
                ms(state.stages.driver),
                ms(state.stages.emulate),
                ms(state.stages.history),
                ms(state.stages.hd),
                ms(status_time),
                ms(views_time),
                ms(present.ui),
                ms(present.paint),
                ms(present.wait),
            ]);
            sums.ran.push(ran);
            sums.emulated += u64::from(ran);
            sums.wall += wall;
            sums.stages.driver += state.stages.driver;
            sums.stages.emulate += state.stages.emulate;
            sums.stages.history += state.stages.history;
            sums.stages.hd += state.stages.hd;
            sums.status += status_time;
            sums.views += views_time;
            sums.present.ui += present.ui;
            sums.present.paint += present.paint;
            sums.present.wait += present.wait;
            if sums.emulated >= request.frames {
                break;
            }
        }
    }
    let cpu = cpu_start.map_or(Duration::ZERO, |start| process_cpu().saturating_sub(start));
    Ok((sums, cpu))
}

/// A replay named relative to the data folder (the way the Android launch names them) when it is not found as given.
fn resolve(replay: &Path) -> PathBuf {
    if replay.is_relative() && !replay.exists() {
        let inside = crate::paths::data_dir().join(replay);
        if inside.exists() {
            return inside;
        }
    }
    replay.to_path_buf()
}

/// Runs the bench and prints its table (and writes it to the report file, if one is asked for).
///
/// # Errors
///
/// A replay cannot be played, or there is no GPU.
pub fn run(
    rom: &[u8],
    hooks: Option<&HookSetup>,
    switch: Option<&HookSwitch>,
    request: &Request,
) -> anyhow::Result<()> {
    let mut report = Report::new(request.report.as_deref());
    let result = run_all(&mut report, rom, hooks, switch, request);
    match &result {
        Ok(()) => report.line("bench-app: done"),
        Err(e) => report.line(&format!("bench-app: failed: {e:#}")),
    }
    result
}

fn run_all(
    report: &mut Report,
    rom: &[u8],
    hooks: Option<&HookSetup>,
    switch: Option<&HookSwitch>,
    request: &Request,
) -> anyhow::Result<()> {
    report.line(&format!(
        "bench-app: the window's frame loop, Enhanced {}x 16:9, art {}, panel {}, {} frames per run, {}x{} target, {}",
        request.enhanced.scale,
        if hd_art::pack(rom).is_some() { "on" } else { "off" },
        if request.panel { "open" } else { "closed" },
        request.frames,
        request.size.0,
        request.size.1,
        if request.lockstep { "lockstep (speed frames per window frame)" } else { "flat out" },
    ));
    report.line(&format!(
        "latency: {}",
        if request.low_latency {
            "low (the picture drawn and shown at once)"
        } else {
            "smooth (the picture drawn while the next frame runs)"
        }
    ));
    report.line(&format!(
        "gpu: {}",
        crate::snapshot::describe_gpu().unwrap_or_else(|e| format!("none ({e:#})"))
    ));
    report.line(
        "replay      speed  emu fps  holds   win ms  frames  driver emulate    hist      hd  status   views      ui   paint    wait  cores",
    );
    let mut runs = Vec::new();
    for replay in &request.replays {
        let name =
            replay.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned());
        for &speed in &request.speeds {
            let (sums, cpu) = run_one(rom, hooks, switch, &resolve(replay), speed, request)?;
            let seconds = sums.wall.as_secs_f64().max(1e-9);
            let fps = sums.emulated as f64 / seconds;
            let target = 60.0 * f64::from(speed);
            let n = sums.window_frames;
            report.line(&format!(
                "{name:<11} {speed:>4}x {fps:>8.0} {:>6} {:>8.2} {:>7.1} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>7.2} {:>6.2}",
                if fps >= target { "yes" } else { "NO" },
                millis(sums.wall, n),
                sums.emulated as f64 / n.max(1) as f64,
                millis(sums.stages.driver, n),
                millis(sums.stages.emulate, n),
                millis(sums.stages.history, n),
                millis(sums.stages.hd, n),
                millis(sums.status, n),
                millis(sums.views, n),
                millis(sums.present.ui, n),
                millis(sums.present.paint, n),
                millis(sums.present.wait, n),
                cpu.as_secs_f64() / seconds,
            ));
            runs.push((name.clone(), speed, sums));
        }
    }
    report.line(
        "(ms per window frame; holds: the flat-out rate reaches 60 x speed; cores: CPU time over wall time)",
    );
    report.line("");
    report.line(
        "per window frame, in ms: p50 / p95 / p99 / max of each stage; late: window frames over 16.7 ms",
    );
    report.line(&format!(
        "{:<11} {:>5}  {:<8} {:>7} {:>7} {:>7} {:>7}",
        "replay", "speed", "stage", "p50", "p95", "p99", "max"
    ));
    for (name, speed, sums) in &runs {
        for (index, stage) in STAGES.iter().enumerate() {
            let mut values: Vec<f32> = sums.samples.iter().map(|sample| sample[index]).collect();
            values.sort_by(f32::total_cmp);
            report.line(&format!(
                "{name:<11} {speed:>4}x  {stage:<8} {:>7.2} {:>7.2} {:>7.2} {:>7.2}",
                percentile(&values, 0.50),
                percentile(&values, 0.95),
                percentile(&values, 0.99),
                values.last().copied().unwrap_or(0.0),
            ));
        }
        let mut ran: Vec<f32> = sums.ran.iter().map(|&frames| frames as f32).collect();
        ran.sort_by(f32::total_cmp);
        let late = sums.samples.iter().filter(|sample| sample[0] > BUDGET_MS).count();
        report.line(&format!(
            "{name:<11} {speed:>4}x  frames per window frame: p50 {:.0}, p99 {:.0}, max {:.0}; late {late} of {} ({:.1}%)",
            percentile(&ran, 0.50),
            percentile(&ran, 0.99),
            ran.last().copied().unwrap_or(0.0),
            sums.samples.len(),
            100.0 * late as f64 / sums.samples.len().max(1) as f64,
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_percentile_is_the_nearest_rank() {
        let values: Vec<f32> = (1..=100).map(|n| n as f32).collect();
        assert_eq!(percentile(&values, 0.0), 1.0);
        assert_eq!(percentile(&values, 0.5), 51.0);
        assert_eq!(percentile(&values, 0.99), 99.0);
        assert_eq!(percentile(&values, 1.0), 100.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
        assert_eq!(percentile(&[7.0], 0.99), 7.0);
    }

    #[test]
    fn the_cpu_ticks_come_from_after_the_name_in_parentheses() {
        let line = "4242 (a b) c) S 1 4242 4242 0 -1 4194560 100 0 0 0 150 25 0 0 20 0 8 0 1000 2000 300 18446744073709551615";
        assert_eq!(stat_ticks(line), Some(175));
        assert_eq!(stat_ticks("garbage"), None);
    }
}
