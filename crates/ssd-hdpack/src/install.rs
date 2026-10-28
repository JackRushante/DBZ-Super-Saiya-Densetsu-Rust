//! The first-launch install of a shared HD art pack (`public`): the player puts `hd-pack.zip` (or
//! the unpacked `hd-pack` folder) next to the game, and the game builds the HD pack of their ROM
//! from the ROM and the art: what `xtask hd-convert` and `hd-import` do for the developers.
//!
//! The pack ends up where the renderer looks for it, `<cache>/hd/<ROM CRC-32>/<converter>/`
//! ([`convert::pack_dir`]), built in a folder beside it and moved into place when it is whole, so a
//! launch that is interrupted leaves nothing half made. A pack that is installed already (the same
//! digest, the same converter) is left alone.

use crate::{convert, import, public};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};

/// What the install leaves in the pack's folder, to know the work is done.
pub const INSTALLED: &str = "installed.toml";

/// The folder the pack is made in, beside the pack's own.
const WORK: &str = "install-work";

/// The lock of an install that is running.
const LOCK: &str = "install.lock";

/// A lock older than this is a crash's and is taken over.
const STALE_LOCK_SECONDS: u64 = 3600;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Installed {
    digest: String,
    revision: String,
    converter: u32,
    rom_crc32: String,
}

/// What an install did.
#[derive(Debug)]
pub enum Outcome {
    /// The pack of the ROM was made from this very art before: nothing was done.
    Current(PathBuf),
    /// The pack was made.
    Installed { pack: PathBuf, report: import::Report },
}

/// Where the shared art pack is: `explicit` (a zip file, a folder with `hd-pack.toml`, or a folder
/// that holds `hd-pack.zip` or `hd-pack`), else beside the game in `exe_dir` (`hd-pack.zip`, a
/// folder `hd-pack`, or any zip whose name has `hd-pack` in it).
#[must_use]
pub fn find_source(explicit: Option<&Path>, exe_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return in_folder(path).or_else(|| path.is_file().then(|| path.to_path_buf()));
    }
    in_folder(exe_dir?)
}

/// The pack in `dir`, when `dir` is the pack or holds it.
fn in_folder(dir: &Path) -> Option<PathBuf> {
    if !dir.is_dir() {
        return None;
    }
    if dir.join(public::MANIFEST).is_file() {
        return Some(dir.to_path_buf());
    }
    for name in ["hd-pack.zip", "hd-pack"] {
        let path = dir.join(name);
        if path.is_file() || path.join(public::MANIFEST).is_file() {
            return Some(path);
        }
    }
    let mut zips: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
            path.is_file()
                && name.contains("hd-pack")
                && Path::new(&name).extension().is_some_and(|ext| ext == "zip")
        })
        .collect();
    zips.sort();
    zips.pop()
}

/// A pack's files: read one by name, or unpack them all.
enum Files {
    Folder(PathBuf),
    Zip { archive: zip::ZipArchive<std::io::BufReader<std::fs::File>>, prefix: String },
}

impl Files {
    fn open(source: &Path) -> anyhow::Result<Self> {
        if source.is_dir() {
            return Ok(Self::Folder(source.to_path_buf()));
        }
        let file = std::fs::File::open(source)
            .with_context(|| format!("opening the HD pack {}", source.display()))?;
        let archive = zip::ZipArchive::new(std::io::BufReader::new(file))
            .with_context(|| format!("{} is not a zip file", source.display()))?;
        // The files may sit in one folder of the zip (a zipped folder)
        let suffix = format!("/{}", public::MANIFEST);
        let prefix = archive
            .file_names()
            .filter_map(Result::ok)
            .find(|name| *name == public::MANIFEST || name.ends_with(&suffix))
            .with_context(|| format!("{} has no {}", source.display(), public::MANIFEST))?
            .strip_suffix(public::MANIFEST)
            .unwrap_or("")
            .to_owned();
        Ok(Self::Zip { archive, prefix })
    }

