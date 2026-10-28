//! `hd-pack-to-store <pack> <art store> [--masks <dir>]`: puts the art of a shared pack
//! (`hd-pack.zip`, see `ssd_hdpack::public`) back into the art store, the reverse of
//! `hd-pack-public`. A pack holds the art of one moment (the release it was built from); the store
//! is the archive the art is made in, with the rich `index.json` of each folder (what the art was
//! made of, what it cost). This brings the archive up to what the pack holds:
//!
//! * every art file of the pack is decoded and written as a PNG beside the store's own files (a
//!   pack stores them as WebP, the store keeps PNG); a file the store already has with the same
//!   pixels is left alone, and the `maps` art is the one place the store is always better, since a
//!   pack stores it as a lossy WebP, so a tile the store already has keeps its PNG;
//! * the entries are merged into the store's `index.json` by `id`: the fields the pack defines
//!   become the entry's (so the entry names the art that is now in the store), the store's own
//!   fields stay (descriptions, what the art cost, how it was made), and every name of an art file
//!   in an entry ends in `.png`;
//! * the outlines the pack holds (`masks.bin`) are written as PNGs into the silhouettes folder for
//!   the keys that have none there.
//!
//! Where two entries of the same id both carry a `revision` and the store's is not lower, the
//! store's fields win and the pack only fills in what the store lacks; a pack carries none, so its
//! fields replace the store's and the store's own revision is kept.
//! The fighters' and the field's poses are one folder in a pack and two in the store: the field's
//! (`field-*`) go to `class3`. Nothing of the ROM is read or written.

use anyhow::Context;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Write;
use std::path::{Path, PathBuf};

/// What one pass of the merge did.
#[derive(Debug, Default)]
struct Report {
    added: usize,
    updated: usize,
    kept: usize,
    /// A map tile whose lossless PNG the store already had.
    kept_lossless: usize,
    masks: usize,
}

/// Where the pack's files are read from: a zip, or a folder of the same layout.
enum Source {
    Zip(zip::ZipArchive<std::fs::File>),
    Dir(PathBuf),
}

impl Source {
    /// Opens `path`, a `.zip` or a folder.
    fn open(path: &Path) -> anyhow::Result<Self> {
        if path.is_dir() {
            Ok(Self::Dir(path.to_owned()))
        } else {
            let file = std::fs::File::open(path)
                .with_context(|| format!("opening the pack {}", path.display()))?;
            Ok(Self::Zip(
                zip::ZipArchive::new(file)
                    .with_context(|| format!("reading the pack {}", path.display()))?,
            ))
        }
    }

    /// The bytes of `name` (`class2/battle-goku-000.webp`), if the pack holds it.
    fn read(&mut self, name: &str) -> anyhow::Result<Option<Vec<u8>>> {
        match self {
            Self::Dir(dir) => {
                let path = dir.join(name);
                if path.is_file() { Ok(Some(std::fs::read(&path)?)) } else { Ok(None) }
            }
            Self::Zip(zip) => match zip.by_name(name) {
                Ok(mut entry) => {
                    let mut bytes = Vec::with_capacity(entry.size() as usize);
                    std::io::Read::read_to_end(&mut entry, &mut bytes)?;
                    Ok(Some(bytes))
                }
                Err(zip::result::ZipError::FileNotFound) => Ok(None),
                Err(error) => Err(error).with_context(|| format!("reading {name} from the pack")),
            },
        }
    }

    /// The names of the pack's files under `folder`, relative to it (`map-00/t0_0.webp`).
    fn files(&self, folder: &str) -> Vec<String> {
        let prefix = format!("{folder}/");
        let mut out = Vec::new();
        match self {
            Self::Dir(dir) => {
                let base = dir.join(folder);
                for (root, files) in walk(&base) {
                    for name in files {
                        if let Ok(rel) = root.join(&name).strip_prefix(&base) {
                            out.push(rel.to_string_lossy().replace('\\', "/"));
                        }
                    }
                }
            }
            Self::Zip(zip) => {
                for name in zip.file_names() {
                    let Ok(name) = name else {
                        continue;
                    };
                    if let Some(rest) = name.strip_prefix(&prefix)
                        && !rest.is_empty()
                    {
                        out.push(rest.to_owned());
                    }
                }
            }
        }
        out.sort();
        out.dedup();
        out
    }
}

