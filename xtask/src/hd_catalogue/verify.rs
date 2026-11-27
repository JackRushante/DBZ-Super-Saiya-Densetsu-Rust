//! `hd-catalogue verify`: for every entry of the catalogue, the art the renderer would draw, scored.

use super::identity::{Compare, Identity};
use super::score;
use super::{Category, Entry, parse_key, read_catalogue};
use anyhow::Context;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use ssd_hd::art_index::{ArtIndex, Object, Palette};
use ssd_hd::resolve::{self, PoseRef, Resolver};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The silhouette share under which a piece of art is not the shape of its pose.
const MIN_IOU: f32 = 0.80;
/// The mean distance (RGB, 0 to 441) of the art as drawn from the colour of its pixels' palette index over which the
/// colours are wrong. Spot-checked on pairs of native pictures and the art as drawn: redraws with other shading sit under
/// 90, the art of another colour scheme (a hair colour the recolouring cannot reach) over 100.
const MAX_COLOUR_DISTANCE: f32 = 95.0;
/// The luminance correlation under which a portrait is not the picture of its source.
const MIN_LUMA: f32 = 0.5;
/// The unit of Super Saiyan Goku (the party record that makes the renderer draw his Super Saiyan art).
const SUPER_SAIYAN_GOKU: u8 = 59;

/// The flags of a result.
pub const FLAGS: [&str; 6] = ["missing", "shape", "colour", "identity", "label", "shared"];

/// The art an entry resolved to.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtRef {
    pub id: String,
    /// The art file, relative to the art store.
    pub file: String,
    pub fighter: Option<String>,
    pub description: Option<String>,
    /// The text the identity check read.
    pub label: String,
    /// The identities the art is made for, by its name.
    pub made_for: Vec<String>,
    /// How far the art's palette is from the entry's (the renderer recolours the art when not 0).
    pub palette_distance: Option<u32>,
    /// The renderer recolours the art to the entry's palette: the picture of the art as drawn is `drawn`.
    pub recoloured: bool,
    /// The art as the renderer draws it, relative to the output folder, when it is recoloured.
    pub drawn: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Scores {
    pub iou: Option<f32>,
    pub colour_share: Option<f32>,
    pub colour_distance: Option<f32>,
    pub luma_correlation: Option<f32>,
    /// Per palette index of the native picture: pixels measured, mean colour distance, share that quantises back.
    pub colour_by_index: Vec<(u8, u32, f32, f32)>,
}

/// The verdict on one entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Verdict {
    pub id: String,
    pub category: Category,
    /// Whether the game can show it (see `scripts`); only a used entry counts for the gate.
    pub used: bool,
    pub basis: String,
    pub identity: Vec<String>,
    pub flags: Vec<String>,
    pub suspicion: f32,
    /// How the art was found: `exact`, `mirror`, `cluster`, `shaped`, `portrait` or `none`.
    pub found: String,
    pub art: Option<ArtRef>,
    pub scores: Scores,
    pub details: Vec<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Summary {
    /// The art store the report was made against.
    pub art_store: String,
    pub entries: usize,
    pub checked: usize,
    pub not_checked: usize,
    pub by_flag: BTreeMap<String, usize>,
    pub by_category_flag: BTreeMap<String, usize>,
    pub found_by: BTreeMap<String, usize>,
    pub without_identity: usize,
    /// Entries whose art is shared with another identity of the same sprite (not flagged).
    pub shared_siblings: usize,
    /// Entries the game can show, and entries it cannot (a pose no script sets).
    pub used: usize,
    pub unused: usize,
    /// Used entries by how the use is known (`observed`, `script`, `branch`, `portrait`).
    pub used_by_basis: BTreeMap<String, usize>,
    /// The flags of unused entries, kept apart (`by_flag` counts used entries only).
    pub unused_by_flag: BTreeMap<String, usize>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Report {
    pub summary: Summary,
    pub results: Vec<Verdict>,
}

/// The id of an art without the number of its pose: `battle-saibaman-004` is `battle-saibaman`.
pub fn stem_of(id: &str) -> String {
    ssd_hd::resolve::unit_of_id(id)
}

/// One art entry of class 2 or 3.
struct PoseArt {
    id: String,
    file: PathBuf,
    rel: String,
    fighter: Option<String>,
    description: Option<String>,
    label: String,
}

/// One portrait art of class 1.
struct PortraitArt {
    id: String,
    file: PathBuf,
    rel: String,
    stream: String,
    half: u8,
    palette_object: Option<u8>,
    description: Option<String>,
    label: String,
}

/// `rom 16:91D4 half 1 (...)` to `16:91D4` and the half.
fn parse_source(text: &str) -> Option<(String, u8)> {
    let mut words = text.split_whitespace();
    (words.next()? == "rom").then_some(())?;
    let stream = words.next()?.to_owned();
    (words.next()? == "half").then_some(())?;
    Some((stream, words.next()?.parse().ok()?))
}

/// The part of a portrait's prompt that says who it shows.
fn prompt_subject(prompt: &str) -> String {
    let start = prompt.find("portrait of ").map_or(0, |at| at + "portrait of ".len());
    let end = prompt[start..].find(" as a high-definition").map_or(prompt.len(), |at| start + at);
    prompt[start..end].to_owned()
}

fn string_field(entry: &serde_json::Value, key: &str) -> Option<String> {
    entry.get(key).and_then(serde_json::Value::as_str).map(str::to_owned)
}

fn read_index(path: &Path) -> anyhow::Result<serde_json::Value> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// The pose art of the store (every `class*` folder but class 1) as the renderer's index sees it.
fn load_poses(
    art: &Path,
    masks: &FxHashMap<u64, (u16, u16, Vec<u8>)>,
) -> anyhow::Result<(ArtIndex, Vec<PoseArt>)> {
    let mut objects = Vec::new();
    let mut infos = Vec::new();
    let mut folders: Vec<String> = std::fs::read_dir(art)?
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|name| {
            name.starts_with("class")
                && name != "class1"
                && art.join(name).join("index.json").is_file()
        })
        .collect();
    folders.sort();
    for folder in folders {
        let index = read_index(&art.join(&folder).join("index.json"))?;
        for entry in index["assets_list"].as_array().into_iter().flatten() {
            let (Some(id), Some(file), Some(key)) = (
                string_field(entry, "id"),
                string_field(entry, "file"),
                string_field(entry, "source_key"),
            ) else {
                continue;
            };
            let Some(key) = parse_key(&key) else { continue };
            let Some(size) = entry["source_size"].as_array() else { continue };
            let (w, h) =
                (size[0].as_u64().unwrap_or(0) as u16, size[1].as_u64().unwrap_or(0) as u16);
            let mut palette = Palette::default();
            for (index, colour) in entry["palette_indexes_used"].as_object().into_iter().flatten() {
                let (Ok(index), Some(text)) = (index.parse::<usize>(), colour.as_str()) else {
                    continue;
                };
                let hex = text.trim_start_matches('#');
                if index < 16 && hex.len() == 6 {
                    let channel = |at: usize| u8::from_str_radix(&hex[at..at + 2], 16).unwrap_or(0);
                    palette.colors[index] = [channel(0), channel(2), channel(4)];
                    palette.mask |= 1 << index;
                }
            }
            let mask = match masks.get(&key) {
                Some((mw, mh, bits)) if (*mw, *mh) == (w, h) => bits.clone(),
                _ => Vec::new(),
            };
            let luma = entry.get("luma").and_then(serde_json::Value::as_bool).unwrap_or(false);
            let description = string_field(entry, "description");
            let fighter = string_field(entry, "fighter");
            let label = format!(
                "{} {} {}",
                description.clone().unwrap_or_default(),
                fighter.clone().unwrap_or_default(),
                id
            );
            let rel = format!("{folder}/{file}");
            objects.push(Object {
                key,
                id: id.clone(),
                luma,
                palette,
                width: w,
                height: h,
                mask,
                art: rel.clone(),
            });
            infos.push(PoseArt {
                id,
                file: art.join(&folder).join(&file),
                rel,
                fighter,
                description,
                label,
            });
        }
    }
    Ok((ArtIndex::new(Vec::new(), objects), infos))
}

