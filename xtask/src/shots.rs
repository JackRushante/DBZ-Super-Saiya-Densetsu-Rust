//! `shots`: plays an input script headless from power-on and writes the screen at chosen frames
//! as PNG files, with a fingerprint of each frame and the values of chosen WRAM bytes, so
//! one can find out where the game is without a window (`CONTRIBUTING.md`). It is how the
//! scripts of the menu, shop and save replays were authored. Output goes to `.work/shots/` and
//! is never tracked (the screenshots are derived from the game).
//!
//! The script is the name of a script of `ssd_game::scripts` (`battle`, `menus`, ...) or the path
//! of a timeline file (one `<frames> <keys>` per line, see `ssd_game::scripts::parse_timeline`), or
//! of a replay (`.ssdreplay`), which also gives the SRAM it powers on with.

use anyhow::{Context, bail};
use ssd_game::scripts::{self, REPLAYS};
use ssd_hd::world::{FieldState, Rom, WideRun};
use ssd_hd::{HdOptions, Look};
use ssd_oracle::{Joypad, Machine, MemorySaveStore, NullAudio, Symbols};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The command line of `shots`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub script: String,
    pub frames: Vec<u64>,
    pub every: Option<u64>,
    /// Last frame to play (default: the script's last, or the last of `--frames`).
    pub upto: Option<u64>,
    pub out: Option<PathBuf>,
    /// Put the shots in one contact sheet of this many columns (half size) instead of files.
    pub sheet: Option<usize>,
    pub no_png: bool,
    /// Variable names or hex bus addresses to print with each shot.
    pub peek: Vec<String>,
    pub state_in: Option<PathBuf>,
    pub state_out: Option<PathBuf>,
    /// Write the 128 KiB of WRAM at the end (to find variables by diffing two runs).
    pub wram_out: Option<PathBuf>,
    /// Write the script as an input timeline for `tools/re/trace.lua` (`DBZ_INPUTS`) and stop.
    pub mesen_inputs: Option<PathBuf>,
    /// Power on with this battery save (8 KiB) instead of a blank cartridge.
    pub sram: Option<PathBuf>,
    /// WRAM bytes (variables or hex bus addresses) whose CPU writes are printed, one line per
    /// change, with the frame they happened in.
    pub watch: Vec<String>,
    /// From this frame on the run goes one CPU cycle at a time and each watched write also says
    /// where the CPU was (the program counter after the writing instruction's cycle).
    pub watch_from: Option<u64>,
    /// Data packs (a folder or a `.ssdpack` each, repeatable): the machine runs the image of those packs with the
    /// redirectors installed (`ssd_game::mods`), as the Enhanced app does. A pack may hold the ROM's bytes (`--local`).
    pub mods: Vec<PathBuf>,
    /// Write the battery SRAM (8 KiB) at the end: a save the game wrote itself, when the script
    /// ended with the map menu's Save.
    pub sram_out: Option<PathBuf>,
    /// Draw the Enhanced picture at this scale (2 to 8) instead of the PPU frame, from the pack's
    /// art: the size the game's window shows. `--state-in` with it is how a saved moment is seen in
    /// HD without a window.
    pub hd_scale: Option<u32>,
    /// The look of the `--hd-scale` picture: `--enhanced-look crisp|smooth` [default: crisp].
    pub look: Look,
    /// The HD pack folder of `--hd-scale` [default: the converter's under the cache directory].
    pub hd_pack: Option<PathBuf>,
    /// With `--hd-scale`: write the field, fighter and effect poses the frames show with no art
    /// (`field-missing-<key>.json`, as `hd-tour --gaps` does), for making that art.
    pub gaps: Option<PathBuf>,
    /// With `--gaps`: also map this box of the picture, `x,y,w,h` in the wide view's pixels: for
    /// each pixel the layer on top (OAM `O`, BG `U`/`L`/`H`/`B`) and a letter per picture.
    pub box_map: Option<(i32, i32, usize, usize)>,
    /// Host writes to work RAM, `FRAME:ADDR=HEX` (repeatable): the bytes are written before frame FRAME runs, as the
    /// Enhanced app's host writes are (`docs/mods/art.md`, the Kaioken's pose variant proof).
    pub wram: Vec<String>,
}