/// Every directory under `dir`, with the names of the files in it.
fn walk(dir: &Path) -> Vec<(PathBuf, Vec<String>)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    let mut files = Vec::new();
    let mut sub = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            sub.push(path);
        } else if let Some(name) = path.file_name() {
            files.push(name.to_string_lossy().into_owned());
        }
    }
    out.push((dir.to_owned(), files));
    for path in sub {
        out.extend(walk(&path));
    }
    out
}

/// A folder of the store and the keys of its index whose value is a list of entries.
struct Folder {
    /// `class2`.
    name: &'static str,
    /// The keys of the index that list its entries.
    keys: &'static [&'static str],
}

/// The folders of the store a pack holds art for. `class2` is the one place a pack merges two
/// folders: its objects hold the fighters' and the field's poses, which the store keeps apart.
const FOLDERS: &[Folder] = &[
    Folder { name: "class1", keys: &["assets"] },
    Folder { name: "class2", keys: &["assets_list"] },
    Folder { name: "cards", keys: &["tiles", "tips"] },
    Folder { name: "bg", keys: &["layers"] },
    Folder { name: "maps", keys: &["layers"] },
    Folder { name: "mode7", keys: &["planes"] },
    Folder { name: "variants", keys: &["variants"] },
];

/// Where an object of the pack goes in the store: the field's poses have their own folder.
fn object_folder(id: &str) -> &'static str {
    if id.starts_with("field-") { "class3" } else { "class2" }
}

/// The store's path of the art of a pack file: the same path with a `.png` name. A file of the
/// objects' folder goes to `class3` when it is a field pose's.
fn store_path(folder: &'static str, file: &str) -> PathBuf {
    let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
    let target = if folder == "class2" { object_folder(stem) } else { folder };
    Path::new(target).join(format!("{stem}.png"))
}

/// The revision of an entry, when it has one.
fn revision(entry: &Value) -> Option<i64> {
    entry.get("revision").and_then(Value::as_i64)
}

/// Every name of an art file in `value` becomes a PNG's: a pack names WebP files.
fn png_names(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, item) in map.iter_mut() {
                match item {
                    Value::String(name)
                        if key != "description"
                            && Path::new(name.as_str())
                                .extension()
                                .is_some_and(|ext| ext.eq_ignore_ascii_case("webp")) =>
                    {
                        name.truncate(name.len() - ".webp".len());
                        name.push_str(".png");
                    }
                    Value::Object(_) | Value::Array(_) => png_names(item),
                    _ => {}
                }
            }
        }
        Value::Array(list) => {
            for item in list {
                png_names(item);
            }
        }
        _ => {}
    }
}

/// Merges `pack` into `store`'s fields. The pack's fields replace the store's (they name the art
/// now in the store), unless both carry a `revision` and the store's is not lower: then the pack
/// only fills in the fields the store lacks. Every art name becomes a PNG's, and a field only the
/// store has is kept.
fn merge_entry(store: &mut Value, pack: &Value) {
    let store_wins =
        matches!((revision(store), revision(pack)), (Some(store), Some(pack)) if store >= pack);
    let mut pack = pack.clone();
    png_names(&mut pack);
    let (Some(store), Some(pack)) = (store.as_object_mut(), pack.as_object()) else {
        return;
    };
    for (key, value) in pack {
        if !store_wins || !store.contains_key(key) {
            store.insert(key.clone(), value.clone());
        }
    }
}

/// Whether the file at `path` holds exactly `width` x `height` pixels of `rgba`.
fn same_pixels(path: &Path, width: u32, height: u32, rgba: &[u8]) -> bool {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| ssd_hd::png::read_rgba(&bytes).ok())
        .is_some_and(|(w, h, pixels)| (w, h) == (width, height) && pixels == rgba)
}

