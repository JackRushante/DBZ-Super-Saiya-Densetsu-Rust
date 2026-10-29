//! `save-edit`, `save-dump` and `replay-from-save`: crafting battery saves and the replays that
//! start from them. The save format is described in `docs/re/data/save-format.md`.
//!
//! The named fields of `--set` need the game's own tables (the level rows, the names, the item
//! attributes), which are read from the cached ROM; the raw forms (`image:OFFSET=HEX`,
//! `page:OFFSET=HEX`) do not.

use anyhow::{Context, bail};
use ssd_catalogue::game::{new_record, set_level};
use ssd_game::scripts;
use ssd_oracle::{CORE_VERSION, HostEvent, Machine, MemorySaveStore, Replay};
use ssd_save::{FLAG_BYTES, Item, Position, SaveFile, Slot, SlotStatus};

/// Story flags in a slot.
const FLAG_BITS: usize = FLAG_BYTES * 8;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The command line of `save-edit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditArgs {
    pub input: PathBuf,
    pub output: PathBuf,
    /// The slot to edit, 0 to 2 (the command line counts from 1).
    pub slot: usize,
    /// Where to write the edited slot in the output (default: the same slot).
    pub to_slot: Option<usize>,
    /// `(field, value)` in the order given.
    pub sets: Vec<(String, String)>,
}

impl EditArgs {
    /// Whether some `--set` needs the game's tables (every one but the raw forms).
    #[must_use]
    pub fn needs_game(&self) -> bool {
        self.sets.iter().any(|(field, _)| !is_raw(field))
    }
}

fn is_raw(field: &str) -> bool {
    field.starts_with("image:") || field.starts_with("page:")
}

fn slot_number(value: Option<&&str>) -> Result<usize, String> {
    value
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| (1..=ssd_save::SLOT_COUNT).contains(n))
        .map(|n| n - 1)
        .ok_or_else(|| "--slot takes 1, 2 or 3".to_owned())
}

/// Parses `<in.sav> <out.sav> [--slot N] [--to-slot N] [--set field=value ...]`.
///
/// # Errors
///
/// A usage message.
pub fn parse_edit_args(args: &[&str]) -> Result<EditArgs, String> {
    let mut paths = Vec::new();
    let mut slot = 0;
    let mut to_slot = None;
    let mut sets = Vec::new();
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--slot" => slot = slot_number(it.next())?,
            "--to-slot" => to_slot = Some(slot_number(it.next())?),
            "--set" => {
                let v = it.next().ok_or("--set takes field=value")?;
                let (field, value) =
                    v.split_once('=').ok_or_else(|| format!("`{v}` is not field=value"))?;
                sets.push((field.to_owned(), value.to_owned()));
            }
            option if option.starts_with("--") => return Err(format!("unknown option `{option}`")),
            path => paths.push(PathBuf::from(path)),
        }
    }
    let [input, output] = <[PathBuf; 2]>::try_from(paths)
        .map_err(|_| "needs an input and an output save file".to_owned())?;
    Ok(EditArgs { input, output, slot, to_slot, sets })
}

/// The command line of `replay-from-save`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayArgs {
    pub save: PathBuf,
    pub script: PathBuf,
    pub output: PathBuf,
    /// The slot Continue loads, 0 to 2.
    pub slot: usize,
    /// Stop at the title's slot list (no Continue): the script starts on it.
    pub title: bool,
    /// Write the scripted-loss check's extreme cheats into the cheat block at frame 300, in the enhanced mode.
    pub cheats: bool,
    /// Record the save's item list as the bag of an enhanced run, bound at this frame (see `replay_from_save`).
    pub bind: Option<usize>,
}

/// Parses `<save.sav> <script.txt> <out.ssdreplay> [--slot N] [--title] [--cheats] [--bind FRAME]`.
///
/// # Errors
///
/// A usage message.
pub fn parse_replay_args(args: &[&str]) -> Result<ReplayArgs, String> {
    let mut paths = Vec::new();
    let mut slot = 0;
    let mut title = false;
    let mut cheats = false;
    let mut bind = None;
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--slot" => slot = slot_number(it.next())?,
            "--title" => title = true,
            "--cheats" => cheats = true,
            "--bind" => bind = Some(frame_number(it.next())?),
            option if option.starts_with("--") => return Err(format!("unknown option `{option}`")),
            path => paths.push(PathBuf::from(path)),
        }
    }
    let [save, script, output] = <[PathBuf; 3]>::try_from(paths)
        .map_err(|_| "needs a save file, a script and an output replay".to_owned())?;
    Ok(ReplayArgs { save, script, output, slot, title, cheats, bind })
}

/// The frame number after `--bind`.
fn frame_number(value: Option<&&str>) -> Result<usize, String> {
    value
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| "--bind takes a frame number".to_owned())
}

/// The title screen up to the slot list of Continue (`--title`): Start, Right to Continue, Start.
const TITLE_SLOTS_PREFIX: &str = "1700 -\n4 start\n60 -\n4 right\n20 -\n4 start\n100 -\n";
/// The frame the cheat block is written at for `--cheats`: after the game's intro, which does not clear it.
const CHEATS_WRITE_AT: usize = 300;

fn read_save(path: &Path) -> anyhow::Result<SaveFile> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    SaveFile::from_bytes(&bytes).with_context(|| path.display().to_string())
}

/// The highest map number that loads (`$01A0`); from 48 up the game never comes back from the
/// load (observed: the emulation hangs), and 32 to 47 all show the same cave.
const MAP_MAX: u8 = 0x2F;

pub use ssd_catalogue::Game;

fn number<T: std::str::FromStr>(field: &str, value: &str) -> anyhow::Result<T> {
    value.trim().parse().map_err(|_| anyhow::anyhow!("`{field}`: `{value}` is not a number"))
}

fn pair(field: &str, value: &str) -> anyhow::Result<Position> {
    let (x, y) = value.split_once(',').with_context(|| format!("`{field}` takes x,y"))?;
    Ok(Position { x: number(field, x)?, y: number(field, y)? })
}

/// Applies a raw `image:OFFSET=HEX` or `page:OFFSET=HEX`; false when `field` is not one.
fn apply_raw(slot: &mut Slot, field: &str, value: &str) -> anyhow::Result<bool> {
    let Some((area, offset)) = field.split_once(':') else {
        return Ok(false);
    };
    let offset = usize::from_str_radix(offset.trim_start_matches("0x"), 16)
        .with_context(|| format!("`{field}`: the offset is hexadecimal"))?;
    if !value.len().is_multiple_of(2) || !value.is_ascii() {
        bail!("`{value}` is not a string of hexadecimal bytes");
    }
    let bytes: Vec<u8> = (0..value.len() / 2)
        .map(|i| u8::from_str_radix(&value[2 * i..2 * i + 2], 16))
        .collect::<Result<_, _>>()
        .with_context(|| format!("`{value}` is not a string of hexadecimal bytes"))?;
    for (i, byte) in bytes.into_iter().enumerate() {
        match area {
            "image" if offset + i < ssd_save::IMAGE_LEN => slot.set_image_byte(offset + i, byte),
            "page" if offset + i < ssd_save::PAGE_LEN => slot.set_page_byte(offset + i, byte),
            _ => bail!("`{field}`: not inside the {area}"),
        }
    }
    Ok(true)
}

/// `flags=` takes a list of `+N` (set flag N), `-N` (clear it) or `none` (clear all), in order;
/// N is decimal or `0x` hexadecimal.
fn apply_flags(slot: &mut Slot, value: &str) -> anyhow::Result<()> {
    for item in value.split(',').map(str::trim).filter(|i| !i.is_empty()) {
        if item == "none" {
            slot.clear_flags();
            continue;
        }
        let (set, number) = match item.split_at(1) {
            ("+", n) => (true, n),
            ("-", n) => (false, n),
            _ => bail!("flag `{item}`: write +N, -N or none"),
        };
        let n = match number.strip_prefix("0x") {
            Some(hex) => usize::from_str_radix(hex, 16),
            None => number.parse(),
        }
        .with_context(|| format!("flag `{item}`: not a number"))?;
        slot.set_flag(n, set)?;
    }
    Ok(())
}

/// Applies one `--set` to a slot. The fields:
///
/// - `party=goku,piccolo,...`: the party in this order (names or ids); a character with no record
///   in use gets a new one at level 1;
/// - `<character>.level=N`: the stats of that level (battle power, HP, KI, EXP, HP and KI full);
///   `<character>.hp|ki|maxhp|maxki|bp|exp=N` set one value (after the level);
/// - `zeni=N`; `items=turtle,bulma,...` (names or object ids, at most ten, replacing the list);
/// - `leader=x,y` and `follower=x,y` (map pixels); `map=N`;
/// - `flags=+3,-0x1F` or `flags=none`: story flags (see [`apply_flags`]); `scene=N`: the scene id;
/// - `page:OFFSET=HEX`, `image:OFFSET=HEX`: raw bytes (hexadecimal offset).
///
/// # Errors
///
/// An unknown field or character, or a value out of range.
pub fn apply(slot: &mut Slot, game: Option<&Game>, field: &str, value: &str) -> anyhow::Result<()> {
    if apply_raw(slot, field, value)? {
        return Ok(());
    }
    match field {
        "flags" => return apply_flags(slot, value),
        "scene" => {
            slot.set_scene_id(number(field, value)?);
            return Ok(());
        }
        _ => {}
    }
    let game = game.with_context(|| format!("`{field}` needs the game's tables (the ROM)"))?;
    match field {
        "zeni" => slot.set_zeni(number(field, value)?)?,
        "map" => {
            let id: u8 = number(field, value)?;
            if id > MAP_MAX {
                bail!(
                    "map {id}: the maps seen go from 0 to {MAP_MAX} (above it the game hangs: no map data)"
                );
            }
            slot.set_page_byte(0xA0, id);
        }
        "leader" | "follower" => {
            let position = pair(field, value)?;
            let (leader, follower) = (slot.leader(), slot.follower());
            if field == "leader" {
                slot.set_position(position, follower);
            } else {
                slot.set_position(leader, position);
            }
        }
        "items" => {
            let items: Vec<Item> = value
                .split(',')
                .filter(|s| !s.trim().is_empty())
                .map(|s| game.item(s.trim()))
                .collect::<Result<_, _>>()?;
            slot.set_items(&items)?;
        }
        "party" => {
            let mut ids = Vec::new();
            for spec in value.split(',').filter(|s| !s.trim().is_empty()) {
                let character = game.character(spec.trim())?;
                let joined = ids.len();
                let record = slot.record(character.id)?;
                if record.flags() != 0xC0 {
                    let order = u8::try_from(joined + 1).expect("at most nine");
                    let record = if character.levels.is_empty() {
                        game.unit_record(character, order)?
                    } else {
                        new_record(character, 1, order)?
                    };
                    slot.set_record(character.id, &record)?;
                }
                ids.push(character.id);
            }
            slot.set_party(&ids)?;
        }
        _ => {
            let (who, what) =
                field.split_once('.').ok_or_else(|| anyhow::anyhow!("unknown field `{field}`"))?;
            let character = game.character(who)?;
            let mut record = slot.record(character.id)?;
            if record.flags() != 0xC0 {
                bail!(
                    "{} is not in the save: add it to `party` first (or `{who}.level` after it)",
                    character.name.trimmed()
                );
            }
            match what {
                "level" => set_level(&mut record, character, number(field, value)?)?,
                "hp" => record.set_hp(number(field, value)?),
                "ki" => record.set_ki(number(field, value)?),
                "maxhp" => record.set_max_hp(number(field, value)?),
                "maxki" => record.set_max_ki(number(field, value)?),
                "bp" => record.set_bp(number(field, value)?),
                "exp" => record.set_exp(number(field, value)?),
                other => bail!("unknown field `{who}.{other}`"),
            }
            slot.set_record(character.id, &record)?;
        }
    }
    Ok(())
}

/// `save-edit`.
///
/// # Errors
///
/// A file, a field or a value is bad.
pub fn edit(
    args: &EditArgs,
    game: Option<&Game>,
    out: &mut dyn std::io::Write,
) -> anyhow::Result<()> {
    let mut file = read_save(&args.input)?;
    let mut slot = file.slot(args.slot).with_context(|| args.input.display().to_string())?;
    for (field, value) in &args.sets {
        apply(&mut slot, game, field, value).with_context(|| format!("--set {field}={value}"))?;
    }
    let target = args.to_slot.unwrap_or(args.slot);
    file.set_slot(target, &slot)?;
    if let Some(dir) = args.output.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&args.output, file.as_bytes())
        .with_context(|| format!("writing {}", args.output.display()))?;
    writeln!(
        out,
        "slot {} of {} written as slot {} to {}",
        args.slot + 1,
        args.input.display(),
        target + 1,
        args.output.display()
    )?;
    Ok(())
}

/// The decoded fields of a slot as text, with names where the game is known.
#[must_use]
pub fn describe(slot: &Slot, game: Option<&Game>) -> String {
    let mut text = String::new();
    let name = |id: u8| {
        game.and_then(|g| g.catalogue.characters.get(usize::from(id)))
            .map_or_else(|| format!("#{id}"), |c| c.name.trimmed().to_owned())
    };
    for id in slot.party() {
        let Ok(r) = slot.record(id) else { continue };
        let _ = writeln!(
            text,
            "  {:<9} id {id:<2} level {:<2} HP {}/{} KI {}/{} BP {} EXP {} flags {:02X}",
            name(id),
            r.level(),
            r.hp(),
            r.max_hp(),
            r.ki(),
            r.max_ki(),
            r.bp(),
            r.exp(),
            r.flags()
        );
    }
    let items: Vec<String> = slot
        .items()
        .iter()
        .map(|i| {
            game.and_then(|g| g.catalogue.items.get(usize::from(i.id)))
                .map_or_else(|| format!("#{}", i.id), |o| o.name.trimmed().to_owned())
        })
        .collect();
    let _ = writeln!(text, "  items: {}", items.join(", "));
    let _ = writeln!(text, "  zeni: {}", slot.zeni());
    let (l, f) = (slot.leader(), slot.follower());
    let _ = writeln!(
        text,
        "  map {} leader ({}, {}) follower ({}, {}) scene {:#04X}",
        slot.map_id(),
        l.x,
        l.y,
        f.x,
        f.y,
        slot.scene_id()
    );
    let flags: Vec<String> =
        (0..FLAG_BITS).filter(|&n| slot.flag(n)).map(|n| format!("{n:#04X}")).collect();
    let _ = writeln!(
        text,
        "  story flags set: {}",
        if flags.is_empty() { "none".to_owned() } else { flags.join(" ") }
    );
    text
}

/// `save-dump`: the status of each slot and what the used ones hold.
///
/// # Errors
///
/// The file is bad.
pub fn dump(path: &Path, game: Option<&Game>, out: &mut dyn std::io::Write) -> anyhow::Result<()> {
    let file = read_save(path)?;
    for i in 0..ssd_save::SLOT_COUNT {
        let status = file.status(i)?;
        writeln!(out, "slot {}: {status:?}", i + 1)?;
        if status != SlotStatus::Empty {
            let slot = file.slot(i)?;
            write!(out, "{}", describe(&slot, game))?;
        }
    }
    Ok(())
}

/// The input from power-on to the first frame the player controls after Continue loaded slot
/// `slot`: the title screen's Start, Right to Continue, Start, Down to the slot, A on it, A on
/// Yes. Frames are counted from power-on; the game is on its map from about frame 2 300.
#[must_use]
pub fn continue_prefix(slot: usize) -> String {
    let mut text = String::from(
        "# power-on, title screen, Continue\n1700 -\n4 start\n60 -\n4 right\n20 -\n4 start\n100 -\n",
    );
    for _ in 0..slot {
        text.push_str("4 down\n14 -\n");
    }
    text.push_str("4 a\n200 -\n8 a\n200 -\n");
    text
}

/// The replay for a save and a script: power-on with the save in the cartridge's SRAM, then
/// [`continue_prefix`], then the script's frames.
///
/// # Errors
///
/// A file or the script is bad, the slot is empty, or the cartridge does not take the save.
pub fn replay_from_save(
    rom: &[u8],
    args: &ReplayArgs,
    out: &mut dyn std::io::Write,
) -> anyhow::Result<()> {
    let file = read_save(&args.save)?;
    if file.status(args.slot)? == SlotStatus::Empty {
        bail!("slot {} of {} is empty", args.slot + 1, args.save.display());
    }
    let text = std::fs::read_to_string(&args.script)
        .with_context(|| format!("reading {}", args.script.display()))?;
    let prefix =
        if args.title { TITLE_SLOTS_PREFIX.to_owned() } else { continue_prefix(args.slot) };
    let timeline = format!("{prefix}{text}");
    let inputs = scripts::parse_timeline(&timeline)
        .map_err(|e| anyhow::anyhow!("{}: {e}", args.script.display()))?;
    let mut saves = MemorySaveStore::default();
    saves.0.insert("sav".to_owned(), file.as_bytes().to_vec());
    let machine = Machine::new(rom.to_vec(), Box::new(saves))?;
    let mut replay = Replay::of(&machine);
    anyhow::ensure!(
        replay.initial_sram.as_deref() == Some(file.as_bytes()),
        "the cartridge did not take the save as its SRAM"
    );
    let frames = inputs.len();
    replay.inputs = inputs;
    if args.cheats {
        // The extreme choices of the scripted-loss check (defence x10, damage x10, enemy power x5, EXP x10).
        let mut extreme = ssd_menus::cheats::Cheats::neutral();
        extreme.set_index(ssd_menus::cheats::Setting::Defence, 5);
        extreme.set_index(ssd_menus::cheats::Setting::Damage, 5);
        extreme.set_index(ssd_menus::cheats::Setting::Power, 4);
        extreme.set_index(ssd_menus::cheats::Setting::Exp, 4);
        replay.mode = ssd_oracle::MODE_ENHANCED;
        replay.events = (0..frames)
            .map(|frame| {
                if frame == CHEATS_WRITE_AT {
                    ssd_game::cheats::block_events(&extreme)
                } else {
                    Vec::new()
                }
            })
            .collect();
    }
    if let Some(frame) = args.bind {
        // The save's list as the bag of an enhanced run, recorded as its bind: the app binds a replay only from its
        // recorded bind event. Each entry is a unit with state word 0 (the slot's items keep no state words here).
        let mut bag = ssd_menus::bag::Bag::new(ssd_menus::bag::Rules::default());
        for item in file.slot(args.slot)?.items() {
            let _ = bag.add(item.id, 0, ssd_menus::bag::Class::of(item.id, false));
        }
        if replay.events.len() != frames {
            replay.events = vec![Vec::new(); frames];
        }
        replay.mode = ssd_oracle::MODE_ENHANCED;
        let at = frame.min(frames.saturating_sub(1));
        replay.events[at].push(HostEvent::App {
            tag: ssd_game::menu::keeper::BIND_TAG,
            payload: bag.to_bytes(),
        });
    }
    replay.write_to(&args.output, CORE_VERSION)?;
    writeln!(
        out,
        "{} frames ({} before the script) from slot {} of {} written to {}",
        replay.frames(),
        prefix
            .lines()
            .filter_map(|l| l.split_whitespace().next()?.parse::<u64>().ok())
            .sum::<u64>(),
        args.slot + 1,
        args.save.display(),
        args.output.display()
    )?;
    Ok(())
}

/// `chapter-save`: the save slot a chapter of the packs starts from (`ssd_mod::chapter::synthesize`, with the game's tables), as
/// `<out>/ssd.sav`, and, when the chapter sets extended flags, the `ssd.mods` sidecar of that slot beside it. A chapter session
/// starts from the same two files in its own folder. The tool never overwrites a file.
///
/// # Errors
///
/// A usage problem, a set of packs the checks refuse, no chapter of that id, a table the game does not give, or a file in `out`
/// that exists already.
pub fn chapter_save(args: &[&str], rom: &[u8]) -> anyhow::Result<String> {
    let mut packs = Vec::new();
    let mut id = None;
    let mut out = None;
    let mut it = args.iter();
    while let Some(&arg) = it.next() {
        match arg {
            "--chapter" => id = Some((*it.next().context("--chapter takes an id")?).to_owned()),
            "--out" => out = Some(PathBuf::from(*it.next().context("--out takes a folder")?)),
            option if option.starts_with("--") => bail!("unknown option `{option}`"),
            path => packs.push(PathBuf::from(path)),
        }
    }
    let (Some(id), Some(out)) = (id, out) else {
        bail!("chapter-save takes one or more packs, then --chapter <id> and --out <folder>");
    };
    anyhow::ensure!(!packs.is_empty(), "chapter-save needs at least one pack");
    let packs = packs
        .iter()
        .map(|path| ssd_mod::Pack::open(path))
        .collect::<Result<Vec<_>, _>>()
        .context("reading the packs")?;
    if let Err(refused) = ssd_mod::check_set(rom, &packs, ssd_mod::CheckOptions::default()) {
        let lines: Vec<String> = refused.iter().map(ToString::to_string).collect();
        bail!("the packs are refused:\n{}", lines.join("\n"));
    }
    let mut chapters = Vec::new();
    for pack in &packs {
        for entry in &pack.manifest.chapters {
            chapters.extend(pack.chapters(entry)?.unwrap_or_default());
        }
    }
    let chapter = chapters
        .into_iter()
        .find(|chapter| chapter.id == id)
        .with_context(|| format!("no chapter `{id}` in the packs"))?;
    let game = Game::from_rom(rom)?;
    let synthesized = ssd_mod::chapter::synthesize(&game, &chapter)?;
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    let save = out.join("ssd.sav");
    anyhow::ensure!(!save.exists(), "{} exists: chapter-save does not overwrite", save.display());
    std::fs::write(&save, synthesized.save.as_bytes())
        .with_context(|| format!("writing {}", save.display()))?;
    let mut text = format!("wrote {} (chapter `{id}`)\n", save.display());
    if let Some(flagx) = synthesized.flagx {
        let at = ssd_save::SLOT_BASES[0] + ssd_save::CHECKSUM_OFFSET;
        let bytes = synthesized.save.as_bytes();
        let checksum = u16::from_le_bytes([bytes[at], bytes[at + 1]]);
        let mut sidecar = ssd_menus::mods_state::ModsSidecar::empty();
        sidecar.store(
            0,
            checksum,
            ssd_menus::mods_state::ModState { flags: Some(flagx), ..Default::default() },
        );
        let path = out.join("ssd.mods");
        anyhow::ensure!(
            !path.exists(),
            "{} exists: chapter-save does not overwrite",
            path.display()
        );
        std::fs::write(&path, sidecar.to_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        let _ = writeln!(text, "wrote {} (the extended flags of slot 1)", path.display());
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_catalogue::game::stats_at;
    use ssd_catalogue::{CardRules, LevelRow, Name};
    use ssd_catalogue::{Catalogue, Character};

    fn name(text: &str) -> Name {
        Name { tiles: Vec::new(), text: text.to_owned() }
    }

    /// Three characters with three level rows, one with none, and a few objects.
    fn game() -> Game {
        let rows = |bp: [u32; 3]| -> Vec<LevelRow> {
            (0..3)
                .map(|i| LevelRow {
                    level: u8::try_from(i + 1).unwrap(),
                    bp_gain: bp[i],
                    exp_start: [0, 70, 310][i],
                    max_hp: [80, 88, 97][i],
                    ki_gain: [12, 4, 4][i],
                    unknown_0a: 1,
                })
                .collect()
        };
        let character = |id: u8, text: &str, levels: Vec<LevelRow>| Character {
            id,
            name: name(text),
            unit_id: id + 20,
            levels,
            fixed_stats_unit: None,
            level_cap: 47,
        };
        let characters = vec![
            character(0, "Goku", rows([416, 84, 98])),
            character(1, "Gohan", rows([250, 60, 70])),
            character(2, "Piccolo", rows([408, 82, 98])),
            character(3, "Nail", Vec::new()),
        ];
        let items = [("Kami", 0), ("Turtle", 1), ("\"DB'1\"", 2)]
            .iter()
            .map(|&(text, id)| ssd_catalogue::Item {
                id,
                name: name(text),
                attribute: 0,
                price: None,
                effect: None,
                help: Vec::new(),
            })
            .collect();
        let catalogue = Catalogue {
            characters,
            enemies: Vec::new(),
            encounters: Vec::new(),
            cards: CardRules::from_table([1; 32]),
            attacks: Vec::new(),
            items,
        };
        Game::new(catalogue, vec![0xC8, 0xD2, 0x25])
    }

    fn set(slot: &mut Slot, field: &str, value: &str) -> anyhow::Result<()> {
        apply(slot, Some(&game()), field, value)
    }

    #[test]
    fn edit_arguments_take_two_files_slots_and_sets() {
        let a = parse_edit_args(&[
            "in",
            "out",
            "--slot",
            "2",
            "--to-slot",
            "3",
            "--set",
            "zeni=5",
            "--set",
            "a.b=1",
        ])
        .unwrap();
        assert_eq!((a.slot, a.to_slot, a.sets.len()), (1, Some(2), 2));
        assert_eq!(a.sets[0], ("zeni".to_owned(), "5".to_owned()));
        assert!(a.needs_game());
        let raw = parse_edit_args(&["in", "out", "--set", "page:B4=0100"]).unwrap();
        assert!(!raw.needs_game() && raw.to_slot.is_none());
        for bad in [
            &["in"][..],
            &["a", "b", "--slot", "4"],
            &["a", "b", "--to-slot", "0"],
            &["a", "b", "--set", "x"],
            &["a", "b", "--z"],
        ] {
            assert!(parse_edit_args(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn story_flags_and_the_scene_are_set_without_the_rom() {
        let mut slot = Slot::empty();
        apply(&mut slot, None, "flags", "+3, +0x1F,+9").unwrap();
        assert_eq!(slot.story_flags()[..4], [0x08, 0x02, 0x00, 0x80]);
        apply(&mut slot, None, "flags", "-3,-0x1F").unwrap();
        assert_eq!(slot.story_flags()[..4], [0, 2, 0, 0]);
        apply(&mut slot, None, "flags", "+127,none,+1").unwrap();
        assert_eq!((slot.flag(1), slot.flag(127)), (true, false));
        assert!(apply(&mut slot, None, "flags", "+128").is_err(), "past the last flag");
        assert!(apply(&mut slot, None, "flags", "3").is_err(), "needs + or -");
        apply(&mut slot, None, "scene", "52").unwrap();
        assert_eq!(slot.scene_id(), 52);
    }

    #[test]
    fn a_character_without_a_level_table_joins_with_the_stats_of_its_unit() {
        let mut game = game();
        let unit = |id: u8| ssd_catalogue::Enemy {
            id,
            name: name("Unit"),
            unknown_00: 3,
            level: 25,
            hp: 375,
            bp: 42_000,
            ki: 100,
            unknown_09: 30,
            unknown_0a: 3,
            exp_reward: 0,
            unknown_0d: 0,
            commands: Vec::new(),
        };
        game.catalogue.enemies = (0..24).map(unit).collect();
        let mut slot = Slot::empty();
        apply(&mut slot, Some(&game), "party", "nail").unwrap();
        let nail = slot.record(3).unwrap();
        assert_eq!((nail.flags(), nail.unit_id(), nail.character_id()), (0xC0, 23, 3));
        assert_eq!(
            (nail.level(), nail.hp(), nail.max_hp(), nail.bp(), nail.ki()),
            (25, 375, 375, 42_000, 100)
        );
        assert!(apply(&mut slot, Some(&game), "nail.level", "5").is_err(), "no level table");
    }

    #[test]
    fn replay_arguments_take_a_save_a_script_and_an_output() {
        let a = parse_replay_args(&["s.sav", "t.txt", "o.ssdreplay", "--slot", "3"]).unwrap();
        assert_eq!(a.slot, 2);
        assert!(parse_replay_args(&["s.sav", "t.txt"]).is_err());
    }

    #[test]
    fn raw_pokes_write_bytes_and_stay_inside_their_area() {
        let mut slot = Slot::empty();
        apply(&mut slot, None, "page:B4", "e703").unwrap();
        assert_eq!(slot.zeni(), 999);
        apply(&mut slot, None, "image:0x10", "ff").unwrap();
        assert_eq!(slot.image()[0x10], 0xFF);
        assert!(apply(&mut slot, None, "page:FF", "0102").is_err(), "runs past the page");
        assert!(apply(&mut slot, None, "page:00", "1").is_err(), "odd number of digits");
        assert!(apply(&mut slot, None, "zeni", "5").is_err(), "needs the game");
    }

    #[test]
    fn a_party_gets_records_and_a_level_sets_the_stats_of_its_row() {
        let mut slot = Slot::empty();
        set(&mut slot, "party", "goku, 2").unwrap();
        assert_eq!(slot.party(), [0, 2]);
        let goku = slot.record(0).unwrap();
        assert_eq!((goku.flags(), goku.unit_id(), goku.character_id()), (0xC0, 20, 0));
        assert_eq!(goku.level(), 1);
        assert_eq!((goku.bp(), goku.max_hp(), goku.max_ki(), goku.exp()), (416, 80, 12, 0));
        assert_eq!(slot.record(2).unwrap().0[0x1B], 2, "the join order");
        set(&mut slot, "goku.level", "3").unwrap();
        let goku = slot.record(0).unwrap();
        assert_eq!(goku.level(), 3);
        assert_eq!(
            (goku.bp(), goku.max_hp(), goku.max_ki(), goku.exp()),
            (416 + 84 + 98, 97, 20, 310)
        );
        assert_eq!((goku.hp(), goku.ki()), (97, 20), "full");
        set(&mut slot, "goku.hp", "5").unwrap();
        set(&mut slot, "goku.exp", "311").unwrap();
        assert_eq!((slot.record(0).unwrap().hp(), slot.record(0).unwrap().exp()), (5, 311));
        // A character already in use keeps its record when the party is set again.
        set(&mut slot, "party", "piccolo,goku").unwrap();
        assert_eq!(slot.party(), [2, 0]);
        assert_eq!(slot.record(0).unwrap().level(), 3);
        assert_eq!(slot.page()[0xBB], 2);
    }

    #[test]
    fn levels_and_characters_are_checked() {
        let mut slot = Slot::empty();
        set(&mut slot, "party", "goku").unwrap();
        assert!(set(&mut slot, "goku.level", "4").is_err(), "the table has three rows");
        assert!(set(&mut slot, "goku.level", "0").is_err());
        assert!(set(&mut slot, "gohan.level", "2").is_err(), "not in the party yet");
        assert!(set(&mut slot, "party", "bulma").is_err(), "no such character");
        assert!(set(&mut slot, "party", "nail").is_err(), "no level table");
        assert!(set(&mut slot, "goku.speed", "2").is_err());
        assert!(set(&mut slot, "nothing", "2").is_err());
    }

    #[test]
    fn items_come_from_names_or_ids_with_their_attribute() {
        let mut slot = Slot::empty();
        set(&mut slot, "items", "turtle, 0, DB'1, Kami#1").unwrap();
        assert_eq!(
            slot.items(),
            [
                Item { id: 1, attr: 0xD2 },
                Item { id: 0, attr: 0xC8 },
                Item { id: 2, attr: 0x25 },
                Item { id: 0, attr: 0xC8 }
            ]
        );
        assert!(set(&mut slot, "items", "scouter").is_err());
        assert!(set(&mut slot, "items", "kami#2").is_err(), "only one object of that name");
        assert!(set(&mut slot, "items", "7").is_err(), "past the attribute table");
        set(&mut slot, "items", "").unwrap();
        assert_eq!(slot.items().len(), 0);
    }

    #[test]
    fn money_places_and_maps_are_set_and_checked() {
        let mut slot = Slot::empty();
        set(&mut slot, "zeni", "250").unwrap();
        assert_eq!(slot.zeni(), 250);
        assert!(set(&mut slot, "zeni", "1000").is_err());
        set(&mut slot, "leader", "100,200").unwrap();
        set(&mut slot, "follower", "90,190").unwrap();
        assert_eq!(slot.leader(), Position { x: 100, y: 200 });
        assert_eq!(slot.follower(), Position { x: 90, y: 190 });
        assert!(set(&mut slot, "leader", "100").is_err());
        set(&mut slot, "map", "16").unwrap();
        assert_eq!(slot.map_id(), 16);
        assert!(set(&mut slot, "map", "48").is_err());
    }

    #[test]
    fn an_edit_writes_the_file_with_a_good_checksum_in_the_chosen_slot() {
        let dir = tempfile::tempdir().unwrap();
        let (input, output) = (dir.path().join("in.sav"), dir.path().join("out.sav"));
        let mut file = SaveFile::blank();
        let mut slot = Slot::empty();
        slot.set_zeni(5).unwrap();
        file.set_slot(0, &slot).unwrap();
        std::fs::write(&input, file.as_bytes()).unwrap();
        let args = EditArgs {
            input,
            output: output.clone(),
            slot: 0,
            to_slot: Some(2),
            sets: vec![("page:B4".to_owned(), "0900".to_owned())],
        };
        edit(&args, None, &mut Vec::new()).unwrap();
        let written = SaveFile::from_bytes(&std::fs::read(&output).unwrap()).unwrap();
        assert_eq!(written.status(0), Ok(SlotStatus::Good));
        assert_eq!(written.status(1), Ok(SlotStatus::Empty));
        assert_eq!(written.status(2), Ok(SlotStatus::Good));
        assert_eq!(written.slot(0).unwrap().zeni(), 5);
        assert_eq!(written.slot(2).unwrap().zeni(), 9);
        let mut text = Vec::new();
        dump(&output, None, &mut text).unwrap();
        let text = String::from_utf8(text).unwrap();
        assert!(text.contains("slot 2: Empty") && text.contains("zeni: 9"), "{text}");
    }

    #[test]
    fn the_continue_prefix_moves_the_cursor_to_the_slot() {
        let first = scripts::parse_timeline(&continue_prefix(0)).unwrap();
        let third = scripts::parse_timeline(&continue_prefix(2)).unwrap();
        assert_eq!(third.len(), first.len() + 2 * 18);
        assert!(first[1700].start && first[1764].right);
        assert_eq!(first.len(), 1700 + 4 + 60 + 4 + 20 + 4 + 100 + 4 + 200 + 8 + 200);
    }

    /// With the ROM (`SSD_ROM_PATH`, any accepted image) and a save (`SSD_SAVE_PATH`): the level
    /// rows give back the stats the game itself saved for the members of the first slot.
    #[test]
    #[ignore = "needs the ROM and a save file"]
    fn the_level_rows_rebuild_the_saved_stats() {
        let rom = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let rom = ssd_rom::normalize(&rom, None).unwrap().bytes;
        let game = Game::from_rom(&rom).unwrap();
        let save = SaveFile::from_bytes(
            &std::fs::read(std::env::var("SSD_SAVE_PATH").expect("SSD_SAVE_PATH")).unwrap(),
        )
        .unwrap();
        let slot = save.slot(0).unwrap();
        assert_ne!(slot.party().len(), 0);
        for id in slot.party() {
            let saved = slot.record(id).unwrap();
            let character = &game.catalogue.characters[usize::from(id)];
            let stats = stats_at(character, saved.level()).unwrap();
            assert_eq!(saved.bp(), stats.bp, "{}", character.name.trimmed());
            assert_eq!(saved.max_hp(), stats.max_hp, "{}", character.name.trimmed());
            assert_eq!(saved.max_ki(), stats.max_ki, "{}", character.name.trimmed());
            assert!(saved.exp() >= stats.exp, "{}", character.name.trimmed());
            assert_eq!(saved.byte_0e(), stats.byte_0e);
        }
        // The item attribute bytes are the ones the game stored in the list.
        for item in slot.items() {
            assert_eq!(game.item(&item.id.to_string()).unwrap(), item);
        }
    }

    /// With the ROM (`SSD_ROM_PATH`) and a save (`SSD_SAVE_PATH`): an edited save, put through
    /// `replay-from-save` and played to its end, is what the game holds in memory: the party
    /// records (HP and level of a raised member), the party list and size, the zeni, the items and
    /// the map. One emulation, about ten seconds.
    #[test]
    #[ignore = "needs the ROM and a save file"]
    fn a_crafted_save_is_what_the_game_loads() {
        let rom = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let rom = ssd_rom::normalize(&rom, None).unwrap().bytes;
        let game = Game::from_rom(&rom).unwrap();
        let mut file = SaveFile::from_bytes(
            &std::fs::read(std::env::var("SSD_SAVE_PATH").expect("SSD_SAVE_PATH")).unwrap(),
        )
        .unwrap();
        let mut slot = file.slot(0).unwrap();
        let first = slot.party()[0];
        let who = game.catalogue.characters[usize::from(first)].name.trimmed().to_owned();
        for (field, value) in [
            (format!("{who}.level"), "10"),
            ("zeni".to_owned(), "321"),
            ("items".to_owned(), "turtle#2,d.radar,bulma"),
        ] {
            apply(&mut slot, Some(&game), &field, value).unwrap();
        }
        file.set_slot(0, &slot).unwrap();

        let dir = tempfile::tempdir().unwrap();
        let (sav, script, out) =
            (dir.path().join("a.sav"), dir.path().join("a.txt"), dir.path().join("a.ssdreplay"));
        std::fs::write(&sav, file.as_bytes()).unwrap();
        std::fs::write(
            &script, "30 -
",
        )
        .unwrap();
        let args = ReplayArgs {
            save: sav,
            script,
            output: out.clone(),
            slot: 0,
            title: false,
            cheats: false,
            bind: None,
        };
        replay_from_save(&rom, &args, &mut Vec::new()).unwrap();

        let replay = crate::shared::read_replay(&out, &rom).unwrap();
        let mut machine = replay.power_on(rom).unwrap();
        replay.play(&mut machine, &mut ssd_oracle::NullAudio).unwrap();
        let wram = machine.wram();
        for (i, id) in slot.party().into_iter().enumerate() {
            let want = slot.record(id).unwrap();
            assert_eq!(wram[0xB00 + 32 * i..0xB00 + 32 * (i + 1)], want.0, "party member {i}");
        }
        assert_eq!(wram[0x1BB], u8::try_from(slot.party().len()).unwrap());
        assert_eq!(u16::from_le_bytes([wram[0x1B4], wram[0x1B5]]), 321);
        assert_eq!(wram[0x1B2], 3);
        assert_eq!(&wram[0x100..0x106], &[0x19_u8, 0xD2, 0x17, 0x44, 2, 0xC8][..]);
        assert_eq!(wram[0x1A0], slot.map_id());
    }

    #[test]
    fn chapter_save_names_what_it_lacks_before_it_reads_anything() {
        let no_pack = chapter_save(&["--chapter", "androids", "--out", "x"], &[]).unwrap_err();
        assert!(no_pack.to_string().contains("needs at least one pack"), "{no_pack}");
        let no_id = chapter_save(&["pack", "--out", "x"], &[]).unwrap_err();
        assert!(no_id.to_string().contains("--chapter <id>"), "{no_id}");
        let unknown = chapter_save(&["pack", "--fast"], &[]).unwrap_err();
        assert!(unknown.to_string().contains("unknown option `--fast`"), "{unknown}");
    }

    /// With the ROM (`SSD_ROM_PATH`): a chapter's party records are the ones `save-edit` makes for the same party and levels (the
    /// same builder), and the chapter's map, party, positions, flags and money read back with `save-dump`'s own description.
    #[test]
    #[ignore = "needs the ROM (SSD_ROM_PATH)"]
    fn a_chapter_slot_has_the_records_save_edit_makes() {
        let rom = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let rom = ssd_rom::normalize(&rom, None).unwrap().bytes;
        let game = Game::from_rom(&rom).unwrap();
        let text = "[[chapter]]\nid = \"androids\"\nname = \"The Androids\"\nmap = 64\nleader = [1024, 768]\n\
                    follower = [896, 648]\nparty = [{ character = 0, level = 30 }, { character = 1, level = 20 }]\n\
                    flags = [3, 29]\nflagx = [1]\nitems = [{ object = 5, count = 3 }]\nzeni = 200\n";
        let chapter = ssd_mod::chapter::parse(text).unwrap().remove(0);
        let synthesized = ssd_mod::chapter::synthesize(&game, &chapter).unwrap();
        let chapter_slot = synthesized.save.slot(0).unwrap();
        let mut edited = Slot::empty();
        apply(&mut edited, Some(&game), "party", "goku,gohan").unwrap();
        apply(&mut edited, Some(&game), "goku.level", "30").unwrap();
        apply(&mut edited, Some(&game), "gohan.level", "20").unwrap();
        for id in [0_u8, 1] {
            assert_eq!(chapter_slot.record(id).unwrap(), edited.record(id).unwrap(), "record {id}");
        }
        assert_eq!(chapter_slot.party(), edited.party());
        let description = describe(&chapter_slot, Some(&game));
        assert!(
            description.contains("map 64 leader (1024, 768) follower (896, 648)"),
            "{description}"
        );
        assert!(description.contains("story flags set: 0x03 0x1D"), "{description}");
        assert!(description.contains("zeni: 200"), "{description}");
        assert!(description.contains("level 30"), "{description}");
        assert!(description.contains("level 20"), "{description}");
    }
}