/// Parses `<script> [--frames a,b,c] [--every N] [--upto N] [--out dir] [--sheet COLS]
/// [--no-png] [--peek a,b] [--watch a,b] [--watch-from N] [--state-in f] [--state-out f]
/// [--wram-out f] [--sram f] [--sram-out f] [--hd-scale N] [--enhanced-look crisp|smooth] [--pack DIR]
/// [--gaps DIR] [--mods PACK]...`.
///
/// # Errors
///
/// An unknown option, a bad value, or no script.
pub fn parse_args(args: &[&str]) -> Result<Args, String> {
    let mut parsed = Args {
        script: String::new(),
        frames: Vec::new(),
        every: None,
        upto: None,
        out: None,
        sheet: None,
        no_png: false,
        peek: Vec::new(),
        state_in: None,
        state_out: None,
        wram_out: None,
        mesen_inputs: None,
        sram: None,
        watch: Vec::new(),
        watch_from: None,
        sram_out: None,
        hd_scale: None,
        look: Look::Crisp,
        hd_pack: None,
        gaps: None,
        box_map: None,
        wram: Vec::new(),
        mods: Vec::new(),
    };
    let number = |flag: &str, v: Option<&&str>| -> Result<u64, String> {
        let v = v.ok_or(format!("{flag} needs a value"))?;
        v.parse().map_err(|_| format!("bad {flag} `{v}`"))
    };
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--frames" => {
                let v = it.next().ok_or("--frames needs a list")?;
                for f in v.split(',').filter(|f| !f.is_empty()) {
                    parsed.frames.push(f.parse().map_err(|_| format!("bad frame `{f}`"))?);
                }
            }
            "--every" => parsed.every = Some(number(arg, it.next())?.max(1)),
            "--upto" => parsed.upto = Some(number(arg, it.next())?),
            "--sheet" => {
                parsed.sheet =
                    Some(usize::try_from(number(arg, it.next())?.clamp(1, 16)).unwrap_or(4));
            }
            "--out" => {
                parsed.out = Some(PathBuf::from(it.next().ok_or("--out needs a directory")?));
            }
            "--no-png" => parsed.no_png = true,
            "--peek" => {
                let v = it.next().ok_or("--peek needs a list")?;
                parsed.peek.extend(v.split(',').filter(|p| !p.is_empty()).map(str::to_owned));
            }
            "--sram-out" => {
                parsed.sram_out = Some(PathBuf::from(it.next().ok_or("--sram-out needs a file")?));
            }
            "--watch-from" => parsed.watch_from = Some(number(arg, it.next())?),
            "--watch" => {
                let v = it.next().ok_or("--watch needs a list")?;
                parsed.watch.extend(v.split(',').filter(|p| !p.is_empty()).map(str::to_owned));
            }
            "--state-in" => {
                parsed.state_in = Some(PathBuf::from(it.next().ok_or("--state-in needs a file")?));
            }
            "--state-out" => {
                parsed.state_out =
                    Some(PathBuf::from(it.next().ok_or("--state-out needs a file")?));
            }
            "--mesen-inputs" => {
                parsed.mesen_inputs =
                    Some(PathBuf::from(it.next().ok_or("--mesen-inputs needs a file")?));
            }
            "--sram" => {
                parsed.sram = Some(PathBuf::from(it.next().ok_or("--sram needs a file")?));
            }
            "--wram-out" => {
                parsed.wram_out = Some(PathBuf::from(it.next().ok_or("--wram-out needs a file")?));
            }
            "--hd-scale" => {
                let scale = u32::try_from(number(arg, it.next())?).unwrap_or(4).clamp(2, 8);
                parsed.hd_scale = Some(scale);
            }
            "--enhanced-look" => {
                parsed.look = match it.next().copied() {
                    Some("crisp") => Look::Crisp,
                    Some("smooth") => Look::Smooth,
                    _ => return Err("--enhanced-look takes crisp or smooth".to_owned()),
                };
            }
            "--pack" => {
                parsed.hd_pack = Some(PathBuf::from(it.next().ok_or("--pack needs a folder")?));
            }
            "--gaps" => {
                parsed.gaps = Some(PathBuf::from(it.next().ok_or("--gaps needs a folder")?));
            }
            "--mods" => {
                parsed.mods.push(PathBuf::from(it.next().ok_or("--mods needs a pack")?));
            }
            "--wram" => {
                parsed.wram.push(it.next().ok_or("--wram needs FRAME:ADDR=HEX")?.to_string());
            }
            "--box" => {
                let v = it.next().ok_or("--box needs x,y,w,h")?;
                let parts: Vec<i32> = v
                    .split(',')
                    .map(|p| p.trim().parse::<i32>())
                    .collect::<Result<_, _>>()
                    .map_err(|_| "--box takes x,y,w,h".to_owned())?;
                if parts.len() != 4 {
                    return Err("--box takes x,y,w,h".to_owned());
                }
                parsed.box_map = Some((parts[0], parts[1], parts[2] as usize, parts[3] as usize));
            }
            option if option.starts_with("--") => return Err(format!("unknown option `{option}`")),
            script if parsed.script.is_empty() => script.clone_into(&mut parsed.script),
            other => return Err(format!("one script only (`{other}` is a second one)")),
        }
    }
    if parsed.script.is_empty() {
        return Err("needs a script name or a timeline file".to_owned());
    }
    if parsed.frames.is_empty()
        && parsed.every.is_none()
        && parsed.state_out.is_none()
        && parsed.wram_out.is_none()
        && parsed.mesen_inputs.is_none()
        && parsed.watch.is_empty()
        && parsed.sram_out.is_none()
    {
        return Err("needs --frames or --every (or --state-out, --wram-out, --sram-out, --watch)"
            .to_owned());
    }
    Ok(parsed)
}

/// The input of a script: its joypad per frame and its length.
pub struct Script {
    pub name: String,
    pub inputs: Box<dyn Fn(u64) -> Joypad>,
    pub frames: u64,
    /// The SRAM a replay powers on with (`--sram` wins over it).
    pub sram: Option<Vec<u8>>,
}

/// A script of `ssd_game::scripts` by name, or a timeline file.
///
/// # Errors
///
/// Neither a known name nor a readable, valid timeline.
pub fn load_script(spec: &str) -> anyhow::Result<Script> {
    if let Some(&(name, frames, input)) = REPLAYS.iter().find(|(name, ..)| *name == spec) {
        return Ok(Script { name: name.to_owned(), inputs: Box::new(input), frames, sram: None });
    }
    let path = Path::new(spec);
    if !path.is_file() {
        let names: Vec<&str> = REPLAYS.iter().map(|(n, ..)| *n).collect();
        bail!("`{spec}` is neither a script ({}) nor a timeline file", names.join(", "));
    }
    let text = std::fs::read_to_string(path).with_context(|| format!("reading {spec}"))?;
    let inputs = scripts::parse_timeline(&text).map_err(|e| anyhow::anyhow!("{spec}: {e}"))?;
    let frames = inputs.len() as u64;
    Ok(Script {
        name: path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
        inputs: Box::new(move |frame| {
            usize::try_from(frame).ok().and_then(|f| inputs.get(f)).copied().unwrap_or_default()
        }),
        frames,
        sram: None,
    })
}

