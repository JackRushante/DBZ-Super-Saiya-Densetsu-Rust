//! Which art of an index draws a pose: the lookup the renderer does for each pose of a frame, as
//! functions of the index and the pose's pixels, so that a tool can ask the same question of a
//! pose it has not seen on screen (the art catalogue of `xtask hd-catalogue`).
//!
//! The renderer's steps are, in order: the art made for the pose's exact pixels, or for them
//! mirrored ([`Resolver::candidates`]), narrowed to one unit when several units have art for the
//! pixels ([`Resolver::unit_for`]) and to the palette nearest to the frame's
//! ([`Resolver::exact_art`]); then the art of the same silhouette in other colours
//! ([`Resolver::shaped`]).

use super::recolour;
use super::resample::Image;
use crate::art_index::{ArtIndex, Object};
use rustc_hash::FxHashMap;
use sha1_smol::Sha1;
use std::sync::OnceLock;

/// The fewest opaque pixels of a pose found by its shape.
const SHAPE_MIN_OPAQUE: usize = 150;

/// The mark of a unit of Goku's art in the index (`battle-goku-NNN`, `gallery-party-goku-u00-NNN`, see [`unit_of_id`]).
const GOKU_MARK: &str = "goku";
/// The mark of the units of Vegeta's art in the index (`battle-vegeta-NNN`, `gallery-enemy-vegeta-u15-NNN`).
const VEGETA_MARK: &str = "vegeta";
/// The marks of the units of Super Saiyan Vegeta's art: the battle poses (`battle-super-vegeta-NNN`) and the close-ups
/// of the special attacks, of the game's unit 75 (`gallery-enemy-vegeta-u75-NNN`, `gallery-party-vegeta-u75-NNN`).
const SUPER_VEGETA_MARKS: [&str; 2] = ["super-vegeta", "vegeta-u75"];
/// The mark of a unit of Super Saiyan Goku's art (`battle-goku-ssj-NNN`, `gallery-party-goku-ssj-NNN`).
const SUPER_SAIYAN_GOKU_MARK: &str = "goku-ssj";

/// The key of a pose's pixels: the first eight bytes of the SHA-1 of the trimmed palette-index
/// array (row by row) followed by the bytes `height`, `width`, as a number.
#[must_use]
pub fn pose_key(arr: &[u8], w: usize, h: usize) -> u64 {
    digest(arr, [h as u8, w as u8])
}

/// The key of the pose of `arr` (`w` x `h`) facing the other way: the key of its rows read right to left.
#[must_use]
pub fn mirror_pose_key(arr: &[u8], w: usize, h: usize) -> u64 {
    let mut flipped = Vec::with_capacity(arr.len());
    if w > 0 {
        for row in arr.chunks(w) {
            flipped.extend(row.iter().rev());
        }
    }
    digest(&flipped, [h as u8, w as u8])
}

pub(crate) fn digest(data: &[u8], tail: [u8; 2]) -> u64 {
    let mut hash = Sha1::new();
    hash.update(data);
    hash.update(&tail);
    u64::from_be_bytes(hash.digest().bytes()[..8].try_into().expect("eight bytes"))
}

/// The 8-bit colour of a 15-bit one (`0BBBBBGGGGGRRRRR`), as the renderer reads a palette.
#[must_use]
pub fn rgb555(value: u16) -> [u8; 3] {
    super::poses::rgb555(value)
}

/// What the index is asked about a pose: its key, the key of its mirror image (none for an
/// effect), and whether it is a sprite group of the field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoseRef {
    pub key: u64,
    pub mirror: Option<u64>,
    pub field: bool,
}

impl PoseRef {
    /// The pose of a fighter: its trimmed palette-index array.
    #[must_use]
    pub fn fighter(arr: &[u8], w: usize, h: usize) -> Self {
        Self { key: pose_key(arr, w, h), mirror: Some(mirror_pose_key(arr, w, h)), field: false }
    }
}

