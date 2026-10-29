//! The approved art in the Enhanced picture: portraits and faces found through the pack's
//! tile binding, and battle poses and effects found by the key of their pixels (the pack's
//! `art.bin` is [`crate::art_index`]).
//!
//! While the renderer draws the 1x frame it records, for every pixel, what the art passes need
//! ([`ArtPixel`]): the picture and cell a bound character belongs to, the OAM entry an object
//! pixel comes from, and the colour math of the pixel. The pictures are drawn over the picture
//! the look made; a pose replaces the sprites it was made from: the frame is drawn a second time
//! with those sprites left out (the picture under them), and the pose's art is drawn over that,
//! where the sprites were visible in the first one. Nothing here reads anything but the render log
//! and the pack, so the picture is a function of the frame and the pack.
//!
//! The game draws colour variants of an enemy, and the same face in several palette rows, from the
//! same tiles. Art is made for one palette: where an entry matches by its tiles or shape but not
//! by its palette, the art is drawn recoloured to the palette the game uses ([`recolour`]); an
//! entry made for the exact palette is always preferred.

mod bgshadow;
mod draw;
mod field;
mod partial;
mod portrait_check;
pub(crate) mod poses;
mod recolour;
mod resample;
#[cfg(test)]
mod tests;

pub use portrait_check::PortraitUse;
pub use tally::{Fate, FrameTally, ItemKey, TallyCount, TallyItem, TallyLayer};
mod tally;

pub use resample::Image;

/// `image`, made for the palette `source`, in the colours of `row` (see [`recolour::recolour`]).
pub(crate) fn recolour_to(
    image: &Image,
    source: &crate::art_index::Palette,
    row: &[[u8; 3]; 16],
) -> Image {
    recolour::recolour(image, source, row)
}

use crate::art_index::Palette;
use crate::bgart::{self, BgArt, BgImage, Probe, Slot};
use crate::binding::Binder;
use crate::pack::{Pack, PoseVariant};
use crate::screen::{Layer, Plan};
use crate::transfer::{Colouring, Colours, Transfer};
use poses::{Pose, ROW_OF_CAPTURE};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use ssd_oracle::{FrameLog, LineRegs};
use std::collections::VecDeque;
use std::sync::Arc;

/// `ArtPixel::picture` of a pixel with no picture art.
pub(crate) const NO_PICTURE: u32 = u32::MAX;

/// `ArtPixel::asset` of a pixel whose character is bound to no asset of the pack.
pub(crate) const NO_ASSET: u32 = u32::MAX;

/// What layer a recorded pixel comes from (the tally of a tour, see [`FrameTally`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shown {
    /// Nothing is drawn there (or the tally is off).
    Nothing,
    Bg,
    Obj,
    /// A Mode 7 plane pixel.
    Plane,
}

/// The art index of a pose that has no art: a ground shadow, drawn from the frame's own pixels.
pub(crate) const SHADOW: u32 = u32::MAX;

/// The bit of an object index (in the poses found) that says the art is drawn mirrored left to
/// right: the pose is the art's pose facing the other way.
pub(crate) const MIRROR: u32 = 1 << 30;

/// The index of the object `at` names, without the mirror bit (never [`SHADOW`]).
pub(crate) fn real(at: u32) -> usize {
    (at & !MIRROR) as usize
}

/// Where the art of a pose came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ArtSource {
    /// The index has no art for it (or it is a ground shadow, drawn from the frame's pixels).
    #[default]
    None,
    /// The art made for the pose's own pixels.
    Exact,
    /// The art made for the pose facing the other way, drawn mirrored.
    Mirror,
    /// The art of a pose of the same shape in other colours (a palette variant).
    Shaped,
    /// A piece of the art of a bigger pose of the index (a fade-in's stripes, a cut sprite).
    Held,
}

/// The fewest opaque pixels of a pose found by its shape.
const SHAPE_MIN_OPAQUE: usize = 150;

/// The share (per mille) of a pose's opaque pixels with no neighbour left or right from which a pose
/// is a stripy fade-in, which the capture left out and may be part of a pose of the index.
const STRIPY_PARTIAL: u32 = 200;

/// The least share (per cent) of the opaque pixels of the pose of the index that a sprite cut by the
/// screen's edge shows, for it to be taken for a piece of that pose.
const CUT_MIN_SHARE: u32 = 30;

/// Partial poses whose holding pose is remembered by their key.
const PARTIAL_CACHE: usize = 16384;

/// What a recoloured art is kept by: a picture or an object, its index, the key of the pose it was
/// read for (0 when its own palette says what it was made for), and the colours it was drawn in.
type VariantKey = (bool, u32, u64, [[u8; 3]; 16]);

/// The bytes of recoloured art kept (a palette fade makes a new variant every frame).
const VARIANT_BYTES: usize = 96 << 20;

/// Objects whose art is kept decoded (the least recently used one goes first).
const OBJECT_CACHE: usize = 256;

/// Pictures of a layer's art whose colours are numbered and kept (see [`ArtState::numbered_colours`]).
const NUMBERED_PICTURES: usize = 16;

/// `image` flipped left to right: the art of a pose facing the other way, every row read right to left.
fn mirror(image: &Image) -> Image {
    let mut rgba = Vec::with_capacity(image.rgba.len());
    for row in image.rgba.chunks(image.width * 4) {
        for pixel in row.chunks(4).rev() {
            rgba.extend_from_slice(pixel);
        }
    }
    let mut mirrored = Image::new(image.width, image.height, rgba);
    mirrored.luma_ref = image.luma_ref;
    mirrored
}

/// What a four-bit character shows of the pictures.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Choice {
    /// No picture covers it.
    None,
    /// A character of a portrait asset that has no picture: counted, nothing drawn.
    Portrait,
    /// Cell `1` of picture `0`.
    Picture(u32, u8),
}

/// What the art passes need of one pixel of the 1x frame.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArtPixel {
    /// The picture of the art index the pixel's character is part of, or [`NO_PICTURE`].
    pub picture: u32,
    /// The character's cell in the 4 x 4 picture, and the pixel's place in the character
    /// ([`crate::pos`]).
    pub cell: u8,
    pub pos: u8,
    /// The OAM entry of an object pixel ([`crate::NO_OBJECT`]).
    pub oam: u8,
    /// The CGRAM row (16 colours each, the objects' from 8) of a pixel with a picture.
    pub row: u8,
    /// The priority number the pixel won with (lower is in front), and whether it is in mode 0 or 1
    /// (which says how an object's priority bits rank).
    pub prio: u8,
    pub mode_01: bool,
    pub plan: Plan,
    pub brightness: u8,
    /// What the pixel shows, filled only while the tally is on ([`ArtState::set_tally`]): its
    /// layer, the bit depth and VRAM word address of its character, and the pack tile the
    /// character is bound to ([`NO_ASSET`] when none).
    pub shown: Shown,
    pub bpp: u8,
    pub char: u16,
    pub asset: u32,
    pub tile: u32,
    /// What the art of the backgrounds needs of the pixel (see [`bgart`]).
    pub bg: bgart::BgRecord,
    /// For a pixel of a four-bit BG character: the character's VRAM word address, the pixel's
    /// place in it ([`crate::pos`]) and the palette number the BG gives it (a fighter drawn as BG
    /// tiles is read from these); `bg4` says they are set.
    pub bg4: bool,
    pub bg_char: u16,
    pub bg_pos: u8,
    pub bg_palette: u8,
}

impl ArtPixel {
    /// Whether an object with OAM priority bits `bits` would be in front of this pixel.
    pub(crate) fn beaten_by(&self, bits: u8) -> bool {
        object_priority(self.mode_01, bits) <= self.prio
    }

    pub(crate) const NONE: Self = Self {
        picture: NO_PICTURE,
        cell: 0,
        pos: 0,
        oam: crate::NO_OBJECT,
        row: 0,
        prio: u8::MAX,
        mode_01: true,
        plan: Plan { force_black: false, math: None },
        brightness: 15,
        shown: Shown::Nothing,
        bpp: 0,
        char: 0,
        asset: NO_ASSET,
        tile: 0,
        bg: bgart::BgRecord::NONE,
        bg4: false,
        bg_char: 0,
        bg_pos: 0,
        bg_palette: 0,
    };
}

/// What one pose of a frame came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoseStat {
    /// A fighter (palette 1 or 2) or an effect.
    pub fighter: bool,
    /// A sprite group of the field (neither a fighter nor an effect).
    pub field: bool,
    /// A ground shadow (one dark flat ellipse), which the renderer draws from the game's own shape.
    pub shadow: bool,
    /// The OAM entries the pose draws or hides (bit n: entry n), the ones another pose took not counted.
    pub sprites: u128,
    /// The OAM entries the pose had but another pose took (the ones `sprites` leaves out).
    pub excluded: u128,
    /// The pose's sprites mostly show a picture's pixels (a portrait drawn as sprites): it is that
    /// picture, not a pose of its own (see `is_picture`).
    pub picture: bool,
    /// The art index has art for the pose, and it was drawn.
    pub matched: bool,
    /// The art was made for another palette than the pose's, and was drawn recoloured.
    pub recoloured: bool,
    /// Pixels of the frame the pose's sprites show.
    pub visible: u32,
    /// Opaque pixels of the pose, and the share of them with no opaque neighbour left or right
    /// (per mille): the capture left poses of fewer than 40 pixels or 200 per mille out.
    pub opaque: u32,
    pub stripy: u32,
    pub key: u64,
    /// The OAM palette of the pose's first sprite.
    pub palette: u8,
    /// Where the pose's art came from, and which object of the index it is (when it has art).
    pub source: ArtSource,
    pub art: Option<u32>,
    /// Where the pose's array sits (x of the screen, row of the capture) and its size.
    pub org: (i32, i32),
    pub size: (usize, usize),
    /// What the original shows inside the pose's box, as a digest that does not depend on how the
    /// sprites are ordered or grouped (see [`poses::region_digest`]); filled while the art measures.
    pub look: u64,
}

/// A pose of a frame that has no art (a missing or a partial one) with its pixels, for the tour to
/// write out: what art is made from. Kept only while the tally is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoseShape {
    pub key: u64,
    pub fighter: bool,
    /// A sprite group of the field (neither a fighter nor an effect).
    pub field: bool,
    /// A ground shadow (one dark flat ellipse), which the renderer draws from the game's own shape.
    pub shadow: bool,
    /// What became of the pose (never [`Fate::Art`]).
    pub fate: Fate,
    pub w: usize,
    pub h: usize,
    /// The trimmed palette-index array, row by row (0 transparent).
    pub arr: Vec<u8>,
    /// An effect's trimmed colours (RGBA, row by row); empty for a fighter.
    pub rgba: Vec<u8>,
    /// Where the array's top-left pixel is on the screen (x, and the capture's row).
    pub org: (i32, i32),
    /// The OAM palette of the pose's first sprite and the colours of that palette's row.
    pub palette: u8,
    pub row: [[u8; 3]; 16],
}

