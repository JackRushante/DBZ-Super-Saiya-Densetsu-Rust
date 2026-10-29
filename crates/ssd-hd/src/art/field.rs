//! The art of the field's map layer for a frame (`mapart`): the layer of the map the frame shows
//! is found by the pointers of the map's data, and the tiles of its art that the frame's pixels
//! touch are read and sized to the picture's scale (a map is up to 3072 x 3072 game pixels, so the
//! art is never loaded whole).

use super::{ArtState, Image, resample};
use crate::bgart::{BgArt, BgImage, Slot};
use crate::mapart::{self, Entry};
use crate::world::FieldState;
use rustc_hash::{FxHashMap, FxHashSet};
use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};

/// The memory budget of the field maps' tiles at the picture's scale, in bytes: the one limit of
/// their cache, to lower for a device with less memory. The tiles kept and the ones being read
/// ahead (queued, being read, or read and not taken yet) stay within it: 64 tiles of 4 MB at 4x,
/// 16 of 16 MB at 8x. Past it the tile used longest ago goes, and the tiles read ahead are as many
/// as fit beside the frame's.
const TILE_BUDGET: usize = 256 << 20;

/// The tiles read ahead around the ones a frame shows, on each side; one more is read on a side the
/// view moves to.
const MARGIN: usize = 1;

/// A tile of the field maps' art: the layer of the maps' index, the column and the row.
type TileKey = (u32, u16, u16);

fn read(path: &std::path::Path) -> Option<(usize, usize, Vec<u8>)> {
    let bytes = std::fs::read(path).ok()?;
    let (width, height, rgba) = crate::png::read_rgba(&bytes).ok()?;
    Some((width as usize, height as usize, rgba))
}

/// The art of the tile in file `path` at the picture's scale (`side` pixels square); `None` for a
/// file that cannot be read or is not a tile's size.
fn read_tile(path: &Path, side: usize) -> Option<Arc<Image>> {
    read(path).filter(|(w, h, _)| *w == mapart::TILE * mapart::SCALE && h == w).map(
        |(w, h, rgba)| {
            let art = Image::new(w, h, rgba);
            // At the art's own scale (4x) the file is the tile as it is drawn
            Arc::new(if w == side { art } else { resample::resize(&art, side, side) })
        },
    )
}

/// An image whose luma reference is not needed (the field's art is never luma art).
fn plain(width: usize, height: usize, rgba: Vec<u8>) -> Image {
    Image { width, height, rgba, luma_ref: 255 }
}

impl ArtState {
    /// Finds the art of the map a field frame shows (by the size and data pointers of its layer)
    /// and adds it to the frame's background slots; call it after [`ArtState::begin_bg`].
    pub(crate) fn begin_field(&mut self, field: Option<&FieldState>) {
        let Some(entry) = self.field_entry(field) else { return };
        // The map's layer takes BG2's place and leaves room in the records (a frame can match a
        // background class of the pack besides: it keeps the layers behind the map)
        let before: Vec<u32> = self.lookup.bg_slots.iter().map(|slot| slot.entry).collect();
        self.lookup.bg_slots.retain(|slot| slot.bg != 1);
        self.lookup.bg_slots.truncate(crate::bgart::MAX_SLOTS - 1);
        for entry in before {
            if !self.lookup.bg_slots.iter().any(|slot| slot.entry == entry) {
                self.stats.bg_unbound.push(entry);
            }
        }
        self.stats.field_map.bound = Some(entry as u32);
        let (width, height) = self.pack.maps.layer[entry].size();
        self.lookup.bg_slots.push(Slot {
            entry: entry as u32,
            bg: 1,
            large: false,
            fill: true,
            mode: 1,
            map_base: 0,
            char_base: 0,
            size: 0,
            first_row: 0,
            rows: 0,
            field: true,
            field_size: (width as u16, height as u16),
        });
    }

    /// The layer of the maps' index that the map data of `field` names, if the pack has art for it.
    pub(crate) fn field_entry(&self, field: Option<&FieldState>) -> Option<usize> {
        let field = field?;
        if self.pack.maps.layer.is_empty() {
            return None;
        }
        let (grid, metatiles) = field.map.layout();
        self.pack.maps.find(field.map.blocks, grid, metatiles, field.map_id)
    }

    /// The source picture of layer `at` of the maps' index (the layer at 1x in the colours the art
    /// was made for).
    fn field_source(&mut self, at: u32) -> Option<Arc<Image>> {
        if let Some(held) = self.field_sources.get(&at) {
            return held.clone();
        }
        let entry: &Entry = &self.pack.maps.layer[at as usize];
        let path = self.pack.dir.join("art").join("maps").join(&entry.source);
        let loaded = read(&path)
            .filter(|(w, h, _)| (*w, *h) == entry.size())
            .map(|(w, h, rgba)| Arc::new(plain(w, h, rgba)));
        // Only the layer being shown is kept
        self.field_sources.clear();
        self.field_sources.insert(at, loaded.clone());
        loaded
    }

    /// The tiles `needed` (column, row) of the art of layer `at`, at the picture's scale, for a
    /// frame whose pixels of the layer span game pixels `bounds` (see [`FieldTiles::tiles`]).
    fn field_tiles(
        &mut self,
        at: u32,
        needed: &BTreeSet<(u16, u16)>,
        bounds: [usize; 4],
    ) -> FxHashMap<(u16, u16), Arc<Image>> {
        let dir = self.pack.dir.join("art").join("maps");
        let entry = &self.pack.maps.layer[at as usize];
        let side = mapart::TILE * self.scale;
        let (tiles, read, read_ahead) =
            self.field_tiles.tiles(&dir, entry, at, needed, bounds, side, TILE_BUDGET);
        self.stats.field_map.read = read;
        self.stats.field_map.read_ahead = read_ahead;
        tiles
    }

