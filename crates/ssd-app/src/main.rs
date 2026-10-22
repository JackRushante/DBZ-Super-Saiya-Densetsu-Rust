mod app;
mod audio;
mod canvas;
mod chapter;
mod debugger;
mod egui_input;
mod filter;
mod hd;
mod hd_art;
mod hd_install;
mod hook_setup;
mod hotkeys;
mod input;
mod lang;
mod language_pick;
mod layout;
mod options;
mod packs;
mod panel;
mod paths;
mod perf_hint;
mod replay_io;
mod replays;
mod rom_setup;
mod saves;
mod screenshot;
mod setup_ui;
mod snapshot;
mod tab_debugger;
mod tab_memory;
mod tab_replays;
mod tab_routines;
#[cfg(test)]
mod test_support;
mod theme;
mod touch;
mod ui;
mod ui_script;
mod video;

use clap::error::ErrorKind;
use clap::{CommandFactory, Parser};
use hook_setup::{HookSetup, HookSwitch, HooksArg};
use ssd_game::Ruleset;
use ssd_hd::{HdOptions, Look};
use ssd_menus::options::Values;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// No ROM in the data directory and none given.
const EXIT_ROM_MISSING: u8 = 2;
/// The ROM is not one of the accepted images (or the cached copy is damaged).
const EXIT_ROM_REJECTED: u8 = 3;
/// The symbols file cannot be read, is invalid, or names a routine without a Rust function.
const EXIT_SYMBOLS: u8 = 4;
/// A verify run found divergences between the Rust routines and the original.
const EXIT_DIVERGENCES: u8 = 5;
/// `--install-hd-pack` named a pack that cannot be installed.
const EXIT_HD_PACK: u8 = 7;
/// The replay cannot be played or recorded as asked: unreadable, not a replay, for another ROM or
/// emulator version, an initial SRAM the cartridge does not take, or `--record` naming it.
const EXIT_REPLAY: u8 = 6;
/// A data pack cannot be read, or the set of packs is refused by its checks (`--mods`).
const EXIT_PACKS: u8 = 8;