/// The lookup of the art of a pose in an index.
pub struct Resolver<'a> {
    index: &'a ArtIndex,
    /// Super Saiyan Goku is in the party (see [`Resolver::unit_for`]).
    super_saiyan_goku: bool,
    unit_sizes: &'a OnceLock<FxHashMap<String, u32>>,
}

impl<'a> Resolver<'a> {
    /// A resolver over `index`; `unit_sizes` keeps the table of the units' sizes between lookups.
    #[must_use]
    pub fn new(
        index: &'a ArtIndex,
        super_saiyan_goku: bool,
        unit_sizes: &'a OnceLock<FxHashMap<String, u32>>,
    ) -> Self {
        Self { index, super_saiyan_goku, unit_sizes }
    }

    /// The objects of the pose's kind made for its pixels, and for them mirrored (`true`).
    #[must_use]
    pub fn candidates(&self, pose: &PoseRef) -> Vec<(u32, bool)> {
        let index = self.index;
        let of_kind = |&&at: &&u32| index.objects()[at as usize].field() == pose.field;
        let mut found: Vec<(u32, bool)> =
            index.objects_with(pose.key).iter().filter(of_kind).map(|&at| (at, false)).collect();
        if let Some(mirror) = pose.mirror.filter(|&mirror| mirror != pose.key) {
            found.extend(index.objects_with(mirror).iter().filter(of_kind).map(|&at| (at, true)));
        }
        found
    }

    /// The unit an object's art is of: its id without the number of the pose (`battle-saibaman-004`
    /// is of `battle-saibaman`).
    #[must_use]
    pub fn unit_of(&self, at: u32) -> String {
        unit_of_id(&self.index.objects()[at as usize].id)
    }