    /// The art of the field layer of `slot` (slot `index` of the frame's) for the pixels of the
    /// frame: the tiles they touch.
    pub(crate) fn field_image(&mut self, slot: &Slot, index: usize) -> Option<Arc<BgImage>> {
        // (a pack with no source picture draws the art as it is)
        let source = if self.pack.maps.layer[slot.entry as usize].source.is_empty() {
            None
        } else {
            Some(self.field_source(slot.entry)?)
        };
        let mut needed = BTreeSet::new();
        let mut bounds = [usize::MAX, usize::MAX, 0, 0];
        for pixel in &self.pixels {
            let pix = pixel.bg.pix[index];
            if pix.is_set() {
                needed.insert((pix.x / mapart::TILE as u16, pix.y / mapart::TILE as u16));
                let (x, y) = (usize::from(pix.x), usize::from(pix.y));
                bounds = [bounds[0].min(x), bounds[1].min(y), bounds[2].max(x), bounds[3].max(y)];
            }
        }
        let tiles = self.field_tiles(slot.entry, &needed, bounds);
        let base = crate::bgart::parse_palette(&self.pack.maps.layer[slot.entry as usize].palette)
            .map(Arc::new);
        Some(Arc::new(BgImage { art: BgArt::Tiled(tiles), base, source }))
    }
}

/// The tiles of the field maps' art read at the picture's scale, and the thread that reads the
/// tiles around the view ahead of the frames that show them. Past the budget (with the tiles being
/// read) the tile used longest ago goes, one at a time; the tiles read ahead are as many as the
/// budget holds beside the frame's, and each frame uses those it keeps again, so a tile the view
/// left goes first and a view that stays still reads nothing more.
#[derive(Default)]
pub(crate) struct FieldTiles {
    tiles: FxHashMap<TileKey, Held>,
    /// The bytes of the pixels of `tiles`.
    bytes: usize,
    /// The count of the uses of the tiles: the last use of a tile is its `used`.
    clock: u64,
    /// The layer and the centre (in game pixels, twice over) of the last view, the way it moves is
    /// read from.
    view: Option<(u32, (usize, usize))>,
    /// Started at the first tile read ahead; `None` when the thread cannot be started (the tiles are
    /// then read as the frames need them).
    reader: Option<Reader>,
    /// Whether the thread was started once (it is not tried again when it could not be, or when it is gone).
    started: bool,
    /// The tiles the last frame asked to be read ahead: of the tiles still being read, the only ones that hold room in
    /// the budget and that are kept when they come back.
    wanted: FxHashSet<TileKey>,
}

/// A tile kept: its art (`None` for a file that cannot be read) and its last use.
struct Held {
    art: Option<Arc<Image>>,
    used: u64,
}

impl FieldTiles {
    /// Drops every tile (the picture's scale changed). Tiles still being read are refused when
    /// they come back: their size is not the scale's.
    pub(crate) fn clear(&mut self) {
        self.tiles.clear();
        self.bytes = 0;
        self.view = None;
        self.wanted.clear();
    }

    /// The tiles `needed` (column, row) of the art of layer `at` (`entry` of the maps' index whose
    /// folder is `dir`), at `side` pixels square, kept within `budget` bytes; how many of them were
    /// read here (no read ahead had brought them: a tile that is not read yet is read now, as the
    /// frame needs it), and how many tiles the reading thread brought since the last frame. The
    /// tiles within [`MARGIN`] of the frame's, and one more on a side the view moves to (by
    /// `bounds`, the game pixels left, top, right and bottom that the frame shows of the layer),
    /// nearest first and as many as the budget holds beside the frame's, are asked of the reading
    /// thread.
    #[allow(clippy::too_many_arguments)]
    fn tiles(
        &mut self,
        dir: &Path,
        entry: &Entry,
        at: u32,
        needed: &BTreeSet<(u16, u16)>,
        bounds: [usize; 4],
        side: usize,
        budget: usize,
    ) -> (FxHashMap<(u16, u16), Arc<Image>>, u32, u32) {
        let tile_bytes = side * side * 4;
        let read_ahead = self.take_read(side, budget, tile_bytes);
        // A reading thread that is gone (it panicked) leaves the tiles to the frames, and no room taken
        if self.reader.as_ref().is_some_and(|reader| !reader.alive()) {
            self.reader = None;
        }
        if !needed.is_empty() {
            let (columns, rows) =
                (needed.iter().map(|t| usize::from(t.0)), needed.iter().map(|t| usize::from(t.1)));
            let first = (columns.clone().min().unwrap_or(0), rows.clone().min().unwrap_or(0));
            let last = (columns.max().unwrap_or(0), rows.max().unwrap_or(0));
            let centre = (bounds[0] + bounds[2], bounds[1] + bounds[3]);
            let motion = match self.view {
                Some((layer, (x, y))) if layer == at => (sign(centre.0, x), sign(centre.1, y)),
                _ => (0, 0),
            };
            self.view = Some((at, centre));
            let room = room(budget, tile_bytes, needed.len());
            let wanted: Vec<(u16, u16)> =
                ahead(first, last, motion, entry.tile_grid()).into_iter().take(room).collect();
            self.wanted = wanted.iter().map(|&(tx, ty)| (at, tx, ty)).collect();
            // The tiles read ahead that are kept are used again: past the budget a tile the view
            // left goes before them
            for &(tx, ty) in &wanted {
                self.get((at, tx, ty));
            }
            let jobs: Vec<Job> = wanted
                .into_iter()
                .filter(|&(tx, ty)| !self.tiles.contains_key(&(at, tx, ty)))
                .map(|(tx, ty)| Job {
                    key: (at, tx, ty),
                    path: dir.join(entry.tile_file(usize::from(tx), usize::from(ty))),
                    side,
                })
                .collect();
            if !self.started && !jobs.is_empty() {
                self.started = true;
                self.reader = Reader::start();
            }
            if let Some(reader) = &mut self.reader {
                reader.ask(jobs);
            }
        }
        // The frame's own tiles that are kept are used first: past the budget a tile the view left
        // goes before one of them not decoded yet
        for &(tx, ty) in needed {
            self.get((at, tx, ty));
        }
        let mut tiles = FxHashMap::default();
        let mut read = 0;
        for &(tx, ty) in needed {
            let key = (at, tx, ty);
            let art = match self.get(key) {
                Some(held) => held.art.clone(),
                None => {
                    read += 1;
                    let path = dir.join(entry.tile_file(usize::from(tx), usize::from(ty)));
                    let art = read_tile(&path, side);
                    self.insert(key, art.clone(), self.room_left(budget, tile_bytes));
                    art
                }
            };
            if let Some(tile) = art {
                tiles.insert((tx, ty), tile);
            }
        }
        (tiles, read, read_ahead)
    }