#[derive(Parser)]
#[command(name = "ssd-recompiled", about = "Dragon Ball Z: Super Saiya Densetsu Recompiled")]
struct Cli {
    /// ROM to identify, normalise and cache (needed on first launch).
    #[arg(long)]
    rom: Option<PathBuf>,
    /// Klepto v1.02 IPS patch, required with the Japanese Rev 1 ROM.
    #[arg(long, requires = "rom")]
    ips: Option<PathBuf>,
    /// Run N frames without a window, then exit (testing).
    #[arg(long)]
    headless_frames: Option<u64>,
    /// With --headless-frames: write the last frame as a BMP here.
    #[arg(long, requires = "headless_frames")]
    screenshot: Option<PathBuf>,
    /// With --headless-frames: draw the debug panel offscreen (no window) after the script's
    /// actions, and write it as a PNG here. See the README, "UI snapshots".
    #[arg(long, requires = "headless_frames", conflicts_with = "screenshot")]
    ui_snapshot: Option<PathBuf>,
    /// With --ui-snapshot: the actions to apply before the picture, one per line.
    #[arg(long, requires = "ui_snapshot")]
    ui_script: Option<PathBuf>,
    /// With --ui-snapshot: the picture's size in pixels, `WIDTHxHEIGHT` [default: 1440x900].
    #[arg(long, value_parser = parse_size, requires = "ui_snapshot")]
    ui_size: Option<(u32, u32)>,
    /// Take a ROM that is none of the known images as it is: the copier header is stripped and the
    /// game runs on it, but every part built from the canonical image (the catalogue of names and
    /// texts, the HD art, the recompiled code) is not guaranteed to line up. Also lets an HD pack
    /// made for another ROM be installed, when only the text of the ROM differs.
    #[arg(long)]
    force_rom: bool,
    /// Enhanced graphics: the picture drawn again at this many times the PPU's resolution (2 to 8),
    /// from the state the PPU drew it from. Without it the Classic picture is shown. In a window or
    /// with --ui-snapshot.
    #[arg(long, value_parser = parse_hd_scale)]
    hd_scale: Option<u32>,
    /// The HD pack folder to draw art from (made by `xtask hd-convert` and `hd-import`) [default:
    /// the newest pack of the ROM under the cache directory]. Used with --hd-scale and in the HD
    /// menus.
    #[arg(long)]
    hd_pack: Option<PathBuf>,
    /// Install this shared HD art pack (the `hd-pack.zip` file, or its folder) for your ROM before
    /// starting: the game builds its HD data from the ROM and the art, once. Without it, an
    /// `hd-pack.zip` or `hd-pack` folder next to the game is installed on the first Enhanced launch.
    #[arg(long)]
    install_hd_pack: Option<PathBuf>,
    /// Draw no HD art: the Enhanced picture is the look alone.
    #[arg(long)]
    no_hd_art: bool,
    /// Draw no HD text: the game's own letters, made bigger by the look.
    #[arg(long)]
    no_hd_font: bool,
    /// The look of the Enhanced picture: `crisp` (pixel art made bigger, the diagonals smoothed) or
    /// `smooth` [default: crisp]. Needs --hd-scale.
    #[arg(long, value_enum, requires = "hd_scale")]
    enhanced_look: Option<LookArg>,
    /// Which routines run in Rust: none (the original game code), all of them (rust), or all of
    /// them checked against the original code, which stays in charge (verify).
    #[arg(long, value_enum, default_value_t = HooksArg::Original)]
    hooks: HooksArg,
    /// Symbols file naming the routines to hook [default: re/symbols.toml of the workspace, else
    /// symbols.toml next to the executable].
    #[arg(long)]
    symbols: Option<PathBuf>,
    /// Where --hooks verify saves divergence reports [default: .work/divergences].
    #[arg(long)]
    divergence_dir: Option<PathBuf>,
    /// Record the session as a replay to this file: every frame's joypad from power-on and the
    /// SRAM at power-on. Needs --hooks original: a hooked call can run on past a frame with
    /// that frame's joypad, which a replay would not reproduce.
    #[arg(long)]
    record: Option<PathBuf>,
    /// Play this replay from power-on, its initial SRAM in memory (the save file is neither read
    /// nor written). A headless run stops at its end; a windowed one runs on with all buttons
    /// released. With --record, records what is played. Loading a state is disabled while
    /// recording or playing.
    #[arg(long)]
    replay: Option<PathBuf>,
    /// Run on these data packs, Enhanced mode only: each a folder with `pack.toml` or a `.ssdpack` zip,
    /// the option repeated for each pack. The set is checked against the ROM first; a pack that cannot be
    /// read or a set that is refused prints why and ends the run. A replay recorded with packs plays only
    /// with the same packs, and one recorded without them only without.
    #[arg(long, value_name = "PATH", conflicts_with = "no_mods")]
    mods: Vec<PathBuf>,
    /// Run on the ROM alone, with no data packs: the default, present so a launcher can say so.
    #[arg(long)]
    no_mods: bool,
    /// Start the chapter `ID` of the data packs (Enhanced mode only, with --mods): the game starts from the chapter's own save,
    /// in its own folder under `saves/chapters/`, and the main saves are never touched. A folder with a save already continues it.
    #[arg(long, value_name = "ID", conflicts_with = "no_mods")]
    chapter: Option<String>,
    /// Time the window's frame loop without a window (the Enhanced picture at --hd-scale, 4 by
    /// default, and the debug panel, drawn offscreen) over these replays, each at the speeds of
    /// --bench-speeds, flat out; prints the emulated frames per second and where the time went.
    #[arg(long, num_args = 1.., value_name = "REPLAY", conflicts_with_all = ["headless_frames", "replay", "record"])]
    bench_app: Option<Vec<PathBuf>>,
    /// With --bench-app: the speeds run, comma separated [default: 1,2,4,8].
    #[arg(long, requires = "bench_app", value_delimiter = ',', default_values_t = [1_u32, 2, 4, 8])]
    bench_speeds: Vec<u32>,
    /// With --bench-app: the emulated frames timed at each speed [default: 1200].
    #[arg(long, requires = "bench_app", default_value_t = 1200)]
    bench_frames: u64,
    /// With --bench-app: the frames run before the timing starts [default: 120].
    #[arg(long, requires = "bench_app", default_value_t = 120)]
    bench_warm_up: u64,
    /// With --bench-app: draw with the debug panel closed.
    #[arg(long, requires = "bench_app")]
    bench_panel_closed: bool,
    /// The rules and visuals: `classic` (the original, the reference of the replays) or `enhanced`
    /// (the HD menus in the window and in UI snapshots) [default: classic].
    #[arg(long, value_enum, default_value_t = ModeArg::Classic)]
    mode: ModeArg,
    /// Draw the touch controls (always on in the Android app): the buttons over the game, fed by touch events. With
    /// --ui-snapshot the script's `touch` lines set what they show.
    #[arg(long)]
    touch_ui: bool,
    /// The language of the game's text in Enhanced mode: `en`, or `zh-hk` (Traditional Chinese for Hong Kong, drawn
    /// from `zh-hk.tsv` and a Noto Sans HK font in the data folder) [default: the Options' choice, else English]. For
    /// this run only: the Options' choice is not changed.
    #[arg(long, value_enum)]
    lang: Option<LangArg>,
}