/// A replay file as a script: its inputs, and the SRAM it powers on with.
///
/// # Errors
///
/// The file cannot be read or is not a replay of `rom` for this core.
pub fn load_replay_script(path: &Path, rom: &[u8]) -> anyhow::Result<Script> {
    let mut replay = crate::shared::read_replay(path, rom)?;
    let frames = replay.frames();
    let sram = replay.initial_sram.take();
    Ok(Script {
        name: path.file_stem().map_or_else(String::new, |s| s.to_string_lossy().into_owned()),
        inputs: Box::new(move |frame| replay.input(frame)),
        frames,
        sram,
    })
}

/// The timeline text of `tools/re/trace.lua`'s `DBZ_INPUTS` for the first `frames` frames of
/// `inputs`: one `<n> <keys>` row per run of equal joypads.
fn mesen_timeline(inputs: &dyn Fn(u64) -> Joypad, frames: u64) -> String {
    let keys = |p: &Joypad| {
        let all = [
            (p.up, "up"),
            (p.down, "down"),
            (p.left, "left"),
            (p.right, "right"),
            (p.a, "a"),
            (p.b, "b"),
            (p.x, "x"),
            (p.y, "y"),
            (p.l, "l"),
            (p.r, "r"),
            (p.start, "start"),
            (p.select, "select"),
        ];
        let names: Vec<&str> = all.iter().filter(|(on, _)| *on).map(|(_, n)| *n).collect();
        if names.is_empty() { "-".to_owned() } else { names.join(",") }
    };
    let mut out = String::from(
        "# generated by `xtask shots --mesen-inputs`
",
    );
    let mut run: Option<(String, u64)> = None;
    for frame in 0..frames {
        let k = keys(&inputs(frame));
        match &mut run {
            Some((last, n)) if *last == k => *n += 1,
            _ => {
                if let Some((last, n)) = run.take() {
                    let _ = writeln!(out, "{n} {last}");
                }
                run = Some((k, 1));
            }
        }
    }
    if let Some((last, n)) = run {
        let _ = writeln!(out, "{n} {last}");
    }
    out.push_str(
        "@mode idle
",
    );
    out
}

/// A host write to work RAM before a frame (`--wram FRAME:ADDR=HEX`): the bus address `$7E:0000` to `$7F:FFFF` it starts at, and
/// the bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Poke {
    frame: u64,
    address: u32,
    bytes: Vec<u8>,
}

/// The host writes of `specs`, each `FRAME:ADDR=HEX` with ADDR a bus address in work RAM (`7E0C36` or `$7E:0C36`) and HEX the
/// bytes in hexadecimal (`80`, `3B00`).
///
/// # Errors
///
/// A spec that is not `FRAME:ADDR=HEX`, an address outside work RAM, or bytes that are not hexadecimal pairs.
fn resolve_pokes(specs: &[String]) -> anyhow::Result<Vec<Poke>> {
    specs
        .iter()
        .map(|spec| {
            let shape = || format!("`{spec}` is not FRAME:ADDR=HEX");
            let (frame, rest) = spec.split_once(':').with_context(shape)?;
            let (address, hex) = rest.split_once('=').with_context(shape)?;
            let frame: u64 = frame.parse().with_context(shape)?;
            let address = crate::disasm::parse_address(address)?;
            if !(0x7E_0000..=0x7F_FFFF).contains(&address) {
                bail!("`{spec}`: {address:06X} is not in work RAM ($7E:0000 to $7F:FFFF)");
            }
            if hex.is_empty() || hex.len() % 2 != 0 {
                bail!("`{spec}`: the bytes are hexadecimal pairs, such as 80 or 3B00");
            }
            let bytes = (0..hex.len())
                .step_by(2)
                .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).with_context(shape))
                .collect::<anyhow::Result<Vec<u8>>>()?;
            Ok(Poke { frame, address, bytes })
        })
        .collect()
}

/// One WRAM byte range to print: its label and bus offset in WRAM.
struct Peek {
    label: String,
    offset: usize,
    size: usize,
}

fn resolve_peeks(symbols: &Symbols, specs: &[String]) -> anyhow::Result<Vec<Peek>> {
    specs
        .iter()
        .map(|spec| {
            if let Some(v) = symbols.variables().iter().find(|v| v.name == *spec) {
                return Ok(Peek {
                    label: spec.clone(),
                    offset: (v.address & 0x1_FFFF) as usize,
                    size: v.size.min(8) as usize,
                });
            }
            let address = crate::disasm::parse_address(spec)
                .with_context(|| format!("`{spec}` is neither a variable nor an address"))?;
            let offset = match crate::disasm::region(address) {
                crate::disasm::Region::Wram(o) => o as usize,
                _ => bail!("`{spec}` is not in WRAM"),
            };
            Ok(Peek { label: format!("{offset:05X}"), offset, size: 1 })
        })
        .collect()
}

/// RGBA bytes of the machine's last frame, and its size.
fn rgba(machine: &Machine) -> (u32, u32, Vec<u8>) {
    let frame = machine.frame();
    let mut bytes = Vec::with_capacity(frame.pixels.len() * 4);
    for c in frame.pixels {
        bytes.extend_from_slice(&[c.r, c.g, c.b, 0xFF]);
    }
    (frame.size.width, frame.size.height, bytes)
}