    /// Tile `key` if it is kept, now its last use.
    fn get(&mut self, key: TileKey) -> Option<&Held> {
        let held = self.tiles.get_mut(&key)?;
        self.clock += 1;
        held.used = self.clock;
        Some(held)
    }

    /// Keeps `art` for tile `key`, the tiles used longest ago going first while the tiles would
    /// hold more than `budget` bytes.
    fn insert(&mut self, key: TileKey, art: Option<Arc<Image>>, budget: usize) {
        let size = art.as_ref().map_or(0, |image| image.rgba.len());
        if let Some(old) = self.tiles.remove(&key) {
            self.bytes -= old.art.as_ref().map_or(0, |image| image.rgba.len());
        }
        while self.bytes + size > budget
            && let Some(oldest) =
                self.tiles.iter().min_by_key(|(_, held)| held.used).map(|(key, _)| *key)
        {
            if let Some(gone) = self.tiles.remove(&oldest) {
                self.bytes -= gone.art.as_ref().map_or(0, |image| image.rgba.len());
            }
        }
        self.clock += 1;
        self.bytes += size;
        self.tiles.insert(key, Held { art, used: self.clock });
    }

    /// The bytes of `budget` the kept tiles may take: the tiles asked of the reading thread and not
    /// kept yet (queued, being read, or read and not taken) that the frame still wants take their
    /// room first; one asked for an older frame takes none (it is dropped when it comes back), so
    /// the frame's own tiles always have the room the budget gives.
    fn room_left(&self, budget: usize, tile_bytes: usize) -> usize {
        let asked = self.reader.as_ref().map_or(0, |reader| {
            reader.asked.iter().filter(|&key| self.wanted.contains(key)).count()
        });
        budget.saturating_sub(asked * tile_bytes)
    }

    /// Keeps the tiles the reading thread has read since the last frame, but for the ones of
    /// another scale than `side`, the ones read here meanwhile and the ones the last frame did not
    /// want any more; how many it read.
    fn take_read(&mut self, side: usize, budget: usize, tile_bytes: usize) -> u32 {
        let Some(reader) = &mut self.reader else { return 0 };
        let read = reader.take();
        let count = read.len() as u32;
        for (key, read_side, art) in read {
            if read_side == side && !self.tiles.contains_key(&key) && self.wanted.contains(&key) {
                self.insert(key, art, self.room_left(budget, tile_bytes));
            }
        }
        count
    }
}

/// How many tiles of `tile_bytes` are read ahead beside the `shown` tiles of a frame within
/// `budget` bytes.
fn room(budget: usize, tile_bytes: usize, shown: usize) -> usize {
    (budget / tile_bytes.max(1)).saturating_sub(shown)
}