/// `--lang`: the languages of the game's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum LangArg {
    En,
    #[value(name = "zh-hk")]
    ZhHk,
}

impl From<LangArg> for ssd_menus::Language {
    fn from(lang: LangArg) -> Self {
        match lang {
            LangArg::En => Self::English,
            LangArg::ZhHk => Self::ZhHk,
        }
    }
}

/// `--mode`: the two modes of the game (spec §5.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum ModeArg {
    Classic,
    Enhanced,
}

impl ModeArg {
    fn ruleset(self) -> Ruleset {
        match self {
            Self::Classic => Ruleset::classic(),
            Self::Enhanced => Ruleset::enhanced(),
        }
    }
}

pub(crate) const TITLE: &str = "Dragon Ball Z: Super Saiya Densetsu Recompiled";

fn main() -> ExitCode {
    run(std::env::args_os().collect())
}

/// The whole program for the arguments `args` (the first is the program's name). The Android library
/// (`crates/ssd-android`) calls this with the arguments it reads, since a shared library has no `argv`.
pub fn run(args: Vec<std::ffi::OsString>) -> ExitCode {
    // `try_init`: the Android library installs its own logger first.
    let _ = env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init();
    let cli = Cli::parse_from(args);
    if cli.record.is_some() && cli.hooks != HooksArg::Original {
        Cli::command()
            .error(
                clap::error::ErrorKind::ArgumentConflict,
                "--record needs --hooks original: a hooked call can run on past a frame with that \
                 frame's joypad, which a replay would not reproduce",
            )
            .exit();
    }
    if !cli.mods.is_empty() && cli.mode != ModeArg::Enhanced {
        Cli::command()
            .error(
                ErrorKind::ArgumentConflict,
                "--mods needs --mode enhanced: data packs run in the Enhanced mode only",
            )
            .exit();
    }
    if cli.chapter.is_some() && cli.mode != ModeArg::Enhanced {
        Cli::command()
            .error(
                ErrorKind::ArgumentConflict,
                "--chapter needs --mode enhanced: a chapter is part of a data pack, and packs run in the Enhanced mode \
                 only",
            )
            .exit();
    }
    // Before the ROM, like the symbols: a bad script is a mistake of the command line too.
    let script = cli.ui_script.as_deref().map_or_else(Vec::new, read_script);
    // Before the ROM: a bad symbols file is a mistake of the command line, whatever the data dir.
    let hooks =
        match hook_setup::prepare(cli.hooks, cli.symbols.as_deref(), cli.divergence_dir.as_deref())
        {
            Ok(hooks) => hooks,
            Err(e) => {
                eprintln!("symbols error: {e:#}");
                return ExitCode::from(EXIT_SYMBOLS);
            }
        };
    let rom = match rom_setup::resolve_rom(cli.rom.as_deref(), cli.ips.as_deref(), cli.force_rom) {
        Ok(rom_setup::RomSetup::Ready(bytes)) => bytes,
        Ok(rom_setup::RomSetup::Missing) => {
            let msg = "No ROM yet. Start once with:\n\nssd-recompiled --rom \"path\\to\\rom.sfc\"\n\n\
                       Accepted: Klepto v1.02 English translation, or the Japanese Rev 1 ROM with --ips dbz102.ips.";
            eprintln!("{msg}");
            if cli.headless_frames.is_none() {
                let _ = sdl3::messagebox::show_simple_message_box(
                    sdl3::messagebox::MessageBoxFlag::INFORMATION,
                    TITLE,
                    msg,
                    None,
                );
            }
            return ExitCode::from(EXIT_ROM_MISSING);
        }
        Err(e) => {
            eprintln!("ROM rejected: {e:#}");
            return ExitCode::from(EXIT_ROM_REJECTED);
        }
    };
    let mut session =
        match replay_io::Session::new(cli.replay.as_deref(), cli.record.as_deref(), &rom) {
            Ok(session) => session,
            Err(e) => return error_exit(&e),
        };
    let loaded = match load_packs(&cli, &rom) {
        Ok(loaded) => loaded,
        Err(code) => return code,
    };
    if let Some(id) = cli.chapter.as_deref()
        && loaded.image.is_none()
    {
        // A set of packs that is not used (see `load_packs`) has no chapter: the start is an error, not a silent start.
        let why =
            loaded.notice.as_deref().map_or_else(String::new, |notice| format!(" ({notice})"));
        eprintln!("chapter: no chapter `{id}` in the data packs of this run{why}");
        return ExitCode::from(EXIT_PACKS);
    }
    // The Data packs screen's list: the packs of the folder and of the run's set, with the reason the set is not used, if it is not.
    if cli.mode == ModeArg::Enhanced {
        let playing =
            cli.chapter.as_deref().and_then(|id| packs::chapter_name(id, &loaded.applied));
        packs::configure(packs::build(&rom, &loaded.applied, loaded.notice, playing), &rom);
    }
    let image = loaded.image.as_ref().map(|image| image.bytes.clone());
    let overlays = loaded.overlays;
    session.set_packs(loaded.image);
    // The HD pack of the ROM, built from the art pack beside the game when the HD picture or the HD
    // menus will draw it (Classic never does)
    let need = if cli.install_hd_pack.is_some() {
        hd_install::Need::Asked
    } else if !cli.no_hd_art
        && cli.hd_pack.is_none()
        && (cli.hd_scale.is_some() || cli.mode == ModeArg::Enhanced)
    {
        hd_install::Need::Wanted
    } else {
        hd_install::Need::No
    };
    if let Err(hd_install::Failed(message)) =
        hd_install::ensure(&rom, need, cli.install_hd_pack.as_deref(), cli.force_rom)
    {
        eprintln!("HD pack: {message}");
        return ExitCode::from(EXIT_HD_PACK);
    }
    hd_art::configure(hd_art::Config {
        pack: cli.hd_pack.clone(),
        off: cli.no_hd_art,
        font_off: cli.no_hd_font,
        overlays,
        image,
    });
    lang::configure(cli.lang.map(ssd_menus::Language::from));
    // Only a windowed run writes the player's files (see `persists`): set before the first read.
    paths::set_persistent(persists(&cli));
    // The chapter's fresh files: a run that persists writes them, any other keeps them in memory (see `chapter`).
    if let Err(error) = chapter::write_fresh() {
        eprintln!("chapter: {error:#}");
        return ExitCode::from(EXIT_PACKS);
    }
    touch::force(cli.touch_ui);
    // A windowed Enhanced run takes the scale and the look of the options where no flag gives them; headless
    // runs, snapshots and benchmarks ignore the saved file, so they do not depend on this PC's choices.
    let saved_video =
        cli.mode == ModeArg::Enhanced && cli.headless_frames.is_none() && cli.bench_app.is_none();
    let saved =
        if saved_video { options::read_settings().unwrap_or_default() } else { Values::new() };
    let video_start =
        options::video::start(&saved, cli.hd_scale, cli.enhanced_look.map(Look::from));
    let enhanced = if saved_video {
        Some(video_start.shown.hd())
    } else {
        cli.hd_scale.map(|scale| HdOptions {
            scale,
            look: cli.enhanced_look.map_or_else(Look::default, Look::from),
        })
    };
    if cli.hd_scale.is_some() && cli.headless_frames.is_some() && cli.ui_snapshot.is_none() {
        Cli::command()
            .error(ErrorKind::ArgumentConflict, "--hd-scale needs a window or --ui-snapshot")
            .exit()
    }
    if let Some(replays) = &cli.bench_app {
        let request = app::bench::Request {
            replays: replays.clone(),
            speeds: cli.bench_speeds.clone(),
            frames: cli.bench_frames,
            enhanced: enhanced.unwrap_or(HdOptions { scale: 4, look: Look::default() }),
            panel: !cli.bench_panel_closed,
            warm_up: cli.bench_warm_up,
        };
        let switch = load_switch(&cli);
        return match app::bench::run(&rom, hooks.as_ref(), switch.as_ref(), &request) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => error_exit(&e),
        };
    }
    let result = match (cli.headless_frames, cli.ui_snapshot.as_deref()) {
        (Some(frames), None) => {
            app::run_headless(rom, frames, cli.screenshot.as_deref(), hooks.as_ref(), session)
        }
        (Some(frames), Some(out)) => {
            let request = snapshot::Request {
                frames,
                out,
                size: cli.ui_size.unwrap_or(snapshot::DEFAULT_SIZE),
                script: &script,
                touch: cli.touch_ui,
                enhanced,
            };
            let switch = load_switch(&cli);
            app::run_ui_snapshot(
                rom,
                &request,
                hooks.as_ref(),
                switch.as_ref(),
                cli.hooks,
                cli.mode.ruleset(),
                session,
            )
        }
        (None, _) => {
            // The panel can change the hook mode of a running machine; that needs the symbols,
            // whatever mode the run starts in.
            let switch = load_switch(&cli);
            app::run_windowed(
                rom,
                hooks.as_ref(),
                cli.hooks,
                switch.as_ref(),
                cli.mode.ruleset(),
                session,
                enhanced,
                video_start,
            )
        }
    };
    let code = match result {
        Ok(stats) => ExitCode::from(finish(hooks.as_ref(), &stats)),
        Err(e) => error_exit(&e),
    };
    // A set applied, or a chapter asked for, in the window: the game starts again now, after this run has written its files.
    if let Err(error) = packs::start_next_run() {
        eprintln!("the game did not restart: {error}");
    }
    code
}