fn load_portraits(art: &Path) -> anyhow::Result<Vec<PortraitArt>> {
    let path = art.join("class1").join("index.json");
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let index = read_index(&path)?;
    let mut out = Vec::new();
    for entry in index["assets"].as_array().into_iter().flatten() {
        let (Some(id), Some(source)) = (string_field(entry, "id"), string_field(entry, "source"))
        else {
            continue;
        };
        let Some((stream, half)) = parse_source(&source) else { continue };
        let file = string_field(entry, "file").unwrap_or_else(|| format!("{id}.png"));
        let palette_object = string_field(entry, "palette").and_then(|text| {
            let rest = text.strip_prefix("object ")?;
            rest.split_whitespace().next()?.parse().ok()
        });
        let description = string_field(entry, "description");
        let prompt = string_field(entry, "prompt").unwrap_or_default();
        let label = format!(
            "{} {} {}",
            description.clone().unwrap_or_default(),
            prompt_subject(&prompt),
            id
        );
        out.push(PortraitArt {
            id,
            file: art.join("class1").join(&file),
            rel: format!("class1/{file}"),
            stream,
            half,
            palette_object,
            description,
            label,
        });
    }
    Ok(out)
}

/// The palette indexes of an indexed PNG written by `extract`.
pub(crate) fn read_indexed(path: &Path) -> anyhow::Result<(usize, usize, Vec<u8>)> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut reader = png::Decoder::new(std::io::BufReader::new(file)).read_info()?;
    let mut buffer = vec![0; reader.output_buffer_size().context("a large image")?];
    let info = reader.next_frame(&mut buffer)?;
    anyhow::ensure!(
        info.color_type == png::ColorType::Indexed,
        "{}: not an indexed image",
        path.display()
    );
    Ok((
        info.width as usize,
        info.height as usize,
        buffer[..info.width as usize * info.height as usize].to_vec(),
    ))
}

fn hex_colour(text: &str) -> [u8; 3] {
    let hex = text.trim_start_matches('#');
    let channel =
        |at: usize| u8::from_str_radix(hex.get(at..at + 2).unwrap_or("00"), 16).unwrap_or(0);
    [channel(0), channel(2), channel(4)]
}

fn row_of(entry: &Entry) -> [[u8; 3]; 16] {
    std::array::from_fn(|n| entry.palette.get(n).map_or([0; 3], |text| hex_colour(text)))
}

/// The palette indexes a picture uses, as a bit for each.
fn used_indexes(indexes: &[u8]) -> u16 {
    indexes.iter().fold(0, |used, &i| used | 1 << (i & 15))
}

/// Whether two palettes (`#RRGGBB` each) differ in a colour of an index in `used`.
fn palettes_differ(a: &[String], b: &[String], used: u16) -> bool {
    (1..16).any(|i| used & (1 << i) != 0 && a.get(i) != b.get(i))
}