/// The entries listed under any of `keys` of an index, with the key they came from.
fn entries_of<'a>(index: &'a Value, keys: &[&'static str]) -> Vec<(&'static str, &'a Value)> {
    let mut out = Vec::new();
    for &key in keys {
        if let Some(list) = index.get(key).and_then(Value::as_array) {
            out.extend(list.iter().map(|entry| (key, entry)));
        }
    }
    out
}

/// The entry of `list` that is the same thing as `pack`: by `id`, else by `code`.
fn same_thing<'a>(list: &'a mut [Value], pack: &Value) -> Option<&'a mut Value> {
    let id = pack.get("id").and_then(Value::as_str);
    let code = pack.get("code").and_then(Value::as_u64);
    list.iter_mut().find(|item| {
        item.get("id").and_then(Value::as_str) == id && id.is_some()
            || item.get("code").and_then(Value::as_u64) == code && code.is_some()
    })
}

/// Puts the pack's art and entries into the store.
///
/// # Errors
///
/// The pack or the store cannot be read, or a file cannot be written.
pub fn run(
    pack: &Path,
    store: &Path,
    masks: Option<&Path>,
    out: &mut dyn Write,
) -> anyhow::Result<()> {
    let mut source = Source::open(pack)?;
    let manifest = source.read("hd-pack.toml")?.context("the pack has no hd-pack.toml")?;
    writeln!(out, "hd-pack-to-store: pack {}", pack.display())?;
    for line in String::from_utf8(manifest)?.lines().filter(|line| !line.trim().is_empty()) {
        writeln!(out, "  {line}")?;
    }
    let mut report = Report::default();

    for folder in FOLDERS {
        let mut art = 0;
        let mut touched: BTreeMap<&'static str, usize> = BTreeMap::new();
        // The store's indexes of the folders this pass writes to, read once and written at the end.
        let mut indexes: BTreeMap<&'static str, Value> = BTreeMap::new();

        // The art: every file of the folder, as a PNG beside the store's own.
        for file in source.files(folder.name) {
            if file == "index.json" {
                continue;
            }
            let Some(bytes) = source.read(&format!("{}/{}", folder.name, file))? else {
                continue;
            };
            let (width, height, rgba) = ssd_hd::png::read_rgba(&bytes)
                .map_err(|e| anyhow::anyhow!("{}/{}: {e}", folder.name, file))?;
            let target = store_path(folder.name, &file);
            let dir = store.join(target.parent().unwrap_or(Path::new("")));
            std::fs::create_dir_all(&dir)?;
            let path = store.join(&target);
            // The store keeps the maps' art lossless: a pack's lossy WebP never replaces a tile it
            // already has.
            let lossless_kept = folder.name == "maps" && path.is_file();
            if lossless_kept {
                report.kept_lossless += 1;
            } else if same_pixels(&path, width, height, &rgba) {
                report.kept += 1;
            } else {
                ssd_hd::png::write_rgba(&path, width, height, &rgba)
                    .with_context(|| format!("writing {}", path.display()))?;
                report.updated += 1;
                *touched.entry(target_folder(folder.name, &file)).or_default() += 1;
            }
            art += 1;
        }

        // The entries: merged into the store's index of their own folder.
        let Some(text) = source.read(&format!("{}/index.json", folder.name))? else {
            writeln!(out, "{art:>7} files of {}: the pack has no index.json", folder.name)?;
            continue;
        };
        let pack_index: Value =
            serde_json::from_slice(&text).with_context(|| format!("{}/index.json", folder.name))?;
        let pack_entries = entries_of(&pack_index, folder.keys);
        for &(key, entry) in &pack_entries {
            let target = target_folder(folder.name, &entry_name(entry).unwrap_or_default());
            let index = indexes.entry(target).or_insert_with(|| {
                std::fs::read_to_string(store.join(target).join("index.json"))
                    .ok()
                    .and_then(|text| serde_json::from_str(&text).ok())
                    .unwrap_or_else(|| Value::Object(serde_json::Map::new()))
            });
            let Some(map) = index.as_object_mut() else {
                continue;
            };
            let list = map.entry(key.to_owned()).or_insert_with(|| Value::Array(Vec::new()));
            let Some(list) = list.as_array_mut() else {
                continue;
            };
            match same_thing(list, entry) {
                Some(store_entry) => merge_entry(store_entry, entry),
                None => {
                    let mut merged = entry.clone();
                    png_names(&mut merged);
                    list.push(merged);
                    report.added += 1;
                    *touched.entry(target).or_default() += 1;
                }
            }
        }
        for (target, index) in &indexes {
            let path = store.join(target).join("index.json");
            std::fs::write(&path, serde_json::to_vec_pretty(index)?)
                .with_context(|| format!("writing {}", path.display()))?;
        }
        let mut line = format!("{art:>7} files of {}, {} entries", folder.name, pack_entries.len());
        for (target, count) in &touched {
            let _ = write!(line, ", {count} in {target}");
        }
        writeln!(out, "{line}")?;
    }

    if let Some(dir) = masks {
        report.masks = write_masks(&mut source, dir)?;
    }
    writeln!(
        out,
        "hd-pack-to-store: {} entries added, {} art files written, {} art files already there, \
         {} map tiles kept lossless, {} masks written",
        report.added, report.updated, report.kept, report.kept_lossless, report.masks
    )?;
    Ok(())
}