/// The data packs of this run, loaded on `rom`: the image they make (`None` without packs), the HD art of the packs to draw
/// over the installed art, the packs the run uses with their paths, and why a set the data folder enables is not used.
struct Loaded {
    image: Option<ssd_mod::Image>,
    overlays: Vec<ssd_hd::pack::Overlay>,
    applied: Vec<(PathBuf, ssd_mod::Pack)>,
    notice: Option<String>,
}

/// The packs of this run: the `--mods` packs; else, in an Enhanced run that persists, the set `mods/mods.toml` enables; else
/// none. A `--mods` set the checks refuse is printed in full, and ends the run with the packs' exit code. A set from the file
/// that is refused or cannot be read is not loaded: the run goes on without packs, and the Data packs screen says why. A
/// `--chapter` starts that chapter of the set.
fn load_packs(cli: &Cli, rom: &[u8]) -> Result<Loaded, ExitCode> {
    let without =
        |notice| Loaded { image: None, overlays: Vec::new(), applied: Vec::new(), notice };
    let from_file =
        cli.mods.is_empty() && !cli.no_mods && cli.mode == ModeArg::Enhanced && persists(cli);
    let paths = if !cli.mods.is_empty() {
        cli.mods.clone()
    } else if from_file {
        match packs::set_paths() {
            Ok(paths) => paths,
            Err(message) => return Ok(without(Some(message))),
        }
    } else {
        Vec::new()
    };
    if paths.is_empty() {
        return Ok(without(None));
    }
    let mut packs = Vec::new();
    let mut errors = Vec::new();
    for path in &paths {
        match ssd_mod::Pack::open(path) {
            Ok(pack) => packs.push(pack),
            Err(error) => errors.push(error),
        }
    }
    if errors.is_empty()
        && let Err(refused) = ssd_mod::check_set(rom, &packs, ssd_mod::CheckOptions::default())
    {
        errors = refused;
    }
    if !errors.is_empty() {
        if from_file {
            let text = packs::refusal_text(&errors);
            log::warn!("the packs of mods.toml are not used: {text}");
            return Ok(without(Some(text)));
        }
        for error in &errors {
            eprintln!("data pack refused: {error}");
        }
        return Err(ExitCode::from(EXIT_PACKS));
    }
    let image = match ssd_mod::build(rom, &packs) {
        Ok(image) => image,
        Err(error) if from_file => {
            log::warn!("the packs of mods.toml are not used: {error}");
            return Ok(without(Some(error.to_string())));
        }
        Err(error) => {
            eprintln!("data packs: {error}");
            return Err(ExitCode::from(EXIT_PACKS));
        }
    };
    if let Some(id) = cli.chapter.as_deref() {
        match chapter::start(id, &packs, rom) {
            Ok(session) => chapter::configure(session),
            Err(message) => {
                eprintln!("chapter: {message}");
                return Err(ExitCode::from(EXIT_PACKS));
            }
        }
    }
    let overlays = overlays_of(&paths, &packs, &image);
    Ok(Loaded {
        image: Some(image),
        overlays,
        applied: paths.into_iter().zip(packs).collect(),
        notice: None,
    })
}