    /// The art made for exactly the pixels of `pose`, or for them mirrored left to right (drawn
    /// mirrored: the second answer), among the objects of the pose's kind: the art of `unit` when
    /// one is given, in the palette closest to `row`; the art of the pose's own facing before the
    /// art of the other one.
    #[must_use]
    pub fn exact_art(
        &self,
        pose: &PoseRef,
        row: &[[u8; 3]; 16],
        unit: Option<&String>,
    ) -> (Option<u32>, bool) {
        let best = self
            .candidates(pose)
            .into_iter()
            .filter(|&(at, _)| unit.is_none_or(|unit| self.unit_of(at) == *unit))
            .min_by_key(|&(at, mirrored)| {
                let palette = &self.index.objects()[at as usize].palette;
                (u8::from(mirrored), palette.distance(row), u8::from(palette.mask == 0))
            });
        match best {
            Some((at, mirrored)) => (Some(at), mirrored),
            None => (None, false),
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
    /// the Super Saiyan set has. Super Saiyan Vegeta is not in the party record: his units (see
    /// `SUPER_VEGETA_MARKS`) are told from the normal Vegeta's by the colours, the pose being drawn from the units
    /// whose art was made in the palette nearest to the one the game draws it in.
    /// `None` when one unit (or none) has art for the pose.
    #[must_use]
    pub fn unit_for(&self, pose: &PoseRef, row: &[[u8; 3]; 16]) -> Option<String> {
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
            if self.super_saiyan_goku {
                return units.into_iter().find(super_saiyan);
            }
            units.retain(|unit| !super_saiyan(unit));
        }
        self.retain_by_palette(pose, row, &mut units);
        let sizes = self.unit_sizes();
        units.into_iter().max_by_key(|unit| {
            (sizes.get(unit).copied().unwrap_or(0), std::cmp::Reverse(unit.clone()))
        })
    }

    /// Of the units of Super Saiyan Vegeta and the units of the normal Vegeta in `units`, all with art for the pose,
    /// keeps those whose art was made in the colours nearest to `row`, those of the pose in the frame (a tie keeps the
    /// normal ones). `units` is left as it is when it holds no such pair, or when the art of either side has no
    /// palette to compare.
    fn retain_by_palette(&self, pose: &PoseRef, row: &[[u8; 3]; 16], units: &mut Vec<String>) {
        let super_saiyan =
            |unit: &String| SUPER_VEGETA_MARKS.iter().any(|mark| unit.contains(mark));
        let normal = |unit: &String| unit.contains(VEGETA_MARK) && !super_saiyan(unit);
        let candidates = self.candidates(pose);
        let nearest = |side: &dyn Fn(&String) -> bool| {
            candidates
                .iter()
                .filter(|&&(at, _)| side(&self.unit_of(at)))
                .map(|&(at, _)| &self.index.objects()[at as usize].palette)
                .filter(|palette| palette.mask != 0)
                .map(|palette| palette.distance(row))
                .min()
        };
        if !units.iter().any(super_saiyan) || !units.iter().any(normal) {
            return;
        }
        let (Some(normal_distance), Some(super_distance)) =
            (nearest(&normal), nearest(&super_saiyan))
        else {
            return;
        };
        if super_distance < normal_distance {
            units.retain(|unit| super_saiyan(unit) || !normal(unit));
        } else {
            units.retain(|unit| !super_saiyan(unit));
        }
    }

    /// The number of poses of each unit of the index.
    fn unit_sizes(&self) -> &FxHashMap<String, u32> {
        self.unit_sizes.get_or_init(|| {
            let mut sizes: FxHashMap<String, u32> = FxHashMap::default();
            for at in 0..self.index.objects().len() {
                *sizes.entry(self.unit_of(at as u32)).or_default() += 1;
            }
            sizes
        })
    }

    /// The object of the index with the silhouette of the pose `arr` (`w` x `h`, 0 transparent), when the
    /// pose is one a palette variant could be: big enough, of several colours, and not a plain block.
    #[must_use]
    pub fn shaped(&self, arr: &[u8], w: usize, h: usize, field: bool) -> Option<u32> {
        let opaque = arr.iter().filter(|&&c| c != 0).count();
        let mut seen = 0_u16;
        for &c in arr.iter().filter(|&&c| c != 0) {
            seen |= 1 << (c & 15);
        }
        if opaque >= SHAPE_MIN_OPAQUE
            && opaque < arr.len()
            && seen.count_ones() >= 3
            && u16::try_from(w).is_ok()
            && u16::try_from(h).is_ok()
        {
            let art = self.index;
            art.objects_shaped(w as u16, h as u16, opaque as u32).iter().copied().find(|&at| {
                let object = &art.objects()[at as usize];
                object.field() == field
                    && (0..w * h).all(|n| object.opaque_at(n % w, n / w) == (arr[n] != 0))
            })
        } else {
            None
        }
    }
}

/// The unit an art id is of: the id without the number of the pose.
#[must_use]
pub fn unit_of_id(id: &str) -> String {
    match id.rsplit_once('-') {
        Some((unit, number)) if number.bytes().all(|b| b.is_ascii_digit()) => unit.to_owned(),
        _ => id.to_owned(),
    }
}

/// The art of `object` (`rgba`, `width` x `height`) as the renderer draws it for the pose `arr`
/// (`w` x `h`, the pose's pixels in the art's own facing) in the colours of `row`: recoloured from the
/// palette it was made for when that differs, or from the palette its own pixels have when the entry has none
/// and was found by `shaped`. `None` when it is drawn as made (greyscale art takes its colours from the
/// frame, and is never recoloured here).
#[must_use]
pub fn recoloured_art(
    object: &Object,
    (rgba, width, height): (&[u8], usize, usize),
    (arr, w, h): (&[u8], usize, usize),
    row: &[[u8; 3]; 16],
    shaped: bool,
) -> Option<Vec<u8>> {
    if object.luma || w == 0 || !width.is_multiple_of(w) {
        return None;
    }
    let image = Image::new(width, height, rgba.to_vec());
    let palette = if object.palette.mask == 0 {
        if !shaped {
            return None;
        }
        recolour::derive_palette(&image, arr, w, h, width / w)
    } else {
        object.palette
    };
    recolour::differs(&palette, row).then(|| recolour::recolour(&image, &palette, row).rgba)
}
