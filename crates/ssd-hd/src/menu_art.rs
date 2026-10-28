//! The art of a picture the HD menus draw themselves (the Item screen's picture of an object):
//! found by what the picture IS, never by its number or its shape.
//!
//! The menus know the sixteen characters of the object's picture (its tiles as the game uploads
//! them) and its palette. The pack's art for those characters is the picture of the index whose
//! asset holds exactly those tiles, in the cells the picture covers: the same content hash the
//! tile binding uses on screen ([`crate::pack::word_key`]). Several pictures can cover the same
//! characters (the same face made for another palette); the one made for the palette closest to
//! the object's is drawn, recoloured to the object's palette when it was made for another. A
//! picture whose characters the pack has no art for has no art, and the menu draws the original's
//! pixels: art of another picture is never drawn in its place.

use crate::art::Image;
use crate::art_index::Picture;
use crate::pack::{Pack, word_key};

/// The art found for a picture of the menus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MenuPicture {
    /// The name of the art in the pack's index.
    pub name: String,
    pub width: usize,
    pub height: usize,
    /// RGBA pixels, row by row, in the colours of the palette asked for.
    pub rgba: Vec<u8>,
    /// Whether the art was made for another palette and has been recoloured to the one asked for.
    pub recoloured: bool,
}

impl Pack {
    /// The pictures of the index that cover exactly `characters` (the sixteen four-bit characters
    /// of a 4 by 4 picture, as video memory holds them, row by row): each with how far its
    /// palette is from `row` ([`crate::art_index::Palette::distance`]), the nearest first.
    #[must_use]
    pub fn pictures_of_characters(
        &self,
        characters: &[[u16; 16]],
        row: &[[u8; 3]; 16],
    ) -> Vec<(&Picture, u32)> {
        let keys: Vec<u64> = characters.iter().map(|chars| word_key(chars)).collect();
        let Some(&first) = keys.first() else { return Vec::new() };
        let mut found: Vec<(&Picture, u32)> = Vec::new();
        for candidate in self.index.candidates(4, first) {
            let asset = self.index.asset(candidate.asset);
            for &at in self.art.pictures_of(&asset.id) {
                let picture = &self.art.pictures()[at as usize];
                let same = keys.len() == picture.tiles.len()
                    && keys
                        .iter()
                        .zip(&picture.tiles)
                        .all(|(key, &tile)| asset.keys.get(usize::from(tile)) == Some(key));
                if same && !found.iter().any(|(other, _)| other.name == picture.name) {
                    found.push((picture, picture.palette.distance(row)));
                }
            }
        }
        found.sort_by_key(|&(picture, distance)| (distance, u8::from(picture.palette.mask == 0)));
        found
    }