/// The HD art of each pack (`paths` are the packs, in the order of `packs`). A folder's art is read from the folder; a zip's
/// from its files unpacked once into the cache (`packs::unpacked`).
fn overlays_of(
    paths: &[PathBuf],
    packs: &[ssd_mod::Pack],
    image: &ssd_mod::Image,
) -> Vec<ssd_hd::pack::Overlay> {
    paths
        .iter()
        .zip(packs)
        .filter_map(|(path, pack)| {
            let id = pack.manifest.pack.id.as_str();
            if path.is_dir() {
                return Some(ssd_hd::pack::Overlay::of_pack(image, id, path));
            }
            let manifest = &pack.manifest;
            if manifest.map_art.is_empty()
                && manifest.face.is_empty()
                && manifest.pose_variant.is_empty()
            {
                return None;
            }
            match packs::unpacked(path, pack) {
                Ok(dir) => Some(ssd_hd::pack::Overlay::of_pack(image, id, &dir)),
                Err(error) => {
                    log::warn!("the HD art of the zip pack {id} is not drawn: {error:#}");
                    None
                }
            }
        })
        .collect()
}

/// Whether this run writes the player's files (see [`paths::persists`]): a windowed run does. A headless run (a UI
/// snapshot is one), a benchmark and a replay keep their changes in memory for the run (README, "Data and files").
fn persists(cli: &Cli) -> bool {
    cli.headless_frames.is_none() && cli.replay.is_none() && cli.bench_app.is_none()
}