/// The image of the packs at `paths` on `rom`, for `--mods`: checked as `mod-check --local` checks them (a pack
/// that holds the ROM's bytes is accepted: `shots` is a local tool and its output is never tracked), and the HD art of
/// the folder packs (a zip's is not read; the note says so).
///
/// # Errors
///
/// A pack that cannot be read, or a set the checks refuse (each refusal is printed with the error).
fn pack_image(
    rom: &[u8],
    paths: &[PathBuf],
) -> anyhow::Result<(Vec<u8>, Vec<ssd_hd::pack::Overlay>)> {
    let packs =
        paths.iter().map(|path| ssd_mod::Pack::open(path)).collect::<Result<Vec<_>, _>>()?;
    let options = ssd_mod::CheckOptions { allow_rom_bytes: true };
    if let Err(errors) = ssd_mod::check_set(rom, &packs, options) {
        let refused: Vec<String> = errors.iter().map(ToString::to_string).collect();
        bail!("the packs are refused: {}", refused.join("; "));
    }
    let image = ssd_mod::build(rom, &packs)?;
    let mut overlays = Vec::new();
    for (pack, path) in packs.iter().zip(paths) {
        let id = pack.manifest.pack.id.as_str();
        if path.is_dir() {
            overlays.push(ssd_hd::pack::Overlay::of_pack(&image, id, path));
        } else if !pack.manifest.map_art.is_empty() || !pack.manifest.face.is_empty() {
            eprintln!("the HD art of the zip pack {id} is not drawn: unzip the pack into a folder");
        }
    }
    Ok((image.bytes, overlays))
}

/// The pack with the HD art of the data packs of the run on top (`--mods`, see [`pack_image`]).
fn with_overlays(
    pack: std::sync::Arc<ssd_hd::pack::Pack>,
    overlays: &[ssd_hd::pack::Overlay],
) -> std::sync::Arc<ssd_hd::pack::Pack> {
    if overlays.is_empty() {
        return pack;
    }
    let pack = std::sync::Arc::unwrap_or_clone(pack);
    std::sync::Arc::new(
        overlays.iter().fold(pack, |pack, overlay| pack.with_overlay(overlay.clone())),
    )
}

/// Half-size copy of an RGBA image (each pixel the average of 2x2).
fn halve(width: u32, height: u32, rgba: &[u8]) -> (u32, u32, Vec<u8>) {
    let (w, h) = (width as usize / 2, height as usize / 2);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            for channel in 0..3 {
                let at = |dx: usize, dy: usize| {
                    u32::from(rgba[((2 * y + dy) * width as usize + 2 * x + dx) * 4 + channel])
                };
                out.push(((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1)) / 4) as u8);
            }
            out.push(0xFF);
        }
    }
    (w as u32, h as u32, out)
}

/// A contact sheet: the tiles in rows of `cols`, 2 pixels apart on a grey background.
fn sheet(tiles: &[(u32, u32, Vec<u8>)], cols: usize) -> (u32, u32, Vec<u8>) {
    let (tw, th) = (tiles[0].0 as usize, tiles[0].1 as usize);
    let rows = tiles.len().div_ceil(cols);
    let (w, h) = (cols * (tw + 2) + 2, rows * (th + 2) + 2);
    let mut out = vec![0x60_u8; w * h * 4];
    for (i, (_, _, pixels)) in tiles.iter().enumerate() {
        let (ox, oy) = (2 + (i % cols) * (tw + 2), 2 + (i / cols) * (th + 2));
        for y in 0..th {
            let from = y * tw * 4;
            let to = ((oy + y) * w + ox) * 4;
            out[to..to + tw * 4].copy_from_slice(&pixels[from..from + tw * 4]);
        }
    }
    (w as u32, h as u32, out)
}

/// A PNG of an RGBA image with stored (uncompressed) deflate blocks.
pub(crate) fn encode_png(width: u32, height: u32, rgba: &[u8]) -> Vec<u8> {
    fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let start = out.len();
        out.extend_from_slice(&kind);
        out.extend_from_slice(data);
        let crc = ssd_rom::crc32(&out[start..]);
        out.extend_from_slice(&crc.to_be_bytes());
    }
    let row = width as usize * 4;
    let mut raw = Vec::with_capacity(rgba.len() + height as usize);
    for line in rgba.chunks_exact(row) {
        raw.push(0);
        raw.extend_from_slice(line);
    }
    let mut z = vec![0x78, 0x01];
    let mut blocks = raw.chunks(0xFFFF).peekable();
    while let Some(block) = blocks.next() {
        z.push(u8::from(blocks.peek().is_none()));
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    let (mut a, mut b) = (1_u32, 0_u32);
    for &byte in &raw {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    z.extend_from_slice(&((b << 16) | a).to_be_bytes());
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let mut out = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];
    chunk(&mut out, *b"IHDR", &header);
    chunk(&mut out, *b"IDAT", &z);
    chunk(&mut out, *b"IEND", &[]);
    out
}

/// The WRAM offset a CPU write to bus address `bus` lands on (bank `$7E`, or the low 8 KiB mirrored
/// in the system banks), if it is one.
fn wram_offset(bus: u32) -> Option<usize> {
    match (bus >> 16, bus & 0xFFFF) {
        (0x7E, low) => Some(low as usize),
        (0x00..=0x3F | 0x80..=0xBF, low) if low < 0x2000 => Some(low as usize),
        _ => None,
    }
}

