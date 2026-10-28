//! `hd-pack-public <art store> <out.zip>`: builds the shared HD art pack (`ssd_hdpack::public`)
//! from the art store: the art as WebP files, the indexes that find it, and nothing the game's ROM
//! holds. The first launch of the game builds the HD pack of the player's ROM from it
//! (`ssd_hdpack::install`).
//!
//! The pack is made through the private import: the store is imported into a pack with the ROM
//! (`hd-convert` and `hd-import`, with the silhouettes and source pictures of the art store), and
//! the shared pack is written from the typed indexes of that pack, field by field, so what is
//! in the store's own `index.json` files beyond that (descriptions, notes, how a picture was made,
//! the source pictures of the game, the rollback zips) never reaches it. What the shared pack
//! keeps of the game is data a few bytes long: content hashes, sizes, pointers (`BB:AAAA`), the
//! sixteen colours of a palette, a plane's 256 colours, and the outline of each pose (a bit per
//! pixel, no colour). The maps are lossy WebP (the quality is an option); everything else is
//! lossless and checked to decode to the same pixels.
//!
//! The result is checked by installing it into a scratch data directory and comparing the pack
//! that comes out with the one the import made.

use anyhow::Context;
use ssd_hd::art_index::Palette;
use ssd_hd::pack::Pack;
use ssd_hdpack::public::{self, Manifest, Masks};
use ssd_hdpack::{convert, import, install};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The default quality of the lossy WebP the maps are stored as.
const MAPS_QUALITY: f32 = 92.0;

/// How the art of a file is stored.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Codec {
    /// WebP lossless: the decoded pixels are the file's.
    Lossless,
    /// WebP lossy at this quality (0 to 100).
    Lossy(f32),
}

/// One art file to put in the pack.
struct Job {
    /// The file in the import's pack.
    from: PathBuf,
    /// Its name in the shared pack, without the extension.
    stem: String,
    /// The kind of art, for the size report.
    class: &'static str,
    codec: Codec,
}

/// A file as it goes in the zip.
struct Packed {
    name: String,
    bytes: Vec<u8>,
    class: &'static str,
    /// The size of the file it was made from.
    from_bytes: u64,
    /// For a lossy file, the mean and the largest channel difference to the original.
    loss: Option<(f64, u8)>,
}

#[derive(Debug)]
struct Options {
    store: PathBuf,
    out: PathBuf,
    masks: Vec<PathBuf>,
    sources: Option<PathBuf>,
    maps_quality: f32,
    revision: String,
    jobs: usize,
    verify: bool,
    with_masks: bool,
    /// Where to keep the pack the import made (the reference the shared pack is compared with).
    keep_reference: Option<PathBuf>,
}

fn parse(args: &[&str]) -> Option<Options> {
    let mut positional = Vec::new();
    let mut options = Options {
        store: PathBuf::new(),
        out: PathBuf::new(),
        masks: Vec::new(),
        sources: None,
        maps_quality: MAPS_QUALITY,
        revision: "2".to_owned(),
        jobs: std::thread::available_parallelism().map_or(2, |n| n.get().min(4)),
        verify: true,
        with_masks: true,
        keep_reference: None,
    };
    let mut rest = args.iter();
    while let Some(&arg) = rest.next() {
        match arg {
            "--masks" => options.masks.push(PathBuf::from(rest.next()?)),
            "--sources" => options.sources = Some(PathBuf::from(rest.next()?)),
            "--maps-quality" => options.maps_quality = rest.next()?.parse().ok()?,
            "--revision" => (*rest.next()?).clone_into(&mut options.revision),
            "--jobs" => options.jobs = rest.next()?.parse().ok().filter(|&n| n > 0)?,
            "--no-verify" => options.verify = false,
            "--keep-reference" => options.keep_reference = Some(PathBuf::from(rest.next()?)),
            "--without-masks" => options.with_masks = false,
            other if !other.starts_with("--") => positional.push(other),
            _ => return None,
        }
    }
    let [store, out] = positional.as_slice() else { return None };
    options.store = PathBuf::from(store);
    options.out = PathBuf::from(out);
    (options.maps_quality > 0.0 && options.maps_quality <= 100.0).then_some(options)
}

/// A scratch folder for the build, emptied first and removed at the end.
struct Scratch(PathBuf);

