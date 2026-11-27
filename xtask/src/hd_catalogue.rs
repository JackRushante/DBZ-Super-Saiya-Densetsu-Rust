//! `hd-catalogue`: checks the HD art against the game, statically.
//!
//! * `extract` lists everything the game can draw as a battle pose or a portrait, decoded from the
//!   ROM (the pixels, the keys the renderer finds art by, where they come from), into
//!   `<out>/catalogue.json` and `<out>/native/*.png`;
//! * `verify` finds, for every entry, the art the renderer would draw, and scores it against the
//!   native picture (shape, colours) and against who the entry is (`tools/hd-identity.toml`),
//!   into `<out>/report.json`;
//! * `sheet` lays the entries out in `<out>/index.html`, the most suspicious first.
//!
//! Everything written is derived from the game and goes to `.work/` (default `<out>` is
//! `.work/catalogue`); nothing here is tracked.

mod decode;
mod extract;
mod gate;
mod identity;
mod redo;
mod score;
mod scripts;
mod sheet;
mod verify;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The default output folder, under the repository.
const DEFAULT_OUT: &str = ".work/catalogue";

/// What a catalogue entry is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Category {
    BattlePose,
    Face,
    ObjectPortrait,
}

/// One thing the game draws that has HD art: a pose of a unit, or one half of a portrait set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub category: Category,
    /// The unit ids the entry is for (a pose: its unit; a face: every unit whose face table row
    /// names the set).
    pub units: Vec<u8>,
    /// The units' names from the game's name table.
    pub unit_names: Vec<String>,
    pub poseset: Option<u8>,
    pub pose: Option<usize>,
    /// A portrait: the set's `bank:address` and the half (0 or 1).
    pub stream: Option<String>,
    pub half: Option<u8>,
    /// An object portrait: the object id.
    pub object: Option<u8>,
    /// A face: the units that show this half by the rule `ssd_catalogue::face_portrait` has.
    pub shown_by: Vec<u8>,
    pub w: usize,
    pub h: usize,
    /// The pose key and the key of the pose facing the other way (16 hex digits).
    pub key: Option<String>,
    pub mirror_key: Option<String>,
    /// The keys of the pose drawn upside down (the game flips a sprite either way at draw time): the
    /// key and the mirror image's.
    pub vflip_keys: Option<(String, String)>,
    /// Keys of the clusters a pose that falls apart is also looked up by: (key, mirror key).
    pub cluster_keys: Vec<(String, String)>,
    /// Where in the ROM it comes from (the layout of a pose, the compressed set of a portrait).
    pub origin: String,
    /// The 16 colours the native picture is drawn in (`#RRGGBB`).
    pub palette: Vec<String>,
    /// The native picture, relative to the output folder.
    pub png: String,
    /// Why the entry cannot be checked, when it cannot.
    pub note: Option<String>,
    /// Whether the game can show it: a pose some script sets (or a replay loaded); portraits always.
    #[serde(default = "yes")]
    pub used: bool,
    /// What says so (a few places that set the pose, replays that loaded it) or that nothing does.
    #[serde(default)]
    pub evidence: Vec<String>,
    /// How sure: `observed` (a replay loaded it), `script` (a script sets it straight, or a fixed routine), `branch`
    /// (only reachable by following a branch: an upper bound), `none` (not used) or `portrait`.
    #[serde(default)]
    pub basis: String,
}

/// The catalogue file.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Catalogue {
    pub entries: Vec<Entry>,
    /// What could not be decoded, and the known exceptions.
    pub problems: Vec<String>,
}

fn yes() -> bool {
    true
}

pub fn parse_key(text: &str) -> Option<u64> {
    u64::from_str_radix(text, 16).ok()
}

pub fn hex_key(key: u64) -> String {
    format!("{key:016x}")
}