/// The kinds of pose the pixels of the sprites are counted by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Fighter = 0,
    Effect = 1,
    Field = 2,
}

impl Kind {
    fn of(pose: &Pose) -> Self {
        if pose.field {
            Self::Field
        } else if pose.fighter {
            Self::Fighter
        } else {
            Self::Effect
        }
    }
}

/// What a pose drawn from art drew in a frame: the colours it drew against those of the original.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrawnPose {
    pub key: u64,
    pub fighter: bool,
    /// A sprite group of the field.
    pub field: bool,
    /// The art is a luma shape.
    pub luma: bool,
    pub drawn: Drawn,
}

/// Pixels of the frame that the sprites of fighters (or of effects) show, by what became of them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KindPixels {
    /// All of them.
    pub total: u64,
    /// Of a pose whose art was drawn.
    pub art: u64,
    /// Of those, the pixels of poses whose art was made for another palette (drawn recoloured).
    pub recoloured: u64,
    /// Of a pose the capture would have kept (40 pixels, not stripy) with no art in the index.
    pub missing: u64,
    /// Of a pose the capture left out (fewer than 40 pixels, or stripy: a fade-in, a cut sprite).
    pub partial: u64,
}

/// What the art did in the last frame rendered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ArtStats {
    pub fighters: KindPixels,
    pub effects: KindPixels,
    /// The sprites of the field's groups (see [`poses::field_groups`]); with the fighters and the
    /// effects it counts the same sprite again where it is in both kinds of group.
    pub field: KindPixels,
    /// Every pixel the object layer shows, inside the original window and in the margins of the
    /// wide view (where the field's objects the original drops are drawn), by what became of the
    /// sprite that shows it: the art covers the margins as it covers the window, so the two
    /// shares of `art` match.
    pub sprites_window: KindPixels,
    pub sprites_margin: KindPixels,
    /// Pixels of the groups that the edge of the view cuts (or that run into the top or the bottom
    /// of the picture, where the capture's sprites end) that no art holds, and of the sprites that
    /// make no group (fewer than 40 pixels): the pieces that remain are too few to find their
    /// object by, in the window as in the margins, and are left out of the two above.
    pub sprites_cut: u64,
    pub poses: Vec<PoseStat>,
    /// What each pose drawn from art drew, with its colours, when the state measures them.
    pub drawn: Vec<DrawnPose>,
    /// Pixels drawn from a picture (portraits and faces), and pixels of characters of the pack's
    /// portrait assets that had no picture.
    pub picture_pixels: u64,
    /// Of those, the pixels drawn in the palette row the game uses where the picture was made for
    /// another palette (recoloured).
    pub picture_recoloured: u64,
    pub portrait_pixels_without_art: u64,
    /// Pixels of battle backgrounds (the layers of [`bgart::BATTLE_LAYERS`] and their backdrop,
    /// where nothing is in front), and the ones drawn from the art of a layer.
    pub bg_pixels: u64,
    pub bg_art: u64,
    /// The map art of the field frame: the layer of the maps' index its map data names
    /// (`available`, for a frame whose BG2 shows the field's map in whatever main loop state) and
    /// whether the frame draws it (`bound`: the frame is classed as the field and its layer slot was
    /// bound to that art).
    pub field_map: FieldMapArt,
    /// The entries of the backgrounds' index whose content the frame shows and that it does not draw
    /// (the layer's slot was lost: VRAM loaded while it is displayed, or the field's map took its
    /// place).
    pub bg_unbound: Vec<u32>,
}

/// See [`ArtStats::field_map`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FieldMapArt {
    pub available: Option<u32>,
    pub bound: Option<u32>,
    /// The tiles of the art the frame read as it was drawn: no read ahead had brought them.
    pub read: u32,
    /// The tiles of the art the reading thread brought since the frame before (read ahead).
    pub read_ahead: u32,
}

/// The Mode 7 plane a frame shows: the matrix of each line (by scanline; `None` for a line that
/// does not show the plane) and the words of VRAM the plane is made of.
pub(crate) struct PlaneFrame {
    lines: Vec<Option<crate::mode7::Plane>>,
    vram: Vec<u16>,
}

/// The work RAM bytes the conditions of a pack's pose variants are read from (`docs/re/data/kaioken-bp.md`, section 4).
const KAIOKEN_STATE: usize = 0x0C36;
/// The party records: nine of them from `$0B00`, `$20` bytes each, the unit in the second byte.
const PARTY_RECORDS: usize = 0x0B00;
const RECORD_STRIDE: usize = 0x20;
const PARTY_SLOTS: usize = 9;
/// The unit of Super Saiyan Goku.
const SUPER_SAIYAN_GOKU: u8 = 0x3B;
/// The mark of a unit of Goku's art in the index (`battle-goku-NNN`, `gallery-party-goku-u00-NNN`, see [`ArtState::unit_of`]).
const GOKU_MARK: &str = "goku";
/// The mark of a unit of Super Saiyan Goku's art (`battle-goku-ssj-NNN`, `gallery-party-goku-ssj-NNN`).
const SUPER_SAIYAN_GOKU_MARK: &str = "goku-ssj";

/// The conditions a pack's pose variants name (`[[pose_variant]]`, `ssd_mod::pose::Condition`), as the game's work RAM of a
/// frame shows them. The Enhanced picture draws a variant only while its condition holds. `super_saiyan_goku` is the party
/// record of Super Saiyan Goku (see [`ArtState::unit_for`]).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Conditions {
    kaioken: bool,
    super_saiyan_goku: bool,
}

impl Conditions {
    /// The conditions of the work RAM `wram` (the 128 KiB of `$7E` and `$7F`): the Kaioken is on (`$0C36` bit 7) and a party
    /// record has Super Saiyan Goku's unit (`$3B`). A `wram` too short to hold the bytes says no.
    #[must_use]
    pub fn of_work_ram(wram: &[u8]) -> Self {
        let on = wram.get(KAIOKEN_STATE).is_some_and(|byte| byte & 0x80 != 0);
        let goku = (0..PARTY_SLOTS).any(|slot| {
            wram.get(PARTY_RECORDS + RECORD_STRIDE * slot + 1) == Some(&SUPER_SAIYAN_GOKU)
        });
        Self { kaioken: on && goku, super_saiyan_goku: goku }
    }

    fn holds(self, condition: ssd_mod::pose::Condition) -> bool {
        match condition {
            ssd_mod::pose::Condition::Kaioken => self.kaioken,
            ssd_mod::pose::Condition::Always => true,
        }
    }
}

/// The art of a pack, loaded as the frames need it, at the scale of the picture.
pub(crate) struct ArtState {
    pack: Arc<Pack>,
    /// What the capture of a frame's pixels reads of the pack (shared by the threads).
    lookup: Lookup,
    scale: usize,
    pictures: FxHashMap<u32, Option<Arc<Image>>>,
    objects: FxHashMap<u32, Option<Arc<Image>>>,
    recent: VecDeque<u32>,
    /// Art drawn in the colours of a palette other than the one it was made for, by what it is
    /// (a picture or an object), its index and the colours, and the bytes they take.
    variants: FxHashMap<VariantKey, Arc<Image>>,
    variant_bytes: usize,
    /// The pictures of the pack's pose variants at the scale, by the index of the variant and whether it is mirrored (see
    /// [`ArtState::pose_variant`]); `None` for a picture that cannot be read.
    pose_images: FxHashMap<(usize, bool), Option<Arc<Image>>>,
    /// The conditions the pose variants are drawn under: those of the frame (see [`Conditions::of_work_ram`]).
    pub(crate) conditions: Conditions,
    /// The art of the background layers, by the index of the entry.
    bg_images: FxHashMap<u32, Option<Arc<BgImage>>>,
    /// The field maps' art: the source of the layer shown, and the tiles of art read at the
    /// picture's scale, by layer and tile.
    field_sources: FxHashMap<u32, Option<Arc<Image>>>,
    field_tiles: field::FieldTiles,
    /// The Mode 7 plane's art, by the index of the entry.
    plane_arts: FxHashMap<u32, Option<Arc<crate::mode7art::PlaneArt>>>,
    /// The plane the frame shows (see [`ArtState::begin_bg`]): the matrix of each line, by
    /// scanline, and the VRAM words the plane is made of.
    plane: Option<PlaneFrame>,
    /// The CGRAM at the start of the frame, the colours the pictures are drawn in.
    cgram: Vec<u16>,
    /// For the keys of partial poses seen: the object that holds it and where (see [`partial`]).
    partials: FxHashMap<u64, Option<(u32, i32, i32)>>,
    /// The number of poses of each unit of the index (see [`ArtState::unit_for`]).
    unit_sizes: std::sync::OnceLock<FxHashMap<String, u32>>,
    /// For the keys of poses seen that the index has no key for: the object of their shape.
    shapes: FxHashMap<u64, Option<u32>>,
    pub(crate) binder: Option<Binder<'static>>,
    /// Whether [`ArtStats::drawn`] is filled.
    pub(crate) measure: bool,
    /// The layers' art numbered by its colours, kept for the last pictures drawn (see [`ArtState::colourings`]), and
    /// whether the table transfers are made that way (`transfer_cache`).
    numbered: Vec<(Arc<BgImage>, Option<Arc<Colours>>)>,
    pub(crate) transfer_cache: bool,
    /// Draw the backgrounds' art alone (no poses, portraits or shadows): what an audit compares the
    /// layers' colours with the original's by.
    pub(crate) backgrounds_only: bool,
    /// The last answer of [`Lookup::choose`] and what it was asked.
    memo: Memo,
    /// The record of each pixel of the frame being drawn.
    pub(crate) pixels: Vec<ArtPixel>,
    /// The sprites (a bit for each OAM entry) of each pose of the last frame, as `stats.poses`.
    pose_sprites: Vec<u128>,
    /// The poses of the last frame that have no art, with the index of their entry in
    /// `stats.poses` (filled while the tally is on).
    dumps: Vec<(usize, PoseShape)>,
    /// Whether the frames are tallied instead of drawn (see [`ArtState::set_tally`]) and the last
    /// frame's tally.
    tally_on: bool,
    pub(crate) tally: FrameTally,
    pub(crate) width: usize,
    pub(crate) height: usize,
    /// Whether the renderer records the pixels of the lines it draws.
    pub(crate) active: bool,
    /// The main loop is known to be in a field state (the map, a menu or a scene over it, a town),
    /// never a battle: a pose that has art made for a fighter is then still a figure of the field
    /// (see [`ArtState::find_poses_in`]). `None` when the caller does not know.
    pub(crate) field_frame: Option<bool>,
    pub(crate) stats: ArtStats,
}

