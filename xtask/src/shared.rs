//! What the commands that play replays share: the counters table and reading a replay file.

use anyhow::Context;
use ssd_oracle::{CORE_VERSION, Replay};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// A table under `header`, one line per row of `rows`, every line indented by two spaces and
/// ending in a newline: the first column to the left (names), the others to the right (numbers),
/// every column as wide as its widest cell. Nothing without rows.
///
/// # Panics
///
/// A row with another number of cells than the header.
#[must_use]
pub fn render_table(header: &[&str], rows: &[Vec<String>]) -> String {
    if rows.is_empty() {
        return String::new();
    }
    assert!(rows.iter().all(|row| row.len() == header.len()), "a row as wide as the header");
    let widths: Vec<usize> = (0..header.len())
        .map(|column| {
            rows.iter().map(|row| row[column].len()).fold(header[column].len(), usize::max)
        })
        .collect();
    let header: Vec<String> = header.iter().map(|&cell| cell.to_owned()).collect();
    let mut table = String::new();
    for cells in std::iter::once(&header).chain(rows) {
        table.push_str("  ");
        for (column, cell) in cells.iter().enumerate() {
            let width = widths[column];
            let _ = match column {
                0 => write!(table, "{cell:<width$}"),
                _ => write!(table, "  {cell:>width$}"),
            };
        }
        table.push('\n');
    }
    table
}

/// The replay file at `path`, for the ROM `rom` and this build's emulator core.
///
/// # Errors
///
/// The file cannot be read, or is not a replay of `rom` for this core (the message says to
/// record the reference replays again with `make-replays`).
pub fn read_replay(path: &Path, rom: &[u8]) -> anyhow::Result<Replay> {
    let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let bytes = std::fs::read(path).with_context(|| {
        format!("reading {} (the reference replays are made by `make-replays`)", path.display())
    })?;
    Replay::from_bytes(&bytes, ssd_rom::crc32(rom), CORE_VERSION)
        .with_context(|| format!("{name}: regenerate it with `make-replays`"))
}

/// The replays named by `paths` (files, or directories of replays), or the directory
/// `default_dir` when none is named.
///
/// # Errors
///
/// A directory cannot be read.
pub fn replay_files(paths: &[PathBuf], default_dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    let paths = if paths.is_empty() { vec![default_dir.to_path_buf()] } else { paths.to_vec() };
    let mut files = Vec::new();
    for path in &paths {
        if path.is_dir() {
            files.extend(crate::verify_replays::replay_paths(path)?);
        } else {
            files.push(path.clone());
        }
    }
    Ok(files)
}

/// The pack-set tag of `image` (a replay's [`Replay::packs`] for it).
#[must_use]
pub fn pack_tag(image: &ssd_mod::Image) -> ssd_oracle::PackTag {
    ssd_oracle::PackTag { set_id: image.set_id, image_crc32: image.crc32 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(cells: &[&str]) -> Vec<String> {
        cells.iter().map(|&cell| cell.to_owned()).collect()
    }

    #[test]
    fn a_table_has_the_header_and_a_row_each_aligned_by_column() {
        let table = render_table(
            &["routine", "calls"],
            &[row(&["rng_next_a", "7"]), row(&["a", "1234567"])],
        );
        let expected =
            ["  routine       calls", "  rng_next_a        7", "  a           1234567", ""];
        assert_eq!(table, expected.join("\n"));
    }

    #[test]
    fn a_table_without_rows_is_empty() {
        assert_eq!(render_table(&["routine", "calls"], &[]), "");
    }

    #[test]
    #[should_panic(expected = "as wide as the header")]
    fn a_row_of_another_width_is_a_mistake_of_the_caller() {
        let _ = render_table(&["routine", "calls"], &[row(&["rng_next_a"])]);
    }

    #[test]
    fn a_replay_that_cannot_be_read_or_played_says_how_to_get_one() {
        let dir = tempfile::tempdir().unwrap();
        let missing = read_replay(&dir.path().join("battle.ssdreplay"), &[]).unwrap_err();
        let missing = format!("{missing:#}");
        assert!(
            missing.contains("battle.ssdreplay") && missing.contains("make-replays"),
            "{missing}"
        );

        let path = dir.path().join("junk.ssdreplay");
        std::fs::write(&path, b"not a replay").unwrap();
        let junk = format!("{:#}", read_replay(&path, &[]).unwrap_err());
        assert!(junk.contains("junk.ssdreplay") && junk.contains("make-replays"), "{junk}");
        assert!(junk.contains("SSDRPLAY"), "the reason follows: {junk}");
    }
}