/// The store's folder a pack file or entry belongs to: the field's poses have their own.
fn target_folder(folder: &'static str, name: &str) -> &'static str {
    if folder == "class2" { object_folder(name) } else { folder }
}

/// The name an entry's art is known by, for its folder: its `id`, else its `code`.
fn entry_name(entry: &Value) -> Option<String> {
    entry
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| entry.get("code").map(Value::to_string))
}

/// The outlines of `masks.bin` the silhouettes folder has none of, as PNGs (white where the pose is
/// opaque).
fn write_masks(source: &mut Source, dir: &Path) -> anyhow::Result<usize> {
    let Some(bytes) = source.read("masks.bin")? else {
        return Ok(0);
    };
    let masks = ssd_hdpack::public::Masks::decode(&bytes)?;
    std::fs::create_dir_all(dir)?;
    let mut written = 0;
    for (key, width, height, bits) in masks.entries() {
        let path = dir.join(format!("{key:016x}.png"));
        if path.is_file() {
            continue;
        }
        let mut rgba = vec![0_u8; usize::from(width) * usize::from(height) * 4];
        for (at, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            if bits[at / 8] >> (at % 8) & 1 == 1 {
                pixel.copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        ssd_hd::png::write_rgba(&path, u32::from(width), u32::from(height), &rgba)
            .with_context(|| format!("writing {}", path.display()))?;
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_pack_entry_replaces_the_fields_it_defines_and_keeps_the_store_s_own() {
        let mut store =
            json!({"id": "a", "file": "a.png", "source_size": [8, 8], "cost": 1, "revision": 3});
        merge_entry(&mut store, &json!({"id": "a", "file": "a.webp", "source_size": [9, 9]}));
        assert_eq!(
            store,
            json!({"id": "a", "file": "a.png", "source_size": [9, 9], "cost": 1, "revision": 3})
        );
    }

    #[test]
    fn a_newer_store_entry_only_takes_the_fields_it_lacks() {
        let mut store = json!({"id": "a", "source_size": [8, 8], "revision": 4});
        merge_entry(
            &mut store,
            &json!({"id": "a", "source_size": [9, 9], "kind": "x", "revision": 2}),
        );
        assert_eq!(store, json!({"id": "a", "source_size": [8, 8], "kind": "x", "revision": 4}));
    }

    #[test]
    fn field_poses_go_to_their_own_folder() {
        assert_eq!(
            store_path("class2", "field-goku-000.webp"),
            Path::new("class3/field-goku-000.png")
        );
        assert_eq!(
            store_path("class2", "battle-goku-000.webp"),
            Path::new("class2/battle-goku-000.png")
        );
        assert_eq!(store_path("maps", "map-00/t0_0.webp"), Path::new("maps/map-00/t0_0.png"));
    }
}