/// `--hd-scale`: a whole scale from 2 to [`ssd_hd::MAX_SCALE`].
fn parse_hd_scale(text: &str) -> Result<u32, String> {
    text.parse::<u32>()
        .ok()
        .filter(|scale| (2..=ssd_hd::MAX_SCALE).contains(scale))
        .ok_or_else(|| format!("`{text}` is not a scale from 2 to {}", ssd_hd::MAX_SCALE))
}

/// `--enhanced-look`: the look of the Enhanced picture.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum LookArg {
    Crisp,
    Smooth,
}

impl From<LookArg> for Look {
    fn from(look: LookArg) -> Self {
        match look {
            LookArg::Crisp => Self::Crisp,
            LookArg::Smooth => Self::Smooth,
        }
    }
}

/// `--ui-size`: `WIDTHxHEIGHT` in pixels, each side from 1 to [`snapshot::MAX_SIDE`].
fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let (width, height) = text
        .split_once(['x', 'X'])
        .ok_or_else(|| format!("`{text}` is not WIDTHxHEIGHT, such as 1440x900"))?;
    let side = |part: &str| {
        part.parse::<u32>()
            .ok()
            .filter(|side| (1..=snapshot::MAX_SIDE).contains(side))
            .ok_or_else(|| format!("`{text}`: each side is from 1 to {}", snapshot::MAX_SIDE))
    };
    Ok((side(width)?, side(height)?))
}