/// Plays the script and writes what `args` ask for; returns the lines printed (one per shot).
///
/// # Errors
///
/// The script, a state file or a file to write is bad, or the run fails.
pub fn run(
    rom: &[u8],
    symbols: &Symbols,
    args: &Args,
    root: &Path,
    hd_pack: &Path,
    out: &mut dyn std::io::Write,
) -> anyhow::Result<()> {
    let script = if args.script.ends_with(".ssdreplay") {
        load_replay_script(Path::new(&args.script), rom)?
    } else {
        load_script(&args.script)?
    };
    let dir =
        args.out.clone().unwrap_or_else(|| root.join(".work").join("shots").join(&script.name));
    if let Some(path) = &args.mesen_inputs {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, mesen_timeline(&*script.inputs, script.frames))?;
        writeln!(out, "{} frames of {} written to {}", script.frames, script.name, path.display())?;
        return Ok(());
    }
    let peeks = resolve_peeks(symbols, &args.peek)?;
    let mut saves = MemorySaveStore::default();
    if let Some(path) = &args.sram {
        let sram = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        saves.0.insert("sav".to_owned(), sram);
    } else if let Some(sram) = &script.sram {
        saves.0.insert("sav".to_owned(), sram.clone());
    }
    // The HD art of the data packs (`--mods`), drawn over the installed pack's with `--hd-scale`
    let mut overlays: Vec<ssd_hd::pack::Overlay> = Vec::new();
    // The field of the Enhanced picture is read from the image of the packs (their maps are in its banks)
    let mut world: Option<Vec<u8>> = None;
    let mut machine = if args.mods.is_empty() {
        Machine::new(rom.to_vec(), Box::new(saves))?
    } else {
        let (image, found) = pack_image(rom, &args.mods)?;
        world = Some(image.clone());
        overlays = found;
        let original = ssd_rom::crc32(rom);
        Machine::with_image(image, original, Box::new(saves), ssd_oracle::DEFAULT_WRAM_SEED)?
    };
    if !args.mods.is_empty() {
        let mut table = ssd_oracle::HookTable::new();
        ssd_game::mods::install(&mut table);
        machine.set_hook_table(Some(table));
    }
    if let Some(path) = &args.state_in {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        machine.load_state(&bytes)?;
    }
    let last = args
        .upto
        .or_else(|| args.frames.iter().copied().max())
        .unwrap_or(script.frames)
        .max(machine.frame_count());
    let watches = resolve_peeks(symbols, &args.watch)?;
    let pokes = resolve_pokes(&args.wram)?;
    if let Some(poke) = pokes.iter().find(|poke| poke.frame >= last) {
        bail!("`--wram` at frame {} is not reached: the run ends before frame {last}", poke.frame);
    }
    let mut last_seen: Vec<Option<u8>> = vec![None; watches.len()];
    machine.set_cpu_write_log(!watches.is_empty());
    // `--hd-scale`: draw the Enhanced picture from the pack instead of the PPU frame.
    let mut hd_font: Option<Arc<ssd_hd::Font>> = None;
    let mut hd = match args.hd_scale {
        Some(scale) => {
            let pack = with_overlays(crate::hd_art_run::open_pack(hd_pack)?, &overlays);
            let font = Arc::new(ssd_hd::Font::standard_with_pack(rom, Some(&pack.dir))?);
            hd_font = Some(Arc::clone(&font));
            let mut run = WideRun::new(world.as_deref().unwrap_or(rom));
            run.set_art(Some(pack));
            run.set_font(Some(font));
            machine.set_line_log(true);
            Some((run, HdOptions { scale, look: args.look }))
        }
        None => None,
    };
    // `--gaps`: a second run that counts what each frame shows, to write the poses with no art.
    let mut gaps_run = match (&args.gaps, args.hd_scale) {
        (Some(_), Some(_)) => {
            let pack = with_overlays(crate::hd_art_run::open_pack(hd_pack)?, &overlays);
            let mut run = WideRun::new(world.as_deref().unwrap_or(rom));
            run.set_art(Some(pack));
            run.set_font(hd_font.clone());
            run.set_tally(true);
            Some(run)
        }
        _ => None,
    };
    let mut gaps_written: std::collections::HashSet<String> = std::collections::HashSet::new();
    let gaps_options = HdOptions { scale: args.hd_scale.unwrap_or(4), look: args.look };
    let wanted = |frame: u64| {
        args.frames.contains(&frame)
            || args.every.is_some_and(|n| frame.is_multiple_of(n) && frame > 0)
    };
    let mut tiles = Vec::new();
    let mut labels = Vec::new();
    while machine.frame_count() < last {
        let frame = machine.frame_count();
        for poke in pokes.iter().filter(|poke| poke.frame == frame) {
            machine.write_wram(poke.address, &poke.bytes)?;
            let hex = poke.bytes.iter().fold(String::new(), |mut hex, byte| {
                let _ = write!(hex, "{byte:02X}");
                hex
            });
            writeln!(out, "wram {:06X} = {hex} before frame {frame}", poke.address)?;
        }
        if args.watch_from.is_some_and(|from| frame >= from) && !watches.is_empty() {
            let pad = (script.inputs)(frame);
            loop {
                let event = machine.tick(pad, &mut NullAudio)?;
                for (bus, value) in machine.take_cpu_writes() {
                    let wram = wram_offset(bus);
                    let r = machine.cpu_registers();
                    for w in &watches {
                        if wram == Some(w.offset) {
                            writeln!(
                                out,
                                "write frame {:6} {}={value:02X} pc={:02X}:{:04X}",
                                frame + 1,
                                w.label,
                                r.pbr,
                                r.pc
                            )?;
                        }
                    }
                }
                if matches!(event, ssd_oracle::TickEvent::Frame) {
                    break;
                }
            }
        } else {
            machine.run_frame((script.inputs)(frame), &mut NullAudio)?;
        }
        let frame = machine.frame_count();
        if !watches.is_empty() && args.watch_from.is_none_or(|from| frame <= from) {
            for (bus, value) in machine.take_cpu_writes() {
                let wram = wram_offset(bus);
                for (w, seen) in watches.iter().zip(last_seen.iter_mut()) {
                    if wram == Some(w.offset) && *seen != Some(value) {
                        *seen = Some(value);
                        writeln!(out, "write frame {frame:6} {}={value:02X}", w.label)?;
                    }
                }
            }
        }
        let mut line: String;
        let mut picture: Option<(u32, u32, Vec<u8>)> = None;
        if let Some((run, options)) = hd.as_mut() {
            if !wanted(frame) {
                run.note(machine.wram(), machine.frame_log());
                if let Some(tally) = gaps_run.as_mut() {
                    let _ = tally.note_hd(machine.wram(), machine.frame_log(), gaps_options);
                }
                continue;
            }
            match run.note_hd(machine.wram(), machine.frame_log(), *options) {
                Some(shown) => {
                    let (w, h) = (shown.picture.width, shown.picture.height);
                    picture = Some((w, h, shown.picture.rgba));
                    line = format!("frame {frame:6} hd {w}x{h}");
                }
                None => line = format!("frame {frame:6} hd not drawn"),
            }
            if let Some(tally) = gaps_run.as_mut() {
                let shown = tally.note_hd(machine.wram(), machine.frame_log(), gaps_options);
                if let Some(dir) = &args.gaps {
                    if let Some(picture) = shown.as_ref() {
                        let _ = std::fs::write(
                            dir.join(format!("gaps-run-{frame:06}.png")),
                            ssd_hd::png::encode_rgba(
                                picture.picture.width,
                                picture.picture.height,
                                &picture.picture.rgba,
                            ),
                        );
                    }
                    let mode = machine.wram()[0x24];
                    let field = FieldState::from_wram(machine.wram());
                    if let Some(log) = machine.frame_log() {
                        let scene = ssd_hd::world::classify(Rom(rom), mode, field.as_ref(), log);
                        writeln!(
                            out,
                            "frame {frame} mode {mode:02X} scene {scene:?} field_frame {}",
                            matches!(mode & 0x7F, 0x00 | 0x01 | 0x03 | 0x05 | 0x06)
                        )?;
                    }
                    let stats = tally.art_stats();
                    for p in &stats.poses {
                        let kind = if p.field {
                            "field"
                        } else if p.fighter {
                            "fighter"
                        } else {
                            "effect"
                        };
                        writeln!(
                            out,
                            "pose {:016x} {kind} {:<6} pal {} vis {:5} art {:?} src {:?} pic {} exc {:016x}",
                            p.key,
                            if p.matched { "art" } else { "NONE" },
                            p.palette,
                            p.visible,
                            p.art,
                            p.source,
                            if p.picture { "P" } else { "-" },
                            p.excluded
                        )?;
                    }
                    for p in tally.portrait_uses() {
                        writeln!(
                            out,
                            "portrait {} at {},{} cells {:016x} pics {:?} rows {:?} dist {} bare {}",
                            p.name(),
                            p.x,
                            p.y,
                            p.cells,
                            p.pictures,
                            p.rows,
                            p.distance,
                            p.bare
                        )?;
                        if let Some(fault) = p.fault() {
                            writeln!(out, "portrait fault: {fault}")?;
                        }
                        let box_map = tally.picture_map(p.x, p.y, 32, 32);
                        let mut letters: Vec<(u32, char)> = Vec::new();
                        let mut next = b'A';
                        for line in box_map.chunks(32) {
                            let mut row = String::new();
                            for (kind, picture, _) in line {
                                let ch = if let Some(picture) = *picture {
                                    match letters.iter().find(|(id, _)| *id == picture) {
                                        Some((_, letter)) => *letter,
                                        None => {
                                            let letter = char::from(next);
                                            next = next.saturating_add(1).min(b'Z');
                                            letters.push((picture, letter));
                                            letter
                                        }
                                    }
                                } else {
                                    char::from(*kind)
                                };
                                row.push(ch);
                            }
                            writeln!(out, "map {row}")?;
                        }
                        writeln!(out, "map pictures {letters:?}")?;
                    }
                    if let Some((x, y, w, h)) = args.box_map {
                        let map = tally.picture_map(x, y, w, h);
                        let mut letters: Vec<(u32, char)> = Vec::new();
                        let mut next = b'A';
                        for line in map.chunks(w) {
                            let mut row = String::new();
                            for (kind, picture, _) in line {
                                let ch = if let Some(picture) = *picture {
                                    match letters.iter().find(|(id, _)| *id == picture) {
                                        Some((_, letter)) => *letter,
                                        None => {
                                            let letter = char::from(next);
                                            next = next.saturating_add(1).min(b'Z');
                                            letters.push((picture, letter));
                                            letter
                                        }
                                    }
                                } else {
                                    char::from(*kind)
                                };
                                row.push(ch);
                            }
                            writeln!(out, "box {row}")?;
                        }
                        writeln!(out, "box pictures {letters:?}")?;
                    }
                    for (entry, bg, mode, map_base, char_base, size, field) in
                        tally.bg_slot_report()
                    {
                        writeln!(
                            out,
                            "bg entry {entry} bg {bg} mode {mode:02X} map {map_base:04X} char \
                             {char_base:04X} size {size} field {field}"
                        )?;
                    }
                    if let Some(log) = machine.frame_log()
                        && let Some(line) = log.entries.first()
                    {
                        let regs = &line.regs;
                        writeln!(
                            out,
                            "regs mode {} main {:?} sizes {:?} maps {:04X?} chars {:04X?} scrolls h {:04X?} v {:04X?}",
                            regs.bg_mode,
                            regs.main_bg,
                            regs.bg_screen_size,
                            regs.bg_map_base,
                            regs.bg_char_base,
                            regs.bg_h_scroll,
                            regs.bg_v_scroll
                        )?;
                    }
                    if let Some(log) = machine.frame_log()
                        && log
                            .entries
                            .iter()
                            .any(|e| !e.regs.forced_blanking && e.regs.bg_mode == 7)
                    {
                        let key = ssd_hd::mode7art::plane_key(&log.vram);
                        writeln!(
                            out,
                            "mode7 plane key {key:016x} vram writes {}",
                            log.vram_writes.len()
                        )?;
                        let plane = dir.join(format!("m7-{key:016x}"));
                        if !plane.join("key.txt").is_file() {
                            crate::hd_mode7::dump_log(log, machine.wram()[0x24], &plane, out)?;
                        }
                    }
                    write_gaps(dir, &tally.tally(), frame, &mut gaps_written);
                }
            }
        } else {
            if !wanted(frame) {
                continue;
            }
            let (w, h, pixels) = rgba(&machine);
            line = format!("frame {frame:6} crc {:08X}", ssd_rom::crc32(&pixels));
            picture = Some((w, h, pixels));
        }
        for p in &peeks {
            let bytes = &machine.wram()[p.offset..p.offset + p.size];
            let value = bytes.iter().rev().fold(String::new(), |mut text, b| {
                let _ = write!(text, "{b:02X}");
                text
            });
            let _ = write!(line, " {}={value}", p.label);
        }
        if !args.no_png
            && let Some((w, h, pixels)) = &picture
        {
            if args.sheet.is_some() && hd.is_none() {
                tiles.push(halve(*w, *h, pixels));
                labels.push(frame);
            } else {
                std::fs::create_dir_all(&dir)?;
                let path = dir.join(format!("f{frame:06}.png"));
                std::fs::write(&path, encode_png(*w, *h, pixels))
                    .with_context(|| format!("writing {}", path.display()))?;
                let _ = write!(line, " {}", path.display());
            }
        }
        writeln!(out, "{line}")?;
    }
    if hd.is_none()
        && let (Some(cols), false) = (args.sheet, tiles.is_empty())
    {
        let (w, h, pixels) = sheet(&tiles, cols);
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("sheet_{}.png", labels[0]));
        std::fs::write(&path, encode_png(w, h, &pixels))?;
        let list: Vec<String> = labels.iter().map(u64::to_string).collect();
        writeln!(out, "sheet {} ({cols} columns, frames {})", path.display(), list.join(" "))?;
    }
    if let Some(path) = &args.wram_out {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, machine.wram())?;
        writeln!(out, "WRAM at frame {} written to {}", machine.frame_count(), path.display())?;
    }
    if let Some(path) = &args.sram_out {
        let sram = machine.sram().context("the cartridge has no SRAM")?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, sram)?;
        writeln!(out, "SRAM at frame {} written to {}", machine.frame_count(), path.display())?;
    }
    if let Some(path) = &args.state_out {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, machine.save_state())?;
        writeln!(out, "state at frame {} written to {}", machine.frame_count(), path.display())?;
    }
    Ok(())
}

