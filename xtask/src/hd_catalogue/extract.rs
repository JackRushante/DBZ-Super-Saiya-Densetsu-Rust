//! `hd-catalogue extract`: every battle pose and portrait the ROM holds, decoded.

use super::decode::{self, Rom};
use super::{Catalogue, Category, Entry, hex_key};
use anyhow::Context;
use ssd_game::data as d;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;

/// Units whose poses are not drawn the way the others are (documented exceptions of the decoding):
/// poses whose tile data is short, a stream that does not decode, a pose with no pixel. They are
/// listed in the catalogue's problems and their entries carry a note.
const FACE_PALETTE_NOTE: &str =
    "native colours: the corner-face palette (object 28), the screen's palette varies";

/// The text of `palette` as `#RRGGBB` for each colour.
fn hex_palette(words: &[u16; 16]) -> Vec<String> {
    words
        .iter()
        .map(|&word| {
            let [r, g, b] = ssd_hd::resolve::rgb555(word);
            format!("#{r:02X}{g:02X}{b:02X}")
        })
        .collect()
}

/// Writes palette indexes as an indexed PNG in `words` (index 0 transparent): the picture and its
/// indexes in one file.
fn write_native(
    path: &Path,
    w: usize,
    h: usize,
    indexes: &[u8],
    words: &[u16; 16],
) -> std::io::Result<()> {
    let colours: Vec<[u8; 3]> = words.iter().map(|&word| ssd_hd::resolve::rgb555(word)).collect();
    ssd_hd::png::write_indexed(path, w as u32, h as u32, indexes, &colours, Some(0))
}

fn address_text(address: u32) -> String {
    format!("{:02X}:{:04X}", address >> 16, address & 0xFFFF)
}

/// The id of a set's half in the art store (`face-91D4-1`): the bank is part of it when it is not 16.
pub(crate) fn stream_id(address: u32, half: u8) -> String {
    if address >> 16 == 0x16 {
        format!("face-{:04X}-{half}", address & 0xFFFF)
    } else {
        format!("face-{address:06X}-{half}")
    }
}