impl ArtState {
    pub(crate) fn new(pack: Arc<Pack>) -> Self {
        let mut asset_pictures = vec![Vec::new(); pack.index.assets().len()];
        let by_id: FxHashMap<&str, usize> =
            pack.index.assets().iter().enumerate().map(|(at, a)| (a.id.as_str(), at)).collect();
        for (at, picture) in pack.art.pictures().iter().enumerate() {
            if let Some(&asset) = by_id.get(picture.asset.as_str()) {
                asset_pictures[asset].push(at as u32);
            }
        }
        let portrait_asset =
            pack.index.assets().iter().map(|a| a.id.starts_with("portrait/")).collect();
        Self {
            binder: Some(Binder::of_pack(Arc::clone(&pack))),
            lookup: Lookup {
                pack: Arc::clone(&pack),
                track: false,
                asset_pictures,
                portrait_asset,
                bg_classes: bgart::Classes::new(&pack.bg),
                bg_slots: Vec::new(),
            },
            pack,
            scale: 0,
            pictures: FxHashMap::default(),
            objects: FxHashMap::default(),
            recent: VecDeque::new(),
            variants: FxHashMap::default(),
            variant_bytes: 0,
            pose_images: FxHashMap::default(),
            conditions: Conditions::default(),
            bg_images: FxHashMap::default(),
            field_sources: FxHashMap::default(),
            field_tiles: field::FieldTiles::default(),
            plane_arts: FxHashMap::default(),
            plane: None,
            cgram: Vec::new(),
            partials: FxHashMap::default(),
            unit_sizes: std::sync::OnceLock::new(),
            shapes: FxHashMap::default(),
            measure: false,
            numbered: Vec::new(),
            transfer_cache: true,
            backgrounds_only: false,
            memo: None,
            pixels: Vec::new(),
            pose_sprites: Vec::new(),
            tally_on: false,
            dumps: Vec::new(),
            tally: FrameTally::default(),
            width: 0,
            height: 0,
            active: false,
            field_frame: None,
            stats: ArtStats::default(),
        }
    }

    /// Turns the tally on: the pixels record what they show, [`ArtState::tally_frame`] counts
    /// them, and the pictures do not draw the art (only what would be drawn is counted, so a
    /// tour does not decode thousands of art files).
    pub(crate) fn set_tally(&mut self, on: bool) {
        self.tally_on = on;
        self.lookup.track = on;
    }

    pub(crate) fn tallying(&self) -> bool {
        self.tally_on
    }

    /// Whether the pack holds any art to draw.
    pub(crate) fn has_art(&self) -> bool {
        !self.pack.art.pictures().is_empty()
            || !self.pack.art.objects().is_empty()
            || !self.pack.bg.layer.is_empty()
            || !self.pack.maps.layer.is_empty()
            || !self.pack.mode7.plane.is_empty()
    }

    /// Starts a frame of `width` x `height` pixels drawn at `scale`.
    pub(crate) fn begin(&mut self, width: usize, height: usize, scale: usize) {
        if scale != self.scale {
            self.scale = scale;
            self.numbered.clear();
            self.pictures.clear();
            self.objects.clear();
            self.recent.clear();
            self.variants.clear();
            self.variant_bytes = 0;
            self.pose_images.clear();
            self.bg_images.clear();
            self.field_tiles.clear();
            self.plane_arts.clear();
        }
        self.field_frame = None;
        // (a record is a hundred bytes: a frame's are megabytes, written by every worker)
        if self.pixels.len() == width * height {
            let band = crate::pool::band(height) * width;
            self.pixels.par_chunks_mut(band).for_each(|chunk| chunk.fill(ArtPixel::NONE));
        } else {
            self.pixels.clear();
            self.pixels.resize(width * height, ArtPixel::NONE);
        }
        self.width = width;
        self.height = height;
        self.stats = ArtStats::default();
        self.pose_sprites.clear();
        self.dumps.clear();
        self.tally = FrameTally::default();
    }

    /// The background layers of the frame being drawn.
    pub(crate) fn bg_slots(&self) -> &[Slot] {
        &self.lookup.bg_slots
    }

    /// Finds the background layers of the pack that the frame `log` shows (by the content of
    /// their tiles), before its pixels are captured.
    pub(crate) fn begin_bg(&mut self, log: &FrameLog) {
        let mut slots = if self.lookup.bg_classes.is_empty() {
            Vec::new()
        } else {
            self.lookup.bg_classes.find(&self.pack.bg, log)
        };
        if !self.lookup.bg_classes.is_empty() {
            self.stats.bg_unbound = self.lookup.bg_classes.unbound(&self.pack.bg, log, &slots);
        }
        self.plane = None;
        if let Some((at, plane)) = self.find_plane(log) {
            slots.insert(0, Slot::plane(at));
            self.plane = Some(plane);
        }
        self.lookup.bg_slots = slots;
    }

    /// The Mode 7 plane of the pack that the frame shows, if any: found by the content of VRAM, and
    /// with the matrix of each of its lines. A frame that loads VRAM while it is displayed is not
    /// one plane.
    fn find_plane(&self, log: &FrameLog) -> Option<(u32, PlaneFrame)> {
        if self.pack.mode7.plane.is_empty() || !log.vram_writes.is_empty() {
            return None;
        }
        let shows = |regs: &LineRegs| {
            !regs.forced_blanking && regs.bg_mode == 7 && regs.main_bg[0] && !regs.direct_color
        };
        if !log.entries.iter().any(|entry| shows(&entry.regs)) {
            return None;
        }
        let at = self.pack.mode7.find(crate::mode7art::plane_key(&log.vram))?;
        let mut lines = vec![None; usize::from(log.height) + 2];
        for entry in &log.entries {
            if shows(&entry.regs)
                && entry.from_pixel == 0
                && let Some(line) = lines.get_mut(usize::from(entry.scanline))
            {
                *line = Some(crate::mode7::Plane::of(&entry.regs));
            }
        }
        let words = log.vram.len().min(0x4000);
        Some((at as u32, PlaneFrame { lines, vram: log.vram[..words].to_vec() }))
    }

    /// The art of the plane of entry `at`, read whole (the art's own size) and kept.
    fn plane_art(&mut self, at: u32) -> Option<Arc<crate::mode7art::PlaneArt>> {
        if let Some(loaded) = self.plane_arts.get(&at) {
            return loaded.clone();
        }
        let loaded = self.load_plane_art(at);
        self.plane_arts.insert(at, loaded.clone());
        loaded
    }

    fn load_plane_art(&self, at: u32) -> Option<Arc<crate::mode7art::PlaneArt>> {
        use crate::mode7art::{Entry, PLANE, PlaneArt, SCALE, TILE};
        let entry = &self.pack.mode7.plane[at as usize];
        let palette = entry.colours()?;
        let (side, across) = (PLANE * SCALE, Entry::tiles_across());
        let tile_side = TILE * SCALE;
        let tiles: Vec<Option<Vec<u8>>> = (0..across * across)
            .into_par_iter()
            .map(|n| {
                let bytes = std::fs::read(
                    self.pack.dir.join("art/mode7").join(entry.tile_file(n % across, n / across)),
                )
                .ok()?;
                let (w, h, rgba) = crate::png::read_rgba(&bytes).ok()?;
                (w as usize == tile_side && h as usize == tile_side).then_some(rgba)
            })
            .collect();
        let mut rgba = vec![0_u8; side * side * 4];
        for (n, tile) in tiles.into_iter().enumerate() {
            let tile = tile?;
            let (ox, oy) = ((n % across) * tile_side, (n / across) * tile_side);
            for y in 0..tile_side {
                let from = y * tile_side * 4;
                let to = ((oy + y) * side + ox) * 4;
                rgba[to..to + tile_side * 4].copy_from_slice(&tile[from..from + tile_side * 4]);
            }
        }
        Some(Arc::new(PlaneArt::new(side, &rgba, palette)))
    }

    /// The art of background entry `at` at the picture's scale, with its source.
    fn bg_image(&mut self, at: u32) -> Option<Arc<BgImage>> {
        if let Some(loaded) = self.bg_images.get(&at) {
            return loaded.clone();
        }
        let loaded = self.load_bg(at);
        if self.bg_images.len() >= 4 {
            self.bg_images.clear();
        }
        self.bg_images.insert(at, loaded.clone());
        loaded
    }

    fn load_bg(&self, at: u32) -> Option<Arc<BgImage>> {
        let entry = &self.pack.bg.layer[at as usize];
        let large = self.lookup.bg_slots.iter().any(|slot| slot.entry == at && slot.large);
        let (width, height) = entry.art_size(large);
        let read = |relative: &str| -> Option<(usize, usize, Vec<u8>)> {
            let bytes = std::fs::read(self.pack.dir.join(relative)).ok()?;
            let (w, h, rgba) = crate::png::read_rgba(&bytes).ok()?;
            Some((w as usize, h as usize, rgba))
        };
        let (aw, ah, art) = read(&entry.art)?;
        // The files are what the index says: the layer's rows at the art's scale, and at 1x
        if (aw, ah) != (width * bgart::SCALE, height * bgart::SCALE) {
            return None;
        }
        let source = if entry.source.is_empty() {
            None
        } else {
            let (sw, sh, source) = read(&entry.source)?;
            if (sw, sh) != (width, height) {
                return None;
            }
            Some(Arc::new(Image::new(sw, sh, source)))
        };
        let art =
            resample::resize(&Image::new(aw, ah, art), width * self.scale, height * self.scale);
        let base = bgart::parse_palette(&entry.palette).map(Arc::new);
        Some(Arc::new(BgImage { art: BgArt::Whole(art), base, source }))
    }

    fn load(&self, relative: &str, width: usize, height: usize) -> Option<Arc<Image>> {
        let bytes = std::fs::read(self.pack.dir.join(relative)).ok()?;
        let (w, h, rgba) = crate::png::read_rgba(&bytes).ok()?;
        let image = Image::new(w as usize, h as usize, rgba);
        if image.width == 0 || image.height == 0 {
            return None;
        }
        Some(Arc::new(resample::resize(&image, width, height)))
    }

    /// The art of picture `at` of the index: 32 art pixels to a character, at the picture's scale.
    pub(crate) fn picture(&mut self, at: u32) -> Option<Arc<Image>> {
        if let Some(loaded) = self.pictures.get(&at) {
            return loaded.clone();
        }
        let image = self.load_picture(at);
        self.pictures.insert(at, image.clone());
        image
    }

    /// The art of object `at` of the index at the picture's scale.
    pub(crate) fn object(&mut self, at: u32) -> Option<Arc<Image>> {
        if let Some(loaded) = self.objects.get(&at) {
            let loaded = loaded.clone();
            if let Some(place) = self.recent.iter().position(|&n| n == at) {
                self.recent.remove(place);
            }
            self.recent.push_back(at);
            return loaded;
        }
        let image = self.load_object(at);
        self.remember_object(at, image.clone());
        image
    }

    /// Records what the art passes need of the pixels `from..` of line `line` (1-based) that the
    /// renderer `r` has just resolved, in the row `x0 + x` of the record.
    pub(crate) fn capture_line(
        &mut self,
        r: &crate::Renderer,
        regs: &LineRegs,
        binder: &Binder<'_>,
        line: usize,
        from: usize,
    ) {
        for x in from..crate::WIDTH {
            let plan_as = |layer, palette| r.plan_as(regs, x, layer, palette);
            let layer_colour = |bg| r.bg_colour(regs, bg, x);
            let layer_index = |bg| r.bg_index(regs, bg, x);
            let probe = Probe {
                rendered: &r.main.rendered[x],
                plan_as: &plan_as,
                layer_colour: &layer_colour,
                layer_index: &layer_index,
                backdrop: r.cgram[0],
                window_x: r.origin + x as i32,
                line,
                world: None,
            };
            let pixel = self.pixel(&probe, regs, &r.cgram, binder);
            self.pixels[(line - 1) * self.width + x] = pixel;
        }
    }