/// -1, 0 or 1: how `now` compares with `before`.
fn sign(now: usize, before: usize) -> i32 {
    match now.cmp(&before) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

/// The tiles read ahead of a view that shows tiles `first` to `last` (column, row) of a layer of
/// `grid` columns and rows and moves by `motion` (the signs of its moves across and down): the
/// tiles within [`MARGIN`] of the view's, and one more on a side it moves to, inside the layer and
/// without the view's own; nearest first (by the larger of the distances across and down), then
/// by row and column.
fn ahead(
    first: (usize, usize),
    last: (usize, usize),
    motion: (i32, i32),
    grid: (usize, usize),
) -> Vec<(u16, u16)> {
    if grid.0 == 0 || grid.1 == 0 {
        return Vec::new();
    }
    let reach = |sign: i32, side: i32| MARGIN + usize::from(sign == side);
    let (x0, x1) = (
        first.0.saturating_sub(reach(motion.0, -1)),
        (last.0 + reach(motion.0, 1)).min(grid.0 - 1),
    );
    let (y0, y1) = (
        first.1.saturating_sub(reach(motion.1, -1)),
        (last.1 + reach(motion.1, 1)).min(grid.1 - 1),
    );
    let off =
        |at: usize, from: usize, to: usize| from.saturating_sub(at).max(at.saturating_sub(to));
    let mut tiles: Vec<(usize, usize, usize)> = (y0..=y1)
        .flat_map(|y| (x0..=x1).map(move |x| (x, y)))
        .map(|(x, y)| (off(x, first.0, last.0).max(off(y, first.1, last.1)), y, x))
        .filter(|&(distance, _, _)| distance > 0)
        .collect();
    tiles.sort_unstable();
    tiles.into_iter().map(|(_, y, x)| (x as u16, y as u16)).collect()
}

/// A tile to read: which, its file and the side it is read at.
struct Job {
    key: TileKey,
    path: PathBuf,
    side: usize,
}

/// What the drawing and the reading threads share: the tiles to read, in order, the tiles read
/// and not taken yet, and whether the reader is gone.
#[derive(Default)]
struct Shared {
    jobs: VecDeque<Job>,
    read: Vec<(TileKey, usize, Option<Arc<Image>>)>,
    closed: bool,
}

/// A thread that reads tiles of the field maps' art ahead of the frames (see [`FieldTiles`]); it
/// ends when this is dropped.
struct Reader {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    /// The tiles asked for that have not come back.
    asked: FxHashSet<TileKey>,
    /// The thread: once it has ended while the reader is there, it panicked.
    worker: std::thread::JoinHandle<()>,
}

impl Reader {
    /// The reader, its thread started; `None` when the thread cannot be.
    fn start() -> Option<Self> {
        Self::start_with(read_tile)
    }

    /// [`Reader::start`], the tiles read with `read`.
    fn start_with(read: fn(&Path, usize) -> Option<Arc<Image>>) -> Option<Self> {
        let shared = Arc::new((Mutex::new(Shared::default()), Condvar::new()));
        let theirs = Arc::clone(&shared);
        let worker = std::thread::Builder::new()
            .name("hd-field-tiles".to_owned())
            .spawn(move || {
                while let Some(job) = next_job(&theirs) {
                    let art = read(&job.path, job.side);
                    let Ok(mut shared) = theirs.0.lock() else { return };
                    shared.read.push((job.key, job.side, art));
                }
            })
            .ok()?;
        Some(Self { shared, asked: FxHashSet::default(), worker })
    }

    /// Whether the thread still runs (it only ends on its own by a panic).
    fn alive(&self) -> bool {
        !self.worker.is_finished()
    }

    /// Asks for `jobs`, in order, in place of the tiles asked for before that the thread has not
    /// started reading.
    fn ask(&mut self, jobs: Vec<Job>) {
        let (lock, wake) = &*self.shared;
        let Ok(mut shared) = lock.lock() else { return };
        for job in shared.jobs.drain(..) {
            self.asked.remove(&job.key);
        }
        for job in jobs {
            if self.asked.insert(job.key) {
                shared.jobs.push_back(job);
            }
        }
        if !shared.jobs.is_empty() {
            wake.notify_one();
        }
    }

    /// The tiles read since the last call: the tile, the side it was read at and its art.
    fn take(&mut self) -> Vec<(TileKey, usize, Option<Arc<Image>>)> {
        let Ok(mut shared) = self.shared.0.lock() else { return Vec::new() };
        let read = std::mem::take(&mut shared.read);
        for (key, _, _) in &read {
            self.asked.remove(key);
        }
        read
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        let (lock, wake) = &*self.shared;
        if let Ok(mut shared) = lock.lock() {
            shared.closed = true;
            shared.jobs.clear();
        }
        wake.notify_all();
    }
}

/// The next tile the reading thread reads, waiting for one; `None` once the reader is gone.
fn next_job(shared: &(Mutex<Shared>, Condvar)) -> Option<Job> {
    let (lock, wake) = shared;
    let mut shared = lock.lock().ok()?;
    loop {
        if shared.closed {
            return None;
        }
        if let Some(job) = shared.jobs.pop_front() {
            return Some(job);
        }
        shared = wake.wait(shared).ok()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    /// A tile's art of `bytes` bytes (one row of pixels).
    fn art(bytes: usize) -> Arc<Image> {
        Arc::new(Image { width: bytes / 4, height: 1, rgba: vec![0; bytes], luma_ref: 255 })
    }

    fn kept(cache: &FieldTiles) -> Vec<TileKey> {
        let mut keys: Vec<TileKey> = cache.tiles.keys().copied().collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn past_the_budget_the_tile_used_longest_ago_goes_and_only_it() {
        let mut cache = FieldTiles::default();
        let [one, two, three, four, five] = [0, 1, 2, 3, 4].map(|column| (0, column, 0));
        for key in [one, two, three] {
            cache.insert(key, Some(art(100)), 300);
        }
        // The first is used again: the second is now the one used longest ago
        assert!(cache.get(one).is_some());
        cache.insert(four, Some(art(100)), 300);
        assert_eq!(kept(&cache), vec![one, three, four]);
        assert_eq!(cache.bytes, 300);
        cache.insert(five, Some(art(100)), 300);
        assert_eq!(kept(&cache), vec![one, four, five]);
        // A tile twice the size makes room for itself: the two used longest ago go
        cache.insert((1, 0, 0), Some(art(200)), 300);
        assert_eq!(kept(&cache), vec![five, (1, 0, 0)]);
        assert_eq!(cache.bytes, 300);
        // A file that could not be read is kept too (it is not read again), and takes no bytes
        cache.insert((1, 1, 0), None, 300);
        assert!(cache.get((1, 1, 0)).is_some_and(|held| held.art.is_none()));
        assert_eq!(cache.bytes, 300);
    }

    #[test]
    fn the_tiles_never_hold_more_than_the_budget() {
        // 4 MB a tile at 4x: the budget is 64 of them
        let tile = (mapart::TILE * 4) * (mapart::TILE * 4) * 4;
        assert_eq!(TILE_BUDGET / tile, 64);
        // The same, a hundredth of the size
        let (tile, budget) = (tile / 100, TILE_BUDGET / 100);
        let mut cache = FieldTiles::default();
        for n in 0..200_u16 {
            cache.insert((0, n % 20, n / 20), Some(art(tile)), budget);
            assert!(cache.bytes <= budget);
        }
        assert_eq!(cache.tiles.len(), budget / tile);
        assert_eq!(
            cache.bytes,
            cache.tiles.values().map(|h| h.art.as_ref().map_or(0, |i| i.rgba.len())).sum::<usize>()
        );
    }

    #[test]
    fn the_tiles_read_ahead_surround_the_view_nearest_first_inside_the_layer() {
        // A view of 2 x 2 tiles in the middle of a layer of 6 x 5, still: the ring around it
        let ring = ahead((2, 1), (3, 2), (0, 0), (6, 5));
        assert_eq!(ring.len(), 16 - 4);
        assert!(ring.iter().all(|&(x, y)| (1..=4).contains(&x) && (0..=3).contains(&y)));
        assert!(!ring.iter().any(|&(x, y)| (2..=3).contains(&x) && (1..=2).contains(&y)));
        assert_eq!(&ring[..4], &[(1, 0), (2, 0), (3, 0), (4, 0)]);
        // Moving right and up: a second column on the right and a second row above (there is none)
        let moving = ahead((2, 1), (3, 2), (1, -1), (6, 5));
        assert_eq!(moving.len(), 5 * 4 - 4);
        assert!(moving.contains(&(5, 2)) && !moving.contains(&(0, 2)));
        assert_eq!(moving.last(), Some(&(5, 3)));
        assert!(
            moving.iter().position(|&t| t == (4, 1)) < moving.iter().position(|&t| t == (5, 1))
        );
        // At the corner of the layer: nothing outside it
        assert_eq!(ahead((0, 0), (0, 0), (0, 0), (3, 3)), vec![(1, 0), (0, 1), (1, 1)]);
        assert_eq!(ahead((0, 0), (0, 0), (-1, -1), (3, 3)), vec![(1, 0), (0, 1), (1, 1)]);
        assert_eq!(ahead((2, 2), (2, 2), (1, 1), (3, 3)), vec![(1, 1), (2, 1), (1, 2)]);
        // A layer of one tile has none to read ahead, and an empty one none at all
        assert_eq!(ahead((0, 0), (0, 0), (1, 1), (1, 1)), Vec::new());
        assert_eq!(ahead((0, 0), (0, 0), (0, 0), (0, 0)), Vec::new());
    }

    /// An entry of the maps' index of 3 x 3 tiles, `t{x}_{y}.png`, whose tiles are written in `dir`
    /// (tile (x, y) is filled with the colour x * 16 + y).
    fn layer(dir: &Path) -> Entry {
        layer_of(dir, 3, 3)
    }

    /// An entry of the maps' index of `columns` x `rows` tiles (see [`layer`]).
    fn layer_of(dir: &Path, columns: u8, rows: u8) -> Entry {
        let side = mapart::TILE * mapart::SCALE;
        for y in 0..rows {
            for x in 0..columns {
                let rgba: Vec<u8> = [x * 16 + y, y, x, 255].repeat(side * side);
                let path = dir.join(format!("t{x}_{y}.png"));
                crate::png::write_rgba(&path, side as u32, side as u32, &rgba).unwrap();
            }
        }
        Entry {
            id: "map-00".to_owned(),
            maps: vec![0],
            blocks: [columns, rows],
            grid: "06:F360".to_owned(),
            metatiles: "09:EFB0".to_owned(),
            source: String::new(),
            palette: String::new(),
            palette_variant: false,
            tiles: "t{x}_{y}.png".to_owned(),
        }
    }

    #[test]
    fn the_tiles_around_the_view_are_read_ahead_and_the_frame_that_shows_them_reads_none() {
        let dir = tempfile::tempdir().unwrap();
        let entry = layer(dir.path());
        let side = mapart::TILE;
        let mut cache = FieldTiles::default();
        // The first frame shows tile (0, 0) and reads it itself; its neighbours are asked for
        let first = BTreeSet::from([(0, 0)]);
        let (tiles, read, _) =
            cache.tiles(dir.path(), &entry, 0, &first, [10, 10, 200, 200], side, TILE_BUDGET);
        assert_eq!((tiles.len(), read), (1, 1));
        // The reading thread brings (1, 0), (0, 1) and (1, 1)
        let started = Instant::now();
        while kept(&cache).len() < 4 && started.elapsed() < Duration::from_secs(30) {
            std::thread::sleep(Duration::from_millis(5));
            cache.take_read(side, TILE_BUDGET, side * side * 4);
        }
        assert_eq!(kept(&cache), vec![(0, 0, 0), (0, 0, 1), (0, 1, 0), (0, 1, 1)]);
        // A frame that shows those reads none of them, and its tiles are the art of their files
        let next = BTreeSet::from([(0, 0), (1, 0), (0, 1), (1, 1)]);
        let (tiles, read, _) =
            cache.tiles(dir.path(), &entry, 0, &next, [200, 200, 300, 300], side, TILE_BUDGET);
        assert_eq!((tiles.len(), read), (4, 0));
        for (&(x, y), tile) in &tiles {
            let path = dir.path().join(format!("t{x}_{y}.png"));
            assert_eq!(Some(tile), read_tile(&path, side).as_ref(), "tile {x},{y}");
        }
        // Tiles read at another scale are not kept
        cache.clear();
        cache.take_read(side * 2, TILE_BUDGET, side * side * 16);
        let (_, read, _) =
            cache.tiles(dir.path(), &entry, 0, &first, [10, 10, 200, 200], side * 2, TILE_BUDGET);
        assert_eq!(read, 1);
        assert!(cache.tiles.values().all(|h| h.art.as_ref().is_none_or(|i| i.width == side * 2)));
    }

    #[test]
    fn the_tiles_read_ahead_are_as_many_as_the_budget_holds_beside_the_frames() {
        // A 16:9 frame shows up to 3 x 2 tiles: what is left of the budget at 4x, 6x, 7x and 8x
        let bytes = |scale: usize| (mapart::TILE * scale).pow(2) * 4;
        let rooms: Vec<usize> = [4, 6, 7, 8].map(|scale| room(TILE_BUDGET, bytes(scale), 6)).into();
        assert_eq!(rooms, vec![58, 22, 14, 10]);
        // At 8x the nearest of the ring around a 3 x 2 view (14 tiles) are the ones read ahead
        let ring = ahead((2, 2), (4, 3), (0, 0), (8, 8));
        assert_eq!(ring.len(), 14);
        let kept: Vec<(u16, u16)> = ring.iter().copied().take(rooms[3]).collect();
        assert_eq!(kept.len(), 10);
        assert!(kept.iter().all(|&(x, y)| (1..=5).contains(&x) && (1..=4).contains(&y)));
        // A budget smaller than the frame leaves none
        assert_eq!(room(bytes(8) * 4, bytes(8), 6), 0);
    }

    /// Frames of a 3 x 2 view (columns 2 to 4, rows 2 and 3 of a layer of 7 x 6) within a budget of
    /// `tiles` tiles, first moving (one step across and down), then still: once the tiles asked
    /// for are in, a still view asks for none and the reading thread brings none, frame after
    /// frame, and the kept tiles and those asked for never take more than the budget.
    fn still_view(tiles: usize) {
        let dir = tempfile::tempdir().unwrap();
        let entry = layer_of(dir.path(), 7, 6);
        let side = mapart::TILE;
        let tile_bytes = side * side * 4;
        let budget = tiles * tile_bytes;
        let view = BTreeSet::from([(2, 2), (3, 2), (4, 2), (2, 3), (3, 3), (4, 3)]);
        let mut cache = FieldTiles::default();
        let within = |cache: &FieldTiles| {
            let asked = cache.reader.as_ref().map_or(0, |reader| reader.asked.len());
            cache.bytes + asked * tile_bytes <= budget
        };
        let frame = |cache: &mut FieldTiles, bounds: [usize; 4]| {
            let (shown, _, read_ahead) =
                cache.tiles(dir.path(), &entry, 0, &view, bounds, side, budget);
            assert_eq!(shown.len(), 6);
            assert!(
                within(cache),
                "{} bytes kept and {:?} asked",
                cache.bytes,
                cache.reader.as_ref().map(|r| r.asked.len())
            );
            read_ahead
        };
        frame(&mut cache, [600, 600, 1100, 900]);
        frame(&mut cache, [601, 601, 1101, 901]);
        let still = [601, 601, 1101, 901];
        // The tiles asked for come in
        let started = Instant::now();
        loop {
            frame(&mut cache, still);
            let idle = cache.reader.as_ref().is_some_and(|reader| reader.asked.is_empty());
            if idle || started.elapsed() > Duration::from_secs(30) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        // The view and the nearest tiles around it that the budget holds are kept (some read ahead
        // while the view moved may be too), and no more than the budget
        let room = room(budget, tile_bytes, 6);
        let around = ahead((2, 2), (4, 3), (0, 0), (7, 6)).into_iter().take(room);
        for (x, y) in view.iter().copied().chain(around) {
            assert!(cache.tiles.contains_key(&(0, x, y)), "tile {x},{y} kept");
        }
        assert!(kept(&cache).len() <= tiles);
        // Then nothing more is read, and the tiles kept stay
        let before = kept(&cache);
        for _ in 0..40 {
            std::thread::sleep(Duration::from_millis(2));
            assert_eq!(frame(&mut cache, still), 0, "a still view reads nothing more");
            let reader = cache.reader.as_ref().unwrap();
            assert!(reader.asked.is_empty());
        }
        assert_eq!(kept(&cache), before);
    }

    #[test]
    fn a_still_view_reads_nothing_more_at_the_budget_of_6x() {
        still_view(TILE_BUDGET / ((mapart::TILE * 6).pow(2) * 4));
    }

    #[test]
    fn a_still_view_reads_nothing_more_at_the_budget_of_8x() {
        still_view(TILE_BUDGET / ((mapart::TILE * 8).pow(2) * 4));
    }

    /// The 3 x 2 view of [`still_view`], a cache of `tiles` tiles of `side` filled with tiles of
    /// row 5 (a view of long ago) and the layer's files.
    fn full_cache(dir: &Path, tiles: usize) -> (Entry, FieldTiles, BTreeSet<(u16, u16)>) {
        let entry = layer_of(dir, 7, 6);
        let tile_bytes = mapart::TILE * mapart::TILE * 4;
        let mut cache = FieldTiles::default();
        for n in 0..tiles {
            cache.insert(
                (0, (n % 7) as u16, 5 + (n / 7) as u16 * 10),
                Some(art(tile_bytes)),
                tiles * tile_bytes,
            );
        }
        let view = BTreeSet::from([(2, 2), (3, 2), (4, 2), (2, 3), (3, 3), (4, 3)]);
        (entry, cache, view)
    }

    #[test]
    fn the_tiles_asked_for_keep_their_room_in_a_full_budget() {
        let dir = tempfile::tempdir().unwrap();
        let (side, tile_bytes, tiles) = (mapart::TILE, mapart::TILE * mapart::TILE * 4, 16);
        let (entry, mut cache, view) = full_cache(dir.path(), tiles);
        assert_eq!(cache.bytes, tiles * tile_bytes);
        // The view comes in at once: its 6 tiles are read here and 10 around it are asked for; the
        // old tiles make room for both
        let (_, read, _) = cache.tiles(
            dir.path(),
            &entry,
            0,
            &view,
            [600, 600, 1100, 900],
            side,
            tiles * tile_bytes,
        );
        assert_eq!(read, 6);
        let asked = cache.reader.as_ref().map_or(0, |reader| reader.asked.len());
        assert_eq!(asked, 10);
        assert!(cache.bytes + asked * tile_bytes <= tiles * tile_bytes);
        assert_eq!(cache.bytes, 6 * tile_bytes);
    }

    #[test]
    fn the_tiles_read_ahead_that_a_frame_keeps_outlast_the_tiles_of_an_old_view() {
        let dir = tempfile::tempdir().unwrap();
        let (side, tile_bytes) = (mapart::TILE, mapart::TILE * mapart::TILE * 4);
        // A budget of 16 at 8x: 6 shown, the 10 nearest around them read ahead
        let budget = 16 * tile_bytes;
        let around: Vec<(u16, u16)> =
            ahead((2, 2), (4, 3), (0, 0), (7, 6)).into_iter().take(10).collect();
        let mut cache = FieldTiles::default();
        // The tiles around were read ahead long ago; 5 of the view and one of an old view since
        for &(x, y) in &around {
            cache.insert((0, x, y), Some(art(tile_bytes)), budget);
        }
        for key in [(0, 2, 2), (0, 3, 2), (0, 4, 2), (0, 2, 3), (0, 3, 3), (0, 6, 5)] {
            cache.insert(key, Some(art(tile_bytes)), budget);
        }
        assert_eq!(cache.tiles.len(), 16);
        // The frame shows the view (its sixth tile read here): the old view's tile goes, not one
        // read ahead, and nothing is asked for
        let entry = layer_of(dir.path(), 7, 6);
        let view = BTreeSet::from([(2, 2), (3, 2), (4, 2), (2, 3), (3, 3), (4, 3)]);
        let (_, read, _) =
            cache.tiles(dir.path(), &entry, 0, &view, [600, 600, 1100, 900], side, budget);
        assert_eq!(read, 1);
        assert!(!cache.tiles.contains_key(&(0, 6, 5)));
        for &(x, y) in &around {
            assert!(cache.tiles.contains_key(&(0, x, y)), "tile {x},{y} read ahead is kept");
        }
        assert!(cache.reader.is_none(), "nothing to read ahead");
    }

    /// The tile in file `path` read on the spot, as before the tiles were read ahead: decoded and
    /// resized to `side`.
    fn read_as_before(path: &Path, side: usize) -> Image {
        let (w, h, rgba) = read(path).unwrap();
        resample::resize(&Image::new(w, h, rgba), side, side)
    }

    #[test]
    fn a_tile_read_ahead_is_byte_for_byte_the_tile_read_on_the_spot() {
        let dir = tempfile::tempdir().unwrap();
        let entry = layer(dir.path());
        // At 1x (resized) and at 4x (the file's own size, not resized)
        for side in [mapart::TILE, mapart::TILE * mapart::SCALE] {
            let mut reader = Reader::start().unwrap();
            let path = dir.path().join(entry.tile_file(2, 1));
            reader.ask(vec![Job { key: (0, 2, 1), path: path.clone(), side }]);
            let started = Instant::now();
            let mut read = Vec::new();
            while read.is_empty() && started.elapsed() < Duration::from_secs(30) {
                std::thread::sleep(Duration::from_millis(5));
                read = reader.take();
            }
            let [(key, read_side, Some(ahead))] = read.as_slice() else {
                panic!("one tile read ahead")
            };
            assert_eq!((*key, *read_side), ((0, 2, 1), side));
            let here = read_tile(&path, side).unwrap();
            let before = read_as_before(&path, side);
            assert_eq!((ahead.width, ahead.height), (side, side));
            assert_eq!(ahead.rgba, here.rgba, "read ahead and read on the spot, {side} pixels");
            assert_eq!(ahead.rgba, before.rgba, "the same pixels as before, {side} pixels");
            assert_eq!((ahead.luma_ref, here.luma_ref), (before.luma_ref, before.luma_ref));
        }
    }

    #[test]
    fn a_frame_as_big_as_the_budget_keeps_its_tiles_beside_a_tile_an_older_frame_asked_for() {
        let dir = tempfile::tempdir().unwrap();
        let entry = layer(dir.path());
        let (side, tile_bytes) = (mapart::TILE, mapart::TILE * mapart::TILE * 4);
        let budget = 4 * tile_bytes;
        let view = BTreeSet::from([(0, 0), (1, 0), (0, 1), (1, 1)]);
        // A tile an older frame asked for, still being read: out of this view's reach
        let mut reader = Reader::start().unwrap();
        reader.asked.insert((0, 2, 2));
        let mut cache = FieldTiles { reader: Some(reader), started: true, ..FieldTiles::default() };
        let (shown, read, _) =
            cache.tiles(dir.path(), &entry, 0, &view, [10, 10, 500, 500], side, budget);
        assert_eq!((shown.len(), read), (4, 4));
        assert_eq!(kept(&cache).len(), 4, "no tile of the frame made room for it");
        let (_, read, _) =
            cache.tiles(dir.path(), &entry, 0, &view, [10, 10, 500, 500], side, budget);
        assert_eq!(read, 0, "the next frame reads none again");
    }

    fn fails(_: &Path, _: usize) -> Option<Arc<Image>> {
        panic!("a reading thread that fails")
    }

    #[test]
    fn a_reading_thread_that_is_gone_leaves_no_tile_asked_and_the_frames_read_theirs() {
        let dir = tempfile::tempdir().unwrap();
        let entry = layer(dir.path());
        let side = mapart::TILE;
        let mut cache = FieldTiles {
            reader: Reader::start_with(fails),
            started: true,
            ..FieldTiles::default()
        };
        let view = BTreeSet::from([(0, 0)]);
        // The ring is asked for; the thread panics on the first tile
        cache.tiles(dir.path(), &entry, 0, &view, [10, 10, 200, 200], side, TILE_BUDGET);
        let started = Instant::now();
        while cache.reader.as_ref().is_some_and(Reader::alive)
            && started.elapsed() < Duration::from_secs(30)
        {
            std::thread::sleep(Duration::from_millis(5));
        }
        let (_, read, _) =
            cache.tiles(dir.path(), &entry, 0, &view, [10, 10, 200, 200], side, TILE_BUDGET);
        assert_eq!(read, 0);
        assert!(cache.reader.is_none(), "the reader is gone with its tiles asked");
        assert_eq!(cache.room_left(TILE_BUDGET, side * side * 4), TILE_BUDGET);
        // The frames read their tiles themselves, and nothing is started again
        let next = BTreeSet::from([(1, 1)]);
        let (tiles, read, _) =
            cache.tiles(dir.path(), &entry, 0, &next, [300, 300, 400, 400], side, TILE_BUDGET);
        assert_eq!((tiles.len(), read), (1, 1));
        assert!(cache.reader.is_none());
    }

    #[test]
    fn a_tile_the_view_left_goes_before_a_tile_of_the_view() {
        let dir = tempfile::tempdir().unwrap();
        let entry = layer(dir.path());
        let (side, tile_bytes) = (mapart::TILE, mapart::TILE * mapart::TILE * 4);
        let budget = 2 * tile_bytes;
        let mut cache = FieldTiles::default();
        let before = BTreeSet::from([(1, 0), (2, 0)]);
        let (_, read, _) =
            cache.tiles(dir.path(), &entry, 0, &before, [300, 10, 700, 200], side, budget);
        assert_eq!(read, 2);
        // The view moves one tile left: (2, 0) goes for (0, 0), and (1, 0) stays
        let now = BTreeSet::from([(0, 0), (1, 0)]);
        let (shown, read, _) =
            cache.tiles(dir.path(), &entry, 0, &now, [10, 10, 400, 200], side, budget);
        assert_eq!((shown.len(), read), (2, 1), "only the tile that came into view is read");
        assert_eq!(kept(&cache), vec![(0, 0, 0), (0, 1, 0)]);
    }

    #[test]
    fn a_tile_an_older_frame_asked_for_is_dropped_when_it_comes_back() {
        let (side, tile_bytes) = (mapart::TILE, mapart::TILE * mapart::TILE * 4);
        let budget = 4 * tile_bytes;
        // (2, 2) was asked for by an older frame, (1, 1) is still wanted: both come back
        let mut reader = Reader::start().unwrap();
        reader.asked.extend([(0, 2, 2), (0, 1, 1)]);
        if let Ok(mut shared) = reader.shared.0.lock() {
            shared.read.push(((0, 2, 2), side, Some(art(tile_bytes))));
            shared.read.push(((0, 1, 1), side, Some(art(tile_bytes))));
        }
        let mut cache = FieldTiles {
            reader: Some(reader),
            started: true,
            wanted: FxHashSet::from_iter([(0, 1, 1)]),
            ..FieldTiles::default()
        };
        assert_eq!(cache.take_read(side, budget, tile_bytes), 2, "both came back");
        assert_eq!(kept(&cache), vec![(0, 1, 1)], "the one no frame wants is dropped");
        assert_eq!(cache.bytes, tile_bytes);
        assert_eq!(cache.room_left(budget, tile_bytes), budget, "and nothing holds room for it");
    }
}
