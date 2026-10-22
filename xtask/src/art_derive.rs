//! `art-derive <hd.png> <face|tiles> <out dir> [--palette <file>]`: the SNES art of an HD picture (`docs/mods/art.md`).
//!
//! `face` takes a 128 x 128 picture (a face: 4 x 4 characters at 4x) and derives its 16 tiles in the palette given, or in
//! one chosen from the picture. `tiles` takes a map sheet (a multiple of 32 pixels each way: whole 8 x 8 tiles at 4x) and
//! derives its distinct tiles: a cell that is another tile flipped shows it with the flip. The derivation is `ssd_mod::derive`,
//! the code the packs use for their faces. The outputs go to the folder `<out dir>`, which is under `.work/` when it is in
//! the repository (the refusal of `text-dump`):
//!
//! - `palette.toml`: the 16 SNES colours of the palette (word 0 is transparent);
//! - `tiles.bin`: the 4-bit tiles, 32 bytes each, in the SNES layout (the bytes the game holds in VRAM);
//! - `tiles.lz`: the stream the game decodes (`00:C559`, then the regroup `00:C68C`): the tiles of a sheet, or the record of
//!   a face (its 16 tiles in both pictures of 32 tiles);
//! - `map.toml`: the cells row by row, as `[tile, h, v]` (flips 0 or 1), with `columns` and `rows`.

use anyhow::Context;
use ssd_mod::derive::{self, Face, Sheet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The arguments of the command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Args {
    pub hd: PathBuf,
    pub kind: Kind,
    pub out: PathBuf,
    pub palette: Option<PathBuf>,
}

/// What the picture is: a face, or a map sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Face,
    Tiles,
}

/// The arguments `art-derive` takes, in plain words when one is wrong.
///
/// # Errors
///
/// A missing or extra argument, an unknown option, or a kind that is neither `face` nor `tiles`.
pub fn parse(args: &[&str]) -> Result<Args, String> {
    let mut palette = None;
    let mut positional = Vec::new();
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        if arg == "--palette" {
            let file = rest.next().ok_or("--palette needs a file")?;
            palette = Some(PathBuf::from(file));
        } else if arg.starts_with("--") {
            return Err(format!("the option `{arg}` is not known"));
        } else {
            positional.push(arg);
        }
    }
    let [hd, kind, out] = positional.as_slice() else {
        return Err("takes <hd.png> <face|tiles> <out dir> [--palette <file>]".to_owned());
    };
    let kind = match *kind {
        "face" => Kind::Face,
        "tiles" => Kind::Tiles,
        other => return Err(format!("the kind `{other}` is not `face` or `tiles`")),
    };
    Ok(Args { hd: PathBuf::from(hd), kind, out: PathBuf::from(out), palette })
}

/// The command: derives the picture and writes the four files; the refusals are printed.
///
/// # Errors
///
/// A file that cannot be read or written, or a picture or palette that does not derive (the message says why).
pub fn command(args: &[&str]) -> anyhow::Result<ExitCode> {
    let parsed = match parse(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("art-derive: {message}");
            return Ok(crate::usage());
        }
    };
    let out = std::path::absolute(&parsed.out)?;
    if let Some(reason) = crate::text_dump::refused(&crate::repo_root()?, &out) {
        eprintln!("art-derive: {reason}");
        return Ok(crate::usage());
    }
    let summary = run(&parsed.hd, parsed.kind, &out, parsed.palette.as_deref())?;
    println!("art-derive: {summary}, written to {}", out.display());
    Ok(ExitCode::SUCCESS)
}

/// Derives the HD picture `hd` as `kind` and writes the files into `out`; returns a summary line.
///
/// # Errors
///
/// As [`command`].
pub fn run(hd: &Path, kind: Kind, out: &Path, palette: Option<&Path>) -> anyhow::Result<String> {
    let bytes = std::fs::read(hd).with_context(|| format!("reading {}", hd.display()))?;
    let picture = derive::read_png(&bytes)?;
    let given = match palette {
        Some(file) => {
            let text = std::fs::read_to_string(file)
                .with_context(|| format!("reading {}", file.display()))?;
            Some(derive::palette_from_text(&text)?)
        }
        None => None,
    };
    let (palette, tiles, stream, cells, columns, rows, summary) = match kind {
        Kind::Face => {
            // The palette the face is drawn in, if none is given: the colours of the picture, chosen by the same rule
            let palette =
                given.unwrap_or_else(|| derive::choose_palette(&derive::downsample(&picture)));
            let face: Face = derive::derive_face(&picture, palette)?;
            let stream = derive::face_record(&face.tiles).context("the face's record is empty")?;
            let cells: Vec<[u16; 3]> = (0..derive::FACE_TILES)
                .map(|cell| [u16::try_from(cell).unwrap_or(0), 0, 0])
                .collect();
            (
                face.palette,
                face.tiles.to_vec(),
                stream,
                cells,
                4,
                4,
                format!("face: {} tiles", derive::FACE_TILES),
            )
        }
        Kind::Tiles => {
            let sheet: Sheet = derive::derive_sheet(&picture, given)?;
            let stream = derive::sheet_stream(&sheet.tiles).context("the sheet has no tiles")?;
            let cells: Vec<[u16; 3]> = sheet
                .cells
                .iter()
                .map(|cell| [cell.tile, u16::from(cell.h), u16::from(cell.v)])
                .collect();
            let summary = format!(
                "tiles: {} distinct tiles in {} x {} cells",
                sheet.tiles.len(),
                sheet.columns,
                sheet.rows
            );
            (sheet.palette, sheet.tiles, stream, cells, sheet.columns, sheet.rows, summary)
        }
    };
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    let write = |name: &str, bytes: &[u8]| {
        std::fs::write(out.join(name), bytes).with_context(|| format!("writing {name}"))
    };
    write("palette.toml", derive::palette_text(&palette).as_bytes())?;
    write("tiles.bin", &tiles.iter().flatten().copied().collect::<Vec<u8>>())?;
    write("tiles.lz", &stream)?;
    write("map.toml", map_text(columns, rows, &cells)?.as_bytes())?;
    Ok(summary)
}