pub fn run(
    rom_bytes: &[u8],
    out: &Path,
    observed_log: Option<&Path>,
    log: &mut dyn Write,
) -> anyhow::Result<()> {
    let rom = Rom::new(rom_bytes)?;
    let game = ssd_catalogue::Catalogue::from_rom(rom_bytes).context("reading the unit names")?;
    let name_of = |unit: u8| {
        game.enemies.get(usize::from(unit)).map(|enemy| enemy.name.text.clone()).unwrap_or_default()
    };
    let native = out.join("native");
    if native.exists() {
        std::fs::remove_dir_all(&native)?;
    }
    std::fs::create_dir_all(&native)?;
    let mut catalogue = Catalogue::default();
    // Which pose numbers the game can ask for (scripts), and which loads replays saw
    let pose_use = super::scripts::static_poses(rom)?;
    let observed = match observed_log {
        Some(path) => super::scripts::observed(&std::fs::read_to_string(path)?),
        None => BTreeMap::new(),
    };
    for ((unit, pose), replays) in &observed {
        if !pose_use.poses.contains_key(pose) {
            catalogue.problems.push(format!(
                "pose {pose} of unit {unit} was loaded in {} replays but no script sets it: the static set is short",
                replays.len()
            ));
        }
    }

    // The battle poses: each unit draws the poses of its poseset in its own palette
    let mut posesets: BTreeMap<u8, Vec<Result<decode::Pose, String>>> = BTreeMap::new();
    for unit in 0..decode::UNITS {
        let Some(row) = decode::unit_row(rom, unit) else {
            catalogue.problems.push(format!("unit {unit}: no row in the unit table"));
            continue;
        };
        let Some(palette) = decode::unit_palette(rom, row) else {
            catalogue.problems.push(format!("unit {unit}: its palette is not in the ROM"));
            continue;
        };
        if row.poseset >= decode::POSESETS {
            catalogue
                .problems
                .push(format!("unit {unit}: poseset {} is out of range", row.poseset));
            continue;
        }
        let poses = posesets.entry(row.poseset).or_insert_with(|| {
            (0..decode::pose_count(rom, row.poseset))
                .map(|pose| decode::decode_pose(rom, row.poseset, pose))
                .collect()
        });
        if poses.is_empty() {
            catalogue.problems.push(format!("unit {unit}: poseset {} has no poses", row.poseset));
        }
        for (n, decoded) in poses.iter().enumerate() {
            let id = format!("pose-u{unit:03}-p{n:02}");
            let mut entry = Entry {
                id: id.clone(),
                category: Category::BattlePose,
                units: vec![unit],
                unit_names: vec![name_of(unit)],
                poseset: Some(row.poseset),
                pose: Some(n),
                stream: None,
                half: None,
                object: None,
                shown_by: Vec::new(),
                w: 0,
                h: 0,
                key: None,
                mirror_key: None,
                vflip_keys: None,
                cluster_keys: Vec::new(),
                origin: String::new(),
                palette: hex_palette(&palette),
                png: String::new(),
                note: None,
                used: true,
                evidence: Vec::new(),
                basis: String::new(),
            };
            let seen = observed.get(&(unit, n as u8));
            let reached = u8::try_from(n).ok().and_then(|n| pose_use.poses.get(&n));
            entry.used = reached.is_some() || seen.is_some();
            entry.evidence = reached.cloned().unwrap_or_default();
            if let Some(replays) = seen {
                let names: Vec<&str> = replays.iter().map(String::as_str).take(4).collect();
                entry.evidence.push(format!(
                    "loaded in {} replays: {}",
                    replays.len(),
                    names.join(", ")
                ));
            }
            let basis = if seen.is_some() {
                "observed"
            } else if entry.evidence.iter().any(|e| !e.contains("reached by a branch"))
                && entry.used
            {
                "script"
            } else if entry.used {
                "branch"
            } else {
                "none"
            };
            basis.clone_into(&mut entry.basis);
            if !entry.used {
                entry
                    .evidence
                    .push("no script, routine or replay sets this pose number".to_owned());
            }
            match decoded {
                Err(reason) => {
                    entry.note = Some(format!("does not decode: {reason}"));
                    catalogue.problems.push(format!("poseset {} pose {n}: {reason}", row.poseset));
                }
                Ok(pose) => {
                    entry.origin = address_text(pose.layout_at);
                    match &pose.shape {
                        None => entry.note = Some("no opaque pixel".to_owned()),
                        Some(shape) => {
                            let (key, mirror) = shape.keys();
                            entry.w = shape.w;
                            entry.h = shape.h;
                            entry.key = Some(hex_key(key));
                            entry.mirror_key = Some(hex_key(mirror));
                            if pose.short_tiles {
                                entry.note = Some("the tile data is short of a tile the layout names: not the game's key".to_owned());
                            }
                            let (up, up_mirror) = shape.keys_upside_down();
                            entry.vflip_keys = Some((hex_key(up), hex_key(up_mirror)));
                            for cluster in &pose.clusters {
                                let (key, mirror) = cluster.keys();
                                entry.cluster_keys.push((hex_key(key), hex_key(mirror)));
                            }
                            entry.png = format!("native/{id}.png");
                            write_native(
                                &out.join(&entry.png),
                                shape.w,
                                shape.h,
                                &shape.arr,
                                &palette,
                            )?;
                        }
                    }
                }
            }
            if entry.note.is_some() && entry.key.is_none() {
                catalogue
                    .problems
                    .push(format!("{id}: {}", entry.note.as_deref().unwrap_or_default()));
            }
            catalogue.entries.push(entry);
        }
    }

    // The face portraits: the table has a row per unit; the sets are shared, each with two halves
    let view = rom.view();
    let rows = d::FACE_PORTRAIT_POINTERS_SIZE / 3;
    let mut streams: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    for unit in 0..rows.min(u32::from(decode::UNITS)) {
        let entry = d::FACE_PORTRAIT_POINTERS + 3 * unit;
        let (Ok(word), Ok(bank)) = (view.u16(entry), view.u8(entry + 2)) else { continue };
        streams.entry(u32::from(bank) << 16 | u32::from(word)).or_default().push(unit as u8);
    }
    for (&stream, units) in &streams {
        let Some(halves) = decode::set_pictures(rom, stream) else {
            catalogue.problems.push(format!("face set {}: does not decode", address_text(stream)));
            continue;
        };
        let words = units
            .iter()
            .find_map(|&unit| ssd_catalogue::face_portrait(view, unit).ok())
            .map_or([0_u16; 16], |portrait| portrait.palette);
        for picture in halves {
            let shown_by: Vec<u8> = units
                .iter()
                .copied()
                .filter(|&unit| {
                    ssd_catalogue::face_portrait(view, unit).is_ok_and(|p| p.half == picture.half)
                })
                .collect();
            let id = stream_id(stream, picture.half);
            let png = format!("native/{id}.png");
            write_native(&out.join(&png), 32, 32, &picture.pixels, &words)?;
            catalogue.entries.push(Entry {
                id,
                category: Category::Face,
                units: units.clone(),
                unit_names: units.iter().map(|&unit| name_of(unit)).collect(),
                poseset: None,
                pose: None,
                stream: Some(address_text(stream)),
                half: Some(picture.half),
                object: None,
                shown_by,
                w: 32,
                h: 32,
                key: None,
                mirror_key: None,
                vflip_keys: None,
                cluster_keys: Vec::new(),
                origin: address_text(stream),
                palette: hex_palette(&words),
                png,
                note: Some(FACE_PALETTE_NOTE.to_owned()),
                used: true,
                evidence: vec!["portrait set".to_owned()],
                basis: "portrait".to_owned(),
            });
        }
    }

    // The object portraits: one row per object id
    let object_names: Vec<String> = game.items.iter().map(|item| item.name.text.clone()).collect();
    for object in 0..(d::OBJECT_PORTRAIT_POINTERS_SIZE / 3) {
        let object = object as u8;
        let Ok(portrait) = ssd_catalogue::object_portrait(view, object) else {
            catalogue.problems.push(format!("object {object}: its portrait does not decode"));
            continue;
        };
        let at = d::OBJECT_PORTRAIT_POINTERS + 3 * u32::from(object);
        let stream = u32::from(view.u8(at + 2)?) << 16 | u32::from(view.u16(at)?);
        let id = format!("obj-{object:02}");
        let png = format!("native/{id}.png");
        write_native(&out.join(&png), 32, 32, &portrait.pixels, &portrait.palette)?;
        catalogue.entries.push(Entry {
            id,
            category: Category::ObjectPortrait,
            units: Vec::new(),
            unit_names: object_names.get(usize::from(object)).cloned().into_iter().collect(),
            poseset: None,
            pose: None,
            stream: Some(address_text(stream)),
            half: Some(portrait.half),
            object: Some(object),
            shown_by: Vec::new(),
            w: 32,
            h: 32,
            key: None,
            mirror_key: None,
            vflip_keys: None,
            cluster_keys: Vec::new(),
            origin: address_text(stream),
            palette: hex_palette(&portrait.palette),
            png,
            note: None,
            used: true,
            evidence: vec!["portrait set".to_owned()],
            basis: "portrait".to_owned(),
        });
    }

    std::fs::write(out.join("catalogue.json"), serde_json::to_string_pretty(&catalogue)?)?;
    let poses = catalogue.entries.iter().filter(|e| e.category == Category::BattlePose).count();
    let keyed = catalogue.entries.iter().filter(|e| e.key.is_some()).count();
    let distinct: std::collections::BTreeSet<&String> =
        catalogue.entries.iter().filter_map(|e| e.key.as_ref()).collect();
    writeln!(
        log,
        "hd-catalogue extract: {poses} battle poses ({keyed} with a key, {} distinct), {} face halves, {} object portraits, {} problems -> {}",
        distinct.len(),
        catalogue.entries.iter().filter(|e| e.category == Category::Face).count(),
        catalogue.entries.iter().filter(|e| e.category == Category::ObjectPortrait).count(),
        catalogue.problems.len(),
        out.display()
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With the ROM (`SSD_ROM_PATH`): the poses decode to the keys the renderer finds the art by (checked
    /// against the keys of the HD art made from the poses of real frames), and each portrait set both ways.
    #[test]
    #[ignore = "needs the ROM (SSD_ROM_PATH)"]
    fn the_decoded_poses_have_the_keys_of_the_art() {
        let bytes = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
        let rom = Rom::new(&bytes).unwrap();
        let key = |k: u8, pose: usize| {
            let pose = decode::decode_pose(rom, k, pose).unwrap();
            let shape = pose.shape.unwrap();
            let (key, mirror) = shape.keys();
            (hex_key(key), hex_key(mirror), shape.w, shape.h)
        };
        assert_eq!(
            key(0, 0),
            ("3326f5a65ecce1eb".to_owned(), "6fb1a1bab4f00294".to_owned(), 24, 52)
        );
        assert_eq!(
            key(0, 1),
            ("793bfd82d98e905b".to_owned(), "6d856221801be959".to_owned(), 36, 34)
        );
        assert_eq!(
            key(0, 2),
            ("b91264656f00091b".to_owned(), "0ac44e38a8bac877".to_owned(), 63, 24)
        );
        assert_eq!(
            key(0, 5),
            ("732890cac370e639".to_owned(), "b6da57942d121cab".to_owned(), 50, 48)
        );
        // Every unit of the table has a row, and the posesets name between them hundreds of poses
        let total: usize = (0..decode::POSESETS).map(|k| decode::pose_count(rom, k)).sum();
        assert!(total > 400, "{total} poses");
        // A portrait set has two pictures of 32 x 32
        let sets = decode::set_pictures(rom, 0x16_91D4).unwrap();
        assert_eq!(sets[1].pixels.len(), 1024);
        assert_ne!(sets[0].pixels, sets[1].pixels);
    }
}