fn read_rom() -> anyhow::Result<Vec<u8>> {
    let path = std::env::var_os("SSD_ROM_PATH").map_or_else(crate::rom_cache, PathBuf::from);
    std::fs::read(&path).with_context(|| format!("reading the ROM {}", path.display()))
}

/// The options common to the three steps.
struct Options {
    out: PathBuf,
    art: Option<PathBuf>,
    identity: Option<PathBuf>,
    observed: Option<PathBuf>,
    accept: Option<PathBuf>,
    check: bool,
}

fn parse(args: &[&str]) -> Result<Options, String> {
    let mut options = Options {
        out: PathBuf::from(DEFAULT_OUT),
        art: None,
        identity: None,
        observed: None,
        accept: None,
        check: false,
    };
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        if arg == "--check" {
            options.check = true;
            continue;
        }
        let mut value = |name: &str| {
            rest.next().map(PathBuf::from).ok_or_else(|| format!("{name} needs a folder or file"))
        };
        match arg {
            "--out" => options.out = value("--out")?,
            "--art" => options.art = Some(value("--art")?),
            "--identity" => options.identity = Some(value("--identity")?),
            "--observed" => options.observed = Some(value("--observed")?),
            "--accept" => options.accept = Some(value("--accept")?),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(options)
}

const USAGE: &str = "hd-catalogue extract [--out DIR] [--observed LOG] | verify --art <store> [--out DIR] [--identity FILE] [--check [--accept FILE]] | redo [--out DIR] | sheet [--out DIR]";

/// `hd-catalogue <step> ...`
pub fn run(args: &[&str]) -> anyhow::Result<ExitCode> {
    let Some((&step, rest)) = args.split_first() else {
        eprintln!("hd-catalogue: {USAGE}");
        return Ok(ExitCode::from(2));
    };
    let options = match parse(rest) {
        Ok(options) => options,
        Err(message) => {
            eprintln!("hd-catalogue: {message}\n{USAGE}");
            return Ok(ExitCode::from(2));
        }
    };
    let root = crate::repo_root()?;
    let out = if options.out.is_absolute() { options.out.clone() } else { root.join(&options.out) };
    match step {
        "extract" => {
            let rom = read_rom()?;
            extract::run(&rom, &out, options.observed.as_deref(), &mut std::io::stdout())?;
        }
        "verify" => {
            let Some(art) = &options.art else {
                eprintln!("hd-catalogue: verify needs --art <art store>\n{USAGE}");
                return Ok(ExitCode::from(2));
            };
            let identity =
                options.identity.clone().unwrap_or_else(|| root.join("tools/hd-identity.toml"));
            let report = verify::run(&out, art, &identity, &mut std::io::stdout())?;
            if options.check {
                let accept = options
                    .accept
                    .clone()
                    .unwrap_or_else(|| root.join("tools/hd-catalogue-accept.toml"));
                let outcome = gate::check(&report, &accept)?;
                gate::print(&outcome, gate::accepted_count(&accept), &mut std::io::stdout())?;
                if !outcome.passes() {
                    return Ok(ExitCode::FAILURE);
                }
            }
        }
        "sheet" => sheet::run(&out, &mut std::io::stdout())?,
        "redo" => {
            let identity =
                options.identity.clone().unwrap_or_else(|| root.join("tools/hd-identity.toml"));
            let accept = options
                .accept
                .clone()
                .unwrap_or_else(|| root.join("tools/hd-catalogue-accept.toml"));
            redo::run(&out, &identity, &accept, &mut std::io::stdout())?;
        }
        other => {
            eprintln!("hd-catalogue: unknown step `{other}`\n{USAGE}");
            return Ok(ExitCode::from(2));
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Reads `<out>/catalogue.json`.
pub(crate) fn read_catalogue(out: &Path) -> anyhow::Result<Catalogue> {
    let path = out.join("catalogue.json");
    let text = std::fs::read_to_string(&path).with_context(|| {
        format!("reading {} (run `hd-catalogue extract` first)", path.display())
    })?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}