    /// What the art passes need of one resolved pixel.
    pub(crate) fn pixel(
        &mut self,
        probe: &Probe<'_>,
        regs: &LineRegs,
        cgram: &[u16; 256],
        binder: &Binder<'_>,
    ) -> ArtPixel {
        self.lookup.pixel(
            probe,
            regs,
            cgram,
            binder,
            &mut self.memo,
            &mut self.stats.portrait_pixels_without_art,
        )
    }

    /// The part of the state that captures the pixels of rows in parallel: the lookup, and the
    /// record of the frame to split by rows.
    pub(crate) fn capture_parts(&mut self) -> (&Lookup, &mut [ArtPixel]) {
        (&self.lookup, &mut self.pixels)
    }

    /// The background slots of the last picture, one row per slot: the index entry, its BG, the
    /// registers its layer was shown with (mode, map base, char base, size) and whether it is a
    /// field map's layer.
    pub(crate) fn bg_slot_report(&self) -> Vec<(u32, u8, u8, u16, u16, u8, bool)> {
        self.lookup
            .bg_slots
            .iter()
            .map(|s| (s.entry, s.bg, s.mode, s.map_base, s.char_base, s.size, s.field))
            .collect()
    }

    /// The box `w` x `h` at `(x, y)` of the last picture, one row per pixel: the layer on top (an
    /// object `O`, a BG's `U`nder/`L`ayer/`H`idden, or `B`), the picture it is part of (`None` when
    /// it is no picture's) and its CGRAM row. It says which part of a portrait is drawn from a
    /// picture and which is left as the game's own pixels.
    pub(crate) fn picture_map(
        &self,
        x: i32,
        y: i32,
        w: usize,
        h: usize,
    ) -> Vec<(u8, Option<u32>, u8)> {
        let width = self.width;
        let mut out = Vec::with_capacity(w * h);
        for j in 0..h {
            for i in 0..w {
                let (x, y) = (x + i as i32, y + j as i32);
                let Some(pixel) = (x >= 0 && y >= 0)
                    .then(|| (y as usize) * width + (x as usize))
                    .and_then(|at| self.pixels.get(at))
                else {
                    out.push((b'?', None, 0));
                    continue;
                };
                let kind = if pixel.oam != crate::NO_OBJECT {
                    b'O'
                } else {
                    match pixel.bg.kind {
                        bgart::Kind::Under => b'U',
                        bgart::Kind::Layer => b'L',
                        bgart::Kind::Hidden => b'H',
                        bgart::Kind::Object => b'B',
                    }
                };
                let picture = if pixel.picture == NO_PICTURE { None } else { Some(pixel.picture) };
                out.push((kind, picture, pixel.row));
            }
        }
        out
    }

    /// The poses of the frame `log` whose art the index holds, with the art's object index.
    pub(crate) fn find_poses(&mut self, log: &FrameLog) -> Vec<(Pose, u32)> {
        self.find_poses_in(log, &poses::View::window())
    }

    /// [`ArtState::find_poses`] for the sprites of `view`: the window alone, or a wide view of the
    /// field with its margins and the objects the original drops.
    pub(crate) fn find_poses_in(&mut self, log: &FrameLog, view: &poses::View) -> Vec<(Pose, u32)> {
        self.cgram.clear();
        self.cgram.extend_from_slice(&log.cgram_start);
        if self.pack.art.objects().is_empty() || self.backgrounds_only {
            return Vec::new();
        }
        let sprites = poses::sprites_in(log, view);
        let overscan = poses::overscan_sprites_in(log, view);
        if sprites.is_empty() && overscan.is_empty() {
            return Vec::new();
        }
        let mut found = poses::poses_in(log, &sprites, view);
        // The fighters the game draws with the other palettes (an enemy of a formation), when the
        // index has art for them, before the effects the same sprites make
        // (only in a frame of fighters: a field group of such a palette is the field's)
        let battle = self.field_frame != Some(true)
            && found.iter().any(|pose| pose.fighter && !self.candidates(pose).is_empty());
        // Outside a battle, a field group with no art is read again (see `poses::refine_field`)
        if !battle {
            found = poses::refine_field(found, &sprites, log, view, &mut |pose| {
                !self.candidates(pose).is_empty() || self.shaped(pose).is_some()
            });
            found = poses::join_band(found, &sprites, &overscan, log, view, &mut |pose| {
                !self.candidates(pose).is_empty() || self.shaped(pose).is_some()
            });
        }
        let extra: Vec<Pose> = if battle {
            poses::fighter_candidates(&sprites)
                .into_iter()
                .filter(|pose| !self.candidates(pose).is_empty())
                .collect()
        } else {
            Vec::new()
        };
        if !extra.is_empty() {
            let at = found.iter().position(|pose| !pose.fighter).unwrap_or(found.len());
            found.splice(at..at, extra);
        }
        // The fighters drawn as BG tiles (the end slots of a formation of five), when the index has
        // art for their pixels
        // The speed lines of a cutscene that reach under line 224 (a frame of 239 lines shows them):
        // the pieces under it are poses of their own, found only by the exact pixels the index has
        // art for (or all of them, when the tour looks for what has no art)
        if !battle {
            // (the pieces a figure of the field took with it are drawn by its art)
            let joined =
                found.iter().filter(|pose| pose.field).fold(0_u128, |all, pose| all | pose.sprites);
            for mut pose in poses::overscan_effects(log, &overscan) {
                if pose.sprites & !joined == 0 {
                    continue;
                }
                // (a cluster that also holds pieces a figure took in the band is placed on the
                // others: the figure's art draws those, the effect's the rest)
                pose.excluded = pose.sprites & joined;
                if self.tally_on || !self.candidates(&pose).is_empty() {
                    found.push(pose);
                }
            }
        }
        let bg_fighters = self.bg_fighters(log);
        if !bg_fighters.is_empty() {
            found.extend(bg_fighters);
        }
        // The visible pixels of each sprite, and those of them that belong to a picture (a portrait
        // drawn as sprites); every worker counts a stretch of the frame's records
        let stretch = self.pixels.len().div_ceil(rayon::current_num_threads().max(1)).max(8192);
        let (shown, pictured) = self
            .pixels
            .par_chunks(stretch)
            .fold(
                || ([0_u32; 128], [0_u32; 128]),
                |(mut shown, mut pictured), pixels| {
                    for pixel in pixels.iter().filter(|p| p.oam != crate::NO_OBJECT) {
                        let at = usize::from(pixel.oam) & 127;
                        shown[at] += 1;
                        pictured[at] += u32::from(pixel.picture != NO_PICTURE);
                    }
                    (shown, pictured)
                },
            )
            .reduce(
                || ([0_u32; 128], [0_u32; 128]),
                |mut a, b| {
                    for (x, y) in a.0.iter_mut().zip(b.0) {
                        *x += y;
                    }
                    for (x, y) in a.1.iter_mut().zip(b.1) {
                        *x += y;
                    }
                    a
                },
            );
        let sum_of = |counts: &[u32; 128], pose: &Pose| {
            (0..128).filter(|&at| pose.sprites >> at & 1 != 0).map(|at| counts[at]).sum::<u32>()
        };
        let seen = |pose: &Pose| sum_of(&shown, pose);
        // Sprites that show a picture are that picture's, whatever fighter's or effect's silhouette
        // they share: a pose is never made of them but by the exact pixels it has art for
        let is_picture = |pose: &Pose| {
            let (all, own) = (sum_of(&shown, pose), sum_of(&pictured, pose));
            own > 0 && own * 2 >= all
        };
        let mut taken = 0_u128;
        let mut stats = Vec::new();
        let mut all_masks: Vec<(u128, Kind)> = Vec::new();
        for pose in &found {
            all_masks.push((pose.sprites, Kind::of(pose)));
            stats.push(PoseStat {
                fighter: pose.fighter,
                field: pose.field,
                shadow: pose.shadow,
                sprites: pose.sprites & !pose.excluded,
                excluded: pose.excluded,
                picture: is_picture(pose),
                matched: false,
                recoloured: false,
                visible: seen(pose),
                opaque: pose.arr.iter().filter(|&&c| c != 0).count() as u32,
                stripy: stripy_permille(pose),
                key: pose.key,
                palette: pose.palette,
                source: ArtSource::None,
                art: None,
                org: pose.org,
                size: (pose.w, pose.h),
                look: if self.measure {
                    poses::region_digest(&sprites, pose.org, pose.w, pose.h)
                } else {
                    0
                },
            });
        }
        let rows: Vec<[[u8; 3]; 16]> = found
            .iter()
            .map(|pose| {
                if pose.bg {
                    Self::bg_palette_row(log, pose.palette)
                } else {
                    Self::object_palette_row(log, pose.palette)
                }
            })
            .collect();
        let free = |pose: &Pose, taken: u128| (pose.sprites & !pose.excluded) & taken == 0;
        let mut placed: Vec<Option<(Pose, u32)>> = vec![None; found.len()];
        let mut sources: Vec<ArtSource> = vec![ArtSource::None; found.len()];
        // First the poses whose pixels the index has a key for (the art made for the exact pose),
        // in the order of the frame; then the ground shadows; then the poses found by their shape
        // or as part of another pose (a fade-in, a cut by the screen's edge)
        // Outside a battle the poses with the most pixels come first (a field group before an
        // effect's cluster of as many): the whole figure takes its sprites before a piece of it
        // does (a cloud's 42 x 13 corner that an effect's art fits must not take them from the
        // group, nor the pieces a group is cut in from the whole figure that art was made for), so
        // a figure is drawn from the same art in every frame whatever way its sprites are grouped
        let mut order: Vec<usize> = (0..found.len()).collect();
        if !battle {
            let opaque = |n: usize| found[n].arr.iter().filter(|&&c| c != 0).count();
            order.sort_by_key(|&n| (std::cmp::Reverse(opaque(n)), !found[n].field));
        }
        for n in order {
            let pose = &found[n];
            if pose.shadow || !free(pose, taken) {
                continue;
            }
            let unit = self.unit_for(pose);
            let (best, mirrored) = self.exact_art(pose, &rows[n], unit.as_ref());
            if let Some(at) = best {
                taken |= pose.sprites & !pose.excluded;
                let placed_at = if mirrored { at | MIRROR } else { at };
                placed[n] = Some((self.placed(pose, at, &rows[n], false), placed_at));
                sources[n] = if mirrored { ArtSource::Mirror } else { ArtSource::Exact };
            }
        }
        for (n, pose) in found.iter().enumerate() {
            if pose.shadow && free(pose, taken) {
                taken |= pose.sprites;
                placed[n] = Some((pose.clone(), SHADOW));
            }
        }
        for (n, pose) in found.iter().enumerate() {
            if placed[n].is_some() || pose.shadow || !free(pose, taken) || is_picture(pose) {
                continue;
            }
            // A pose of the sprites of a colour variant (their tiles are the same, the key is
            // not: an effect's reads its colours) is the pose of the same shape
            let (found_pose, at, shaped) = match self.shaped(pose) {
                Some(at) => (pose.clone(), at, true),
                // Only a pose the capture would have left out is a part of another one: a fade-in's
                // stripes or a sprite the screen's edge cuts. A whole pose that has no art (a spike
                // of Porunga's shoulder) is missing, never drawn as some other fighter's piece
                None if (pose.fighter || pose.cut)
                    && stats[n].opaque >= partial::MIN_OPAQUE
                    && (pose.cut || stats[n].stripy >= STRIPY_PARTIAL) =>
                {
                    match self.holding(pose, &rows[n]) {
                        Some((held, at)) => (held, at, false),
                        None => continue,
                    }
                }
                None => continue,
            };
            taken |= pose.sprites;
            placed[n] = Some((self.placed(&found_pose, at, &rows[n], shaped), at));
            sources[n] = if shaped { ArtSource::Shaped } else { ArtSource::Held };
        }
        // Art that cannot be loaded (a missing or unreadable file) draws nothing, and its sprites
        // are not hidden: they stay as the game draws them
        let wanted: Vec<(Pose, u32)> =
            placed.iter().flatten().filter(|(_, at)| *at != SHADOW).cloned().collect();
        self.prefetch(&std::collections::BTreeSet::new(), &wanted);
        let mut matches: Vec<(Pose, u32)> = Vec::new();
        for (n, slot) in placed.into_iter().enumerate() {
            if let Some((pose, at)) = slot {
                if at != SHADOW && self.object(at).is_none() {
                    continue;
                }
                stats[n].matched = true;
                stats[n].recoloured = pose.recolour.is_some();
                if at != SHADOW {
                    stats[n].source = sources[n];
                    stats[n].art = Some(real(at) as u32);
                }
                matches.push((pose, at));
            }
        }
        // The art of the layers draws under a fighter drawn as BG tiles that the art draws
        for (pose, _) in matches.iter().filter(|(pose, at)| pose.bg && *at != SHADOW) {
            for n in (0..pose.arr.len()).filter(|&n| pose.arr[n] != 0) {
                let (fx, fy) =
                    (pose.org.0 + (n % pose.w) as i32, pose.org.1 + (n / pose.w) as i32 - 1);
                let at = fy as usize * self.width + fx as usize;
                if let Some(pixel) = self.pixels.get_mut(at)
                    && pixel.bg.kind == bgart::Kind::Under
                {
                    pixel.bg.kind = bgart::Kind::Layer;
                }
            }
        }
        if self.tally_on {
            for (n, pose) in found.iter().enumerate() {
                if !stats[n].matched {
                    let shape = PoseShape {
                        key: pose.key,
                        fighter: pose.fighter,
                        field: pose.field,
                        shadow: pose.shadow,
                        fate: Fate::Missing,
                        w: pose.w,
                        h: pose.h,
                        arr: pose.arr.clone(),
                        rgba: pose.rgba.clone(),
                        org: pose.org,
                        palette: pose.palette,
                        row: rows[n],
                    };
                    self.dumps.push((n, shape));
                }
            }
        }
        // Pixels by what became of the sprite that shows them: art beats missing beats partial
        let mut masks = [[0_u128; 5]; 3];
        let mut drawn_by_any = 0_u128;
        for (pose, _) in &matches {
            drawn_by_any |= pose.sprites;
        }
        for (stat, (mask, kind)) in stats.iter().zip(&all_masks) {
            let kind = *kind as usize;
            let class =
                if stat.matched || (kind == Kind::Field as usize && mask & !drawn_by_any == 0) {
                    0
                } else if stat.opaque >= 40 && stat.stripy < STRIPY_PARTIAL {
                    1
                } else {
                    2
                };
            masks[kind][class] |= mask;
            masks[kind][3] |= mask;
            if stat.recoloured {
                masks[kind][4] |= mask;
            }
        }
        let width = self.width.max(1);
        let cut_without_art = found
            .iter()
            .zip(&stats)
            .filter(|(pose, stat)| {
                pose.field && (pose.cut || pose.at_vertical_edge()) && !stat.matched
            })
            .fold(0_u128, |all, (pose, _)| all | pose.sprites);
        for (at, pixel) in self.pixels.iter().enumerate().filter(|(_, p)| p.oam != crate::NO_OBJECT)
        {
            let bit = 1_u128 << pixel.oam;
            let mut any = [0_u128; 5];
            for (kind, mask) in masks.iter().enumerate() {
                if mask[3] & bit == 0 {
                    continue;
                }
                let counts = match kind {
                    0 => &mut self.stats.fighters,
                    1 => &mut self.stats.effects,
                    _ => &mut self.stats.field,
                };
                count_pixel(counts, mask, bit, pixel);
                for (all, one) in any.iter_mut().zip(mask) {
                    *all |= one;
                }
            }
            // A sprite no pose holds (the capture left groups of fewer than 40 pixels out: what is
            // left of an object cut by the view's edge) is left out too
            if (cut_without_art & bit != 0 || any[3] & bit == 0) && pixel.picture == NO_PICTURE {
                self.stats.sprites_cut += 1;
                continue;
            }
            let region = if view.in_margin((at % width) as i32) {
                &mut self.stats.sprites_margin
            } else {
                &mut self.stats.sprites_window
            };
            count_pixel(region, &any, bit, pixel);
        }
        self.pose_sprites = all_masks.iter().map(|&(mask, _)| mask).collect();
        self.stats.poses = stats;
        matches
    }

