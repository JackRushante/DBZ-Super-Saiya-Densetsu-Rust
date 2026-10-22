//! The data packs of a run (M10, batch 5b): the packs installed in the `mods` folder of the data folder, the set the run uses
//! (the `--mods` packs, else the set `mods/mods.toml` enables in a run that persists), the list the Data packs screen shows,
//! and what the screen's requests do. A change of the set, and a chapter's start, restart the game: the new run reads the
//! set it asks for, and its image is built at its start.

use crate::{hd, paths};
use ssd_menus::packs::{Catalog, ChapterInfo, PackInfo, Request};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
#[cfg(not(target_os = "android"))]
use std::{
    ffi::OsString,
    sync::{Mutex, PoisonError},
};

/// The catalog of this run, the list of the Data packs screen (set once by `main` in an Enhanced run).
static CATALOG: OnceLock<Catalog> = OnceLock::new();
/// The ROM of this run: the checks of a set the screen applies read it.
static ROM: OnceLock<Vec<u8>> = OnceLock::new();

/// Keeps the catalog of this run and its ROM (the first call wins).
pub fn configure(catalog: Catalog, rom: &[u8]) {
    let _ = CATALOG.set(catalog);
    let _ = ROM.set(rom.to_vec());
}

/// The catalog of this run: the list the Data packs screen shows (empty without one).
#[must_use]
pub fn catalog() -> Catalog {
    CATALOG.get().cloned().unwrap_or_default()
}

/// The folder of the installed packs, under the data folder.
#[must_use]
pub fn mods_dir() -> PathBuf {
    paths::data_dir().join(ssd_mod::set::FOLDER)
}

/// The files of the zipped pack at `path` unpacked into a folder of the cache, once per pack version and zip size, for the
/// HD art its entries name (the Enhanced picture reads pictures from a folder).
///
/// # Errors
///
/// The zip cannot be read or the folder written.
pub fn unpacked(path: &Path, pack: &ssd_mod::Pack) -> anyhow::Result<PathBuf> {
    let info = &pack.manifest.pack;
    let size = std::fs::metadata(path)?.len();
    let dir = paths::data_dir()
        .join("cache")
        .join("mods")
        .join(format!("{}-{}-{size}", info.id, info.version));
    let done = dir.join(".unpacked");
    if done.is_file() {
        return Ok(dir);
    }
    let mut archive = zip::ZipArchive::new(std::fs::File::open(path)?)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(name) = entry.enclosed_name() else { continue };
        let out = dir.join(name);
        if entry.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::io::copy(&mut entry, &mut std::fs::File::create(&out)?)?;
    }
    std::fs::create_dir_all(&dir)?;
    std::fs::write(&done, b"")?;
    Ok(dir)
}

/// The path of the file that names the enabled packs.
fn set_path() -> PathBuf {
    mods_dir().join(ssd_mod::set::FILE)
}