/// The silhouette bitset of a pose (row by row, first pixel in the low bit), and of its mirror image.
fn silhouette(arr: &[u8], w: usize) -> (Vec<u8>, Vec<u8>) {
    let (mut plain, mut mirrored) =
        (vec![0_u8; arr.len().div_ceil(8)], vec![0_u8; arr.len().div_ceil(8)]);
    for (n, &c) in arr.iter().enumerate() {
        if c != 0 {
            plain[n / 8] |= 1 << (n % 8);
            let flipped = n / w * w + (w - 1 - n % w);
            mirrored[flipped / 8] |= 1 << (flipped % 8);
        }
    }
    (plain, mirrored)
}

fn read_art(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let bytes = std::fs::read(path).ok()?;
    ssd_hd::png::read_rgba(&bytes).ok()
}

/// The rows of a `w`-wide picture read right to left.
fn mirror_rows(indexes: &[u8], w: usize) -> Vec<u8> {
    indexes.chunks(w).flat_map(|row| row.iter().rev().copied()).collect()
}

pub fn run(
    out: &Path,
    art: &Path,
    identity_path: &Path,
    log: &mut dyn Write,
) -> anyhow::Result<Report> {
    let catalogue = read_catalogue(out)?;
    let identity = Identity::read(identity_path)?;
    let mut unit_poseset: BTreeMap<u8, u8> = BTreeMap::new();
    let mut unit_palettes: BTreeMap<u8, Vec<String>> = BTreeMap::new();
    // The units whose poses have each key (as decoded, mirrored or upside down)
    let mut key_units: BTreeMap<u64, BTreeSet<u8>> = BTreeMap::new();
    for entry in catalogue.entries.iter().filter(|e| e.category == Category::BattlePose) {
        let every = [&entry.key, &entry.mirror_key]
            .into_iter()
            .flatten()
            .chain(entry.vflip_keys.iter().flat_map(|(a, b)| [a, b]));
        for key in every.filter_map(|k| parse_key(k)) {
            key_units.entry(key).or_default().extend(entry.units.iter().copied());
        }
    }
    for entry in &catalogue.entries {
        if let (Some(&unit), Some(set)) = (entry.units.first(), entry.poseset)
            && entry.category == Category::BattlePose
        {
            unit_poseset.insert(unit, set);
            unit_palettes.entry(unit).or_insert_with(|| entry.palette.clone());
        }
    }
    // The silhouettes of the poses (the renderer's index holds them from the capture of the art):
    // a pose of the catalogue gives its own, and its mirror image's
    let mut masks: FxHashMap<u64, (u16, u16, Vec<u8>)> = FxHashMap::default();
    for entry in catalogue.entries.iter().filter(|e| e.category == Category::BattlePose) {
        let (Some(key), Some(mirror)) = (
            entry.key.as_deref().and_then(parse_key),
            entry.mirror_key.as_deref().and_then(parse_key),
        ) else {
            continue;
        };
        if masks.contains_key(&key) && masks.contains_key(&mirror) {
            continue;
        }
        let (w, h, arr) = read_indexed(&out.join(&entry.png))?;
        let (plain, flipped) = silhouette(&arr, w);
        masks.entry(key).or_insert((w as u16, h as u16, plain));
        masks.entry(mirror).or_insert((w as u16, h as u16, flipped));
    }
    let (index, poses) = load_poses(art, &masks)?;
    let portraits = load_portraits(art)?;
    writeln!(
        log,
        "hd-catalogue verify: {} pose arts, {} portrait arts, {} catalogue entries",
        poses.len(),
        portraits.len(),
        catalogue.entries.len()
    )?;

    let mut ctx = Ctx {
        out,
        index: &index,
        poses: &poses,
        identity: &identity,
        unit_poseset: &unit_poseset,
        unit_palettes: &unit_palettes,
        key_units: &key_units,
        sizes: OnceLock::new(),
        cache: FxHashMap::default(),
    };
    let mut cache: FxHashMap<PathBuf, Option<(u32, u32, Vec<u8>)>> = FxHashMap::default();
    let mut verdicts: Vec<Verdict> = Vec::new();
    // What drew an art: (verdict, entry, art id), for the sharing check
    let mut served: Vec<(usize, usize, String)> = Vec::new();

    for (n, entry) in catalogue.entries.iter().enumerate() {
        let slugs = identity.of_entry(entry);
        let mut verdict = Verdict {
            id: entry.id.clone(),
            category: entry.category,
            used: entry.used,
            basis: entry.basis.clone(),
            identity: slugs.clone(),
            flags: Vec::new(),
            suspicion: 0.0,
            found: "none".to_owned(),
            art: None,
            scores: Scores::default(),
            details: Vec::new(),
        };
        match entry.category {
            Category::BattlePose => {
                if entry.key.is_none()
                    || entry.note.as_deref().is_some_and(|note| note.contains("short"))
                {
                    "skipped".clone_into(&mut verdict.found);
                    verdict.details.push(format!(
                        "not checked: {}",
                        entry.note.as_deref().unwrap_or("no key")
                    ));
                    verdicts.push(verdict);
                    continue;
                }
                let key = entry.key.as_deref().and_then(parse_key).context("a pose key")?;
                let mirror =
                    entry.mirror_key.as_deref().and_then(parse_key).context("a mirror key")?;
                if let Some((found, art)) = ctx.pose(entry, &entry.id, (key, mirror), false)? {
                    if let Some(art) = art {
                        served.push((verdicts.len(), n, art));
                    }
                    verdicts.push(found);
                }
                // The pose drawn upside down is a pose of its own: checked when the index has art for it
                if let Some((k, m)) = &entry.vflip_keys
                    && let (Some(k), Some(m)) = (parse_key(k), parse_key(m))
                    && let Some((found, art)) =
                        ctx.pose(entry, &format!("{}~v", entry.id), (k, m), true)?
                {
                    if let Some(art) = art {
                        served.push((verdicts.len(), n, art));
                    }
                    verdicts.push(found);
                }
                continue;
            }
            Category::Face | Category::ObjectPortrait => {
                let stream = entry.stream.clone().unwrap_or_default();
                let half = entry.half.unwrap_or(0);
                let mut candidates: Vec<&PortraitArt> =
                    portraits.iter().filter(|p| p.stream == stream && p.half == half).collect();
                if let Some(object) = entry.object {
                    candidates.sort_by_key(|p| (p.palette_object != Some(object), p.id.clone()));
                } else {
                    candidates.sort_by_key(|p| (p.id != entry.id, p.id.clone()));
                }
                let Some(portrait) = candidates.first() else {
                    if entry.category == Category::ObjectPortrait
                        || !entry.shown_by.is_empty()
                        || !slugs.is_empty()
                    {
                        verdict.flags.push("missing".to_owned());
                        verdict.details.push("no portrait art for this set and half".to_owned());
                    } else {
                        "unused".clone_into(&mut verdict.found);
                        verdict.details.push(
                            "no unit is known to show this half and the store has no art for it"
                                .to_owned(),
                        );
                    }
                    verdicts.push(verdict);
                    continue;
                };
                "portrait".clone_into(&mut verdict.found);
                verdict.art = Some(ArtRef {
                    id: portrait.id.clone(),
                    file: portrait.rel.clone(),
                    fighter: None,
                    description: portrait.description.clone(),
                    label: portrait.label.clone(),
                    made_for: Vec::new(),
                    palette_distance: None,
                    recoloured: false,
                    drawn: None,
                });
                if candidates.len() > 1 {
                    verdict.details.push(format!(
                        "{} arts share this set and half: {}",
                        candidates.len(),
                        candidates.iter().map(|p| p.id.as_str()).collect::<Vec<_>>().join(", ")
                    ));
                }
                let (_, _, native) = read_indexed(&out.join(&entry.png))?;
                let loaded =
                    cache.entry(portrait.file.clone()).or_insert_with(|| read_art(&portrait.file));
                match loaded {
                    None => {
                        verdict.flags.push("missing".to_owned());
                        verdict
                            .details
                            .push(format!("the art file {} cannot be read", portrait.rel));
                    }
                    Some((aw, ah, rgba)) => {
                        match score::downsample(rgba, *aw as usize, *ah as usize, 32, 32) {
                            None => {
                                verdict.flags.push("shape".to_owned());
                                verdict.details.push(format!(
                                    "the art is {aw}x{ah}, not a whole multiple of 32x32"
                                ));
                            }
                            Some(down) => {
                                let row = row_of(entry);
                                // A face is drawn in whatever palette the screen has: its structure is read off the indexes alone
                                let correlation = if entry.category == Category::Face {
                                    score::index_structure(&native, &down)
                                } else {
                                    score::luma_correlation(&native, &row, &down)
                                };
                                verdict.scores.luma_correlation = Some(correlation);
                                if correlation < MIN_LUMA {
                                    verdict.flags.push("shape".to_owned());
                                    verdict.details.push(format!("the art's light and dark follow the picture's at {correlation:.2}"));
                                }
                                if entry.category == Category::ObjectPortrait
                                    && portrait.palette_object == entry.object
                                    && let Some(colour) =
                                        score::colour(&native, 32, &down, &row.map(Some))
                                {
                                    verdict.scores.colour_share = Some(colour.share);
                                    verdict.scores.colour_distance = Some(colour.distance);
                                    if colour.distance > MAX_COLOUR_DISTANCE {
                                        verdict.flags.push("colour".to_owned());
                                        verdict.details.push(format!(
                                        "only {:.0}% of the art's pixels are nearest to the colour of their palette index",
                                        colour.share * 100.0
                                    ));
                                    }
                                }
                            }
                        }
                    }
                }
                // Who it shows: any identity of the entry may be the one the label names
                if !slugs.is_empty() {
                    let problems: Vec<String> = slugs
                        .iter()
                        .filter_map(|slug| identity.label_problem(slug, &portrait.label))
                        .collect();
                    if problems.len() == slugs.len() {
                        verdict.flags.push("identity".to_owned());
                        verdict.details.push(format!("{}: {}", portrait.id, problems[0]));
                    }
                }
                served.push((verdicts.len(), n, portrait.id.clone()));
            }
        }
        verdicts.push(verdict);
    }

    // One art file that draws entries of different identities
    let mut by_art: BTreeMap<&str, Vec<(usize, usize)>> = BTreeMap::new();
    for (v, n, id) in &served {
        by_art.entry(id.as_str()).or_default().push((*v, *n));
    }
    let mut siblings_shared = 0;
    for (art_id, members) in &by_art {
        let slugs: BTreeSet<String> =
            members.iter().filter_map(|&(v, _)| verdicts[v].identity.first().cloned()).collect();
        if slugs.len() < 2 {
            continue;
        }
        let sets: BTreeSet<Option<u8>> =
            members.iter().map(|&(_, n)| catalogue.entries[n].poseset).collect();
        let portrait = catalogue.entries[members[0].1].category != Category::BattlePose;
        let slug_list: Vec<&String> = slugs.iter().collect();
        // Looks that are told apart share art only when a pose of one is drawn in other colours than the other's
        let used_of = |entry: usize| -> u16 {
            read_indexed(&out.join(&catalogue.entries[entry].png))
                .map_or(0xFFFF, |(_, _, arr)| used_indexes(&arr))
        };
        let told_apart = members.iter().any(|&(va, na)| {
            members.iter().any(|&(vb, nb)| {
                let (a, b) = (verdicts[va].identity.first(), verdicts[vb].identity.first());
                matches!((a, b), (Some(a), Some(b)) if identity.told_apart(a, b))
                    && palettes_differ(
                        &catalogue.entries[na].palette,
                        &catalogue.entries[nb].palette,
                        used_of(na),
                    )
            })
        });
        // Several units of one sprite (the same poseset) draw from one art by design, and so do several units of
        // one character; other characters sharing art across posesets, or looks told apart in other colours, are flagged
        let families: BTreeSet<&str> = slug_list.iter().map(|s| identity.family(s)).collect();
        if !told_apart && !portrait && (sets.len() < 2 || families.len() < 2) {
            siblings_shared += members.len();
            continue;
        }
        let list = slug_list.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
        for &(v, _) in members {
            verdicts[v].flags.push("shared".to_owned());
            verdicts[v].details.push(format!("{art_id} draws {} identities: {list}", slugs.len()));
        }
    }

    // Order and summary
    let mut summary = Summary {
        art_store: std::fs::canonicalize(art)
            .unwrap_or_else(|_| art.to_path_buf())
            .display()
            .to_string(),
        entries: verdicts.len(),
        shared_siblings: siblings_shared,
        ..Summary::default()
    };
    for verdict in &mut verdicts {
        verdict.flags.sort_by_key(|flag| FLAGS.iter().position(|f| f == flag).unwrap_or(99));
        verdict.flags.dedup();
        let has = |flag: &str| verdict.flags.iter().any(|f| f == flag);
        verdict.suspicion = 100.0 * f32::from(u8::from(has("identity")))
            + 50.0 * f32::from(u8::from(has("shared")))
            + 60.0 * f32::from(u8::from(has("shape")))
            + 40.0 * f32::from(u8::from(has("colour")))
            + 40.0 * f32::from(u8::from(has("label")))
            + 10.0 * f32::from(u8::from(has("missing")))
            + verdict.scores.iou.map_or(0.0, |iou| 20.0 * (1.0 - iou))
            + verdict.scores.colour_distance.map_or(0.0, |d| 20.0 * (d / 200.0).min(1.0));
        if !verdict.used {
            verdict.suspicion *= 0.1;
        }
        if verdict.used {
            summary.used += 1;
            *summary.used_by_basis.entry(verdict.basis.clone()).or_default() += 1;
        } else {
            summary.unused += 1;
        }
        if verdict.found == "skipped" || verdict.found == "unused" {
            summary.not_checked += 1;
        } else {
            summary.checked += 1;
        }
        *summary.found_by.entry(verdict.found.clone()).or_default() += 1;
        if verdict.identity.is_empty() {
            summary.without_identity += 1;
        }
        for flag in &verdict.flags {
            if !verdict.used {
                *summary.unused_by_flag.entry(flag.clone()).or_default() += 1;
                continue;
            }
            *summary.by_flag.entry(flag.clone()).or_default() += 1;
            let category = serde_json::to_value(verdict.category)?;
            *summary
                .by_category_flag
                .entry(format!("{}/{flag}", category.as_str().unwrap_or("?")))
                .or_default() += 1;
        }
    }
    verdicts.sort_by(|a, b| b.suspicion.total_cmp(&a.suspicion).then_with(|| a.id.cmp(&b.id)));
    let report = Report { summary, results: verdicts };
    std::fs::write(out.join("report.json"), serde_json::to_string_pretty(&report)?)?;
    writeln!(
        log,
        "hd-catalogue verify: {} entries ({} used, {} unused), {} checked, {} not checked; found by {:?}; flags of used entries {:?}, of unused {:?}; {} entries without an identity; {} served by sibling art (not flagged)",
        report.summary.entries,
        report.summary.used,
        report.summary.unused,
        report.summary.checked,
        report.summary.not_checked,
        report.summary.found_by,
        report.summary.by_flag,
        report.summary.unused_by_flag,
        report.summary.without_identity,
        report.summary.shared_siblings
    )?;
    Ok(report)
}