    /// Draws the art of the frame over `hd`, the picture of `frame` (the 1x frame with its
    /// sprites): the pictures, then the poses `found`. `x_offset` is where the window starts in
    /// the frame, `clip` the part of it the poses may draw on (left, top, right, bottom).
    pub(crate) fn compose(
        &mut self,
        hd: &mut crate::HdFrame,
        frame: &crate::Frame,
        found: &[(Pose, u32)],
        x_offset: i32,
        clip: (i32, i32, i32, i32),
    ) {
        let scale = self.scale;
        let hidden = found.iter().fold(0_u128, |all, (pose, _)| all | pose.sprites);
        self.repair_portraits();
        let ground_shadows = bgshadow::find(frame, &mut self.pixels);
        self.draw_backgrounds(hd, hidden, x_offset);
        if self.backgrounds_only {
            return;
        }
        let mut images = FxHashMap::default();
        let mut recoloured = std::collections::BTreeSet::new();
        let wanted_rows: std::collections::BTreeSet<(u32, u8)> = self
            .pixels
            .iter()
            .filter(|p| p.picture != NO_PICTURE)
            .map(|p| (p.picture, p.row))
            .collect();
        let wanted: std::collections::BTreeSet<u32> =
            wanted_rows.iter().map(|&(at, _)| at).collect();
        self.prefetch(&wanted, found);
        for (at, row) in wanted_rows {
            if let Some(image) = self.picture(at) {
                let made = self.picture_variant(at, row, Arc::clone(&image));
                if !Arc::ptr_eq(&made, &image) {
                    recoloured.insert((at, row));
                }
                images.insert((at, row), made);
            }
        }
        self.stats.picture_pixels =
            draw_pictures(&mut hd.rgba, scale, self.width, &self.pixels, &images);
        self.stats.picture_recoloured = if recoloured.is_empty() {
            0
        } else {
            let drawn = |p: &&ArtPixel| p.picture != NO_PICTURE;
            self.pixels
                .iter()
                .filter(drawn)
                .filter(|p| recoloured.contains(&(p.picture, p.row)))
                .count() as u64
        };
        // The shadows go under the fighters
        let shadows: Vec<&Pose> =
            found.iter().filter(|(_, at)| *at == SHADOW).map(|(pose, _)| pose).collect();
        let others = found
            .iter()
            .filter(|(_, at)| *at != SHADOW)
            .fold(0_u128, |all, (pose, _)| all | pose.sprites);
        draw_shadows(&mut hd.rgba, scale, frame, &self.pixels, &shadows, others, x_offset, clip);
        bgshadow::draw(&mut hd.rgba, scale, frame, &self.pixels, &ground_shadows, others, clip);
        let mut placed: Vec<Placed<'_>> = Vec::new();
        // The poses a pose variant replaces: drawn by the variant, after the others
        let mut replaced: Vec<(&Pose, Arc<Image>, i32)> = Vec::new();
        for (pose, at) in found.iter().filter(|(_, at)| *at != SHADOW) {
            if let Some((image, margin)) = self.pose_variant(*at) {
                replaced.push((pose, image, margin));
                continue;
            }
            let luma = self.pack.art.objects()[real(*at)].luma;
            if let Some(image) = self.object(*at) {
                let image = match &pose.recolour {
                    Some(variant) => self.object_variant(pose, *at, image, variant),
                    None => image,
                };
                placed.push((pose, image, luma));
            }
        }
        let measure = self.measure;
        let drawn = draw_objects(
            &mut hd.rgba,
            scale,
            frame,
            &self.pixels,
            &placed,
            x_offset,
            clip,
            measure,
        );
        for (pose, image, margin) in &replaced {
            let x_offset = if pose.bg { 0 } else { x_offset };
            let origin = (pose.org.0 + x_offset - margin, pose.org.1 + ROW_OF_CAPTURE - margin);
            let sprites = pose.sprites & !pose.excluded;
            draw_variant(&mut hd.rgba, scale, frame, &self.pixels, sprites, image, origin, clip);
        }
        if measure {
            self.stats.drawn = placed
                .iter()
                .zip(drawn)
                .map(|(&(pose, _, luma), drawn)| DrawnPose {
                    key: pose.key,
                    fighter: pose.fighter,
                    field: pose.field,
                    luma,
                    drawn,
                })
                .collect();
        }
    }

    /// Draws the art of the background layers over `hd`, where the original showed a layer, the
    /// backdrop or an object that an HD pose hides (`hidden`), and counts the pixels of battle
    /// backgrounds and the ones drawn.
    fn draw_backgrounds(&mut self, hd: &mut crate::HdFrame, hidden: u128, x_offset: i32) {
        self.stats.bg_pixels = self.pixels.iter().filter(|p| p.bg.battle).count() as u64;
        let slots = self.lookup.bg_slots.clone();
        if slots.is_empty() || self.tally_on {
            return;
        }
        let images: Vec<Option<Arc<BgImage>>> = slots
            .iter()
            .enumerate()
            .map(|(j, slot)| {
                if slot.is_plane() {
                    None
                } else if slot.field {
                    self.field_image(slot, j)
                } else {
                    self.bg_image(slot.entry)
                }
            })
            .collect();
        if let Some(j) = slots.iter().position(Slot::is_plane)
            && let Some(plane) = self.plane.take()
        {
            if let Some(art) = self.plane_art(slots[j].entry) {
                draw::draw_plane(
                    &mut hd.rgba,
                    self.scale,
                    self.width,
                    &self.pixels,
                    &draw::PlaneDraw {
                        frame: &plane,
                        cgram: &self.cgram,
                        art: &art,
                        x_offset,
                        slot: j,
                        hidden,
                    },
                );
            }
            self.plane = Some(plane);
        }
        let fills: Vec<bool> = slots.iter().map(|slot| slot.fill).collect();
        let colourings = self.colourings(&images, hidden);
        self.stats.bg_art = draw::draw_backgrounds(
            &mut hd.rgba,
            self.scale,
            self.width,
            &self.pixels,
            &images,
            &colourings,
            &fills,
            hidden,
        );
    }