/// The script of `--ui-script`, read and checked; a bad one is a usage error (exit 2).
fn read_script(path: &Path) -> Vec<ui_script::Line> {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| script_usage_error(path, &e.to_string()));
    ui_script::parse(&text).unwrap_or_else(|e| script_usage_error(path, &e.to_string()))
}

fn script_usage_error(path: &Path, message: &str) -> ! {
    Cli::command()
        .error(ErrorKind::ValueValidation, format!("--ui-script {}: {message}", path.display()))
        .exit()
}

/// The symbols and hook switch of the panel, if the symbols file can be read: a warning says why
/// the hook mode cannot be changed otherwise.
fn load_switch(cli: &Cli) -> Option<HookSwitch> {
    HookSwitch::load(cli.symbols.as_deref(), cli.divergence_dir.as_deref())
        .inspect_err(|e| log::warn!("the hook mode cannot be changed in the panel: {e:#}"))
        .ok()
}

/// Reports a failed run: a rejected replay exits with [`EXIT_REPLAY`], anything else with 1.
fn error_exit(error: &anyhow::Error) -> ExitCode {
    if replay_io::is_rejection(error) {
        eprintln!("{error:#}");
        return ExitCode::from(EXIT_REPLAY);
    }
    eprintln!("error: {error:#}");
    ExitCode::FAILURE
}

