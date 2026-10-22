//! The packs a player has installed, and the set a run uses (M10, batch 5b). The packs of the data folder are the folders
//! with a `pack.toml` and the `.ssdpack` zips in its `mods` folder. The file `mods/mods.toml` names the enabled packs, by
//! id, in the order they load in:
//!
//! ```toml
//! enabled = ["super.kaioken", "free.progression"]
//! ```
//!
//! A missing file enables none. The file is written by the loader screen when it applies a set, and read at the start of a
//! run that persists (see the README, "Data packs").

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The folder, under the data folder, that holds the packs and the enabled set's file.
pub const FOLDER: &str = "mods";
/// The file in [`FOLDER`] that names the enabled packs.
pub const FILE: &str = "mods.toml";
/// The extension of a pack zip (`name.ssdpack`).
pub const ZIP_EXTENSION: &str = "ssdpack";

/// The `mods.toml` table as it is written. An unknown key is refused, as a manifest's is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Enabled {
    #[serde(default)]
    enabled: Vec<String>,
}

/// The ids of the enabled packs of a `mods.toml`, in their order.
///
/// # Errors
///
/// The text is not TOML, has a key other than `enabled`, or `enabled` is not a list of text.
pub fn parse(text: &str) -> Result<Vec<String>, String> {
    toml::from_str::<Enabled>(text)
        .map(|enabled| enabled.enabled)
        .map_err(|error| error.to_string())
}

/// The text of a `mods.toml` that enables `ids`, in that order.
#[must_use]
pub fn text(ids: &[String]) -> String {
    toml::to_string(&Enabled { enabled: ids.to_vec() }).unwrap_or_default()
}

/// The packs installed in `folder`: each folder with a `pack.toml` and each `.ssdpack` file, by path, in file-name order.
/// A folder that cannot be read has none.
#[must_use]
pub fn installed(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            if path.is_dir() {
                path.join(crate::pack::MANIFEST).is_file()
            } else {
                path.extension().is_some_and(|extension| extension == ZIP_EXTENSION)
            }
        })
        .collect();
    found.sort();
    found
}

/// The size of a pack in bytes: its file's length, or the sum of the files under its folder.
#[must_use]
pub fn size(path: &Path) -> u64 {
    if !path.is_dir() {
        return std::fs::metadata(path).map_or(0, |metadata| metadata.len());
    }
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries.filter_map(Result::ok).map(|entry| size(&entry.path())).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder under the system's temporary folder, named by the test (removed by the caller).
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ssd-mod-set-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_enabled_list_keeps_its_order_and_a_damaged_file_is_refused() {
        let ids = vec!["free.progression".to_owned(), "super.kaioken".to_owned()];
        let text = text(&ids);
        assert_eq!(parse(&text), Ok(ids), "the order is kept: {text}");
        assert_eq!(parse("").unwrap(), Vec::<String>::new(), "no key: none enabled");
        assert!(parse("enabled = 3").is_err(), "not a list");
        assert!(parse("enable = []").is_err(), "a misspelt key is refused");
        assert!(parse("enabled = [").is_err(), "not TOML");
    }

    #[test]
    fn the_installed_packs_are_the_folders_with_a_manifest_and_the_zips() {
        let dir = scratch("installed");
        std::fs::create_dir_all(dir.join("a-folder")).unwrap();
        std::fs::write(dir.join("a-folder").join("pack.toml"), "").unwrap();
        std::fs::create_dir_all(dir.join("not-a-pack")).unwrap();
        std::fs::write(dir.join("b.ssdpack"), b"zip").unwrap();
        std::fs::write(dir.join("c.zip"), b"zip").unwrap();
        std::fs::write(dir.join("mods.toml"), "enabled = []").unwrap();
        let found: Vec<String> = installed(&dir)
            .iter()
            .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(found, ["a-folder", "b.ssdpack"]);
        assert_eq!(installed(&dir.join("missing")), Vec::<PathBuf>::new());
        assert_eq!(size(&dir.join("b.ssdpack")), 3);
        assert_eq!(size(&dir.join("a-folder")), 0, "an empty manifest is zero bytes");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