    fn manifest(&mut self) -> anyhow::Result<public::Manifest> {
        let text = match self {
            Self::Folder(dir) => std::fs::read_to_string(dir.join(public::MANIFEST))?,
            Self::Zip { archive, prefix } => {
                let mut file = archive.by_name(&format!("{prefix}{}", public::MANIFEST))?;
                let mut text = String::new();
                std::io::Read::read_to_string(&mut file, &mut text)?;
                text
            }
        };
        public::Manifest::parse(&text)
    }

    /// The folder holding the art: the pack's own folder, or `into` with the zip's files unpacked
    /// (hard links and copies of the files are made from it, so it is removed afterwards).
    fn unpack(&mut self, into: &Path, say: &mut dyn FnMut(&str)) -> anyhow::Result<PathBuf> {
        let Self::Zip { archive, prefix } = self else {
            let Self::Folder(dir) = self else { unreachable!() };
            return Ok(dir.clone());
        };
        std::fs::create_dir_all(into)?;
        let total = archive.len();
        let mut last = 0;
        for at in 0..total {
            let mut entry = archive.by_index(at)?;
            if entry.is_dir() {
                continue;
            }
            let Some(name) = entry.enclosed_name() else {
                anyhow::bail!("the HD pack holds a file outside its folder: {:?}", entry.name());
            };
            let Ok(relative) = name.strip_prefix(prefix.trim_end_matches('/')) else { continue };
            let target = into.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = std::fs::File::create(&target)
                .with_context(|| format!("writing {}", target.display()))?;
            std::io::copy(&mut entry, &mut out)?;
            let percent = (at + 1) * 100 / total;
            if percent >= last + 10 {
                last = percent;
                say(&format!("HD pack: unpacking the art, {percent}%"));
            }
        }
        Ok(into.to_path_buf())
    }
}

/// A lock on the install of one ROM's pack, removed when it is dropped.
struct Lock(PathBuf);

impl Lock {
    fn take(folder: &Path) -> anyhow::Result<Self> {
        let path = folder.join(LOCK);
        for _ in 0..2 {
            match std::fs::OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    let _ = writeln!(file, "{}", std::process::id());
                    return Ok(Self(path));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let age = std::fs::metadata(&path)
                        .and_then(|meta| meta.modified())
                        .ok()
                        .and_then(|time| time.elapsed().ok())
                        .map_or(u64::MAX, |age| age.as_secs());
                    anyhow::ensure!(
                        age >= STALE_LOCK_SECONDS,
                        "another launch is installing the HD pack ({} exists)",
                        path.display()
                    );
                    std::fs::remove_file(&path)?;
                }
                Err(e) => return Err(e.into()),
            }
        }
        anyhow::bail!("cannot take the lock {}", path.display())
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A writer that hands each line it is given to a callback.
struct Lines<'a> {
    say: &'a mut dyn FnMut(&str),
    pending: Vec<u8>,
}