impl Scratch {
    fn new(root: &Path) -> anyhow::Result<Self> {
        let dir = root.join(".work").join("hd-pack-public");
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The silhouettes of the folders `masks` merged into one folder of hard links (the import reads
/// one), the first folder that has a file winning.
fn merge_masks(masks: &[PathBuf], into: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(into)?;
    for dir in masks {
        for entry in std::fs::read_dir(dir).with_context(|| format!("{}", dir.display()))? {
            let entry = entry?;
            let target = into.join(entry.file_name());
            if !target.exists() {
                std::fs::hard_link(entry.path(), &target)
                    .or_else(|_| std::fs::copy(entry.path(), &target).map(|_| ()))?;
            }
        }
    }
    Ok(())
}

fn webp_config(codec: Codec, has_alpha: bool) -> anyhow::Result<webp::WebPConfig> {
    let mut config = webp::WebPConfig::new().map_err(|()| anyhow::anyhow!("a WebP config"))?;
    config.method = 5;
    match codec {
        Codec::Lossless => {
            config.lossless = 1;
            // (for a lossless file the quality is the effort: 100 takes many seconds a picture
            // for a few percent)
            config.quality = 75.0;
            // The pixels under a fully transparent one are kept too, so the file decodes to the
            // very same RGBA
            config.exact = 1;
        }
        Codec::Lossy(quality) => {
            config.lossless = 0;
            config.quality = quality;
            config.use_sharp_yuv = 1;
            config.segments = 4;
            config.alpha_quality = 100;
            config.alpha_compression = i32::from(has_alpha);
        }
    }
    Ok(config)
}

/// An encoded file, and for a lossy one the mean and the largest channel difference to the original.
type Encoded = (Vec<u8>, Option<(f64, u8)>);

/// Encodes `rgba` as WebP and checks the result: a lossless file must decode to the same pixels,
/// a lossy one is measured.
fn encode(width: u32, height: u32, rgba: &[u8], codec: Codec) -> anyhow::Result<Encoded> {
    let has_alpha = rgba.as_chunks::<4>().0.iter().any(|pixel| pixel[3] != 255);
    let config = webp_config(codec, has_alpha)?;
    let memory = webp::Encoder::from_rgba(rgba, width, height)
        .encode_advanced(&config)
        .map_err(|e| anyhow::anyhow!("WebP encoding failed: {e:?}"))?;
    let bytes = memory.to_vec();
    let (w, h, decoded) = ssd_hd::png::read_rgba(&bytes).map_err(|e| anyhow::anyhow!("{e}"))?;
    anyhow::ensure!((w, h) == (width, height), "the WebP has another size");
    match codec {
        Codec::Lossless => {
            anyhow::ensure!(decoded == rgba, "a lossless WebP decodes to other pixels");
            Ok((bytes, None))
        }
        Codec::Lossy(_) => {
            let (mut total, mut worst) = (0_u64, 0_u8);
            for (a, b) in decoded.as_chunks::<4>().0.iter().zip(rgba.as_chunks::<4>().0) {
                for channel in 0..3 {
                    let difference = a[channel].abs_diff(b[channel]);
                    total += u64::from(difference);
                    worst = worst.max(difference);
                }
            }
            Ok((bytes, Some((total as f64 / f64::from(width * height * 3), worst))))
        }
    }
}

fn run_job(job: &Job) -> anyhow::Result<Packed> {
    let from_bytes = std::fs::metadata(&job.from)?.len();
    let bytes = std::fs::read(&job.from).with_context(|| format!("{}", job.from.display()))?;
    let (width, height, rgba) = ssd_hd::png::read_rgba(&bytes)
        .map_err(|e| anyhow::anyhow!("{}: {e}", job.from.display()))?;
    let (webp, loss) = encode(width, height, &rgba, job.codec)
        .with_context(|| format!("{}", job.from.display()))?;
    Ok(Packed {
        name: format!("{}.webp", job.stem),
        bytes: webp,
        class: job.class,
        from_bytes,
        loss,
    })
}

/// Runs `jobs` on `threads` threads and returns the results in order.
fn run_jobs(jobs: &[Job], threads: usize, out: &mut dyn Write) -> anyhow::Result<Vec<Packed>> {
    let next = AtomicUsize::new(0);
    let counted = AtomicUsize::new(0);
    let results: Mutex<Vec<Option<anyhow::Result<Packed>>>> =
        Mutex::new((0..jobs.len()).map(|_| None).collect());
    let started = std::time::Instant::now();
    std::thread::scope(|scope| {
        for _ in 0..threads {
            scope.spawn(|| {
                loop {
                    let at = next.fetch_add(1, Ordering::Relaxed);
                    let Some(job) = jobs.get(at) else { break };
                    let done = run_job(job);
                    let finished = counted.fetch_add(1, Ordering::Relaxed) + 1;
                    if finished.is_multiple_of(500) || finished == jobs.len() {
                        eprintln!(
                            "hd-pack-public: {finished} of {} files encoded ({:.0?})",
                            jobs.len(),
                            started.elapsed()
                        );
                    }
                    results.lock().unwrap_or_else(std::sync::PoisonError::into_inner)[at] =
                        Some(done);
                }
            });
        }
        // (the scope waits for them; progress is the log's job below)
    });
    writeln!(out, "hd-pack-public: {} files encoded in {:.0?}", jobs.len(), started.elapsed())?;
    results
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .into_iter()
        .map(|done| done.expect("every job ran"))
        .collect()
}

fn hex_colour(colour: [u8; 3]) -> String {
    format!("#{:02X}{:02X}{:02X}", colour[0], colour[1], colour[2])
}

/// The colours a palette has, as the `index to #RRGGBB` map that the indexes carry.
fn palette_json(palette: &Palette) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for index in 0..16 {
        if palette.mask & (1 << index) != 0 {
            map.insert(index.to_string(), hex_colour(palette.colors[index]).into());
        }
    }
    map.into()
}

/// `portrait/16:C60A/4bpp` and the cells of a picture -> `rom 16:C60A half N`.
fn picture_source(asset: &str, first_cell: u16) -> anyhow::Result<String> {
    let address = asset.split('/').nth(1).context("an asset id `class/BB:AAAA/Nbpp`")?;
    Ok(format!("rom {address} half {}", u8::from(first_cell != 0)))
}

fn with_webp(pattern: &str) -> String {
    pattern.strip_suffix(".png").map_or_else(|| pattern.to_owned(), |stem| format!("{stem}.webp"))
}

fn stem(name: &str) -> &str {
    name.rsplit_once('.').map_or(name, |(stem, _)| stem)
}

/// Everything the shared pack is made of: the files to encode and the files to write as they are.
struct Plan {
    jobs: Vec<Job>,
    texts: Vec<(String, Vec<u8>)>,
    counts: BTreeMap<String, usize>,
}

/// The files to encode, and how many there are of each kind.
#[derive(Default)]
struct Planner {
    stems: std::collections::BTreeSet<String>,
    jobs: Vec<Job>,
    counts: BTreeMap<String, usize>,
}

impl Planner {
    fn add(&mut self, from: PathBuf, stem: String, class: &'static str, codec: Codec) {
        // (two layers may name the same art files: they are in the pack once)
        if !self.stems.insert(stem.clone()) {
            return;
        }
        *self.counts.entry(class.to_owned()).or_default() += 1;
        self.jobs.push(Job { from, stem, class, codec });
    }
}

fn counts_palettes(
    counts: &mut BTreeMap<String, usize>,
    palettes: &ssd_hdpack::import::RefPalettes,
) {
    counts.insert("layer palettes".to_owned(), palettes.bg.len() + palettes.maps.len());
}

/// The plan for the pack that `import` made in `reference`.
fn plan(reference: &Path, with_masks: bool, maps_quality: f32) -> anyhow::Result<Plan> {
    let pack = Pack::open(reference).map_err(|e| anyhow::anyhow!("{e}"))?;
    let art = &pack.art;
    let mut planner = Planner::default();
    let mut texts = Vec::new();
    let json = |value: &serde_json::Value| -> anyhow::Result<Vec<u8>> {
        Ok(serde_json::to_vec_pretty(value)?)
    };

    // Portraits and faces
    let mut assets = Vec::new();
    for picture in art.pictures() {
        planner.add(
            reference.join(&picture.art),
            format!("class1/{}", picture.name),
            "portraits",
            Codec::Lossless,
        );
        assets.push(serde_json::json!({
            "id": picture.name,
            "source": picture_source(&picture.asset, picture.tiles[0])?,
            "file": format!("{}.webp", picture.name),
            "palette_colours": palette_json(&picture.palette),
        }));
    }
    texts.push(("class1/index.json".to_owned(), json(&serde_json::json!({ "assets": assets }))?));

    // Poses and effects, the field's sprites
    let mut assets = Vec::new();
    let mut masks = Masks::default();
    for object in art.objects() {
        let class = if object.field() { "field sprites" } else { "fighters and effects" };
        planner.add(
            reference.join(&object.art),
            format!("class2/{}", object.id),
            class,
            Codec::Lossless,
        );
        let mut entry = serde_json::json!({
            "id": object.id,
            "file": format!("{}.webp", object.id),
            "source_key": format!("{:016x}", object.key),
            "source_size": [object.width, object.height],
            "palette_indexes_used": palette_json(&object.palette),
        });
        if object.luma {
            entry["luma"] = true.into();
        }
        assets.push(entry);
        if with_masks && !object.mask.is_empty() {
            if let Some((w, h, held)) = masks.get(object.key) {
                anyhow::ensure!(
                    (w, h, held) == (object.width, object.height, object.mask.as_slice()),
                    "two poses of the key {:016x} have different outlines",
                    object.key
                );
            } else {
                masks.insert(object.key, object.width, object.height, object.mask.clone());
            }
        }
    }
    texts.push((
        "class2/index.json".to_owned(),
        json(&serde_json::json!({ "assets_list": assets }))?,
    ));
    if !masks.is_empty() {
        texts.push((public::MASKS.to_owned(), masks.encode()));
        planner.counts.insert("pose outlines".to_owned(), masks.len());
    }

    // The poses' bigger art
    let listed =
        std::fs::read_to_string(reference.join(ssd_hd::pack::POSE_ART)).unwrap_or_default();
    let listed: ssd_hd::pack::PoseArt = toml::from_str(&listed)?;
    if !listed.variants.is_empty() {
        let mut variants = Vec::new();
        for variant in &listed.variants {
            let id = stem(variant.file.rsplit('/').next().unwrap_or(&variant.file)).to_owned();
            planner.add(
                reference.join(&variant.file),
                format!("variants/{id}"),
                "pose variants",
                Codec::Lossless,
            );
            variants.push(serde_json::json!({
                "id": id,
                "art": variant.art,
                "when": variant.when,
                "margin": variant.margin,
                "file": format!("{id}.webp"),
            }));
        }
        texts.push((
            "variants/index.json".to_owned(),
            json(&serde_json::json!({ "variants": variants }))?,
        ));
    }

    // The cards
    let cards_text = std::fs::read_to_string(reference.join(ssd_hd::cards::INDEX));
    if let Ok(text) = cards_text {
        let index: ssd_hd::cards::Index = toml::from_str(&text)?;
        let mut layer = |name: &Option<String>| -> Option<String> {
            let name = name.as_ref()?;
            planner.add(
                reference.join("art/cards").join(name),
                format!("cards/{}", stem(name)),
                "cards",
                Codec::Lossless,
            );
            Some(format!("{}.webp", stem(name)))
        };
        let mut tiles = Vec::new();
        for entry in &index.tile {
            let (base, roles) = (layer(&entry.base), layer(&entry.roles));
            let mut tile = serde_json::json!({ "code": entry.code, "base": base, "roles": roles });
            if let Some(set) = &entry.set {
                tile["set"] = serde_json::json!(set);
            }
            if entry.carve {
                tile["carve"] = serde_json::json!(true);
            }
            tiles.push(tile);
        }
        let mut tips = Vec::new();
        for tip in &index.tip {
            let name = layer(&Some(tip.image.clone())).context("a tip image")?;
            tips.push(serde_json::json!({ "code": tip.code, "image": name }));
        }
        texts.push((
            "cards/index.json".to_owned(),
            json(&serde_json::json!({ "tiles": tiles, "tips": tips }))?,
        ));
    }

    // Battle backgrounds and the big pictures
    if !pack.bg.layer.is_empty() {
        let mut layers = Vec::new();
        for layer in &pack.bg.layer {
            planner.add(
                reference.join(&layer.art),
                format!("bg/{}", layer.id),
                "backgrounds",
                Codec::Lossless,
            );
            layers.push(serde_json::json!({
                "id": layer.id,
                "key": layer.key,
                "bg": layer.bg,
                "mode": layer.mode,
                "map_base": layer.map_base,
                "char_base": layer.char_base,
                "size": layer.size,
                "rows": layer.rows,
                "fill": layer.fill,
                "palette_key": layer.palette_key,
                "file": format!("{}.webp", layer.id),
            }));
        }
        texts.push(("bg/index.json".to_owned(), json(&serde_json::json!({ "layers": layers }))?));
    }

    // The field maps
    if !pack.maps.layer.is_empty() {
        let mut layers = Vec::new();
        for layer in &pack.maps.layer {
            let (columns, rows) = layer.tile_grid();
            for y in 0..rows {
                for x in 0..columns {
                    let file = layer.tile_file(x, y);
                    planner.add(
                        reference.join("art/maps").join(&file),
                        format!("maps/{}", stem(&file)),
                        "maps",
                        // (100 is lossless: a pack to check the recolouring with, not to ship)
                        if maps_quality >= 100.0 {
                            Codec::Lossless
                        } else {
                            Codec::Lossy(maps_quality)
                        },
                    );
                }
            }
            layers.push(serde_json::json!({
                "id": layer.id,
                "maps": layer.maps,
                "blocks": layer.blocks,
                "grid": layer.grid,
                "metatiles": layer.metatiles,
                "palette_variant": layer.palette_variant,
                "tiles": with_webp(&layer.tiles),
            }));
        }
        texts.push((
            "maps/index.json".to_owned(),
            json(&serde_json::json!({
                "version": 1, "scale": ssd_hd::mapart::SCALE, "tile": ssd_hd::mapart::TILE,
                "layers": layers
            }))?,
        ));
    }

    // The Mode 7 planes
    if !pack.mode7.plane.is_empty() {
        let mut planes = Vec::new();
        for plane in &pack.mode7.plane {
            let across = ssd_hd::mode7art::Entry::tiles_across();
            for y in 0..across {
                for x in 0..across {
                    let file = plane.tile_file(x, y);
                    planner.add(
                        reference.join("art/mode7").join(&file),
                        format!("mode7/{}", stem(&file)),
                        "mode 7 planes",
                        Codec::Lossless,
                    );
                }
            }
            planes.push(serde_json::json!({
                "id": plane.id,
                "key": plane.key,
                "palette": plane.palette,
                "tiles": with_webp(&plane.tiles),
            }));
        }
        texts.push((
            "mode7/index.json".to_owned(),
            json(&serde_json::json!({
                "version": 1, "scale": ssd_hd::mode7art::SCALE, "tile": ssd_hd::mode7art::TILE,
                "planes": planes
            }))?,
        ));
    }
    // The palettes the art of the layers was made in
    let palettes = ssd_hdpack::import::RefPalettes {
        bg: pack
            .bg
            .layer
            .iter()
            .filter(|l| !l.palette.is_empty())
            .map(|l| (l.id.clone(), l.palette.clone()))
            .collect(),
        maps: pack
            .maps
            .layer
            .iter()
            .filter(|l| !l.palette.is_empty())
            .map(|l| (l.id.clone(), l.palette.clone()))
            .collect(),
    };
    if !palettes.bg.is_empty() || !palettes.maps.is_empty() {
        counts_palettes(&mut planner.counts, &palettes);
        texts.push((public::REF_PALETTES.to_owned(), json(&serde_json::to_value(&palettes)?)?));
    }
    Ok(Plan { jobs: planner.jobs, texts, counts: planner.counts })
}

/// Writes the zip: every file in name order, the art stored (WebP does not compress further), the
/// texts deflated, with no timestamp so the same input gives the same bytes.
fn write_zip(out: &Path, files: &BTreeMap<String, Vec<u8>>) -> anyhow::Result<()> {
    if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut writer = zip::ZipWriter::new(std::io::BufWriter::new(std::fs::File::create(out)?));
    let base =
        zip::write::SimpleFileOptions::default().last_modified_time(zip::DateTime::default());
    for (name, bytes) in files {
        let method = if name.rsplit_once('.').is_some_and(|(_, ext)| ext == "webp") {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };
        writer.start_file(name, base.compression_method(method))?;
        writer.write_all(bytes)?;
    }
    writer.finish()?.flush()?;
    Ok(())
}

const PACK_README: &str = "\
Dragon Ball Z: Super Saiya Densetsu - HD art pack\r\n\
\r\n\
Put this file (hd-pack.zip, as it is) next to ssd-recompiled.exe and start the game with\r\n\
\"Play Remastered.bat\". On the first launch the game builds its HD data from your own ROM and\r\n\
this art, once (about a minute); after that nothing else is needed.\r\n\
\r\n\
The pack holds new art only: no ROM data, no original graphics.\r\n\
It is AI-assisted art, curated by hand, made to follow the original game and the anime.\r\n\
It is shared under CC BY-NC-SA 4.0 (non-commercial use): see LICENSE-ART.md in this zip.\r\n";

/// Compares the pack the shared one installs into with the one the import made, index by index.
fn compare(reference: &Path, installed: &Path, out: &mut dyn Write) -> anyhow::Result<()> {
    let (want, got) = (
        Pack::open(reference).map_err(|e| anyhow::anyhow!("{e}"))?,
        Pack::open(installed).map_err(|e| anyhow::anyhow!("{e}"))?,
    );
    anyhow::ensure!(
        std::fs::read(reference.join("index.bin"))? == std::fs::read(installed.join("index.bin"))?,
        "the tile index differs"
    );
    let (a, b) = (want.art.pictures(), got.art.pictures());
    anyhow::ensure!(a.len() == b.len(), "{} pictures, installed {}", a.len(), b.len());
    for (x, y) in a.iter().zip(b) {
        anyhow::ensure!(
            (&x.name, &x.asset, x.tiles, x.palette) == (&y.name, &y.asset, y.tiles, y.palette)
                && stem(&x.art) == stem(&y.art),
            "the picture {} differs",
            x.name
        );
    }
    let (a, b) = (want.art.objects(), got.art.objects());
    anyhow::ensure!(a.len() == b.len(), "{} poses, installed {}", a.len(), b.len());
    let mut outlines = 0;
    for (x, y) in a.iter().zip(b) {
        anyhow::ensure!(
            (x.key, &x.id, x.luma, x.palette, x.width, x.height, &x.mask)
                == (y.key, &y.id, y.luma, y.palette, y.width, y.height, &y.mask)
                && stem(&x.art) == stem(&y.art),
            "the pose {} differs",
            x.id
        );
        outlines += usize::from(!x.mask.is_empty());
    }
    anyhow::ensure!(want.pose_art.len() == got.pose_art.len(), "the pose variants differ");
    for (x, y) in want.pose_art.iter().zip(&got.pose_art) {
        anyhow::ensure!(
            (&x.art, x.when, x.margin) == (&y.art, y.when, y.margin)
                && stem(&x.file) == stem(&y.file),
            "the pose variant {} differs",
            x.art
        );
    }
    anyhow::ensure!(want.bg.layer.len() == got.bg.layer.len(), "the background layers differ");
    for (x, y) in want.bg.layer.iter().zip(&got.bg.layer) {
        let mut y = y.clone();
        x.art.clone_into(&mut y.art);
        x.source.clone_into(&mut y.source);
        anyhow::ensure!(*x == y, "the background layer {} differs", x.id);
    }
    anyhow::ensure!(want.maps.layer.len() == got.maps.layer.len(), "the map layers differ");
    for (x, y) in want.maps.layer.iter().zip(&got.maps.layer) {
        let mut y = y.clone();
        x.tiles.clone_into(&mut y.tiles);
        x.source.clone_into(&mut y.source);
        anyhow::ensure!(*x == y, "the map layer {} differs", x.id);
    }
    anyhow::ensure!(want.mode7.plane.len() == got.mode7.plane.len(), "the planes differ");
    for (x, y) in want.mode7.plane.iter().zip(&got.mode7.plane) {
        let mut y = y.clone();
        x.tiles.clone_into(&mut y.tiles);
        anyhow::ensure!(*x == y, "the plane {} differs", x.id);
    }
    let cards = |dir: &Path| -> anyhow::Result<String> {
        let text = std::fs::read_to_string(dir.join(ssd_hd::cards::INDEX)).unwrap_or_default();
        Ok(text.replace(".png", ""))
    };
    anyhow::ensure!(
        cards(reference)?.replace(".webp", "") == cards(installed)?.replace(".webp", ""),
        "the cards' index differs"
    );
    writeln!(
        out,
        "hd-pack-public: verified: the installed pack has the same {} pictures, {} poses ({outlines} with an outline), {} pose variants, {} background layers, {} map layers and {} planes as the import",
        want.art.pictures().len(),
        want.art.objects().len(),
        want.pose_art.len(),
        want.bg.layer.len(),
        want.maps.layer.len(),
        want.mode7.plane.len()
    )?;
    Ok(())
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_048_576.0)
}

/// Builds the shared pack (see the module's documentation).
pub fn run(
    rom: &[u8],
    repo: &Path,
    args: &[&str],
    out: &mut dyn Write,
) -> anyhow::Result<ExitCode> {
    let Some(options) = parse(args) else { return Ok(ExitCode::from(crate::EXIT_USAGE)) };
    anyhow::ensure!(options.store.is_dir(), "{} is not a folder", options.store.display());
    let scratch = Scratch::new(repo)?;
    let reference = scratch.0.join("import");
    // 1. The private import: what the game would draw from this art store
    convert::run_with(rom, &reference, false, &mut std::io::sink())?;
    let masks_dir = scratch.0.join("masks");
    let masks = if options.with_masks && !options.masks.is_empty() {
        merge_masks(&options.masks, &masks_dir)?;
        Some(masks_dir.as_path())
    } else {
        None
    };
    let report =
        import::run(rom, &options.store, options.sources.as_deref(), masks, &reference, &mut *out)?;
    anyhow::ensure!(
        report.skipped.is_empty() || report.pictures + report.objects > 0,
        "the import skipped everything"
    );
    // 2. The plan, and the files
    let plan = plan(&reference, options.with_masks, options.maps_quality)?;
    writeln!(out, "hd-pack-public: {} art files to encode", plan.jobs.len())?;
    let packed = run_jobs(&plan.jobs, options.jobs, out)?;
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut classes: BTreeMap<&str, (usize, u64, u64)> = BTreeMap::new();
    let mut loss = (0.0_f64, 0_u8, 0_usize);
    for file in packed {
        let entry = classes.entry(file.class).or_default();
        entry.0 += 1;
        entry.1 += file.from_bytes;
        entry.2 += file.bytes.len() as u64;
        if let Some((mean, worst)) = file.loss {
            loss.0 += mean;
            loss.1 = loss.1.max(worst);
            loss.2 += 1;
        }
        anyhow::ensure!(
            files.insert(file.name.clone(), file.bytes).is_none(),
            "{} twice",
            file.name
        );
    }
    for (name, bytes) in plan.texts {
        files.insert(name, bytes);
    }
    files.insert("README.txt".to_owned(), PACK_README.as_bytes().to_vec());
    let licence = ["tools/public/LICENSE-ART.md", "LICENSE-ART.md"]
        .iter()
        .find_map(|name| std::fs::read(repo.join(name)).ok())
        .context("LICENSE-ART.md is missing")?;
    files.insert("LICENSE-ART.md".to_owned(), licence);
    let sizes: BTreeMap<String, u64> =
        files.iter().map(|(name, bytes)| (name.clone(), bytes.len() as u64)).collect();
    let manifest = Manifest {
        format: public::FORMAT,
        name: "Dragon Ball Z: Super Saiya Densetsu HD art pack".to_owned(),
        revision: options.revision.clone(),
        rom_crc32: format!("{:08X}", ssd_rom::crc32(rom)),
        digest: public::digest(&sizes),
        counts: plan.counts,
    };
    files.insert(public::MANIFEST.to_owned(), toml::to_string(&manifest)?.into_bytes());
    write_zip(&options.out, &files)?;
    // 3. The report
    writeln!(out, "hd-pack-public: size by kind (store -> pack):")?;
    for (class, (count, from, to)) in &classes {
        writeln!(
            out,
            "  {class:<22} {count:>6} files  {:>10} -> {:>10}",
            megabytes(*from),
            megabytes(*to)
        )?;
    }
    let others: u64 = files
        .iter()
        .filter(|(name, _)| name.rsplit_once('.').is_none_or(|(_, ext)| ext != "webp"))
        .map(|(_, bytes)| bytes.len() as u64)
        .sum();
    writeln!(
        out,
        "  {:<22} {:>6} files  {:>10}",
        "indexes and outlines",
        files.len() - classes.values().map(|c| c.0).sum::<usize>(),
        megabytes(others)
    )?;
    if loss.2 > 0 {
        writeln!(
            out,
            "hd-pack-public: the maps' lossy tiles: mean channel difference {:.2} of 255, largest {}",
            loss.0 / loss.2 as f64,
            loss.1
        )?;
    }
    let size = std::fs::metadata(&options.out)?.len();
    writeln!(out, "hd-pack-public: wrote {} ({})", options.out.display(), megabytes(size))?;
    // 4. Install it into a scratch data directory and compare
    if options.verify {
        let cache = scratch.0.join("cache");
        let mut lines = Vec::new();
        let done = install::install(
            rom,
            &options.out,
            &cache,
            &mut |line| lines.push(line.to_owned()),
            false,
        )?;
        let install::Outcome::Installed { pack, .. } = done else {
            anyhow::bail!("the scratch install found a pack already")
        };
        compare(&reference, &pack, out)?;
        // and a second install is a no-op
        let again = install::install(rom, &options.out, &cache, &mut |_| {}, false)?;
        anyhow::ensure!(matches!(again, install::Outcome::Current(_)), "a second install did work");
    }
    if let Some(keep) = &options.keep_reference {
        if keep.exists() {
            std::fs::remove_dir_all(keep)?;
        }
        std::fs::rename(&reference, keep)
            .with_context(|| format!("keeping the reference pack in {}", keep.display()))?;
        writeln!(out, "hd-pack-public: the import's pack is kept in {}", keep.display())?;
    }
    Ok(ExitCode::SUCCESS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pictures_source_names_its_stream_and_half() {
        assert_eq!(picture_source("portrait/16:C60A/4bpp", 0).unwrap(), "rom 16:C60A half 0");
        assert_eq!(picture_source("portrait/16:C60A/4bpp", 8).unwrap(), "rom 16:C60A half 1");
        assert!(picture_source("nonsense", 0).is_err());
    }

    #[test]
    fn names_follow_the_extension_swap() {
        assert_eq!(with_webp("map-01/t{x}_{y}.png"), "map-01/t{x}_{y}.webp");
        assert_eq!(with_webp("noext"), "noext");
        assert_eq!(stem("art/object/a.b.png"), "art/object/a.b");
    }

    fn picture(width: u32, height: u32) -> Vec<u8> {
        (0..width * height)
            .flat_map(|i| {
                let (x, y) = (i % width, i / width);
                [(x * 3) as u8, (y * 5) as u8, 90, if x < 2 { 0 } else { 255 }]
            })
            .collect()
    }

    #[test]
    fn lossless_files_decode_to_the_same_pixels_and_lossy_ones_are_measured() {
        let rgba = picture(40, 30);
        let (bytes, loss) = encode(40, 30, &rgba, Codec::Lossless).unwrap();
        assert!(loss.is_none() && ssd_hd::png::is_webp(&bytes));
        // (a pixel with no alpha keeps its colour, as the renderer's art files need)
        assert_eq!(ssd_hd::png::read_rgba(&bytes).unwrap().2, rgba);
        let opaque: Vec<u8> =
            rgba.as_chunks::<4>().0.iter().flat_map(|p| [p[0], p[1], p[2], 255]).collect();
        let (lossy, loss) = encode(40, 30, &opaque, Codec::Lossy(92.0)).unwrap();
        let (mean, worst) = loss.expect("a lossy file is measured");
        assert!(mean < 3.0 && worst < 40, "mean {mean}, worst {worst}");
        assert_eq!(ssd_hd::png::dimensions(&lossy).unwrap(), (40, 30));
    }

    #[test]
    fn the_command_line_takes_a_store_a_zip_and_options() {
        let options =
            parse(&["store", "out.zip", "--masks", "a", "--masks", "b", "--maps-quality", "88"])
                .unwrap();
        assert_eq!((options.masks.len(), options.maps_quality), (2, 88.0));
        assert!(options.verify && options.with_masks);
        assert!(!parse(&["s", "o.zip", "--no-verify", "--without-masks"]).unwrap().verify);
        assert!(parse(&["store"]).is_none());
        assert!(parse(&["s", "o", "--maps-quality", "0"]).is_none());
        assert!(parse(&["s", "o", "--nope"]).is_none());
    }

    #[test]
    fn masks_of_several_folders_are_merged_with_the_first_winning() {
        let dir = tempfile::tempdir().unwrap();
        for (folder, files) in [("a", &["1.png"][..]), ("b", &["1.png", "2.png"][..])] {
            std::fs::create_dir_all(dir.path().join(folder)).unwrap();
            for file in files {
                std::fs::write(dir.path().join(folder).join(file), folder).unwrap();
            }
        }
        let merged = dir.path().join("merged");
        merge_masks(&[dir.path().join("a"), dir.path().join("b")], &merged).unwrap();
        assert_eq!(std::fs::read_to_string(merged.join("1.png")).unwrap(), "a");
        assert_eq!(std::fs::read_to_string(merged.join("2.png")).unwrap(), "b");
    }
}