    /// The art of the picture made of `characters` in the colours of `row`, read from the pack, or
    /// `None` when the pack has none for those characters (or its file cannot be read).
    #[must_use]
    pub fn menu_picture(
        &self,
        characters: &[[u16; 16]],
        row: &[[u8; 3]; 16],
    ) -> Option<MenuPicture> {
        let (picture, _) = *self.pictures_of_characters(characters, row).first()?;
        let bytes = std::fs::read(self.dir.join(&picture.art)).ok()?;
        let (width, height, rgba) = crate::png::read_rgba(&bytes).ok()?;
        let image = Image::new(width as usize, height as usize, rgba);
        if image.width == 0 || image.height == 0 {
            return None;
        }
        let drawn = crate::art::recolour_to(&image, &picture.palette, row);
        Some(MenuPicture {
            name: picture.name.clone(),
            width: drawn.width,
            height: drawn.height,
            recoloured: drawn.rgba != image.rgba,
            rgba: drawn.rgba,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::art_index::{ArtIndex, Palette};
    use crate::pack::{Asset, Index, Manifest, Mode};

    /// A character whose first word is `seed` (never blank, never another seed's).
    fn character(seed: u16) -> [u16; 16] {
        let mut chars = [0_u16; 16];
        chars[0] = seed;
        chars
    }

    fn palette(colour_1: [u8; 3]) -> Palette {
        let mut palette = Palette { mask: 0b10, ..Palette::default() };
        palette.colors[1] = colour_1;
        palette
    }

    /// A pack with one asset of 32 characters (two halves of sixteen: the characters `seed 1`
    /// to `seed 32`) and the art of the pictures `(name, half, palette colour 1, art colour)`.
    fn pack(dir: &std::path::Path, pictures: &[(&str, usize, [u8; 3], [u8; 4])]) -> Pack {
        let asset = Asset {
            id: "portrait/16:8000/4bpp".to_owned(),
            bpp: 4,
            mode: Mode::Indexed,
            columns: 16,
            keys: (1..=32).map(|seed| word_key(&character(seed))).collect(),
            art: String::new(),
        };
        let pictures = pictures
            .iter()
            .map(|&(name, half, colour, art)| {
                let data = art.repeat(16 * 16);
                std::fs::create_dir_all(dir.join("art")).unwrap();
                crate::png::write_rgba(&dir.join(format!("art/{name}.png")), 16, 16, &data)
                    .unwrap();
                Picture {
                    name: name.to_owned(),
                    asset: asset.id.clone(),
                    tiles: std::array::from_fn(|cell| (half * 16 + cell) as u16),
                    palette: palette(colour),
                    art: format!("art/{name}.png"),
                }
            })
            .collect();
        let index = Index::new(vec![asset]);
        let manifest = Manifest {
            format: crate::pack::FORMAT,
            scale: 4,
            converter: 1,
            rom_crc32: "00000000".to_owned(),
            assets: 1,
        };
        Pack {
            dir: dir.to_path_buf(),
            manifest,
            index,
            art: ArtIndex::new(pictures, Vec::new()),
            maps: crate::mapart::Index::default(),
            bg: crate::bgart::Index::default(),
            mode7: crate::mode7art::Index::default(),
            variants: Vec::new(),
            pose_art: Vec::new(),
        }
    }

    /// The characters of half `half` of the asset.
    fn half(half: u16) -> Vec<[u16; 16]> {
        (0..16).map(|cell| character(half * 16 + cell + 1)).collect()
    }

    fn row(colour_1: [u8; 3]) -> [[u8; 3]; 16] {
        let mut row = [[0_u8; 3]; 16];
        row[1] = colour_1;
        row
    }

    #[test]
    fn a_picture_gets_the_art_of_its_own_characters_and_never_another_pictures() {
        let dir = tempfile::tempdir().unwrap();
        // Half 0 is the picture of one object, half 1 the picture of another: the names mean
        // nothing (the art called `obj-13` is half 0)
        let pack = pack(
            dir.path(),
            &[
                ("obj-13", 0, [255, 0, 0], [200, 0, 0, 255]),
                ("obj-10", 1, [0, 255, 0], [0, 200, 0, 255]),
            ],
        );
        let first = pack.menu_picture(&half(0), &row([255, 0, 0])).unwrap();
        assert_eq!((first.name.as_str(), first.rgba[0]), ("obj-13", 200));
        let second = pack.menu_picture(&half(1), &row([0, 255, 0])).unwrap();
        assert_eq!((second.name.as_str(), second.rgba[1]), ("obj-10", 200));
        // Characters the pack has no art for (a third half, or half of one picture and half of
        // another) have none: the menu draws the original's pixels
        let mut mixed = half(0);
        mixed[15] = character(32);
        assert!(pack.menu_picture(&mixed, &row([255, 0, 0])).is_none());
        let unknown: Vec<[u16; 16]> = (0..16).map(|cell| character(100 + cell)).collect();
        assert!(pack.menu_picture(&unknown, &row([255, 0, 0])).is_none());
        // Even when only the first character is the picture's
        let mut one = unknown;
        one[0] = character(1);
        assert!(pack.menu_picture(&one, &row([255, 0, 0])).is_none());
    }

    #[test]
    fn the_art_made_for_the_nearest_palette_is_drawn_and_another_is_recoloured() {
        let dir = tempfile::tempdir().unwrap();
        let pack = pack(
            dir.path(),
            &[
                ("red", 0, [255, 0, 0], [200, 0, 0, 255]),
                ("green", 0, [0, 255, 0], [0, 200, 0, 255]),
            ],
        );
        let characters = half(0);
        let green = pack.menu_picture(&characters, &row([0, 250, 0])).unwrap();
        assert_eq!(green.name, "green");
        assert!(!green.recoloured || green.rgba[1] > 150, "{green:?}");
        // A palette that neither was made for: the nearer art, recoloured to blue-ish
        let blue = pack.menu_picture(&characters, &row([0, 0, 255])).unwrap();
        assert!(blue.recoloured);
        assert!(
            blue.rgba[2] > blue.rgba[0] && blue.rgba[2] > blue.rgba[1],
            "{:?}",
            &blue.rgba[..4]
        );
        let order = pack.pictures_of_characters(&characters, &row([250, 0, 0]));
        assert_eq!(
            order.iter().map(|(p, _)| p.name.as_str()).collect::<Vec<_>>(),
            ["red", "green"]
        );
    }
}