    /// How the colours of each slot's art are transferred to the frame's (see [`crate::transfer`]). A table transfer of a
    /// whole picture is applied to the picture's own colours, once each (see [`Colours`]), the picture numbered once
    /// (`numbered`); a fade, a tiled art and a picture with too many colours are applied to each colour as it is drawn.
    /// With `transfer_cache` off every table transfer is applied so: the reference `bench-hd --check` compares with.
    fn colourings(&mut self, images: &[Option<Arc<BgImage>>], hidden: u128) -> Vec<Colouring> {
        let mut colourings = Vec::with_capacity(images.len());
        for (slot, image) in images.iter().enumerate() {
            let Some(image) = image else {
                colourings.push(Colouring::Same);
                continue;
            };
            let pairs = draw::pairs_of(image, slot, &self.pixels, hidden);
            if pairs.is_identity() {
                colourings.push(Colouring::Same);
                continue;
            }
            let transfer = pairs.transfer();
            let numbered = match &transfer {
                Transfer::Table(_) if self.transfer_cache => self.numbered_colours(image),
                _ => None,
            };
            colourings.push(match numbered {
                Some(colours) => {
                    let transferred = colours.transferred(&transfer);
                    Colouring::Numbered { colours, transferred }
                }
                None => Colouring::Each(transfer),
            });
        }
        colourings
    }

    /// The colours of a picture of a layer's art, numbered (see [`Colours`]): `None` for a tiled art and for a picture
    /// with too many colours. The last pictures numbered are kept, so a picture is numbered once while it is drawn.
    fn numbered_colours(&mut self, image: &Arc<BgImage>) -> Option<Arc<Colours>> {
        if let Some((_, colours)) =
            self.numbered.iter().find(|(source, _)| Arc::ptr_eq(source, image))
        {
            return colours.clone();
        }
        let colours = match &image.art {
            BgArt::Whole(art) => Colours::of(art).map(Arc::new),
            BgArt::Tiled(_) => None,
        };
        if self.numbered.len() >= NUMBERED_PICTURES {
            self.numbered.remove(0);
        }
        self.numbered.push((Arc::clone(image), colours.clone()));
        colours
    }

    /// Numbers the layers' art by its colours (`on`, the default), or applies each table transfer to each colour as it
    /// is drawn (off: the reference that `bench-hd --check` compares with; the pictures are the same).
    pub(crate) fn set_transfer_cache(&mut self, on: bool) {
        self.transfer_cache = on;
        self.numbered.clear();
    }

    /// Picture `at` in the colours of CGRAM row `row` (see [`ArtPixel::row`]) when the picture was
    /// made for other colours: its art recoloured, or `image` itself.
    fn picture_variant(&mut self, at: u32, row: u8, image: Arc<Image>) -> Arc<Image> {
        let base = usize::from(row) * 16;
        let Some(words) = self.cgram.get(base..base + 16) else { return image };
        let colours: [[u8; 3]; 16] = std::array::from_fn(|n| poses::rgb555(words[n]));
        let palette = self.pack.art.pictures()[at as usize].palette;
        self.variant((true, at, 0), image, &palette, &colours)
    }

    /// Object `at` drawn for `pose` in the colours of `variant`: recoloured from the palette it
    /// was made for, or from the palette its art has when the pose says which index is which.
    fn object_variant(
        &mut self,
        pose: &Pose,
        at: u32,
        image: Arc<Image>,
        variant: &recolour::Variant,
    ) -> Arc<Image> {
        let object = &self.pack.art.objects()[real(at)];
        if !variant.derive {
            let palette = object.palette;
            return self.variant((false, at, 0), image, &palette, &variant.row);
        }
        if let Some(found) = self.variants.get(&(false, at, pose.key, variant.row)) {
            return Arc::clone(found);
        }
        let palette = recolour::derive_palette(&image, &pose.arr, pose.w, pose.h, self.scale);
        self.variant((false, at, pose.key), image, &palette, &variant.row)
    }

    /// The art `image` (of picture or object `key.1`, made for `palette`) in the colours `row`,
    /// kept while the pictures that need it are drawn.
    fn variant(
        &mut self,
        key: (bool, u32, u64),
        image: Arc<Image>,
        palette: &Palette,
        row: &[[u8; 3]; 16],
    ) -> Arc<Image> {
        let key = (key.0, key.1, key.2, *row);
        if let Some(found) = self.variants.get(&key) {
            return Arc::clone(found);
        }
        if !recolour::differs(palette, row) {
            return image;
        }
        let made = Arc::new(recolour::recolour(&image, palette, row));
        if self.variant_bytes > VARIANT_BYTES {
            self.variants.clear();
            self.variant_bytes = 0;
        }
        self.variant_bytes += made.rgba.len();
        self.variants.insert(key, Arc::clone(&made));
        made
    }

    /// Loads the art a frame needs and the cache lacks, all at once on the thread pool: the first
    /// sight of a pose costs the decoding of its file, and a frame can bring several.
    fn prefetch(&mut self, pictures: &std::collections::BTreeSet<u32>, found: &[(Pose, u32)]) {
        let new_pictures: Vec<u32> =
            pictures.iter().copied().filter(|at| !self.pictures.contains_key(at)).collect();
        let new_objects: std::collections::BTreeSet<u32> = found
            .iter()
            .map(|(_, at)| *at)
            .filter(|at| *at != SHADOW && !self.objects.contains_key(at))
            .collect();
        if new_pictures.len() + new_objects.len() < 2 {
            return;
        }
        let (loaded_pictures, loaded_objects): (Vec<_>, Vec<_>) = rayon::join(
            || new_pictures.par_iter().map(|&at| (at, self.load_picture(at))).collect(),
            || {
                new_objects
                    .iter()
                    .copied()
                    .collect::<Vec<u32>>()
                    .par_iter()
                    .map(|&at| (at, self.load_object(at)))
                    .collect()
            },
        );
        for (at, image) in loaded_pictures {
            self.pictures.insert(at, image);
        }
        for (at, image) in loaded_objects {
            self.remember_object(at, image);
        }
    }

    fn load_picture(&self, at: u32) -> Option<Arc<Image>> {
        let side = 32 * self.scale;
        self.load(&self.pack.art.pictures()[at as usize].art, side, side)
    }

    fn load_object(&self, at: u32) -> Option<Arc<Image>> {
        let object = &self.pack.art.objects()[real(at)];
        let (w, h) =
            (usize::from(object.width) * self.scale, usize::from(object.height) * self.scale);
        let image = self.load(&object.art, w, h)?;
        if at & MIRROR == 0 {
            return Some(image);
        }
        Some(Arc::new(mirror(&image)))
    }

    /// The picture of a pose variant of `object` at the scale, mirrored when the pose faces the other way (as the object's own
    /// picture is). The file must be the object's picture grown by the margin, four art pixels to a game pixel: a file that is
    /// missing or of another size is not drawn, and the log says why.
    fn load_variant(
        &self,
        variant: &PoseVariant,
        object: &crate::art_index::Object,
        mirrored: bool,
    ) -> Option<Arc<Image>> {
        let margin = variant.margin as usize;
        let (w, h) =
            (usize::from(object.width) + 2 * margin, usize::from(object.height) + 2 * margin);
        let refused = |reason: String| {
            log::warn!("the pose variant {} of {} is not drawn: {reason}", variant.file, object.id);
        };
        let bytes = match std::fs::read(self.pack.dir.join(&variant.file)) {
            Ok(bytes) => bytes,
            Err(error) => {
                refused(error.to_string());
                return None;
            }
        };
        let (pw, ph, rgba) = match crate::png::read_rgba(&bytes) {
            Ok(picture) => picture,
            Err(error) => {
                refused(error);
                return None;
            }
        };
        if (pw as usize, ph as usize) != (w * 4, h * 4) {
            refused(format!("it is {pw} x {ph} pixels, not {} x {}", w * 4, h * 4));
            return None;
        }
        let picture = Image::new(pw as usize, ph as usize, rgba);
        let image = resample::resize(&picture, w * self.scale, h * self.scale);
        Some(Arc::new(if mirrored { mirror(&image) } else { image }))
    }

    /// The picture of the pose variant the pack gives the object `at` while the conditions of the frame hold, and its margin
    /// in game pixels: of the object's variants that hold, one for a specific condition before an `always` one
    /// ([`ssd_mod::pose::drawn`]). A data pack's variants ([`Pack::variants`]) come before the pack's own bigger art
    /// ([`Pack::pose_art`]): the own art is drawn only where no data pack's variant of the object holds, or its picture is not
    /// drawn (see [`ArtState::load_variant`]). `None` when no variant holds for the object, or no picture is drawn: the object
    /// is then drawn as it is.
    fn pose_variant(&mut self, at: u32) -> Option<(Arc<Image>, i32)> {
        let pack = Arc::clone(&self.pack);
        let object = &pack.art.objects()[real(at)];
        let mirrored = at & MIRROR != 0;
        // The variants are numbered: the data packs' first, then the pack's own
        let own = pack.variants.len();
        for (variants, first) in [(&pack.variants, 0), (&pack.pose_art, own)] {
            let holding: Vec<usize> = (0..variants.len())
                .filter(|&index| {
                    let variant = &variants[index];
                    variant.art == object.id && self.conditions.holds(variant.when)
                })
                .collect();
            let Some(chosen) =
                ssd_mod::pose::drawn(holding.iter().map(|&index| variants[index].when))
            else {
                continue;
            };
            let index = holding[chosen];
            let variant = &variants[index];
            let margin = variant.margin as i32;
            let loaded = match self.pose_images.get(&(first + index, mirrored)) {
                Some(loaded) => loaded.clone(),
                None => {
                    let loaded = self.load_variant(variant, object, mirrored);
                    self.pose_images.insert((first + index, mirrored), loaded.clone());
                    loaded
                }
            };
            if let Some(image) = loaded {
                return Some((image, margin));
            }
        }
        None
    }

    /// Keeps `image` as the art of object `at`, the least recently used one making room.
    fn remember_object(&mut self, at: u32, image: Option<Arc<Image>>) {
        if self.recent.len() >= OBJECT_CACHE
            && let Some(oldest) = self.recent.pop_front()
        {
            self.objects.remove(&oldest);
        }
        self.recent.push_back(at);
        self.objects.insert(at, image);
    }