impl Write for Lines<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            (self.say)(String::from_utf8_lossy(&line).trim_end());
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Installs the shared art pack at `source` for `rom` under `cache`; every step is said through
/// `say`. Does nothing when the pack of the ROM was made from this art already. A pack made for
/// another ROM is refused unless `force` is set, which says so and installs it anyway: the art can
/// still line up when only the text of the ROM differs.
///
/// # Errors
///
/// The pack is not a pack of this format, or (unless `force`) not of this ROM, or a file cannot be
/// read or written; the pack of the ROM is the one it was before.
pub fn install(
    rom: &[u8],
    source: &Path,
    cache: &Path,
    say: &mut dyn FnMut(&str),
    force: bool,
) -> anyhow::Result<Outcome> {
    let started = std::time::Instant::now();
    let mut files = Files::open(source)?;
    let manifest = files.manifest()?;
    let crc = ssd_rom::crc32(rom);
    if !manifest.rom_crc32.eq_ignore_ascii_case(&format!("{crc:08X}")) {
        anyhow::ensure!(
            force,
            "this HD pack is made for the ROM {}, yours is {crc:08X}",
            manifest.rom_crc32
        );
        say(&format!(
            "HD pack made for the ROM {} is being installed on {crc:08X} anyway (--force-rom)",
            manifest.rom_crc32
        ));
    }
    let target = convert::pack_dir(cache, crc);
    let wanted = Installed {
        digest: manifest.digest.clone(),
        revision: manifest.revision.clone(),
        converter: convert::CONVERTER,
        rom_crc32: format!("{crc:08X}"),
    };
    let done = std::fs::read_to_string(target.join(INSTALLED))
        .ok()
        .and_then(|text| toml::from_str::<Installed>(&text).ok());
    if done.as_ref() == Some(&wanted) && ssd_hd::pack::Pack::open(&target).is_ok() {
        return Ok(Outcome::Current(target));
    }
    let folder = target.parent().context("the pack folder has a parent")?.to_path_buf();
    std::fs::create_dir_all(&folder)?;
    let _lock = Lock::take(&folder)?;
    say(&format!(
        "HD pack: installing \"{}\" (revision {}) for your ROM. This takes a minute, once.",
        manifest.name, manifest.revision
    ));
    let work = folder.join(WORK);
    if work.exists() {
        std::fs::remove_dir_all(&work)?;
    }
    let pack = work.join("pack");
    let result = (|| -> anyhow::Result<import::Report> {
        say("HD pack: reading your ROM's graphics...");
        convert::run_with(rom, &pack, false, &mut Lines { say: &mut *say, pending: Vec::new() })?;
        let store = files.unpack(&work.join("store"), say)?;
        say("HD pack: putting the art in place...");
        let report = import::run(
            rom,
            &store,
            None,
            None,
            &pack,
            &mut Lines { say: &mut *say, pending: Vec::new() },
        )?;
        std::fs::write(pack.join(INSTALLED), toml::to_string(&wanted)?)?;
        anyhow::ensure!(
            ssd_hd::pack::Pack::open(&pack).is_ok(),
            "the pack that was made does not open"
        );
        Ok(report)
    })();
    let report = match result {
        Ok(report) => report,
        Err(error) => {
            let _ = std::fs::remove_dir_all(&work);
            return Err(error.context("installing the HD pack"));
        }
    };
    // Put the pack in place: the old one (a pack made by an older version of the art) is moved
    // aside first and removed when the new one is in
    let old = folder.join(format!("{}.old", convert::CONVERTER));
    if old.exists() {
        std::fs::remove_dir_all(&old)?;
    }
    let had_old = target.exists();
    if had_old {
        std::fs::rename(&target, &old)?;
    }
    if let Err(error) = std::fs::rename(&pack, &target) {
        if had_old {
            let _ = std::fs::rename(&old, &target);
        }
        let _ = std::fs::remove_dir_all(&work);
        return Err(anyhow::Error::from(error).context("moving the HD pack into place"));
    }
    let _ = std::fs::remove_dir_all(&old);
    let _ = std::fs::remove_dir_all(&work);
    say(&format!("HD pack: ready in {} seconds.", started.elapsed().as_secs()));
    Ok(Outcome::Installed { pack: target, report })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(rom_crc32: &str) -> String {
        format!(
            "format = 1\nname = \"Test art\"\nrevision = \"t1\"\nrom_crc32 = \"{rom_crc32}\"\ndigest = \"00000001\"\n"
        )
    }

    #[test]
    fn the_pack_is_found_next_to_the_game_in_any_of_its_forms() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(find_source(None, Some(dir.path())), None);
        std::fs::write(dir.path().join("notes.zip"), b"x").unwrap();
        assert_eq!(find_source(None, Some(dir.path())), None, "a zip that is not a pack's name");
        std::fs::write(dir.path().join("DBZ-hd-pack-1.zip"), b"x").unwrap();
        assert_eq!(find_source(None, Some(dir.path())), Some(dir.path().join("DBZ-hd-pack-1.zip")));
        std::fs::create_dir_all(dir.path().join("hd-pack")).unwrap();
        std::fs::write(dir.path().join("hd-pack").join(public::MANIFEST), manifest("0")).unwrap();
        assert_eq!(find_source(None, Some(dir.path())), Some(dir.path().join("hd-pack")));
        std::fs::write(dir.path().join("hd-pack.zip"), b"x").unwrap();
        assert_eq!(find_source(None, Some(dir.path())), Some(dir.path().join("hd-pack.zip")));
        // An explicit path wins: a zip, the folder of the pack, or a folder that holds it
        let zip = dir.path().join("mine.zip");
        std::fs::write(&zip, b"x").unwrap();
        assert_eq!(find_source(Some(&zip), Some(dir.path())), Some(zip));
        let inner = dir.path().join("hd-pack");
        assert_eq!(find_source(Some(&inner), None), Some(inner));
        assert_eq!(find_source(Some(&dir.path().join("nothing")), Some(dir.path())), None);
    }

    fn zipped(dir: &Path, folder: &str, rom_crc32: &str) -> PathBuf {
        let path = dir.join("hd-pack.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file(format!("{folder}{}", public::MANIFEST), options).unwrap();
        writer.write_all(manifest(rom_crc32).as_bytes()).unwrap();
        writer.start_file(format!("{folder}class2/a.webp"), options).unwrap();
        writer.write_all(b"art").unwrap();
        writer.finish().unwrap();
        path
    }

    #[test]
    fn a_zip_unpacks_into_the_art_folder_whether_or_not_it_is_zipped_in_a_folder() {
        for folder in ["", "pack/"] {
            let dir = tempfile::tempdir().unwrap();
            let path = zipped(dir.path(), folder, "A8CF4F2A");
            let mut files = Files::open(&path).unwrap();
            assert_eq!(files.manifest().unwrap().rom_crc32, "A8CF4F2A");
            let store = files.unpack(&dir.path().join("store"), &mut |_| {}).unwrap();
            assert_eq!(std::fs::read(store.join("class2/a.webp")).unwrap(), b"art");
            assert!(store.join(public::MANIFEST).is_file());
        }
    }

    #[test]
    fn a_pack_for_another_rom_or_that_is_not_a_pack_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let rom = vec![0_u8; 0x8000];
        let wrong = zipped(dir.path(), "", "12345678");
        let error =
            install(&rom, &wrong, &dir.path().join("cache"), &mut |_| {}, false).unwrap_err();
        assert!(error.to_string().contains("12345678"), "{error:#}");
        std::fs::write(dir.path().join("x.zip"), b"not a zip").unwrap();
        assert!(install(&rom, &dir.path().join("x.zip"), dir.path(), &mut |_| {}, false).is_err());
    }

    #[test]
    fn a_pack_for_another_rom_is_installed_when_forced_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let rom = vec![0_u8; 0x10_0000];
        let other = shared_pack(dir.path(), 0xDEAD_BEEF);
        let mut said = Vec::new();
        let out = install(
            &rom,
            &other,
            &dir.path().join("cache"),
            &mut |l| said.push(l.to_owned()),
            true,
        )
        .unwrap();
        assert!(matches!(out, Outcome::Installed { .. }));
        assert!(said.iter().any(|l| l.contains("DEADBEEF")), "{said:?}");
    }

    /// A shared pack of one pose (a lossless WebP, its outline in masks.bin), one background layer
    /// with no source picture, and a manifest for `crc`.
    fn shared_pack(dir: &Path, crc: u32) -> PathBuf {
        shared_pack_of(dir, crc, false)
    }

    /// [`shared_pack`], with the bigger art of the pose (`variants/`: the pose's picture and 4 art pixels of margin each
    /// side, `always`) when `variants` is set, and the digest of a pack that has it.
    fn shared_pack_of(dir: &Path, crc: u32, variants: bool) -> PathBuf {
        let path = dir.join("hd-pack.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
        let stored = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        let mut put = |name: &str, bytes: &[u8]| {
            writer.start_file(name, stored).unwrap();
            writer.write_all(bytes).unwrap();
        };
        let webp = |width: u32, height: u32| {
            let rgba = vec![200_u8; (width * height * 4) as usize];
            let mut file = Vec::new();
            image_webp_encode(&mut file, &rgba, width, height);
            file
        };
        put("class2/p.webp", &webp(8, 8));
        put(
            "class2/index.json",
            br##"{"assets_list": [{"id": "p", "file": "p.webp", "source_key": "00000000000000ab",
                "source_size": [2, 2], "palette_indexes_used": {"1": "#FF0000"}}]}"##,
        );
        let mut masks = public::Masks::default();
        masks.insert(0xAB, 2, 2, vec![0b0110]);
        put(public::MASKS, &masks.encode());
        // A layer of one screen, rows 0 to 2 (256 x 16 game pixels, 4x), and no source file
        put("bg/b.webp", &webp(1024, 64));
        put(
            "bg/index.json",
            br#"{"layers": [{"id": "b", "key": "00000000000000ff", "bg": 3, "mode": 0,
                "map_base": 28672, "char_base": 20480, "size": 0, "rows": [0, 2], "fill": true,
                "file": "b.webp"}]}"#,
        );
        if variants {
            put("variants/p.webp", &webp(16, 16));
            put(
                "variants/index.json",
                br#"{"variants": [{"id": "p", "art": "p", "when": "always", "margin": 4,
                    "file": "p.webp"}]}"#,
            );
        }
        let digest = if variants { "00000003" } else { "00000002" };
        let text = format!(
            "format = 1
name = \"Test art\"
revision = \"t1\"
rom_crc32 = \"{crc:08X}\"
digest = \"{digest}\"
"
        );
        put(public::MANIFEST, text.as_bytes());
        writer.finish().unwrap();
        path
    }

    fn image_webp_encode(out: &mut Vec<u8>, rgba: &[u8], width: u32, height: u32) {
        ssd_hd::png::encode_webp_lossless(out, rgba, width, height);
    }

    #[test]
    fn a_shared_pack_is_installed_once_and_its_art_opens_without_source_pictures() {
        let dir = tempfile::tempdir().unwrap();
        let rom = vec![0_u8; 0x10_0000];
        let crc = ssd_rom::crc32(&rom);
        let zip = shared_pack(dir.path(), crc);
        let cache = dir.path().join("cache");
        let mut said = Vec::new();
        let Outcome::Installed { pack, report } =
            install(&rom, &zip, &cache, &mut |line| said.push(line.to_owned()), false).unwrap()
        else {
            panic!("the first install does the work")
        };
        assert_eq!((report.objects, report.bg), (1, 1), "{said:?}");
        assert!(said.iter().any(|line| line.contains("ready")), "{said:?}");
        assert_eq!(pack, convert::pack_dir(&cache, crc));
        let opened = ssd_hd::pack::Pack::open(&pack).unwrap();
        let object = &opened.art.objects()[0];
        assert_eq!((object.key, object.width, object.height), (0xAB, 2, 2));
        assert_eq!(object.mask, vec![0b0110], "the outline comes from masks.bin");
        assert_eq!(object.palette.colors[1], [255, 0, 0]);
        assert!(Path::new(&object.art).extension().is_some_and(|e| e == "webp"));
        assert!(pack.join(&object.art).is_file());
        assert_eq!(opened.bg.layer.len(), 1);
        assert!(opened.bg.layer[0].source.is_empty(), "no source picture of the game");
        assert!(opened.pose_art.is_empty(), "a pack without bigger art has none");
        // The work folder, the store and the lock are gone, and a second install does nothing
        let folder = pack.parent().unwrap();
        let names: Vec<String> = std::fs::read_dir(folder)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["1"], "{names:?}");
        assert!(matches!(
            install(&rom, &zip, &cache, &mut |_| {}, false).unwrap(),
            Outcome::Current(_)
        ));
    }

    #[test]
    fn a_pack_with_bigger_pose_art_installs_it_and_replaces_one_without() {
        let dir = tempfile::tempdir().unwrap();
        let rom = vec![0_u8; 0x10_0000];
        let crc = ssd_rom::crc32(&rom);
        let cache = dir.path().join("cache");
        // The pack as it was shipped, without the art: installed, and as it was
        // (each in a folder of its own: both are `hd-pack.zip`)
        let (old_dir, new_dir) = (dir.path().join("old"), dir.path().join("new"));
        std::fs::create_dir_all(&old_dir).unwrap();
        std::fs::create_dir_all(&new_dir).unwrap();
        let old = shared_pack(&old_dir, crc);
        let Outcome::Installed { pack, .. } =
            install(&rom, &old, &cache, &mut |_| {}, false).unwrap()
        else {
            panic!("the first install does the work")
        };
        assert!(
            ssd_hd::pack::Pack::open(&pack).unwrap().pose_art.is_empty(),
            "a pack without bigger art has none"
        );
        // The new pack: another digest, so the installed pack is made again, with the art
        let new = shared_pack_of(&new_dir, crc, true);
        let Outcome::Installed { pack, report } =
            install(&rom, &new, &cache, &mut |_| {}, false).unwrap()
        else {
            panic!("a new pack is installed over the old one")
        };
        assert_eq!(report.variants, 1);
        let opened = ssd_hd::pack::Pack::open(&pack).unwrap();
        assert_eq!(opened.pose_art.len(), 1);
        let variant = &opened.pose_art[0];
        assert_eq!(
            (variant.art.as_str(), variant.margin),
            ("p", 1),
            "the margin: 4 art pixels, a game pixel"
        );
        assert_eq!(format!("{:?}", variant.when), "Always");
        assert!(pack.join(&variant.file).is_file(), "{}", variant.file);
        assert!(
            Path::new(&variant.file).extension().is_some_and(|ext| ext == "webp"),
            "the picture is the pack's own WebP"
        );
        // The poses are what they were
        assert_eq!(opened.art.objects().len(), 1);
        assert_eq!(opened.bg.layer.len(), 1);
        assert!(matches!(
            install(&rom, &new, &cache, &mut |_| {}, false).unwrap(),
            Outcome::Current(_)
        ));
        // And the old zip installs again over the new pack, without the art
        let Outcome::Installed { pack, .. } =
            install(&rom, &old, &cache, &mut |_| {}, false).unwrap()
        else {
            panic!("an old pack is installed over a new one")
        };
        assert!(
            ssd_hd::pack::Pack::open(&pack).unwrap().pose_art.is_empty(),
            "a pack without bigger art has none"
        );
    }

    #[test]
    fn a_lock_is_taken_once_and_released() {
        let dir = tempfile::tempdir().unwrap();
        let lock = Lock::take(dir.path()).unwrap();
        assert!(Lock::take(dir.path()).is_err(), "another launch is installing");
        drop(lock);
        assert!(Lock::take(dir.path()).is_ok());
    }

    #[test]
    fn the_lines_of_a_report_go_to_the_callback_one_by_one() {
        let mut said = Vec::new();
        {
            let mut say = |line: &str| said.push(line.to_owned());
            let mut lines = Lines { say: &mut say, pending: Vec::new() };
            write!(lines, "one\ntw").unwrap();
            writeln!(lines, "o").unwrap();
        }
        assert_eq!(said, ["one", "two"]);
    }
}