/// Writes the poses of `tally` that no art holds, one JSON per pose and each pose once (see
/// `hd-tour --gaps`): the key, the palette-index array and the palette row the art is made from.
fn write_gaps(
    dir: &Path,
    tally: &ssd_hd::FrameTally,
    frame: u64,
    written: &mut std::collections::HashSet<String>,
) {
    let _ = std::fs::create_dir_all(dir);
    for shape in &tally.shapes {
        let name = format!(
            "{}-{}-{:016x}",
            if shape.field {
                "field"
            } else if shape.fighter {
                "fighter"
            } else {
                "effect"
            },
            if shape.fate == ssd_hd::Fate::Partial { "partial" } else { "missing" },
            shape.key
        );
        if !written.insert(name.clone()) || dir.join(format!("{name}.json")).exists() {
            continue;
        }
        if !shape.rgba.is_empty() {
            let _ = ssd_hd::png::write_rgba(
                &dir.join(format!("{name}.png")),
                shape.w as u32,
                shape.h as u32,
                &shape.rgba,
            );
        }
        let json = serde_json::json!({
            "key": format!("{:016x}", shape.key),
            "fighter": shape.fighter,
            "field": shape.field,
            "shadow": shape.shadow,
            "fate": format!("{:?}", shape.fate),
            "width": shape.w,
            "height": shape.h,
            "origin": [shape.org.0, shape.org.1],
            "oam_palette": shape.palette,
            "palette_row": shape.row,
            "indexes": shape.arr,
            "first_seen": format!("state frame {frame}"),
        });
        let _ = std::fs::write(dir.join(format!("{name}.json")), json.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arguments_need_a_script_and_a_way_to_pick_frames() {
        let a = parse_args(&[
            "menus",
            "--frames",
            "10,20",
            "--sheet",
            "4",
            "--peek",
            "in_battle,7E0010",
        ])
        .unwrap();
        assert_eq!(a.script, "menus");
        assert_eq!(a.frames, [10, 20]);
        assert_eq!(a.sheet, Some(4));
        assert_eq!(a.peek, ["in_battle", "7E0010"]);
        assert!(parse_args(&["menus", "--every", "300", "--no-png"]).unwrap().no_png);
        assert!(parse_args(&["menus", "--state-out", "s"]).is_ok());
        for bad in
            [&[][..], &["menus"], &["a", "b", "--every", "3"], &["a", "--every"], &["a", "--zz"]]
        {
            assert!(parse_args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn watch_and_save_options_are_parsed_and_enough_to_run() {
        let a = parse_args(&[
            "x.ssdreplay",
            "--watch",
            "7E01D8,scene_id",
            "--watch-from",
            "2200",
            "--sram-out",
            "o.sav",
        ])
        .unwrap();
        assert_eq!(a.watch, ["7E01D8", "scene_id"]);
        assert_eq!(a.watch_from, Some(2200));
        assert_eq!(a.sram_out, Some(PathBuf::from("o.sav")));
        assert!(parse_args(&["x", "--sram-out", "o.sav"]).is_ok());
        assert!(parse_args(&["x", "--watch", "scene_id"]).is_ok());
        assert!(parse_args(&["x", "--watch"]).is_err());
        assert!(parse_args(&["x", "--watch-from", "soon"]).is_err());
    }

    #[test]
    fn a_host_write_is_a_frame_a_work_ram_address_and_hex_bytes() {
        let pokes =
            resolve_pokes(&["2400:7E0C36=80".to_owned(), "3000:$7E:0B01=3B00".to_owned()]).unwrap();
        assert_eq!(
            pokes,
            [
                Poke { frame: 2400, address: 0x7E_0C36, bytes: vec![0x80] },
                Poke { frame: 3000, address: 0x7E_0B01, bytes: vec![0x3B, 0x00] },
            ]
        );
        for bad in [
            "2400:0C36=80",
            "x:7E0C36=80",
            "2400:7E0C36=8",
            "2400:7E0C36=",
            "2400:7E0C36=GG",
            "2400:7E0C36",
        ] {
            assert!(resolve_pokes(&[bad.to_owned()]).is_err(), "{bad}");
        }
    }

    #[test]
    fn mods_are_repeatable_and_need_a_pack() {
        let a = parse_args(&[
            "x.ssdreplay",
            "--frames",
            "10",
            "--mods",
            "a.ssdpack",
            "--mods",
            "folder",
        ])
        .unwrap();
        assert_eq!(a.mods, [PathBuf::from("a.ssdpack"), PathBuf::from("folder")]);
        assert!(parse_args(&["x", "--frames", "10", "--mods"]).unwrap_err().contains("--mods"));
        assert!(
            parse_args(&["x", "--frames", "10"]).unwrap().mods.is_empty(),
            "no packs by default"
        );
    }

    #[test]
    fn a_script_becomes_a_mesen_timeline_of_runs() {
        let pad = |frame: u64| Joypad { a: frame >= 3, start: frame == 1, ..Joypad::default() };
        let text = mesen_timeline(&pad, 5);
        let rows: Vec<&str> = text.lines().filter(|l| !l.starts_with('#')).collect();
        assert_eq!(rows, ["1 -", "1 start", "1 -", "2 a", "@mode idle"]);
    }

    #[test]
    fn a_png_has_its_signature_header_and_a_valid_length() {
        let pixels = vec![0x80_u8; 4 * 4 * 4];
        let png = encode_png(4, 4, &pixels);
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
        // 4 rows of (1 filter byte + 16 pixel bytes), one stored block, zlib header and Adler-32.
        let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
        let len = u32::from_be_bytes(png[idat - 4..idat].try_into().unwrap());
        assert_eq!(len as usize, 2 + 5 + 4 * 17 + 4);
    }

    #[test]
    fn a_sheet_places_half_size_tiles_in_a_grid() {
        let tile = (4_u32, 4_u32, vec![0xFF_u8; 64]);
        let (w, h, pixels) = sheet(&[tile.clone(), tile.clone(), tile], 2);
        assert_eq!((w, h), (2 * 6 + 2, 2 * 6 + 2));
        assert_eq!(pixels.len(), (w * h * 4) as usize);
        let (hw, hh, half) = halve(4, 4, &[0x40_u8; 64]);
        assert_eq!((hw, hh, half.len()), (2, 2, 16));
        assert_eq!(&half[..4], &[0x40, 0x40, 0x40, 0xFF]);
    }

    #[test]
    fn a_timeline_file_is_a_script() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.txt");
        std::fs::write(&path, "2 -\n3 a,start\n").unwrap();
        let script = load_script(path.to_str().unwrap()).unwrap();
        assert_eq!(script.frames, 5);
        assert!(!(script.inputs)(1).a && (script.inputs)(2).a && (script.inputs)(4).start);
        assert!(!(script.inputs)(5).a, "released after the end");
        assert!(load_script("nothing-like-this").is_err());
    }

    #[test]
    fn the_enhanced_look_is_crisp_unless_asked_for_smooth() {
        let crisp = parse_args(&["menus", "--frames", "1", "--hd-scale", "4"]).unwrap();
        assert_eq!(crisp.look, Look::Crisp);
        let smooth =
            parse_args(&["menus", "--frames", "1", "--hd-scale", "4", "--enhanced-look", "smooth"])
                .unwrap();
        assert_eq!(smooth.look, Look::Smooth);
        assert!(parse_args(&["menus", "--frames", "1", "--enhanced-look", "sharp"]).is_err());
    }
}