/// The end of a successful run, and its exit code: a verify run prints its table and fails if
/// anything diverged.
fn finish(hooks: Option<&HookSetup>, stats: &app::RunStats) -> u8 {
    let Some(hooks) = hooks.filter(|hooks| hooks.is_verify()) else {
        return 0;
    };
    println!("{}", hook_setup::stats_table(stats));
    let divergences = hook_setup::divergence_count(stats);
    if divergences == 0 {
        return 0;
    }
    let report_errors = hook_setup::report_error_count(stats);
    eprintln!(
        "{}",
        hook_setup::divergence_summary(divergences, report_errors, hooks.divergence_dir())
    );
    EXIT_DIVERGENCES
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_oracle::HookStats;
    use std::path::Path;

    const REPO_SYMBOLS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../re/symbols.toml");

    fn stats(divergences: u64) -> app::RunStats {
        vec![("rng_next_a".to_owned(), HookStats { calls: 9, divergences, ..HookStats::default() })]
    }

    fn setup(mode: HooksArg) -> Option<HookSetup> {
        hook_setup::prepare(mode, Some(Path::new(REPO_SYMBOLS)), None).unwrap()
    }

    #[test]
    fn only_a_verify_run_with_divergences_fails() {
        assert_eq!(finish(setup(HooksArg::Verify).as_ref(), &stats(0)), 0);
        assert_eq!(finish(setup(HooksArg::Verify).as_ref(), &stats(1)), EXIT_DIVERGENCES);
        assert_eq!(EXIT_DIVERGENCES, 5);
        // Nothing is compared outside verify, whatever the counters say.
        assert_eq!(finish(setup(HooksArg::Rust).as_ref(), &stats(1)), 0);
        assert_eq!(finish(None, &stats(1)), 0);
    }

    /// The command lines that do not persist: a headless run, a UI snapshot, a replay and a benchmark.
    const NOT_PERSISTING: [&[&str]; 4] = [
        &["--headless-frames", "600"],
        &["--headless-frames", "600", "--ui-snapshot", "panel.png"],
        &["--replay", "battle.ssdreplay"],
        &["--bench-app", "battle.ssdreplay"],
    ];

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("ssd-recompiled").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn the_pack_options_parse_and_do_not_mix() {
        let cli = parse(&["--mode", "enhanced", "--mods", "a.ssdpack", "--mods", "pack-dir"]);
        assert_eq!(cli.mods, [PathBuf::from("a.ssdpack"), PathBuf::from("pack-dir")]);
        assert!(!cli.no_mods);
        assert!(parse(&["--no-mods"]).mods.is_empty(), "the default is the ROM alone");
        let both = ["ssd-recompiled", "--mods", "a.ssdpack", "--no-mods"];
        assert!(Cli::try_parse_from(both).is_err(), "--mods and --no-mods conflict");
    }

    #[test]
    fn a_headless_configuration_writes_no_option_file_and_a_windowed_one_does() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let file = dir.path().join("saves").join("ssd.settings");
        let mut values = Values::new();
        values.set("video.scale", 5);
        for args in NOT_PERSISTING {
            paths::set_persistent(persists(&parse(args)));
            options::write_settings(&values).unwrap();
            assert!(!file.exists(), "{args:?} wrote the PC file");
        }
        paths::set_persistent(persists(&parse(&[])));
        options::write_settings(&values).unwrap();
        assert!(file.exists(), "a windowed run writes the PC file");
        paths::set_persistent(false);
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn a_headless_configuration_writes_no_save_file_and_a_windowed_one_does() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let saves = dir.path().join("saves");
        // What a run writes when it ends: its battery save and its bag and cheat files.
        let write_saves = |args: &[&str]| {
            paths::set_persistent(persists(&parse(args)));
            saves::battery_saves().unwrap().persist("sav", &[0x5A; 8192]).unwrap();
            hd::write_beside(&saves.join("ssd.bag"), b"bags").unwrap();
            hd::write_beside(&saves.join("ssd.cheats"), b"cheats").unwrap();
            hd::write_beside(&saves.join("ssd.mods"), b"mods").unwrap();
        };
        for args in NOT_PERSISTING {
            write_saves(args);
            assert!(!saves.exists(), "{args:?} created the saves folder");
        }
        write_saves(&[]);
        assert!(saves.join("ssd.sav").exists(), "a windowed run writes the battery save");
        assert!(saves.join("ssd.bag").exists() && saves.join("ssd.cheats").exists());
        assert!(saves.join("ssd.mods").exists());
        paths::set_persistent(false);
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn the_set_the_file_enables_loads_in_a_run_that_persists_and_a_refused_one_is_not_used() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let data = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", data.path()) };
        let rom = vec![0_u8; ssd_mod::ROM_LEN];
        let game = format!("{:08X}", ssd_rom::crc32(&rom));
        let folder = packs::mods_dir().join("plain");
        std::fs::create_dir_all(&folder).unwrap();
        let manifest = format!(
            "[pack]\nid = \"plain.pack\"\nname = \"Plain\"\nversion = \"1.0.0\"\ngame = \"{game}\"\n"
        );
        std::fs::write(folder.join("pack.toml"), manifest).unwrap();
        let windowed = parse(&["--mode", "enhanced"]);
        paths::set_persistent(true);
        packs::write_set(&["plain.pack".to_owned()]).unwrap();
        let Ok(loaded) = load_packs(&windowed, &rom) else { panic!("the enabled set is loaded") };
        assert_eq!(loaded.applied.len(), 1, "the pack the file enables");
        assert!(loaded.image.is_some() && loaded.notice.is_none());
        // A set the folder does not hold is not used: the run goes on without packs, and says why.
        packs::write_set(&["gone.pack".to_owned()]).unwrap();
        let Ok(loaded) = load_packs(&windowed, &rom) else { panic!("a refused file set goes on") };
        assert!(loaded.image.is_none() && loaded.applied.is_empty());
        assert_eq!(
            loaded.notice.as_deref(),
            Some("mods.toml enables `gone.pack`, which is not in the mods folder")
        );
        // A run that does not persist reads no set file, as it reads no option file.
        paths::set_persistent(false);
        let headless = parse(&["--mode", "enhanced", "--headless-frames", "1"]);
        let Ok(loaded) = load_packs(&headless, &rom) else { panic!("no packs is not a refusal") };
        assert!(loaded.image.is_none() && loaded.notice.is_none());
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }
}