/// The ids the set file enables, in their order: none when the file does not exist. A file that cannot be read says so, and
/// the run starts without packs.
///
/// # Errors
///
/// The file exists and is not a set file, or cannot be read.
pub fn read_set() -> Result<Vec<String>, String> {
    match std::fs::read_to_string(set_path()) {
        Ok(text) => {
            ssd_mod::set::parse(&text).map_err(|error| format!("mods.toml cannot be read: {error}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(format!("mods.toml cannot be read: {error}")),
    }
}

/// Writes the set file: the packs `ids` enables, in their order. A run that does not persist writes nothing.
///
/// # Errors
///
/// The file cannot be written.
pub fn write_set(ids: &[String]) -> anyhow::Result<()> {
    hd::write_beside(&set_path(), ssd_mod::set::text(ids).as_bytes())
}

/// The installed packs by id: each folder or zip of the mods folder whose pack opens, with its path.
fn installed() -> Vec<(String, PathBuf, ssd_mod::Pack)> {
    ssd_mod::set::installed(&mods_dir())
        .into_iter()
        .filter_map(|path| {
            let pack = ssd_mod::Pack::open(&path).ok()?;
            Some((pack.manifest.pack.id.clone(), path, pack))
        })
        .collect()
}

/// The paths of the packs `ids` names, in their order.
///
/// # Errors
///
/// An id the mods folder does not hold (or whose pack does not open).
pub fn resolve(ids: &[String]) -> Result<Vec<PathBuf>, String> {
    let installed = installed();
    ids.iter()
        .map(|id| {
            installed
                .iter()
                .find(|(pack, _, _)| pack == id)
                .map(|(_, path, _)| path.clone())
                .ok_or_else(|| format!("mods.toml enables `{id}`, which is not in the mods folder"))
        })
        .collect()
}

/// The paths of the packs the set file enables, in their order (none when it enables none).
///
/// # Errors
///
/// The file cannot be read, or names a pack the folder does not hold.
pub fn set_paths() -> Result<Vec<PathBuf>, String> {
    resolve(&read_set()?)
}

/// Whether the packs at `paths` are a set the checks accept on `rom`.
///
/// # Errors
///
/// Why the set is refused, in plain words, one reason per line.
pub fn check(paths: &[PathBuf], rom: &[u8]) -> Result<(), String> {
    let mut packs = Vec::new();
    let mut errors = Vec::new();
    for path in paths {
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
    if errors.is_empty() { Ok(()) } else { Err(refusal_text(&errors)) }
}

/// Why packs are refused, in plain words, one reason after the other.
#[must_use]
pub fn refusal_text(errors: &[ssd_mod::PackError]) -> String {
    errors.iter().map(plain).collect::<Vec<_>>().join(" ")
}

/// One refusal in plain words: a pack made for another ROM says which, the other refusals are the checks' own words.
fn plain(error: &ssd_mod::PackError) -> String {
    match error {
        ssd_mod::PackError::GameMismatch { game, rom, .. } => {
            format!("This pack was made for another ROM (CRC {game}). Your game is {rom:08X}.")
        }
        other => other.to_string(),
    }
}

/// The name of chapter `id` in the packs of the run's set, if one of them offers it.
#[must_use]
pub fn chapter_name(id: &str, applied: &[(PathBuf, ssd_mod::Pack)]) -> Option<String> {
    applied.iter().find_map(|(_, pack)| {
        pack.manifest.chapters.iter().find_map(|entry| {
            pack.chapters(entry).ok().flatten()?.into_iter().find(|c| c.id == id).map(|c| c.name)
        })
    })
}

/// The chapters of a pack, as the Data packs screen lists them.
fn chapters_of(pack: &ssd_mod::Pack) -> Vec<ChapterInfo> {
    pack.manifest
        .chapters
        .iter()
        .flat_map(|entry| pack.chapters(entry).ok().flatten().unwrap_or_default())
        .map(|chapter| ChapterInfo {
            id: chapter.id,
            name: chapter.name,
            description: chapter.description,
            waits: chapter.needs_completed,
        })
        .collect()
}

/// What a pack needs, as the details window says it.
fn needs_of(depends: &[ssd_mod::Dependency]) -> String {
    if depends.is_empty() {
        return "nothing".to_owned();
    }
    depends
        .iter()
        .map(|dependency| format!("{} {} or later", dependency.id, dependency.version))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The list of the Data packs screen: every pack of the mods folder and each pack of the run's set (`applied`, with its path)
/// the folder does not hold. The packs of the set come first, in its order; a pack the checks refuse is listed with the reason
/// and stays off. `notice` is the line the screen shows for the run (why the set is not in use); `playing` the name of the
/// chapter the run plays, if it is a chapter session.
#[must_use]
pub fn build(
    rom: &[u8],
    applied: &[(PathBuf, ssd_mod::Pack)],
    notice: Option<String>,
    playing: Option<String>,
) -> Catalog {
    let mut found: Vec<(PathBuf, ssd_mod::Pack)> = Vec::new();
    let mut unreadable: Vec<(PathBuf, ssd_mod::PackError)> = Vec::new();
    for path in ssd_mod::set::installed(&mods_dir()) {
        match ssd_mod::Pack::open(&path) {
            Ok(pack) => found.push((path, pack)),
            Err(error) => unreadable.push((path, error)),
        }
    }
    for (path, pack) in applied {
        if !found.iter().any(|(known, _)| known == path) {
            found.push((path.clone(), pack.clone()));
        }
    }
    let applied_ids: Vec<&str> =
        applied.iter().map(|(_, pack)| pack.manifest.pack.id.as_str()).collect();
    let rank = |id: &str| applied_ids.iter().position(|applied| *applied == id);
    let mut packs: Vec<(Option<usize>, PackInfo)> = Vec::new();
    for (path, pack) in &found {
        let mut info = pack_info(rom, path, pack);
        let place = rank(&info.id);
        info.on = place.is_some();
        packs.push((place, info));
    }
    for (path, error) in &unreadable {
        let info = PackInfo {
            name: path
                .file_name()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned()),
            size: ssd_mod::set::size(path),
            refusal: Some(plain(error)),
            ..PackInfo::default()
        };
        packs.push((None, info));
    }
    packs.sort_by_key(|(rank, _)| rank.unwrap_or(usize::MAX));
    Catalog {
        packs: packs.into_iter().map(|(_, info)| info).collect(),
        completed: paths::saves_dir().join("completed").is_file(),
        playing,
        notice,
    }
}

/// The list line of one pack: its manifest, its size, whether the run's set holds it, and why the checks refuse it, if they do.
/// The set-level refusals (a dependency missing, a collision) are the set's, not the pack's, and are left out here.
fn pack_info(rom: &[u8], path: &Path, pack: &ssd_mod::Pack) -> PackInfo {
    let info = &pack.manifest.pack;
    let refusal =
        match ssd_mod::check_set(rom, std::slice::from_ref(pack), ssd_mod::CheckOptions::default())
        {
            Ok(()) => None,
            Err(errors) => {
                let own: Vec<ssd_mod::PackError> = errors
                    .into_iter()
                    .filter(|error| {
                        !matches!(
                            error,
                            ssd_mod::PackError::Dependency { .. }
                                | ssd_mod::PackError::Cycle { .. }
                                | ssd_mod::PackError::Collision { .. }
                                | ssd_mod::PackError::DuplicateId { .. }
                        )
                    })
                    .collect();
                (!own.is_empty()).then(|| refusal_text(&own))
            }
        };
    PackInfo {
        id: info.id.clone(),
        name: info.name.clone(),
        version: info.version.clone(),
        author: info.author.clone(),
        description: info.description.clone(),
        needs: needs_of(&pack.manifest.depends),
        size: ssd_mod::set::size(path),
        on: false,
        refusal,
        chapters: chapters_of(pack),
    }
}

/// The arguments of this run for a restart: its own, without the set's options (`--mods`, `--no-mods`, `--chapter`), which
/// the restart gives, and `--chapter id` when a chapter starts.
#[cfg(not(target_os = "android"))]
#[must_use]
pub fn run_args(chapter: Option<&str>) -> Vec<OsString> {
    let mut args = Vec::new();
    let mut skip_value = false;
    for arg in std::env::args_os().skip(1) {
        if std::mem::take(&mut skip_value) {
            continue;
        }
        let text = arg.to_string_lossy().into_owned();
        if text == "--mods" || text == "--chapter" {
            skip_value = true;
        } else if text != "--no-mods"
            && !text.starts_with("--mods=")
            && !text.starts_with("--chapter=")
        {
            args.push(arg);
        }
    }
    if let Some(id) = chapter {
        args.push(OsString::from("--chapter"));
        args.push(OsString::from(id));
    }
    args
}

/// The arguments of the run that starts once this run has ended (a set applied, a chapter or the main game asked for).
#[cfg(not(target_os = "android"))]
static NEXT_RUN: Mutex<Option<Vec<OsString>>> = Mutex::new(None);

/// Asks for a new run with `args` once this one has ended: `main` starts it then, after this run has written its files, so
/// the new run never reads files this one is still writing.
#[cfg(not(target_os = "android"))]
fn ask_next_run(args: Vec<OsString>) {
    *NEXT_RUN.lock().unwrap_or_else(PoisonError::into_inner) = Some(args);
}

/// Starts the run asked for, if one was asked for (see [`ask_next_run`]): `main` calls it when the window's run has ended and
/// has written its files. The game starts again at the title, with the arguments asked for.
///
/// # Errors
///
/// The program's file cannot be found, or the new run cannot start.
#[cfg(not(target_os = "android"))]
pub fn start_next_run() -> std::io::Result<()> {
    let Some(args) = NEXT_RUN.lock().unwrap_or_else(PoisonError::into_inner).take() else {
        return Ok(());
    };
    let exe = std::env::current_exe()?;
    std::process::Command::new(exe).args(args).spawn().map(|_| ())
}

/// Android starts its game again through the launch loop, so there is no run to start here.
///
/// # Errors
///
/// None.
#[cfg(target_os = "android")]
pub fn start_next_run() -> std::io::Result<()> {
    Ok(())
}

/// Carries out a request of the Data packs screen. Apply checks the set on the run's ROM, writes the set file and asks for a
/// new run; a chapter asks for a new run into its own saves; Main game for one into the main saves. The window ends this run
/// when this returns `Ok`, and `main` starts the new run after that.
///
/// # Errors
///
/// A set the checks refuse (the reasons, to the screen), or a set file that cannot be written.
#[cfg(not(target_os = "android"))]
pub fn carry_out(request: Request) -> Result<(), String> {
    let rom = ROM.get().map(Vec::as_slice).unwrap_or_default();
    match request {
        Request::Apply(ids) => {
            check(&resolve(&ids)?, rom)?;
            write_set(&ids).map_err(|error| format!("mods.toml was not written: {error:#}"))?;
            ask_next_run(run_args(None));
            Ok(())
        }
        Request::Chapter(id) => {
            ask_next_run(run_args(Some(&id)));
            Ok(())
        }
        Request::Main => {
            ask_next_run(run_args(None));
            Ok(())
        }
    }
}

/// Carries out a request of the Data packs screen on Android: a set is applied through the launch loop (it starts the game
/// again, which reads the set file); a chapter does not start there yet.
///
/// # Errors
///
/// A set the checks refuse, or a chapter or Main game, which Android does not start yet.
#[cfg(target_os = "android")]
pub fn carry_out(request: Request) -> Result<(), String> {
    let rom = ROM.get().map(Vec::as_slice).unwrap_or_default();
    match request {
        Request::Apply(ids) => {
            check(&resolve(&ids)?, rom)?;
            write_set(&ids).map_err(|error| format!("mods.toml was not written: {error:#}"))?;
            crate::android_setup::request(crate::android_setup::Forced::Relaunch);
            Ok(())
        }
        Request::Chapter(_) | Request::Main => {
            Err("Chapters do not start on Android yet.".to_owned())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(not(target_os = "android"))]
    fn the_restart_keeps_the_run_and_gives_its_own_set_and_chapter() {
        // The test process's own arguments carry none of the set's options: they pass through, and the chapter is added.
        let plain = run_args(None);
        assert!(plain.iter().all(|arg| arg.to_string_lossy() != "--chapter"));
        let chapter = run_args(Some("androids"));
        let n = chapter.len();
        assert_eq!(chapter[n - 2], OsString::from("--chapter"));
        assert_eq!(chapter[n - 1], OsString::from("androids"));
    }

    /// A pack folder under `mods` with the id, for `rom` (or for another ROM when `game` is given).
    fn write_pack(mods: &Path, folder: &str, id: &str, game: &str) {
        let dir = mods.join(folder);
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = format!(
            "[pack]\nid = \"{id}\"\nname = \"{id}\"\nversion = \"1.0.0\"\ngame = \"{game}\"\n\
             author = \"A player\"\ndescription = \"Does nothing.\"\n"
        );
        std::fs::write(dir.join("pack.toml"), manifest).unwrap();
    }

    #[test]
    fn a_zipped_pack_is_unpacked_once_into_the_cache() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let data = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", data.path()) };
        let source = data.path().join("source");
        write_pack(&source, "art", "art.pack", "00000000");
        std::fs::create_dir_all(source.join("art/art/hd")).unwrap();
        std::fs::write(source.join("art/art/hd/pose.png"), b"picture").unwrap();
        let pack = ssd_mod::Pack::from_dir(&source.join("art")).unwrap();
        // The zip with the picture beside the manifest, as a player's `.ssdpack` holds it.
        let zip_path = data.path().join("art.ssdpack");
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        for (name, bytes) in [
            ("pack.toml", std::fs::read(source.join("art/pack.toml")).unwrap()),
            ("art/hd/pose.png", b"picture".to_vec()),
        ] {
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut zip, &bytes).unwrap();
        }
        zip.finish().unwrap();
        let dir = unpacked(&zip_path, &pack).unwrap();
        assert_eq!(std::fs::read(dir.join("art/hd/pose.png")).unwrap(), b"picture");
        std::fs::remove_file(dir.join("art/hd/pose.png")).unwrap();
        assert_eq!(unpacked(&zip_path, &pack).unwrap(), dir, "the second run finds it unpacked");
        assert!(!dir.join("art/hd/pose.png").exists(), "and does not unpack it again");
    }

    #[test]
    fn the_set_file_names_the_packs_in_order_and_a_missing_pack_is_named() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let data = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", data.path()) };
        let rom = vec![0_u8; ssd_mod::ROM_LEN];
        let game = format!("{:08X}", ssd_rom::crc32(&rom));
        write_pack(&mods_dir(), "alpha", "alpha.pack", &game);
        assert_eq!(read_set(), Ok(Vec::new()), "no file: none enabled");
        paths::set_persistent(true);
        write_set(&["alpha.pack".to_owned()]).unwrap();
        paths::set_persistent(false);
        assert_eq!(read_set(), Ok(vec!["alpha.pack".to_owned()]));
        assert_eq!(set_paths(), Ok(vec![mods_dir().join("alpha")]));
        assert_eq!(
            resolve(&["gone.pack".to_owned()]),
            Err("mods.toml enables `gone.pack`, which is not in the mods folder".to_owned())
        );
        assert!(check(&[mods_dir().join("alpha")], &rom).is_ok());
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn the_list_checks_each_installed_pack_and_names_a_refusal_in_plain_words() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let data = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", data.path()) };
        let rom = vec![0_u8; ssd_mod::ROM_LEN];
        let game = format!("{:08X}", ssd_rom::crc32(&rom));
        write_pack(&mods_dir(), "good", "good.pack", &game);
        write_pack(&mods_dir(), "old", "old.pack", "00000001");
        let catalog = build(&rom, &[], None, Some("The Androids".to_owned()));
        assert_eq!(catalog.playing.as_deref(), Some("The Androids"));
        let good = catalog.packs.iter().find(|p| p.id == "good.pack").expect("the good pack");
        assert_eq!(
            (good.refusal.clone(), good.on, good.author.as_str()),
            (None, false, "A player")
        );
        assert_eq!((good.needs.as_str(), good.description.as_str()), ("nothing", "Does nothing."));
        let old = catalog.packs.iter().find(|p| p.id == "old.pack").expect("the old pack");
        assert_eq!(
            old.refusal.as_deref(),
            Some(
                format!("This pack was made for another ROM (CRC 00000001). Your game is {game}.")
                    .as_str()
            )
        );
        // The pack of the set comes first, and is on.
        let applied =
            [(mods_dir().join("good"), ssd_mod::Pack::open(&mods_dir().join("good")).unwrap())];
        let catalog = build(&rom, &applied, None, None);
        assert_eq!(catalog.packs[0].id, "good.pack");
        assert!(catalog.packs[0].on, "the set's pack is on");
        assert!(!catalog.packs[1].on);
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn a_pack_made_for_another_rom_is_named_in_plain_words() {
        let error = ssd_mod::PackError::GameMismatch {
            pack: "old".to_owned(),
            game: "1A2B3C4D".to_owned(),
            rom: 0xA8CF_4F2A,
        };
        assert_eq!(
            plain(&error),
            "This pack was made for another ROM (CRC 1A2B3C4D). Your game is A8CF4F2A."
        );
    }
}