/// What the verdict on a pose needs of the store and the tables.
struct Ctx<'a> {
    out: &'a Path,
    index: &'a ArtIndex,
    poses: &'a [PoseArt],
    identity: &'a Identity,
    unit_poseset: &'a BTreeMap<u8, u8>,
    unit_palettes: &'a BTreeMap<u8, Vec<String>>,
    key_units: &'a BTreeMap<u64, BTreeSet<u8>>,
    sizes: OnceLock<FxHashMap<String, u32>>,
    cache: FxHashMap<PathBuf, Option<(u32, u32, Vec<u8>)>>,
}

impl Ctx<'_> {
    /// The verdict on `entry` drawn as the pose with keys `(key, mirror)`: facing as the native
    /// picture does, or `upside_down`. The second answer is the art it drew, if any. An upside-down
    /// pose is answered only when the index has art for its keys.
    fn pose(
        &mut self,
        entry: &Entry,
        id: &str,
        (key, mirror): (u64, u64),
        upside_down: bool,
    ) -> anyhow::Result<Option<(Verdict, Option<String>)>> {
        let slugs = self.identity.of_entry(entry);
        let mut verdict = Verdict {
            id: id.to_owned(),
            category: entry.category,
            used: entry.used,
            basis: entry.basis.clone(),
            identity: slugs.clone(),
            flags: Vec::new(),
            suspicion: 0.0,
            found: "none".to_owned(),
            art: None,
            scores: Scores::default(),
            details: Vec::new(),
        };
        let row = row_of(entry);
        let unit = entry.units[0];
        let resolver = Resolver::new(self.index, unit == SUPER_SAIYAN_GOKU, &self.sizes);
        let ask = |pose: &PoseRef| {
            let picked = resolver.unit_for(pose, &row);
            resolver.exact_art(pose, &row, picked.as_ref())
        };
        let whole = PoseRef { key, mirror: Some(mirror), field: false };
        let (mut best, mut mirrored) = ask(&whole);
        let mut found = if mirrored { "mirror" } else { "exact" };
        let (_, _, mut arr) = read_indexed(&self.out.join(&entry.png))?;
        if upside_down {
            arr = arr.chunks(entry.w).rev().flatten().copied().collect();
            if best.is_none() {
                return Ok(None);
            }
            found = if mirrored { "vflip-mirror" } else { "vflip" };
        } else {
            if best.is_none() {
                for (k, m) in &entry.cluster_keys {
                    let part = PoseRef {
                        key: parse_key(k).unwrap_or(0),
                        mirror: parse_key(m),
                        field: false,
                    };
                    let (at, flip) = ask(&part);
                    if at.is_some() {
                        (best, mirrored, found) = (at, flip, "cluster");
                        break;
                    }
                }
            }
            if best.is_none()
                && let Some(at) = resolver.shaped(&arr, entry.w, entry.h, false)
            {
                (best, found) = (Some(at), "shaped");
            }
        }
        let Some(at) = best else {
            if entry.used {
                verdict.flags.push("missing".to_owned());
                verdict
                    .details
                    .push("no art for this pose, its mirror image or its shape".to_owned());
            } else {
                "unused".clone_into(&mut verdict.found);
                verdict.details.push("no art, and the game never shows this pose".to_owned());
            }
            return Ok(Some((verdict, None)));
        };
        found.clone_into(&mut verdict.found);
        let object = &self.index.objects()[at as usize];
        let info = &self.poses[at as usize];
        let made_for = self.identity.art_slugs(&info.id);
        let palette_distance = (object.palette.mask != 0).then(|| object.palette.distance(&row));
        verdict.art = Some(ArtRef {
            id: info.id.clone(),
            file: info.rel.clone(),
            fighter: info.fighter.clone(),
            description: info.description.clone(),
            label: info.label.clone(),
            made_for: made_for.clone(),
            palette_distance,
            recoloured: false,
            drawn: None,
        });
        // Shape and colours against the art's pixels
        let loaded = self.cache.entry(info.file.clone()).or_insert_with(|| read_art(&info.file));
        match loaded {
            None => {
                verdict.flags.push("missing".to_owned());
                verdict.details.push(format!("the art file {} cannot be read", info.rel));
            }
            Some((aw, ah, rgba)) => {
                let (aw, ah) = (*aw as usize, *ah as usize);
                // What the renderer draws: the art recoloured to the unit's palette when it was made for another
                let facing = if mirrored { mirror_rows(&arr, entry.w) } else { arr.clone() };
                let drawn = resolve::recoloured_art(
                    object,
                    (rgba, aw, ah),
                    (&facing, entry.w, entry.h),
                    &row,
                    found == "shaped",
                );
                if let Some(drawn) = &drawn {
                    let name = format!("drawn/{}.png", id.replace('~', "-"));
                    std::fs::create_dir_all(self.out.join("drawn"))?;
                    std::fs::write(
                        self.out.join(&name),
                        ssd_hd::png::encode_rgba_compressed(aw as u32, ah as u32, drawn),
                    )?;
                    if let Some(art) = verdict.art.as_mut() {
                        art.recoloured = true;
                        art.drawn = Some(name);
                    }
                }
                let seen = drawn.as_deref().unwrap_or(rgba);
                match score::downsample(seen, aw, ah, entry.w, entry.h) {
                    None => {
                        verdict.flags.push("shape".to_owned());
                        verdict.details.push(format!(
                            "the art is {aw}x{ah}, not a whole multiple of the pose's {}x{}",
                            entry.w, entry.h
                        ));
                    }
                    Some(down) => {
                        let down = if mirrored { down.mirrored() } else { down };
                        let iou = score::iou(&arr, &down);
                        verdict.scores.iou = Some(iou);
                        if iou < MIN_IOU {
                            verdict.flags.push("shape".to_owned());
                            verdict.details.push(format!(
                                "the art covers {:.0}% of the pose's silhouette",
                                iou * 100.0
                            ));
                        }
                        if !object.luma {
                            let reference: [Option<[u8; 3]>; 16] = row.map(Some);
                            if let Some(colour) = score::colour(&arr, entry.w, &down, &reference) {
                                verdict.scores.colour_share = Some(colour.share);
                                verdict.scores.colour_distance = Some(colour.distance);
                                colour.per_index.clone_into(&mut verdict.scores.colour_by_index);
                                if colour.distance > MAX_COLOUR_DISTANCE {
                                    verdict.flags.push("colour".to_owned());
                                    verdict.details.push(format!(
                                        "the art as drawn is {:.0} from the colours of its palette indexes on average ({:.0}% nearest to their own)",
                                        colour.distance,
                                        colour.share * 100.0
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
        // Who the art is made for
        if let Some(slug) = slugs.first() {
            let siblings = |made_for: &[String]| {
                let sets: BTreeSet<u8> = (0..ssd_game_units())
                    .filter(|&u| {
                        self.identity.of_unit(u).is_some_and(|s| made_for.iter().any(|m| m == s))
                    })
                    .filter_map(|u| self.unit_poseset.get(&u).copied())
                    .collect();
                entry.poseset.is_some_and(|set| sets.contains(&set))
            };
            let mut label_applies = true;
            // Art found by its pixels is drawn from the pose of this entry; when the units its name says
            // it was made for cannot draw those pixels, its name is wrong, not its picture
            // (an art that numbers its unit is made for that unit; one that does not, for the character)
            let numbered = Identity::art_unit(&info.id).is_some();
            let made_units: BTreeSet<u8> = (0..ssd_game_units())
                .filter(|&u| {
                    self.identity.of_unit(u).is_some_and(|s| {
                        made_for.iter().any(|m| {
                            if numbered { m == s } else { self.identity.same_family(m, s) }
                        })
                    })
                })
                .collect();
            let drawers = self.key_units.get(&object.key);
            let mislabelled = matches!(found, "exact" | "mirror" | "vflip" | "vflip-mirror")
                && !made_units.is_empty()
                && drawers.is_some_and(|d| d.is_disjoint(&made_units));
            let compared =
                if mislabelled { Compare::Unknown } else { self.identity.compare(slug, &made_for) };
            if mislabelled {
                label_applies = false;
                verdict.flags.push("label".to_owned());
                verdict.details.push(format!(
                    "{} says it is made for {}, but no unit of that name draws the pixels it matches (units {:?} do): the name is wrong, the picture is of this pose",
                    info.id,
                    made_for.join(", "),
                    drawers.map(|d| d.iter().copied().collect::<Vec<_>>()).unwrap_or_default()
                ));
            }
            match compared {
                Compare::Same | Compare::SameFamily | Compare::Unknown => {}
                Compare::OtherForm(other) => {
                    // The two looks differ by their palettes: a pose that uses no colour in which they differ
                    // is drawn the same for both, and the art made for one is as good for the other
                    let used = used_indexes(&arr);
                    let same_colours = (0..ssd_game_units())
                        .filter(|&u| self.identity.of_unit(u) == Some(other.as_str()))
                        .filter_map(|u| self.unit_palettes.get(&u))
                        .any(|palette| !palettes_differ(palette, &entry.palette, used));
                    if same_colours {
                        label_applies = false;
                        verdict.details.push(format!(
                            "{slug} and {other} are drawn alike in this pose (the colours it uses are the same), so {} serves both",
                            info.id
                        ));
                    } else {
                        verdict.flags.push("identity".to_owned());
                        verdict.details.push(format!(
                            "{slug} is drawn from art made for {other}, another look of the same character ({})",
                            info.id
                        ));
                    }
                }
                Compare::Other(other) => {
                    if siblings(&made_for) {
                        label_applies = false;
                    } else {
                        verdict.flags.push("identity".to_owned());
                        verdict.details.push(format!(
                            "{slug} is drawn from art made for {other}, a different character ({})",
                            info.id
                        ));
                    }
                }
            }
            if label_applies && let Some(problem) = self.identity.label_problem(slug, &info.label) {
                verdict.flags.push("identity".to_owned());
                verdict.details.push(format!("{}: {problem}", info.id));
            }
        }
        Ok(Some((verdict, Some(info.id.clone()))))
    }
}

/// The number of units of the game's unit table.
fn ssd_game_units() -> u8 {
    super::decode::UNITS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_names_a_stream_and_a_half() {
        assert_eq!(
            parse_source("rom 16:91D4 half 1 (32x32, 4bpp)"),
            Some(("16:91D4".to_owned(), 1))
        );
        assert_eq!(parse_source("screen 16:91D4 half 1"), None);
    }

    #[test]
    fn the_subject_of_a_prompt_is_between_its_words() {
        let prompt = "Redraw this pixel-art game portrait of a green Namekian warrior with antennae as a high-definition 2D anime illustration";
        assert_eq!(prompt_subject(prompt), "a green Namekian warrior with antennae");
    }

    #[test]
    fn the_stem_of_an_art_id_drops_the_pose_number() {
        assert_eq!(stem_of("gallery-enemy-vegeta-u75-005"), "gallery-enemy-vegeta-u75");
        assert_eq!(stem_of("face-91D4-1"), "face-91D4");
    }

    #[test]
    fn a_silhouette_and_its_mirror_have_the_same_pixels_counted_from_the_other_side() {
        let arr = [1_u8, 0, 0, 1, 1, 0];
        let (plain, mirrored) = silhouette(&arr, 3);
        assert_eq!(plain, [0b0001_1001]);
        assert_eq!(mirrored, [0b0011_0100]);
    }

    /// A catalogue of one pose of Super Saiyan Vegeta (unit 75), and of Vegeta's (unit 15) when
    /// `with_normal`, whose only art was made for Vegeta (unit 15): its verdict on the Super Saiyan pose.
    fn super_look_case(with_normal: bool) -> Verdict {
        use super::super::{Catalogue, Entry, hex_key};
        let dir = tempfile::tempdir().unwrap();
        let (out, art) = (dir.path().join("out"), dir.path().join("art"));
        std::fs::create_dir_all(art.join("class2")).unwrap();
        // 12 x 12 pose: left half colour 1, right half colour 14 (the colour the two looks differ in)
        let arr: Vec<u8> = (0..144).map(|n| if n % 12 < 6 { 1 } else { 14 }).collect();
        let super_look: Vec<[u8; 3]> =
            (0..16).map(|i| if i == 14 { [250, 250, 0] } else { [i as u8 * 10, 0, 0] }).collect();
        ssd_hd::png::write_indexed(
            &out.join("native/pose.png"),
            12,
            12,
            &arr,
            &super_look,
            Some(0),
        )
        .unwrap();
        let text = |palette: &[[u8; 3]]| -> Vec<String> {
            palette.iter().map(|c| format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2])).collect()
        };
        let (key, mirror) = (
            ssd_hd::resolve::pose_key(&arr, 12, 12),
            ssd_hd::resolve::mirror_pose_key(&arr, 12, 12),
        );
        let entry = Entry {
            id: "pose-u075-p00".to_owned(),
            category: Category::BattlePose,
            units: vec![75],
            unit_names: vec!["Vegeta".to_owned()],
            poseset: Some(10),
            pose: Some(0),
            stream: None,
            half: None,
            object: None,
            shown_by: Vec::new(),
            w: 12,
            h: 12,
            key: Some(hex_key(key)),
            mirror_key: Some(hex_key(mirror)),
            vflip_keys: None,
            cluster_keys: Vec::new(),
            origin: "15:8000".to_owned(),
            palette: text(&super_look),
            png: "native/pose.png".to_owned(),
            note: None,
            used: true,
            evidence: Vec::new(),
            basis: "script".to_owned(),
        };
        let mut entries = vec![entry.clone()];
        if with_normal {
            let normal: Vec<[u8; 3]> = (0..16).map(|i| [i as u8 * 10, 0, 0]).collect();
            entries.push(Entry {
                id: "pose-u015-p00".to_owned(),
                units: vec![15],
                palette: text(&normal),
                ..entry
            });
        }
        let catalogue = Catalogue { entries, problems: Vec::new() };
        std::fs::write(out.join("catalogue.json"), serde_json::to_string(&catalogue).unwrap())
            .unwrap();
        // The art: the pose at 4x, left half red, right half grey
        let mut rgba = Vec::new();
        for _y in 0..48 {
            for x in 0..48 {
                rgba.extend_from_slice(if x < 24 { &[10, 0, 0, 255] } else { &[10, 10, 10, 255] });
            }
        }
        ssd_hd::png::write_rgba(&art.join("class2/a-001.png"), 48, 48, &rgba).unwrap();
        let index = serde_json::json!({"assets_list": [{
            "id": "gallery-enemy-vegeta-u15-001", "file": "a-001.png", "source_key": hex_key(key),
            "source_size": [12, 12], "fighter": "vegeta", "description": "Vegeta (black flame-shaped hair)",
            "palette_indexes_used": {"1": "#0A0000", "14": "#0A0A0A"}
        }]});
        std::fs::write(art.join("class2/index.json"), index.to_string()).unwrap();
        let identity = dir.path().join("identity.toml");
        std::fs::write(
            &identity,
            "[units]
15 = \"vegeta\"
75 = \"vegeta-ssj\"
[forms]
vegeta = [\"vegeta\", \"vegeta-ssj\"]
             [identities.vegeta-ssj]
form_words = [\"super saiyan\"]
",
        )
        .unwrap();
        run(&out, &art, &identity, &mut Vec::new()).unwrap();
        let report: Report =
            serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap())
                .unwrap();
        report.results.into_iter().find(|v| v.id == "pose-u075-p00").unwrap()
    }

    /// The art is found by its key, fits the pose, and is flagged as another look's when the normal look draws
    /// those pixels too.
    #[test]
    fn a_super_look_drawn_from_the_normal_looks_art_is_flagged() {
        let verdict = super_look_case(true);
        assert_eq!(verdict.found, "exact");
        assert_eq!(verdict.art.as_ref().unwrap().id, "gallery-enemy-vegeta-u15-001");
        assert!((verdict.scores.iou.unwrap() - 1.0).abs() < 1e-6, "{:?}", verdict.scores);
        assert_eq!(verdict.flags, ["identity", "shared"], "{:?}", verdict.details);
    }

    /// When the unit the art's name gives cannot draw the pixels it matches, its name is wrong, not the picture.
    #[test]
    fn art_named_for_a_unit_that_cannot_draw_its_pixels_has_a_wrong_label_not_a_wrong_picture() {
        let verdict = super_look_case(false);
        assert_eq!(verdict.flags, ["label"], "{:?}", verdict.details);
    }

    #[test]
    fn two_looks_differ_only_in_the_colours_a_pose_uses() {
        let a: Vec<String> = (0..16).map(|i| format!("#{i:02X}0000")).collect();
        let mut b = a.clone();
        b[14] = "#FFFF00".to_owned();
        assert!(palettes_differ(&a, &b, used_indexes(&[1, 14, 0])));
        assert!(
            !palettes_differ(&a, &b, used_indexes(&[1, 2, 3])),
            "the pose does not use the colour they differ in"
        );
    }
}