    /// The pose of the index that holds the partial fighter `pose` (see [`partial::containing`]),
    /// as the pose to draw: the holding pose's silhouette placed where the partial one's pixels
    /// fall on it, with the art's object index. The answer is kept by the partial pose's key.
    fn holding(&mut self, pose: &Pose, row: &[[u8; 3]; 16]) -> Option<(Pose, u32)> {
        let found = match self.partials.get(&pose.key) {
            Some(found) => *found,
            None => {
                let found = partial::containing(&self.pack.art, row, pose);
                if self.partials.len() >= PARTIAL_CACHE {
                    self.partials.clear();
                }
                self.partials.insert(pose.key, found);
                found
            }
        };
        let (at, dx, dy) = found?;
        let object = &self.pack.art.objects()[at as usize];
        // A sprite the screen's edge cuts shows a good part of its pose: a few pixels that fit
        // into some big art (a speed line's end) are not a piece of it
        if stripy_permille(pose) < STRIPY_PARTIAL {
            let shown = pose.arr.iter().filter(|&&c| c != 0).count() as u32;
            if shown * 100 < object.opaque() * CUT_MIN_SHARE {
                return None;
            }
        }
        let (w, h) = (usize::from(object.width), usize::from(object.height));
        let arr = (0..h * w).map(|n| u8::from(object.opaque_at(n % w, n / w))).collect();
        Some((Pose { arr, w, h, org: (pose.org.0 - dx, pose.org.1 - dy), ..pose.clone() }, at))
    }

    /// `pose` as drawn from object `at`: with the colours of `row` when the art was made for
    /// another palette (see [`ArtState::variant_row`]).
    fn placed(&self, pose: &Pose, at: u32, row: &[[u8; 3]; 16], shaped: bool) -> Pose {
        let mut placed = pose.clone();
        placed.recolour = self.variant_row(at, row, shaped);
        placed
    }

    /// The colours to draw object `at` in when it was made for a palette other than `row`.
    /// `shaped`: the object was found by its shape, not its key, so its colours are not known to
    /// differ and its palette may be unknown (it is then read from the pose).
    fn variant_row(&self, at: u32, row: &[[u8; 3]; 16], shaped: bool) -> Option<recolour::Variant> {
        let object = &self.pack.art.objects()[at as usize];
        if object.luma {
            return None;
        }
        let derive = object.palette.mask == 0;
        let variant = recolour::Variant { row: *row, derive };
        if derive {
            shaped.then_some(variant)
        } else {
            recolour::differs(&object.palette, row).then_some(variant)
        }
    }

    /// The object of the index with the silhouette of `pose` (the same box and the same opaque
    /// pixels), when the pose is one a palette variant could be: big enough, of several
    /// colours, and not a plain block. The answer is kept by the pose's key.
    fn shaped(&mut self, pose: &Pose) -> Option<u32> {
        let memo = pose.key ^ if pose.field { 0x5EED_F1E1_D000_0000 } else { 0 };
        if let Some(found) = self.shapes.get(&memo) {
            return *found;
        }
        let opaque = pose.arr.iter().filter(|&&c| c != 0).count();
        let mut seen = 0_u16;
        for &c in pose.arr.iter().filter(|&&c| c != 0) {
            seen |= 1 << (c & 15);
        }
        let found = if opaque >= SHAPE_MIN_OPAQUE
            && opaque < pose.arr.len()
            && seen.count_ones() >= 3
            && u16::try_from(pose.w).is_ok()
            && u16::try_from(pose.h).is_ok()
        {
            let art = &self.pack.art;
            art.objects_shaped(pose.w as u16, pose.h as u16, opaque as u32).iter().copied().find(
                |&at| {
                    let object = &art.objects()[at as usize];
                    object.field() == pose.field
                        && (0..pose.w * pose.h)
                            .all(|n| object.opaque_at(n % pose.w, n / pose.w) == (pose.arr[n] != 0))
                },
            )
        } else {
            None
        };
        if self.shapes.len() >= PARTIAL_CACHE {
            self.shapes.clear();
        }
        self.shapes.insert(memo, found);
        found
    }

    /// The art made for exactly the pixels of `pose`, or for them mirrored left to right (drawn
    /// mirrored: the second answer), among the objects of the pose's kind: the art of `unit` when
    /// one is given, in the palette closest to `row`; the art of the pose's own facing before the
    /// art of the other one.
    fn exact_art(
        &self,
        pose: &Pose,
        row: &[[u8; 3]; 16],
        unit: Option<&String>,
    ) -> (Option<u32>, bool) {
        let best = self
            .candidates(pose)
            .into_iter()
            .filter(|&(at, _)| unit.is_none_or(|unit| self.unit_of(at) == *unit))
            .min_by_key(|&(at, mirrored)| {
                let palette = &self.pack.art.objects()[at as usize].palette;
                (u8::from(mirrored), palette.distance(row), u8::from(palette.mask == 0))
            });
        match best {
            Some((at, mirrored)) => (Some(at), mirrored),
            None => (None, false),
        }
    }

    /// The objects of the pose's kind made for its pixels, and for them mirrored (`true`).
    fn candidates(&self, pose: &Pose) -> Vec<(u32, bool)> {
        let index = &self.pack.art;
        let of_kind = |&&at: &&u32| index.objects()[at as usize].field() == pose.field;
        let mut found: Vec<(u32, bool)> =
            index.objects_with(pose.key).iter().filter(of_kind).map(|&at| (at, false)).collect();
        if let Some(mirror) = pose.mirror_key().filter(|&mirror| mirror != pose.key) {
            found.extend(index.objects_with(mirror).iter().filter(of_kind).map(|&at| (at, true)));
        }
        found
    }

    /// The unit an object's art is of: its id without the number of the pose (`battle-saibaman-004`
    /// is of `battle-saibaman`).
    fn unit_of(&self, at: u32) -> String {
        let id = &self.pack.art.objects()[at as usize].id;
        match id.rsplit_once('-') {
            Some((unit, number)) if number.bytes().all(|b| b.is_ascii_digit()) => unit.to_owned(),
            _ => id.clone(),
        }
    }

    /// The unit whose art draws a pose whose pixels (or their mirror image) more than one unit has
    /// art for. The units of one set of pixels are the same sprite in other colours (the three
    /// Saibamen of the game) and each was drawn on its own, so they differ in the details: the
    /// fighters of a formation are drawn from the unit with the most poses in the index, in the
    /// colours of their own palettes, whatever their colours, and so are the frames of an
    /// animation. While Super Saiyan Goku is in the party, a pose that both his normal and his Super
    /// Saiyan art have is drawn from the Super Saiyan art: the normal art has the same pixels, in the
    /// normal palette, where his hair is black; without him, from the normal art, however many poses
    /// the Super Saiyan set has. `None` when one unit (or none) has art for the pose.
    fn unit_for(&self, pose: &Pose) -> Option<String> {
        let mut units: Vec<String> =
            self.candidates(pose).into_iter().map(|(at, _)| self.unit_of(at)).collect();
        units.sort();
        units.dedup();
        if units.len() < 2 {
            return None;
        }
        let super_saiyan = |unit: &String| unit.contains(SUPER_SAIYAN_GOKU_MARK);
        let normal = |unit: &String| unit.contains(GOKU_MARK) && !super_saiyan(unit);
        if units.iter().any(super_saiyan) && units.iter().any(normal) {
            // Any set of Goku's art (the battle poses, the close-ups of the special attacks) that has a Super Saiyan set for
            // these pixels is drawn from it while he is in the party, and from the normal set otherwise.
            if self.conditions.super_saiyan_goku {
                return units.into_iter().find(super_saiyan);
            }
            units.retain(|unit| !super_saiyan(unit));
        }
        let sizes = self.unit_sizes();
        units.into_iter().max_by_key(|unit| {
            (sizes.get(unit).copied().unwrap_or(0), std::cmp::Reverse(unit.clone()))
        })
    }

    /// The number of poses of each unit of the index.
    fn unit_sizes(&self) -> &rustc_hash::FxHashMap<String, u32> {
        self.unit_sizes.get_or_init(|| {
            let mut sizes: rustc_hash::FxHashMap<String, u32> = rustc_hash::FxHashMap::default();
            for at in 0..self.pack.art.objects().len() {
                *sizes.entry(self.unit_of(at as u32)).or_default() += 1;
            }
            sizes
        })
    }

    fn bg_palette_row(log: &FrameLog, palette: u8) -> [[u8; 3]; 16] {
        let mut row = [[0_u8; 3]; 16];
        for (index, color) in row.iter_mut().enumerate() {
            *color = poses::rgb555(log.cgram_start[usize::from(palette & 7) * 16 + index]);
        }
        row
    }

    /// The fighters the frame draws as four-bit BG tiles in front of the art's layers (BG1: the end
    /// slots of a formation of five enemies are drawn that way): the clusters of such pixels (at
    /// most 2 pixels apart) read as poses by their palette indexes, kept when the art index has
    /// art for their pixels or their mirror image. Their `org` is in frame coordinates.
    fn bg_fighters(&self, log: &FrameLog) -> Vec<Pose> {
        let (width, height) = (self.width, self.pixels.len() / self.width.max(1));
        let candidate = |at: usize| {
            let p = &self.pixels[at];
            p.bg4 && p.oam == crate::NO_OBJECT && p.bg.kind == bgart::Kind::Under
        };
        let points: Vec<usize> = (0..self.pixels.len()).filter(|&at| candidate(at)).collect();
        if points.len() < 40 {
            return Vec::new();
        }
        let mut label: FxHashMap<usize, usize> = FxHashMap::default();
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for &start in &points {
            if label.contains_key(&start) {
                continue;
            }
            let id = groups.len();
            let mut stack = vec![start];
            label.insert(start, id);
            let mut members = Vec::new();
            while let Some(at) = stack.pop() {
                members.push(at);
                let (x, y) = ((at % width) as i32, (at / width) as i32);
                for dy in -2_i32..=2 {
                    for dx in -2_i32..=2 {
                        let (nx, ny) = (x + dx, y + dy);
                        if nx < 0 || ny < 0 || nx >= width as i32 || ny >= height as i32 {
                            continue;
                        }
                        let near = ny as usize * width + nx as usize;
                        if candidate(near) && !label.contains_key(&near) {
                            label.insert(near, id);
                            stack.push(near);
                        }
                    }
                }
            }
            groups.push(members);
        }
        let mut out = Vec::new();
        for members in groups {
            if members.len() < 40 {
                continue;
            }
            let x0 = members.iter().map(|at| at % width).min().unwrap_or(0);
            let x1 = members.iter().map(|at| at % width).max().unwrap_or(0) + 1;
            let y0 = members.iter().map(|at| at / width).min().unwrap_or(0);
            let y1 = members.iter().map(|at| at / width).max().unwrap_or(0) + 1;
            let (w, h) = (x1 - x0, y1 - y0);
            let mut arr = vec![0_u8; w * h];
            for &at in &members {
                let p = &self.pixels[at];
                let (col, row) = (usize::from(p.bg_pos & 7), usize::from(p.bg_pos >> 3 & 7));
                let base = usize::from(p.bg_char);
                let Some(data) = log.vram.get(base..base + 16) else { continue };
                let bit = 7 - col;
                let mut color = 0_u8;
                for plane in 0..2_usize {
                    let word = data[row + 8 * plane];
                    color |= u8::from((word >> bit) & 1 != 0) << (2 * plane);
                    color |= u8::from((word >> (bit + 8)) & 1 != 0) << (2 * plane + 1);
                }
                arr[(at / width - y0) * w + (at % width - x0)] = color;
            }
            let palette = self.pixels[members[0]].bg_palette;
            let pose = poses::bg_pose(arr, w, h, (x0 as i32, y0 as i32 + 1), palette);
            if !self.candidates(&pose).is_empty() {
                out.push(pose);
            }
        }
        out
    }