/// The `map.toml` of a picture: its size in cells, and the cells row by row.
#[derive(serde::Serialize)]
struct MapFile {
    columns: usize,
    rows: usize,
    cells: Vec<[u16; 3]>,
}

fn map_text(columns: usize, rows: usize, cells: &[[u16; 3]]) -> anyhow::Result<String> {
    let text = toml::to_string(&MapFile { columns, rows, cells: cells.to_vec() })?;
    Ok(format!("# The cells row by row: [tile, h flip, v flip].\n{text}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh folder for one test (removed by the test).
    fn scratch(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("art-derive-{name}-{}", std::process::id()))
    }

    /// A PNG of `width` x `height` RGBA pixels, `colour(x, y)` opaque.
    fn png(width: u32, height: u32, colour: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let mut rgba = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let [r, g, b] = colour(x, y);
                rgba.extend_from_slice(&[r, g, b, 255]);
            }
        }
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.write_header().unwrap().write_image_data(&rgba).unwrap();
        out
    }

    #[test]
    fn the_arguments_are_read_or_named_in_plain_words() {
        assert_eq!(
            parse(&["a.png", "tiles", "out", "--palette", "p.toml"]).unwrap(),
            Args {
                hd: PathBuf::from("a.png"),
                kind: Kind::Tiles,
                out: PathBuf::from("out"),
                palette: Some(PathBuf::from("p.toml")),
            }
        );
        assert_eq!(parse(&["a.png", "face", "out"]).unwrap().kind, Kind::Face);
        assert!(parse(&["a.png", "room", "out"]).unwrap_err().contains("`room`"));
        assert!(parse(&["a.png", "face"]).unwrap_err().contains("<out dir>"));
        assert!(parse(&["a.png", "face", "out", "--size", "4"]).unwrap_err().contains("--size"));
        assert!(parse(&["a.png", "face", "out", "--palette"]).unwrap_err().contains("--palette"));
    }

    #[test]
    fn a_sheet_writes_tiles_that_the_game_decodes_back_to_the_same_bytes() {
        let dir = scratch("sheet");
        let hd = dir.join("sheet.png");
        std::fs::create_dir_all(&dir).unwrap();
        // 64 x 32 HD: two tiles side by side, black and white
        std::fs::write(&hd, png(64, 32, |x, _| if x < 32 { [0, 0, 0] } else { [255, 255, 255] }))
            .unwrap();
        let out = dir.join("out");
        let summary = run(&hd, Kind::Tiles, &out, None).unwrap();
        assert!(summary.contains("2 distinct tiles"), "{summary}");
        let tiles = std::fs::read(out.join("tiles.bin")).unwrap();
        assert_eq!(tiles.len(), 64);
        // The game's LZ decoder and its regroup give the tiles back from tiles.lz
        let stream =
            ssd_catalogue::graphics::lz_decode(&std::fs::read(out.join("tiles.lz")).unwrap(), 0)
                .unwrap();
        let mut regrouped = stream.data;
        ssd_catalogue::graphics::regroup(&mut regrouped);
        assert_eq!(regrouped, tiles);
        let palette =
            derive::palette_from_text(&std::fs::read_to_string(out.join("palette.toml")).unwrap())
                .unwrap();
        assert_eq!(palette[1], 0);
        let map = std::fs::read_to_string(out.join("map.toml")).unwrap();
        assert!(map.contains("columns = 2") && map.contains("rows = 1"), "{map}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_face_writes_the_record_of_both_pictures_the_game_decodes() {
        let dir = scratch("face");
        std::fs::create_dir_all(&dir).unwrap();
        let hd = dir.join("face.png");
        std::fs::write(
            &hd,
            png(
                128,
                128,
                |x, y| if (x / 16 + y / 16) % 2 == 0 { [200, 40, 40] } else { [30, 30, 200] },
            ),
        )
        .unwrap();
        let out = dir.join("out");
        let summary = run(&hd, Kind::Face, &out, None).unwrap();
        assert_eq!(summary, "face: 16 tiles");
        let tiles = std::fs::read(out.join("tiles.bin")).unwrap();
        assert_eq!(tiles.len(), 16 * 32);
        let stream =
            ssd_catalogue::graphics::lz_decode(&std::fs::read(out.join("tiles.lz")).unwrap(), 0)
                .unwrap();
        assert_eq!(stream.data.len(), 1024, "32 tiles: two pictures");
        let mut regrouped = stream.data;
        ssd_catalogue::graphics::regroup(&mut regrouped);
        for cell in 0..16 {
            let face_tile = &tiles[cell * 32..cell * 32 + 32];
            assert_eq!(
                &regrouped[derive::FIRST_PICTURE[cell] * 32..][..32],
                face_tile,
                "first picture, cell {cell}"
            );
            assert_eq!(
                &regrouped[derive::SECOND_PICTURE[cell] * 32..][..32],
                face_tile,
                "second picture, cell {cell}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