    fn object_palette_row(log: &FrameLog, palette: u8) -> [[u8; 3]; 16] {
        let mut row = [[0_u8; 3]; 16];
        for (index, color) in row.iter_mut().enumerate() {
            *color = poses::rgb555(log.cgram_start[128 + usize::from(palette) * 16 + index]);
        }
        row
    }
}

/// The record of the pixels of some rows of a frame, filled by one thread: the rows of
/// [`ArtState::pixels`] that a segment of lines draws.
pub(crate) struct Rows<'a> {
    lookup: &'a Lookup,
    pixels: &'a mut [ArtPixel],
    width: usize,
    /// The 1-based line of the first row of `pixels`.
    first_line: usize,
    memo: Memo,
    /// Characters of portraits that had no picture.
    pub(crate) portrait: u64,
}

impl<'a> Rows<'a> {
    /// The background layers of the frame (see [`ArtState::bg_slots`]).
    pub(crate) fn bg_slots(&self) -> &[Slot] {
        &self.lookup.bg_slots
    }

    pub(crate) fn new(
        lookup: &'a Lookup,
        pixels: &'a mut [ArtPixel],
        width: usize,
        first_line: usize,
    ) -> Self {
        Self { lookup, pixels, width, first_line, memo: None, portrait: 0 }
    }

    /// Records pixel `x` of line `line` (1-based), resolved as `rendered`.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn put(
        &mut self,
        line: usize,
        x: usize,
        probe: &Probe<'_>,
        regs: &LineRegs,
        cgram: &[u16; 256],
        binder: &Binder<'_>,
    ) {
        let pixel =
            self.lookup.pixel(probe, regs, cgram, binder, &mut self.memo, &mut self.portrait);
        self.pixels[(line - self.first_line) * self.width + x] = pixel;
    }
}

/// The memo of [`Lookup::pixel`]: the last character asked about, and the answer.
pub(crate) type Memo = Option<((bool, u16, u8, u32), Choice)>;

/// What the capture of pixels reads of the pack: which pictures cover which characters.
pub(crate) struct Lookup {
    pack: Arc<Pack>,
    /// Whether the pixels record what they show for the tally.
    track: bool,
    /// For each asset of the pack's index, the pictures that cover it.
    asset_pictures: Vec<Vec<u32>>,
    /// The assets that are portraits or faces (they hold pictures somewhere).
    portrait_asset: Vec<bool>,
    /// The pack's background layers by class, and the ones the frame being captured shows.
    bg_classes: bgart::Classes,
    bg_slots: Vec<Slot>,
}

impl Lookup {
    /// What the art passes need of one resolved pixel. `portrait` counts the characters of
    /// portraits that have no picture.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn pixel(
        &self,
        probe: &Probe<'_>,
        regs: &LineRegs,
        cgram: &[u16; 256],
        binder: &Binder<'_>,
        memo: &mut Memo,
        portrait: &mut u64,
    ) -> ArtPixel {
        let rendered = probe.rendered;
        let object = rendered.layer == Layer::Obj;
        let mut pixel = ArtPixel {
            oam: if object { rendered.oam } else { crate::NO_OBJECT },
            prio: rendered.priority,
            mode_01: regs.bg_mode < 2,
            brightness: regs.brightness,
            ..ArtPixel::NONE
        };
        if regs.bg_mode < 2 || regs.bg_mode == 7 {
            pixel.bg = bgart::record(&self.bg_slots, probe, regs);
        }
        if object {
            pixel.plan = probe.plan();
        }
        let bg = match rendered.layer {
            Layer::Bg1 => Some(0),
            Layer::Bg2 => Some(1),
            Layer::Bg3 => Some(2),
            Layer::Bg4 => Some(3),
            _ => None,
        };
        let four_bit = match bg {
            Some(bg) => regs.bg_mode == 1 && bg < 2,
            None => object,
        };
        if four_bit && bg.is_some() {
            pixel.bg4 = true;
            pixel.bg_char = rendered.char;
            pixel.bg_pos = rendered.pos;
            pixel.bg_palette = rendered.palette;
        }
        if self.track {
            pixel.shown = match rendered.layer {
                Layer::Backdrop => Shown::Nothing,
                Layer::Obj => Shown::Obj,
                _ if regs.bg_mode == 7 => Shown::Plane,
                _ => Shown::Bg,
            };
            pixel.char = rendered.char;
            if matches!(pixel.shown, Shown::Bg | Shown::Obj) {
                pixel.bpp = if four_bit { 4 } else { 2 };
                if let Some(tile) = binder.tile(pixel.bpp, rendered.char) {
                    pixel.asset = tile.asset;
                    pixel.tile = tile.tile;
                }
            }
        }
        if !four_bit || self.pack.art.pictures().is_empty() {
            return pixel;
        }
        // The eight pixels of a character in a row share the answer, unless the palette decides
        let key = (object, rendered.char, rendered.palette, binder.epoch());
        let choice = match *memo {
            Some((last, choice)) if last == key => choice,
            _ => {
                let (choice, from_palette) =
                    self.choose(binder, cgram, object, rendered.char, rendered.palette);
                *memo = (!from_palette).then_some((key, choice));
                choice
            }
        };
        match choice {
            Choice::None => {}
            Choice::Portrait => *portrait += 1,
            Choice::Picture(at, cell) => {
                pixel.picture = at;
                pixel.row = rendered.palette + if object { 8 } else { 0 };
                pixel.cell = cell;
                pixel.pos = rendered.pos;
                if !object {
                    pixel.plan = probe.plan();
                }
            }
        }
        pixel
    }

    /// The picture a four-bit character shows a cell of, if the pack has one; and whether the
    /// character's palette decided between several.
    fn choose(
        &self,
        binder: &Binder<'_>,
        cgram: &[u16; 256],
        object: bool,
        char: u16,
        palette: u8,
    ) -> (Choice, bool) {
        let Some(tile) = binder.tile(4, char) else { return (Choice::None, false) };
        let candidates = &self.asset_pictures[tile.asset as usize];
        if candidates.is_empty() {
            let portrait = self.portrait_asset[tile.asset as usize];
            return (if portrait { Choice::Portrait } else { Choice::None }, false);
        }
        let mut covering: Vec<(u32, u8)> = Vec::new();
        for &at in candidates {
            let tiles = &self.pack.art.pictures()[at as usize].tiles;
            if let Some(cell) = tiles.iter().position(|&t| u32::from(t) == tile.tile) {
                covering.push((at, cell as u8));
            }
        }
        let (choice, from_palette) = match covering.as_slice() {
            [] => (None, false),
            [only] => (Some(*only), false),
            many => {
                let base = if object { 0x80 } else { 0 } + usize::from(palette) * 16;
                let mut row = [[0_u8; 3]; 16];
                for (index, color) in row.iter_mut().enumerate() {
                    *color = poses::rgb555(cgram[base + index]);
                }
                let best = many
                    .iter()
                    .min_by_key(|&&(at, _)| {
                        let palette = &self.pack.art.pictures()[at as usize].palette;
                        (palette.distance(&row), u8::from(palette.mask == 0))
                    })
                    .copied();
                (best, true)
            }
        };
        match choice {
            Some((at, cell)) => (Choice::Picture(at, cell), from_palette),
            None if self.portrait_asset[tile.asset as usize] => (Choice::Portrait, from_palette),
            None => (Choice::None, from_palette),
        }
    }
}

/// Counts a pixel that a sprite of a pose of the kind shows, by what became of the pose: `mask`
/// holds the sprites (a bit for each OAM entry) of the poses with art (0), the missing (1) and the
/// partial ones (2), all (3) and the recoloured (4).
fn count_pixel(counts: &mut KindPixels, mask: &[u128; 5], bit: u128, pixel: &ArtPixel) {
    counts.total += 1;
    if mask[0] & bit != 0 {
        counts.art += 1;
        counts.recoloured += u64::from(mask[4] & bit != 0);
    } else if pixel.picture != NO_PICTURE {
        // A face or portrait drawn as sprites: the picture's art covers them
        counts.art += 1;
    } else if mask[1] & bit != 0 {
        counts.missing += 1;
    } else {
        counts.partial += 1;
    }
}

/// The frame rows the picture under the matched poses is needed for: their boxes and the margin
/// the art grows into, as `(first, past the last)`; `None` when there is no pose.
pub(crate) fn pose_rows(found: &[(Pose, u32)], height: usize) -> Option<(usize, usize)> {
    let mut rows: Option<(i32, i32)> = None;
    for (pose, _) in found {
        let first = pose.org.1 + poses::ROW_OF_CAPTURE;
        let span = (first - draw::GROWTH - 1, first + pose.h as i32 + draw::GROWTH + 1);
        rows = Some(rows.map_or(span, |(a, b)| (a.min(span.0), b.max(span.1))));
    }
    rows.map(|(a, b)| (a.clamp(0, height as i32) as usize, b.clamp(0, height as i32) as usize))
}

/// The share of a pose's opaque pixels with no opaque pixel to their left or right, per mille.
fn stripy_permille(pose: &Pose) -> u32 {
    let mut total = 0_u32;
    let mut alone = 0_u32;
    for y in 0..pose.h {
        for x in 0..pose.w {
            if pose.arr[y * pose.w + x] == 0 {
                continue;
            }
            total += 1;
            let left = x > 0 && pose.arr[y * pose.w + x - 1] != 0;
            let right = x + 1 < pose.w && pose.arr[y * pose.w + x + 1] != 0;
            if !left && !right {
                alone += 1;
            }
        }
    }
    (alone * 1000).checked_div(total).unwrap_or(0)
}

/// The resolver priority of an object with OAM priority bits `bits` (lower is in front).
pub(crate) fn object_priority(mode_01: bool, bits: u8) -> u8 {
    match (mode_01, bits) {
        (_, 3) => 0,
        (true, 2) => 3,
        (true, 1) => 6,
        (true, _) => 9,
        (false, 2) => 2,
        (false, 1) => 4,
        (false, _) => 6,
    }
}

pub use draw::{COLOUR_CLASSES, Drawn, colour_class};
pub(crate) use draw::{Placed, draw_objects, draw_pictures, draw_shadows, draw_variant};
