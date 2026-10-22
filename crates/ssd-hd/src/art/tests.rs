//! The art passes on synthetic frames: a pack of a few pictures made in a temporary folder, and
//! frames with one tile or one sprite.

use super::poses::{self, Pose};
use super::{ArtState, Conditions};
use crate::art_index::{ArtIndex, Object, Palette, Picture};
use crate::pack::{Asset, Index, Manifest, Mode, Pack, PoseVariant, word_key};
use crate::world::{HudPlacement, Rom, WideOptions, WideRenderer};
use crate::{Fate, HdOptions, ItemKey, Look, Renderer, SpriteLine, TallyLayer};
use ssd_oracle::{FrameLog, LineEntry, LineRegs};
use std::sync::Arc;

fn regs() -> LineRegs {
    LineRegs {
        brightness: 15,
        bg_mode: 1,
        main_bg: [true, false, false, false],
        main_obj: true,
        bg_map_base: [0x0400, 0, 0, 0],
        bg_char_base: [0x1000, 0, 0, 0],
        ..LineRegs::default()
    }
}

/// A frame of 224 lines with the object sprites evaluated and rebuilt on every line.
fn log(regs: LineRegs) -> FrameLog {
    let entries = (1..=224_u16)
        .map(|scanline| LineEntry {
            scanline,
            from_pixel: 0,
            regs,
            sprites: SpriteLine { eval_line: scanline - 1, rebuilt: true, ..SpriteLine::default() },
        })
        .collect();
    let mut log = FrameLog {
        complete: true,
        height: 224,
        h_hi_res: false,
        v_hi_res: false,
        entries,
        cgram_start: vec![0; 256],
        cgram_writes: Vec::new(),
        vram: vec![0; 32768],
        vram_writes: Vec::new(),
        oam_low: vec![0; 512],
        oam_high: vec![0; 32],
        oam_writes: Vec::new(),
    };
    for n in 0..128 {
        log.oam_low[n * 4 + 1] = 0xF0;
    }
    log
}

/// A solid 4x PNG of `width` x `height` art pixels in `rgba`, with a different colour in the
/// cell of each 32 x 32 block when `cells` (red grows with the cell number).
fn write_png(path: &std::path::Path, width: u32, height: u32, rgba: impl Fn(u32, u32) -> [u8; 4]) {
    let mut data = Vec::new();
    for y in 0..height {
        for x in 0..width {
            data.extend_from_slice(&rgba(x, y));
        }
    }
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    crate::png::write_rgba(path, width, height, &data).unwrap();
}

fn pack_of(dir: &std::path::Path, assets: Vec<Asset>, art: ArtIndex) -> Arc<Pack> {
    let index = Index::new(assets);
    let manifest = Manifest {
        format: crate::pack::FORMAT,
        scale: 4,
        converter: 1,
        rom_crc32: "00000000".to_owned(),
        assets: index.assets().len(),
    };
    Arc::new(Pack {
        dir: dir.to_path_buf(),
        manifest,
        index,
        art,
        maps: crate::mapart::Index::default(),
        bg: crate::bgart::Index::default(),
        mode7: crate::mode7art::Index::default(),
        variants: Vec::new(),
    })
}

/// A 4-bit character whose row 0 is colour 1 in every column, the other rows empty.
fn character() -> [u16; 16] {
    let mut chars = [0_u16; 16];
    chars[0] = 0x00FF;
    chars
}

fn options(scale: u32) -> HdOptions {
    HdOptions { scale, look: Look::Crisp }
}

fn pixel(frame: &crate::HdFrame, x: usize, y: usize) -> [u8; 3] {
    let at = (y * frame.width as usize + x) * 4;
    [frame.rgba[at], frame.rgba[at + 1], frame.rgba[at + 2]]
}

#[test]
fn a_bound_character_draws_its_cell_of_the_picture_flips_included() {
    let dir = tempfile::tempdir().unwrap();
    // The picture: cell (cx, cy) is the colour (cx * 60, cy * 60, 7), and within the cell the
    // art's left half is darker, so a flip shows
    write_png(&dir.path().join("art/p.png"), 128, 128, |x, y| {
        let (cx, cy) = (x / 32, y / 32);
        let dark = if x % 32 < 16 { 0 } else { 40 };
        [(cx * 60 + dark) as u8, (cy * 60) as u8, 7, 255]
    });
    // The asset has the character at its tile 5, which is cell 5 (x 1, y 1) of the picture
    let mut keys = vec![0_u64; 16];
    keys[5] = word_key(&character());
    let asset = Asset {
        id: "portrait/16:8000/4bpp".to_owned(),
        bpp: 4,
        mode: Mode::Indexed,
        columns: 16,
        keys,
        art: "base/x.png".to_owned(),
    };
    let art = ArtIndex::new(
        vec![Picture {
            name: "obj-00".to_owned(),
            asset: asset.id.clone(),
            tiles: std::array::from_fn(|n| n as u16),
            palette: Palette::default(),
            art: "art/p.png".to_owned(),
        }],
        Vec::new(),
    );
    let pack = pack_of(dir.path(), vec![asset], art);

    let mut log = log(regs());
    log.vram[0x1000 + 16..0x1000 + 32].copy_from_slice(&character()); // character 1 of the base
    // BG1 shows character 1 at map row 1 (a BG shows one row below its scanline: the frame row 7
    // is the first row of that tile) and again, flipped across, next to it
    log.vram[0x0400 + 32] = 1;
    log.vram[0x0400 + 33] = 1 | 0x4000;
    log.cgram_start[1] = 0x001F;

    let mut renderer = Renderer::new();
    renderer.set_art(Some(Arc::clone(&pack)));
    let picture = renderer.render_hd(&log, options(4)).unwrap();
    // The first tile's row 1 (the first frame row is scanline 1, a BG shows its row 1 there)...
    // The character's row 0 is the only one with colour; the frame's row 0 shows row 1: empty.
    // Row 1 of the frame is row 2 of the character: also empty; so look at what is drawn from the
    // art instead: the tile's pixels are transparent and draw nothing, and only the colour-1 row
    // is a pixel of the character: frame row 7 shows the tile's row 0 (rows are one down).
    // The pixel's block at x 0..4 is art cell (1, 1), row 0 of the character, columns 0..
    let at_row_0 = 7 * 4; // the HD row of the frame row 7
    let left = pixel(&picture, 0, at_row_0);
    let right = pixel(&picture, 31, at_row_0);
    assert_eq!(left[1], 60, "cell (1, 1) of the picture: its green is 60");
    assert_eq!(left[0], 60, "dark half: red 60");
    assert_eq!(right[0], 100, "the lighter half of the cell is on its right");
    // The flipped tile next to it shows the cell mirrored: its left is the cell's lighter half
    let flipped_left = pixel(&picture, 32, at_row_0);
    assert_eq!(flipped_left[0], 100);
    assert_eq!(pixel(&picture, 32 + 31, at_row_0)[0], 60);
    let stats = renderer.art_stats();
    assert_eq!(stats.picture_pixels, 16, "two tiles of eight pixels");
}

#[test]
fn a_picture_chosen_by_palette_when_two_cover_the_same_characters() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/a.png"), 128, 128, |_, _| [10, 0, 0, 255]);
    write_png(&dir.path().join("art/b.png"), 128, 128, |_, _| [0, 10, 0, 255]);
    let mut keys = vec![0_u64; 16];
    keys[0] = word_key(&character());
    let asset = Asset {
        id: "portrait/16:8000/4bpp".to_owned(),
        bpp: 4,
        mode: Mode::Indexed,
        columns: 16,
        keys,
        art: String::new(),
    };
    let mut palette_a = Palette { mask: 0b10, ..Palette::default() };
    palette_a.colors[1] = [255, 0, 0];
    let mut palette_b = palette_a;
    palette_b.colors[1] = [0, 0, 255];
    let picture = |name: &str, palette, art: &str| Picture {
        name: name.to_owned(),
        asset: asset.id.clone(),
        tiles: std::array::from_fn(|n| n as u16),
        palette,
        art: art.to_owned(),
    };
    let art = ArtIndex::new(
        vec![picture("a", palette_a, "art/a.png"), picture("b", palette_b, "art/b.png")],
        Vec::new(),
    );
    let pack = pack_of(dir.path(), vec![asset], art);
    let mut log = log(regs());
    log.vram[0x1000 + 16..0x1000 + 32].copy_from_slice(&character());
    log.vram[0x0400 + 32] = 1;
    // BG1 palette 0 colour 1 is blue: the second picture
    log.cgram_start[1] = 0x7C00;
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&log, options(4)).unwrap();
    assert_eq!(pixel(&drawn, 0, 28), [0, 10, 0]);
}

/// A fighter of one 16 x 16 sprite with palette 1, solid colour 2 (green), at `(x, y)`.
fn fighter_log(x: u8, y: u8) -> (FrameLog, Pose) {
    let mut log = log(regs());
    // Sprite 0, tile 0, palette 1, the large size; the object characters are at word 0 of VRAM
    log.oam_low[0] = x;
    log.oam_low[1] = y;
    log.oam_low[2] = 0;
    log.oam_low[3] = 1 << 1;
    log.oam_high[0] = 0b10;
    // Colour 2 is plane 1: the high byte of the first eight words of a character; the four
    // characters of a 16 x 16 sprite are tiles 0, 1, 16 and 17
    for tile in [0_usize, 1, 16, 17] {
        for row in 0..8 {
            log.vram[tile * 16 + row] = 0xFF00;
        }
    }
    log.cgram_start[0x80 + 16 + 2] = 0x03E0; // palette 1, colour 2: green
    let sprites = poses::sprites(&log);
    // (a field group is found in every frame too: it is not what these helpers are for)
    let poses: Vec<Pose> =
        poses::poses(&log, &sprites).into_iter().filter(|pose| !pose.field).collect();
    assert_eq!(poses.len(), 1);
    (log, poses.into_iter().next().unwrap())
}

fn object(pose: &Pose, art: &str, luma: bool) -> Object {
    Object {
        key: pose.key,
        id: "battle-x-000".to_owned(),
        luma,
        palette: Palette::default(),
        mask: Vec::new(),
        width: pose.w as u16,
        height: pose.h as u16,
        art: art.to_owned(),
    }
}

#[test]
fn a_pose_is_found_by_its_key_and_its_art_replaces_the_sprites_pixels() {
    let dir = tempfile::tempdir().unwrap();
    // 16 x 16 source pixels, 64 x 64 art pixels: a solid blue square
    write_png(&dir.path().join("art/o.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    let (frame, pose) = fighter_log(40, 50);
    assert_eq!((pose.w, pose.h), (16, 16));
    let pack = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(Vec::new(), vec![object(&pose, "art/o.png", false)]),
    );
    let mut renderer = Renderer::new();
    let plain = renderer.render_hd(&frame, options(4)).unwrap();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();

    // The sprite's row 0 is the frame row 50, its column 0 the column 40
    assert_eq!(pixel(&plain, 41 * 4 + 1, 51 * 4 + 1), [0, 0xFF, 0]);
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 51 * 4 + 1), [0, 0, 255]);
    assert_eq!(pixel(&drawn, 40 * 4, 50 * 4), [0, 0, 255], "the first pixel of the pose");
    assert_eq!(pixel(&drawn, 55 * 4 + 3, 65 * 4 + 3), [0, 0, 255], "and the last");
    assert_ne!(pixel(&drawn, 40 * 4 - 1, 50 * 4), [0, 0, 255], "nothing past its box");
    assert_ne!(pixel(&drawn, 40 * 4, 50 * 4 - 1), [0, 0, 255]);
    // Away from the pose nothing changes
    assert_eq!(pixel(&drawn, 5, 5), pixel(&plain, 5, 5));
    let stats = renderer.art_stats();
    assert_eq!((stats.fighters.total, stats.fighters.art), (256, 256));
    assert_eq!(stats.poses.iter().filter(|pose| !pose.field).count(), 1);
    assert!(stats.poses[0].matched);
}

#[test]
fn a_pose_the_index_lacks_stays_the_look() {
    let dir = tempfile::tempdir().unwrap();
    let (frame, pose) = fighter_log(40, 50);
    let mut other = object(&pose, "art/none.png", false);
    other.key ^= 1;
    let mut renderer = Renderer::new();
    let plain = renderer.render_hd(&frame, options(2)).unwrap();
    renderer.set_art(Some(pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![other]))));
    let drawn = renderer.render_hd(&frame, options(2)).unwrap();
    assert_eq!(drawn, plain);
    let stats = renderer.art_stats();
    assert_eq!((stats.fighters.total, stats.fighters.art), (256, 0));
    assert_eq!(stats.fighters.missing, 256, "a whole pose the capture would have kept");
}

#[test]
fn a_pose_whose_art_file_is_missing_stays_as_the_game_draws_it() {
    let dir = tempfile::tempdir().unwrap();
    // The index names art that is not there (an import in progress, a deleted file): the sprites
    // are not hidden for art that cannot be drawn
    let (frame, pose) = fighter_log(40, 50);
    let pack = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(Vec::new(), vec![object(&pose, "art/gone.png", false)]),
    );
    let mut renderer = Renderer::new();
    let plain = renderer.render_hd(&frame, options(4)).unwrap();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(drawn, plain, "the sprite is still there");
    assert!(renderer.art_stats().poses.iter().all(|pose| !pose.matched));
    // The same through the wide view
    let mut wide = WideRenderer::new();
    wide.set_art(Some(Arc::clone(&renderer_pack(&dir, &pose))));
    let options = WideOptions { width: 372, hud: HudPlacement::Window };
    let drawn =
        wide.render_hd(Rom(&[]), 0x02, None, &frame, options, super::tests::options(4)).unwrap();
    assert_eq!(pixel(&drawn.picture, (58 + 41) * 4 + 1, 51 * 4 + 1), [0, 0xFF, 0]);
}

fn renderer_pack(dir: &tempfile::TempDir, pose: &Pose) -> Arc<Pack> {
    pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(Vec::new(), vec![object(pose, "art/gone.png", false)]),
    )
}

#[test]
fn a_pose_behind_a_background_tile_is_drawn_only_where_the_original_showed_it() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/o.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    let (mut frame, pose) = fighter_log(40, 50);
    // BG1 character 1 (solid colour 1) with the priority bit at map tile (5, 6): it covers the
    // columns 40..48 and the frame rows 47..55 (a BG shows one row below its scanline), in front
    // of the sprite's priority 0
    let mut solid = [0_u16; 16];
    solid[..8].fill(0x00FF);
    frame.vram[0x1000 + 16..0x1000 + 32].copy_from_slice(&solid);
    frame.vram[0x0400 + 6 * 32 + 5] = 1 | 0x2000;
    frame.cgram_start[1] = 0x7FFF;
    let pack = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(Vec::new(), vec![object(&pose, "art/o.png", false)]),
    );
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    assert_ne!(pixel(&drawn, 41 * 4 + 1, 51 * 4 + 1), [0, 0, 255], "behind the tile");
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 60 * 4 + 1), [0, 0, 255], "below it");
    assert_eq!(pixel(&drawn, 50 * 4 + 1, 52 * 4 + 1), [0, 0, 255], "beside it");
    let stats = renderer.art_stats();
    assert_eq!(stats.fighters.total, 256 - 5 * 8, "the covered part is not the sprite's pixels");
}

#[test]
fn luma_art_takes_its_colours_from_the_original() {
    let dir = tempfile::tempdir().unwrap();
    // Greyscale art, bright on the left half and half as bright on the right
    write_png(&dir.path().join("art/o.png"), 64, 64, |x, _| {
        let level = if x < 32 { 200 } else { 100 };
        [level, level, level, 255]
    });
    let (frame, pose) = fighter_log(40, 50);
    let pack = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(Vec::new(), vec![object(&pose, "art/o.png", true)]),
    );
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    // The art's reference luminance keeps 97 percent of the original's colour on average (here
    // 106): the left is the original's full green, the right 100 / 106 of it
    assert_eq!(pixel(&drawn, 41 * 4, 51 * 4), [0, 0xFF, 0]);
    assert_eq!(pixel(&drawn, 50 * 4, 51 * 4), [0, 240, 0]);
}

#[test]
fn shading_follows_the_colour_math_and_the_brightness() {
    use crate::screen::{Math, Plan};
    let plain = Plan { force_black: false, math: None };
    assert_eq!(super::draw::shade(plain, 15, [10, 20, 30]), [10, 20, 30]);
    assert_eq!(super::draw::shade(plain, 0, [10, 20, 30]), [0, 0, 0]);
    // Half brightness rounds
    assert_eq!(super::draw::shade(plain, 8, [255, 100, 1]), [136, 53, 1]);
    // Subtracting the fixed colour: the 5-bit red channel 10 is 82 of 255
    let math =
        Plan { force_black: false, math: Some(Math { sub: 10, subtract: true, divide: false }) };
    assert_eq!(super::draw::shade(math, 15, [200, 50, 50]), [118, 50, 50]);
    let black = Plan { force_black: true, math: None };
    assert_eq!(super::draw::shade(black, 15, [200, 50, 50]), [0, 0, 0]);
}

#[test]
fn the_wide_view_draws_the_art_beside_the_black_borders_of_a_picture() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/o.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    let (frame, pose) = fighter_log(40, 50);
    let pack = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(Vec::new(), vec![object(&pose, "art/o.png", false)]),
    );
    let mut wide = WideRenderer::new();
    wide.set_art(Some(pack));
    let options = WideOptions { width: 372, hud: HudPlacement::Window };
    // The game mode of a battle: a picture of the original's 256 pixels, black on both sides
    let drawn =
        wide.render_hd(Rom(&[]), 0x02, None, &frame, options, super::tests::options(4)).unwrap();
    assert_eq!((drawn.picture.width, drawn.picture.height), (372 * 4, 224 * 4));
    // The window starts 58 pixels in: the pose is 58 pixels further right than in the 256-wide frame
    assert_eq!(drawn.logical.x, 58);
    assert_eq!(drawn.picture.logical.x, 58 * 4);
    assert_eq!(pixel(&drawn.picture, (58 + 41) * 4 + 1, 51 * 4 + 1), [0, 0, 255]);
    assert_eq!(pixel(&drawn.picture, 10, 10), [0, 0, 0], "black beside the window");
    let stats = wide.art_stats();
    assert_eq!((stats.fighters.total, stats.fighters.art), (256, 256));
    // Without art the same frame is the look alone, and the sides are the same
    wide.set_art(None);
    let plain =
        wide.render_hd(Rom(&[]), 0x02, None, &frame, options, super::tests::options(4)).unwrap();
    assert_eq!(pixel(&plain.picture, (58 + 41) * 4 + 1, 51 * 4 + 1), [0, 0xFF, 0]);
    assert_eq!(pixel(&plain.picture, 10, 10), [0, 0, 0]);
}

#[test]
fn a_fighter_fading_in_as_stripes_is_drawn_from_the_whole_pose_where_it_shows() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/o.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    let (full, whole) = fighter_log(40, 50);
    // The same sprite with every other column left out: its pixels are not the pose's key
    let (mut striped, _) = fighter_log(40, 50);
    for tile in [0_usize, 1, 16, 17] {
        for row in 0..8 {
            striped.vram[tile * 16 + row] = 0xAA00;
        }
    }
    let mut palette = Palette { mask: 0b100, ..Palette::default() };
    palette.colors[2] = [0, 255, 0];
    let mut held = object(&whole, "art/o.png", false);
    held.palette = palette;
    held.mask = vec![0xFF; 32];
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![held]));
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));

    let drawn = renderer.render_hd(&striped, options(4)).unwrap();
    // Columns 0, 2, 4 of the sprite show; the art is drawn on them and not between
    assert_eq!(pixel(&drawn, 40 * 4 + 1, 55 * 4 + 1), [0, 0, 255]);
    assert_eq!(pixel(&drawn, 42 * 4 + 1, 55 * 4 + 1), [0, 0, 255]);
    assert_ne!(
        pixel(&drawn, 41 * 4 + 1, 55 * 4 + 1),
        [0, 0, 255],
        "a column the original left out"
    );
    let stats = renderer.art_stats();
    assert_eq!((stats.fighters.total, stats.fighters.art), (128, 128));

    // The whole pose is found by its key as before
    let drawn = renderer.render_hd(&full, options(4)).unwrap();
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 55 * 4 + 1), [0, 0, 255]);
}

#[test]
fn a_whole_fighter_with_no_art_is_not_taken_for_a_piece_of_another_pose() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/o.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    let (full, whole) = fighter_log(40, 50);
    // The index holds a pose of the same silhouette (and colours) under another key: a pose the
    // original draws whole (no stripes, not at an edge) is its own, never a piece of it
    let mut other = object(&whole, "art/o.png", false);
    other.key ^= 1;
    other.mask = vec![0xFF; 32];
    other.palette = Palette { mask: 0b100, ..Palette::default() };
    other.palette.colors[2] = [0, 255, 0];
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![other]));
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&full, options(4)).unwrap();
    assert_ne!(pixel(&drawn, 41 * 4 + 1, 55 * 4 + 1), [0, 0, 255]);
    assert!(!renderer.art_stats().poses.iter().any(|p| p.matched));
}

#[test]
fn every_scale_draws_the_pose_and_the_picture_in_the_same_place() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/o.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    write_png(&dir.path().join("art/p.png"), 128, 128, |_, _| [255, 0, 0, 255]);
    let (mut frame, pose) = fighter_log(100, 100);
    // A BG1 character with colour at map row 1 (frame row 7): the picture's cell
    frame.vram[0x1000 + 16..0x1000 + 32].copy_from_slice(&character());
    frame.vram[0x0400 + 32] = 1;
    frame.cgram_start[1] = 0x001F;
    let mut keys = vec![0_u64; 16];
    keys[0] = word_key(&character());
    let asset = Asset {
        id: "portrait/16:8000/4bpp".to_owned(),
        bpp: 4,
        mode: Mode::Indexed,
        columns: 16,
        keys,
        art: String::new(),
    };
    let picture = Picture {
        name: "obj-00".to_owned(),
        asset: asset.id.clone(),
        tiles: std::array::from_fn(|n| n as u16),
        palette: Palette::default(),
        art: "art/p.png".to_owned(),
    };
    let art = ArtIndex::new(vec![picture], vec![object(&pose, "art/o.png", false)]);
    let pack = pack_of(dir.path(), vec![asset], art);
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    for scale in 1..=8_u32 {
        let drawn = renderer.render_hd(&frame, options(scale)).unwrap();
        let s = scale as usize;
        assert_eq!((drawn.width, drawn.height), (256 * scale, 224 * scale));
        assert_eq!(
            pixel(&drawn, 100 * s, 100 * s),
            [0, 0, 255],
            "the pose's first pixel at {scale}x"
        );
        assert_eq!(pixel(&drawn, 115 * s + s - 1, 115 * s + s - 1), [0, 0, 255], "and its last");
        assert_eq!(pixel(&drawn, 0, 7 * s), [255, 0, 0], "the picture at {scale}x");
    }
}

/// A ground shadow: one 16 x 16 sprite of palette 3 (an effect's) in one dark grey, at `(x, y)`,
/// an ellipse of four rows (the rows 6 to 9 of the sprite, 10 and 16 pixels wide) made of the
/// characters 2, 3, 18 and 19. Character 0 is a solid block (a fighter's).
fn shadow_log(x: u8, y: u8) -> FrameLog {
    let mut log = log(regs());
    log.oam_low[0] = x;
    log.oam_low[1] = y;
    log.oam_low[2] = 2;
    log.oam_low[3] = 3 << 1;
    log.oam_high[0] = 0b10;
    for row in 0..8 {
        log.vram[row] = 0xFF00;
    }
    // Colour 2 is plane 1, the high byte of a row's word
    let rows = [(2_usize, 6, 0x1F), (3, 6, 0xF8), (2, 7, 0xFF), (3, 7, 0xFF)];
    for (tile, row, bits) in rows {
        log.vram[tile * 16 + row] = bits << 8;
    }
    let rows = [(18_usize, 0, 0xFF), (19, 0, 0xFF), (18, 1, 0x1F), (19, 1, 0xF8)];
    for (tile, row, bits) in rows {
        log.vram[tile * 16 + row] = bits << 8;
    }
    log.cgram_start[0x80 + 3 * 16 + 2] = 0x318C; // 12, 12, 12: grey of about 99
    log
}

#[test]
fn a_dark_flat_effect_is_a_shadow_drawn_from_the_originals_colour_with_a_soft_edge() {
    let dir = tempfile::tempdir().unwrap();
    // The index has art for something else: a shadow needs none, and the art that was made of a
    // shadow's pixels (a bright flash) is not used
    let frame = shadow_log(40, 50);
    let sprites = poses::sprites(&frame);
    let found: Vec<Pose> =
        poses::poses(&frame, &sprites).into_iter().filter(|pose| !pose.field).collect();
    assert_eq!(found.len(), 1);
    assert!(found[0].shadow && !found[0].fighter);
    write_png(&dir.path().join("art/flash.png"), 64, 64, |_, _| [255, 255, 0, 255]);
    let flash = object(&found[0], "art/flash.png", false);
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![flash]));
    let mut renderer = Renderer::new();
    let plain = renderer.render_hd(&frame, options(4)).unwrap();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();

    // The ellipse's rows are the frame rows 57 to 60
    let grey = pixel(&plain, 48 * 4, 58 * 4);
    assert!(grey[0] > 90 && grey[0] < 110 && grey[0] == grey[1] && grey[1] == grey[2], "{grey:?}");
    assert_eq!(pixel(&drawn, 48 * 4, 58 * 4), grey, "inside: the original's colour, not the flash");
    assert_eq!(pixel(&drawn, 44 * 4 + 1, 58 * 4 + 1), grey);
    // The edge: a pixel just outside the sprite is partly shadow, a pixel further out is the picture
    let edge = pixel(&drawn, 40 * 4 + 1, 58 * 4 + 1);
    assert!(edge[0] > 0 && edge[0] < grey[0], "soft edge {edge:?}");
    assert_eq!(pixel(&drawn, 40 * 4 - 8, 58 * 4), pixel(&plain, 40 * 4 - 8, 58 * 4));
    let stats = renderer.art_stats();
    assert_eq!((stats.effects.total, stats.effects.art), (52, 52));
}

#[test]
fn a_shadow_is_a_smooth_ellipse_that_darkens_a_lighter_picture() {
    let dir = tempfile::tempdir().unwrap();
    // The same shadow over a bright backdrop (colour 0, 31 of 31 on every channel): the shadow's
    // grey of about 99 is the darkness the original had against 255
    let mut frame = shadow_log(40, 50);
    frame.cgram_start[0] = 0x7FFF;
    let sprites = poses::sprites(&frame);
    let found: Vec<Pose> =
        poses::poses(&frame, &sprites).into_iter().filter(|pose| !pose.field).collect();
    write_png(&dir.path().join("art/flash.png"), 64, 64, |_, _| [255, 255, 0, 255]);
    let flash = object(&found[0], "art/flash.png", false);
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![flash]));
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    assert!(found[0].shadow);
    let middle = pixel(&drawn, 48 * 4, 58 * 4);
    assert!(middle[0] > 90 && middle[0] < 110, "multiplied by the original's darkness {middle:?}");
    // Across the top of the ellipse (rows 56 to 59, 16 wide in the middle): the shade goes from
    // the picture's white to the shadow's grey in more than the four steps of the original's
    // rows, and never gets lighter again going down
    let column: Vec<u8> = (55 * 4..58 * 4).map(|y| pixel(&drawn, 48 * 4, y)[0]).collect();
    assert_eq!(column[0], 255, "{column:?}");
    assert!(column.windows(2).all(|pair| pair[0] >= pair[1]), "{column:?}");
    let levels: std::collections::BTreeSet<u8> = column.iter().copied().collect();
    assert!(levels.len() >= 5, "{column:?}");
    // The corner of the box, which the stepped ellipse leaves out too, is the picture's
    assert_eq!(pixel(&drawn, 40 * 4, 56 * 4), [255, 255, 255]);
    assert_eq!(pixel(&drawn, 40 * 4 - 8, 58 * 4), [255, 255, 255]);
}

#[test]
fn a_shadow_goes_under_a_fighter_drawn_from_art() {
    let dir = tempfile::tempdir().unwrap();
    // A fighter (palette 1) over the shadow: the fighter's art covers part of it, and the
    // shadow's silhouette continues beneath
    write_png(&dir.path().join("art/o.png"), 64, 64, |x, _| {
        if x < 32 { [0, 0, 255, 255] } else { [0, 0, 0, 0] }
    });
    // Sprite 0 is the fighter, in front, and sprite 1 the shadow, eight pixels to its right
    let mut frame = shadow_log(48, 50);
    frame.oam_low.copy_within(0..4, 4);
    frame.oam_low[0..4].copy_from_slice(&[40, 50, 0, 1 << 1]);
    frame.oam_high[0] |= 0b10 << 2;
    frame.cgram_start[0x80 + 16 + 2] = 0x03E0;
    let sprites = poses::sprites(&frame);
    let found = poses::poses(&frame, &sprites);
    let fighter = found.iter().find(|pose| pose.fighter).expect("a fighter");
    let pack = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(Vec::new(), vec![object(fighter, "art/o.png", false)]),
    );
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    // Where the fighter's art is transparent (its right half) the shadow's grey shows, though
    // the original's fighter sprite covered it there
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 51 * 4 + 1), [0, 0, 255], "the fighter's art");
    let under = pixel(&drawn, 52 * 4, 58 * 4);
    assert!(under[0] == under[1] && under[0] > 90 && under[0] < 110, "{under:?}");
}

/// A picture of the portrait asset with one character, made for colour 1 = red (art solid red).
fn red_picture_pack(dir: &std::path::Path, art_colour: [u8; 4]) -> Arc<Pack> {
    write_png(&dir.join("art/a.png"), 128, 128, |_, _| art_colour);
    let mut keys = vec![0_u64; 16];
    keys[0] = word_key(&character());
    let asset = Asset {
        id: "portrait/16:8000/4bpp".to_owned(),
        bpp: 4,
        mode: Mode::Indexed,
        columns: 16,
        keys,
        art: String::new(),
    };
    let mut palette = Palette { mask: 0b10, ..Palette::default() };
    palette.colors[1] = [255, 0, 0];
    let picture = Picture {
        name: "a".to_owned(),
        asset: asset.id.clone(),
        tiles: std::array::from_fn(|n| n as u16),
        palette,
        art: "art/a.png".to_owned(),
    };
    pack_of(dir, vec![asset], ArtIndex::new(vec![picture], Vec::new()))
}

fn portrait_log(colour_1: u16) -> FrameLog {
    let mut log = log(regs());
    log.vram[0x1000 + 16..0x1000 + 32].copy_from_slice(&character());
    log.vram[0x0400 + 32] = 1;
    log.cgram_start[1] = colour_1;
    log
}

#[test]
fn a_picture_with_one_art_is_drawn_in_the_palette_row_the_game_uses() {
    let dir = tempfile::tempdir().unwrap();
    let pack = red_picture_pack(dir.path(), [200, 20, 20, 255]);
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    // The palette the art was made for (colour 1 red): the art as it is
    let same = renderer.render_hd(&portrait_log(0x001F), options(4)).unwrap();
    assert_eq!(pixel(&same, 0, 28), [200, 20, 20]);
    // The same tiles in a blue row: the art recoloured, still lit as the art is (not the 255 of
    // the palette's blue: the art's red was dimmer than the palette's red)
    let blue = renderer.render_hd(&portrait_log(0x7C00), options(4)).unwrap();
    let [r, g, b] = pixel(&blue, 0, 28);
    assert!(b > 150 && r < 40 && g < 40, "blue, not red: {r} {g} {b}");
    assert!(b < 255, "the art's shading is kept: {b}");
    // A frame with the palette in another place of the palette row is its own variant
    let green = renderer.render_hd(&portrait_log(0x03E0), options(4)).unwrap();
    let [r, g, b] = pixel(&green, 0, 28);
    assert!(g > 150 && r < 40 && b < 40, "green: {r} {g} {b}");
}

#[test]
fn a_pose_whose_art_was_made_for_another_palette_is_drawn_recoloured() {
    let dir = tempfile::tempdir().unwrap();
    // The art is the pose's green (palette 1, colour 2), a little darker in its lower half
    write_png(&dir.path().join("art/o.png"), 64, 64, |_, y| {
        if y < 32 { [0, 255, 0, 255] } else { [0, 190, 0, 255] }
    });
    let (mut frame, pose) = fighter_log(40, 50);
    let mut variant = object(&pose, "art/o.png", false);
    variant.palette = Palette { mask: 0b100, ..Palette::default() };
    variant.palette.colors[2] = [0, 255, 0];
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![variant]));
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));

    // In the palette it was made for: the art as it is, nothing recoloured
    let same = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&same, 41 * 4 + 1, 51 * 4 + 1), [0, 255, 0]);
    assert_eq!(pixel(&same, 41 * 4 + 1, 61 * 4 + 1), [0, 190, 0]);
    let stats = renderer.art_stats();
    assert_eq!((stats.fighters.art, stats.fighters.recoloured), (256, 0));
    assert!(!stats.poses[0].recoloured);

    // The same sprite in a red palette (the pose's key does not read colours): the art comes
    // out red, its lower half still the darker
    frame.cgram_start[0x80 + 16 + 2] = 0x001F;
    let red = renderer.render_hd(&frame, options(4)).unwrap();
    let (top, bottom) = (pixel(&red, 41 * 4 + 1, 51 * 4 + 1), pixel(&red, 41 * 4 + 1, 61 * 4 + 1));
    assert_eq!(top, [255, 0, 0]);
    assert!(bottom[0] > 150 && bottom[0] < 255 && bottom[1] == 0 && bottom[2] == 0, "{bottom:?}");
    let stats = renderer.art_stats();
    assert_eq!((stats.fighters.art, stats.fighters.recoloured), (256, 256));
    assert!(stats.poses[0].matched && stats.poses[0].recoloured);

    // The exact palette wins: with an art for the red palette too, that one is drawn as it is
    let mut exact = object(&pose, "art/e.png", false);
    exact.palette = Palette { mask: 0b100, ..Palette::default() };
    exact.palette.colors[2] = [255, 0, 0];
    write_png(&dir.path().join("art/e.png"), 64, 64, |_, _| [255, 0, 0, 255]);
    let mut variant = object(&pose, "art/o.png", false);
    variant.palette = Palette { mask: 0b100, ..Palette::default() };
    variant.palette.colors[2] = [0, 255, 0];
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![variant, exact]));
    renderer.set_art(Some(pack));
    let both = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&both, 41 * 4 + 1, 61 * 4 + 1), [255, 0, 0]);
    assert_eq!(renderer.art_stats().fighters.recoloured, 0);
}

/// An effect-class sprite (palette 4, so keyed by its colours) of 16 x 16 pixels at `(x, y)` with
/// three colours in bands (index 1 above, 2, then 3), its top row and first pixel empty.
fn banded_log(x: u8, y: u8, colours: [u16; 3]) -> (FrameLog, Pose) {
    let mut log = log(regs());
    log.oam_low[0] = x;
    log.oam_low[1] = y;
    log.oam_low[3] = 4 << 1;
    log.oam_high[0] = 0b10;
    let band = |row: usize| match row {
        0 => 0x0000,
        1..=5 => 0x00FF,
        6..=10 => 0xFF00,
        _ => 0xFFFF,
    };
    for (tile, first_row) in [(0_usize, 0), (1, 0), (16, 8), (17, 8)] {
        for row in 0..8 {
            log.vram[tile * 16 + row] = band(first_row + row);
        }
    }
    // Not a plain block: the first pixel of the first row is empty
    log.vram[1] = 0x007F;
    for (index, colour) in colours.into_iter().enumerate() {
        log.cgram_start[0x80 + 4 * 16 + 1 + index] = colour;
    }
    let sprites = poses::sprites(&log);
    // (a field group is found in every frame too: it is not what these helpers are for)
    let poses: Vec<Pose> =
        poses::poses(&log, &sprites).into_iter().filter(|pose| !pose.field).collect();
    assert_eq!(poses.len(), 1);
    (log, poses.into_iter().next().unwrap())
}

#[test]
fn a_colour_variant_of_an_effect_pose_is_found_by_its_shape_and_drawn_in_its_colours() {
    let dir = tempfile::tempdir().unwrap();
    // The art made for another palette (red, green, blue bands): its pose key is not this one's
    write_png(&dir.path().join("art/b.png"), 64, 60, |_, y| match y / 4 + 1 {
        0 => [0, 0, 0, 0],
        1..=5 => [200, 40, 40, 255],
        6..=10 => [40, 200, 40, 255],
        _ => [40, 40, 200, 255],
    });
    let (frame, pose) = banded_log(40, 50, [0x03FF, 0x7FE0, 0x7C1F]);
    assert!(!pose.fighter);
    let mut shape = object(&pose, "art/b.png", false);
    shape.key ^= 1;
    shape.mask = vec![0; pose.arr.len().div_ceil(8)];
    for n in (0..pose.arr.len()).filter(|&n| pose.arr[n] != 0) {
        shape.mask[n / 8] |= 1 << (n % 8);
    }
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![shape]));
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    // The first band's colour 1 (yellow: 0x03FF), the second's (cyan: 0x7FE0), the third's
    // (magenta: 0x7C1F), in the colours the sprite has, not the art's
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 53 * 4 + 1), poses::rgb555(0x03FF));
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 59 * 4 + 1), poses::rgb555(0x7FE0));
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 63 * 4 + 1), poses::rgb555(0x7C1F));
    let stats = renderer.art_stats();
    assert_eq!((stats.effects.art, stats.effects.recoloured), (239, 239));
    assert!(stats.poses[0].matched && stats.poses[0].recoloured);

    // A shape of another size is not it
    let (frame, _) = banded_log(40, 50, [0x03FF, 0x7FE0, 0x7C1F]);
    let mut smaller = object(&pose, "art/b.png", false);
    smaller.key ^= 1;
    smaller.width = 12;
    smaller.mask = vec![0xFF; 18];
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![smaller]));
    renderer.set_art(Some(pack));
    renderer.render_hd(&frame, options(4)).unwrap();
    assert!(!renderer.art_stats().poses[0].matched);
}

#[test]
fn sprites_that_show_a_picture_are_never_drawn_from_a_pose_of_the_same_shape() {
    let dir = tempfile::tempdir().unwrap();
    // The art of a pose with the silhouette of the picture's sprites (a red face-sized block):
    // found by shape only, as a colour variant of a pose would be
    write_png(&dir.path().join("art/b.png"), 64, 60, |_, _| [200, 40, 40, 255]);
    // The picture's own art, green
    write_png(&dir.path().join("art/p.png"), 128, 128, |_, _| [20, 200, 20, 255]);
    let (frame, pose) = banded_log(40, 50, [0x03FF, 0x7FE0, 0x7C1F]);
    let mut shape = object(&pose, "art/b.png", false);
    shape.key ^= 1;
    shape.mask = vec![0; pose.arr.len().div_ceil(8)];
    for n in (0..pose.arr.len()).filter(|&n| pose.arr[n] != 0) {
        shape.mask[n / 8] |= 1 << (n % 8);
    }
    // The sprites' four characters are the first four tiles of a portrait asset's picture
    let chars: Vec<[u16; 16]> = [0_usize, 1, 16, 17]
        .iter()
        .map(|&tile| std::array::from_fn(|word| frame.vram[tile * 16 + word]))
        .collect();
    let asset = Asset {
        id: "portrait/16:8000/4bpp".to_owned(),
        bpp: 4,
        mode: Mode::Indexed,
        columns: 16,
        keys: chars.iter().map(|c| word_key(c)).collect(),
        art: String::new(),
    };
    let picture = Picture {
        name: "face".to_owned(),
        asset: asset.id.clone(),
        tiles: std::array::from_fn(|n| n as u16),
        palette: Palette::default(),
        art: "art/p.png".to_owned(),
    };
    let with_picture =
        pack_of(dir.path(), vec![asset.clone()], ArtIndex::new(vec![picture], vec![shape.clone()]));
    let mut renderer = Renderer::new();
    renderer.set_art(Some(with_picture));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    // The sprites' opaque pixels are the picture's art (green), the pose's red nowhere
    let inside = pixel(&drawn, 41 * 4 + 1, 59 * 4 + 1);
    assert_eq!(inside[1], 200, "the picture's art: {inside:?}");
    assert!(
        drawn.rgba.chunks(4).all(|p| p[..3] != [200, 40, 40]),
        "no pixel of the pose that has the same shape"
    );
    assert!(!renderer.art_stats().poses[0].matched, "the sprites are not a pose");

    // The same sprites with no picture art are the pose's, as before (the shape is a variant)
    let without = pack_of(dir.path(), vec![asset], ArtIndex::new(Vec::new(), vec![shape]));
    renderer.set_art(Some(without));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    assert!(renderer.art_stats().poses[0].matched);
    assert_ne!(pixel(&drawn, 41 * 4 + 1, 59 * 4 + 1)[1], 200, "the pose's art, not the picture's");
}

/// A field character of palette 4 (so a "field" group, and an effect by its colours too): a
/// solid black hair of two pieces with a gap, over a body of two colours, as the field's Goku
/// is drawn. Sprites 0 and 1 are the hair (8 x 8 each, three rows of six pixels), sprite 2 the
/// body (16 x 16, colours 2 and 3 in halves) below them.
fn character_log(x: u8, y: u8) -> FrameLog {
    let mut log = log(regs());
    // The hair: character 4, colour 1 in rows 0 to 2, columns 2 to 7
    for row in 0..3 {
        log.vram[4 * 16 + row] = 0x003F;
    }
    // The body: characters 0, 1, 16, 17, colour 2 (plane 1) on the left half, colour 3 on the right
    for tile in [0_usize, 1, 16, 17] {
        for row in 0..8 {
            log.vram[tile * 16 + row] = if tile % 16 == 0 { 0xFF00 } else { 0xFFFF };
        }
    }
    let attr = 4 << 1;
    log.oam_low[0..12].copy_from_slice(&[x, y, 4, attr, x + 8, y, 4, attr, x, y + 8, 0, attr]);
    log.oam_high[0] = 0b10 << 4;
    log.cgram_start[0x80 + 4 * 16 + 1] = 0x0000; // black hair
    log.cgram_start[0x80 + 4 * 16 + 2] = 0x03E0;
    log.cgram_start[0x80 + 4 * 16 + 3] = 0x7C00;
    log
}

#[test]
fn a_dark_flat_part_of_a_character_is_not_a_shadow() {
    let frame = character_log(100, 100);
    let sprites = poses::sprites(&frame);
    let found = poses::poses(&frame, &sprites);
    assert!(
        found.iter().all(|pose| !pose.shadow && pose.excluded == 0),
        "the hair is part of the character, not a ground shadow"
    );
    // The same flat black pieces as a group of their own are no ellipse either
    let hair: Vec<&poses::Sprite> = sprites.iter().filter(|s| s.idx < 2).collect();
    assert_eq!(hair.len(), 2);
    let none = |_: &poses::Sprite, _: usize, _: u8| [0_u8; 3];
    let black = |_: &poses::Sprite, _: usize, _: u8| [0_u8, 0, 0];
    assert!(!poses::assemble(&hair, false, &black).unwrap().shadow);
    assert!(poses::assemble(&hair, true, &none).is_some());

    // Drawn with the art of the character, the hair stays the art's: nothing is drawn
    // translucent over it
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/o.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    let body = found.iter().find(|pose| pose.field).expect("a field group");
    let mut art = object(body, "art/o.png", false);
    art.id = "field-goku-000".to_owned();
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![art]));
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    // The hair's pixel (row 0, column 3 of the first hair piece) is the art's solid blue
    assert_eq!(pixel(&drawn, 103 * 4 + 1, 101 * 4 + 1), [0, 0, 255]);
    let stats = renderer.art_stats();
    assert_eq!(stats.field.art, stats.field.total);
    assert!(stats.field.total > 0);
}

#[test]
fn a_field_group_is_found_by_its_key_among_the_fields_art_only() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/f.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    write_png(&dir.path().join("art/b.png"), 64, 64, |_, _| [255, 0, 0, 255]);
    let (frame, pose) = fighter_log(40, 50);
    // The art of a field group and the art of a battle pose have the same key (the same palette
    // indexes): a field group draws the field's, a fighter the battle's
    let mut field = object(&pose, "art/f.png", false);
    field.id = "field-npc-000".to_owned();
    let battle = object(&pose, "art/b.png", false);
    let pack = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(Vec::new(), vec![field.clone(), battle.clone()]),
    );
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 51 * 4 + 1), [255, 0, 0], "the fighter's art");
    let stats = renderer.art_stats();
    assert_eq!((stats.fighters.art, stats.field.art), (256, 256));
    // A group of palette 4, the same indexes: the field's art
    let mut frame = frame;
    frame.oam_low[3] = 4 << 1;
    frame.cgram_start[0x80 + 4 * 16 + 2] = 0x03E0;
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 51 * 4 + 1), [0, 0, 255], "the field's art");
    let stats = renderer.art_stats();
    assert_eq!((stats.fighters.total, stats.field.art, stats.field.total), (0, 256, 256));
}

#[test]
fn a_field_frame_draws_a_group_from_the_fields_art_though_a_fighter_has_art_for_it_too() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/f.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    write_png(&dir.path().join("art/b.png"), 64, 64, |_, _| [255, 0, 0, 255]);
    let (frame, pose) = fighter_log(40, 50);
    let mut field = object(&pose, "art/f.png", false);
    field.id = "field-npc-000".to_owned();
    let battle = object(&pose, "art/b.png", false);
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![field, battle]));
    let kind_of_first = |known: Option<bool>| {
        let mut art = ArtState::new(Arc::clone(&pack));
        art.begin(256, 224, 4);
        art.field_frame = known;
        let found = art.find_poses(&frame);
        assert_eq!(found.len(), 1, "one of the two poses is drawn");
        found[0].0.field
    };
    // Where nothing says it, a pose of palette 1 with art made for a fighter is a fighter's
    assert!(!kind_of_first(None));
    // In a field frame it is the field's figure, drawn from the field's art
    assert!(kind_of_first(Some(true)));
}

#[test]
fn a_field_group_cut_by_the_edge_is_drawn_from_the_group_that_holds_it() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/w.png"), 64, 32, |x, _| {
        if x < 32 { [255, 0, 0, 255] } else { [0, 0, 255, 255] }
    });
    // A solid group of 16 x 8 pixels at the left edge: its left sprite is off the screen
    // (x = -8) and is left out of the frame's sprites, only the right one is left
    let mut frame = log(regs());
    for row in 0..8 {
        frame.vram[row] = 0xFF00;
    }
    frame.oam_low[0..4].copy_from_slice(&[0, 80, 0, 4 << 1]);
    frame.cgram_start[0x80 + 4 * 16 + 2] = 0x03E0;
    let sprites = poses::sprites(&frame);
    let found = poses::poses(&frame, &sprites);
    let cut = found.iter().find(|pose| pose.field).expect("a field group");
    assert!(cut.cut && cut.arr.iter().filter(|&&c| c != 0).count() >= 48);
    // The index holds the whole group (a 16 x 8 silhouette, made for green), not the cut one's key
    let mut whole = object(cut, "art/w.png", false);
    whole.id = "field-edge-000".to_owned();
    whole.key ^= 1;
    whole.width = 16;
    whole.height = 8;
    whole.mask = vec![0xFF; 16];
    whole.palette = Palette { mask: 0b100, ..Palette::default() };
    whole.palette.colors[2] = [0, 255, 0];
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![whole]));
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    // The sprite's pixels are the group's left half of the silhouette it falls on (the first match
    // of the box): the art's left half, red, recoloured to the palette (which is the same green)
    let stats = renderer.art_stats();
    assert!(stats.poses.iter().any(|p| p.field && p.matched), "{:?}", stats.poses);
    assert_eq!(stats.field.art, stats.field.total);
    assert_ne!(
        pixel(&drawn, 2 * 4 + 1, 82 * 4 + 1),
        [0, 255, 0],
        "the original's pixel is replaced"
    );
    // A pose that is not at the edge is not looked for among the groups that hold it
    let mut inside = frame.clone();
    inside.oam_low[0] = 100;
    let sprites = poses::sprites(&inside);
    let found = poses::poses(&inside, &sprites);
    assert!(!found.iter().find(|pose| pose.field).unwrap().cut);
    renderer.render_hd(&inside, options(4)).unwrap();
    assert!(!renderer.art_stats().poses.iter().any(|p| p.matched));
}

/// The wide renderer's tally of `frame` with `pack`'s art: a picture frame (the game mode of a
/// battle's command screen, no field), as the tour of the game counts it.
fn tallied(pack: Arc<Pack>, frame: &FrameLog) -> crate::FrameTally {
    let mut renderer = WideRenderer::new();
    renderer.set_art(Some(pack));
    renderer.set_tally(true);
    let options = WideOptions { width: 256, hud: HudPlacement::Window };
    renderer.render_hd(Rom(&[]), 0x02, None, frame, options, self::options(1)).unwrap();
    renderer.tally()
}

#[test]
fn the_tally_says_what_the_art_draws_and_what_it_leaves() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/o.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    write_png(&dir.path().join("art/p.png"), 128, 128, |_, _| [255, 0, 0, 255]);
    let (mut frame, pose) = fighter_log(100, 100);
    // Three characters on BG1 at map row 1 (frame row 7): tile 1 of the portrait asset, which a
    // picture covers; tile 2 of it, which no picture does; and a character no asset has
    let (one, two, three) = (character(), [0xFF00, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], {
        let mut chars = [0_u16; 16];
        chars[0] = 0xFFFF;
        chars
    });
    for (n, chars) in [one, two, three].iter().enumerate() {
        frame.vram[0x1000 + 16 * (n + 1)..0x1000 + 16 * (n + 2)].copy_from_slice(chars);
        frame.vram[0x0400 + 32 + n] = n as u16 + 1;
    }
    frame.cgram_start[1] = 0x001F;
    frame.cgram_start[2] = 0x03E0;
    frame.cgram_start[3] = 0x7C00;
    let mut keys = vec![0_u64; 16];
    keys[1] = word_key(&one);
    keys[2] = word_key(&two);
    let asset = Asset {
        id: "portrait/16:8000/4bpp".to_owned(),
        bpp: 4,
        mode: Mode::Indexed,
        columns: 16,
        keys,
        art: String::new(),
    };
    let mut tiles = [u16::MAX; 16];
    tiles[0] = 1;
    let art = ArtIndex::new(
        vec![Picture {
            name: "obj-00".to_owned(),
            asset: asset.id.clone(),
            tiles,
            palette: Palette::default(),
            art: "art/p.png".to_owned(),
        }],
        vec![object(&pose, "art/o.png", false)],
    );
    let tally = tallied(pack_of(dir.path(), vec![asset], art), &frame);

    let counted = |layer, asset, art, pose| -> u64 {
        tally
            .counts
            .iter()
            .filter(|c| (c.layer, c.asset, c.art, c.pose) == (layer, asset, art, pose))
            .map(|c| c.pixels)
            .sum()
    };
    // The picture draws tile 1; tile 2 is bound and has none; the pose is drawn from art
    assert_eq!(counted(TallyLayer::Bg, Some(0), true, None), 8, "tile 1 draws its picture");
    assert_eq!(counted(TallyLayer::Bg, Some(0), false, None), 8, "tile 2 has no picture");
    assert_eq!(counted(TallyLayer::Bg, None, false, None), 8, "the character no asset has");
    assert_eq!(counted(TallyLayer::Obj, None, true, Some((true, Fate::Art))), 256);
    assert_eq!(tally.pixels(), 24 + 256);
    assert_eq!(tally.art_pixels(), 8 + 256);
    // What is left: the tile with no picture and the unbound character, with their boxes
    assert_eq!(tally.items.len(), 2, "{:?}", tally.items);
    let tile = tally.items.iter().find(|i| matches!(i.key, ItemKey::Tile { .. })).unwrap();
    assert_eq!(tile.key, ItemKey::Tile { asset: 0, tile: 2, object: false });
    assert_eq!((tile.pixels, tile.bbox), (8, (8, 7, 15, 7)));
    let unbound = tally.items.iter().find(|i| matches!(i.key, ItemKey::Char { .. })).unwrap();
    let key = word_key(&[0xFFFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    assert_eq!(unbound.key, ItemKey::Char { object: false, bpp: 4, key });
    assert_eq!(unbound.bbox, (16, 7, 23, 7));
}

#[test]
fn a_pose_the_index_lacks_is_listed_by_its_key_and_fate() {
    let dir = tempfile::tempdir().unwrap();
    let (frame, pose) = fighter_log(40, 50);
    let mut other = object(&pose, "art/none.png", false);
    other.key ^= 1;
    let tally =
        tallied(pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![other])), &frame);
    assert_eq!(tally.art_pixels(), 0);
    assert_eq!(tally.items.len(), 1);
    assert_eq!(
        tally.items[0].key,
        ItemKey::Pose { key: pose.key, fighter: true, fate: Fate::Missing }
    );
    assert_eq!((tally.items[0].pixels, tally.items[0].bbox), (256, (40, 50, 55, 65)));
}

#[test]
fn a_tally_draws_no_art() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/o.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    let (frame, pose) = fighter_log(40, 50);
    let pack = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(Vec::new(), vec![object(&pose, "art/o.png", false)]),
    );
    let mut renderer = WideRenderer::new();
    renderer.set_art(Some(pack));
    let wide = WideOptions { width: 256, hud: HudPlacement::Window };
    let with_art = renderer.render_hd(Rom(&[]), 0x02, None, &frame, wide, options(1)).unwrap();
    renderer.set_tally(true);
    let counted = renderer.render_hd(Rom(&[]), 0x02, None, &frame, wide, options(1)).unwrap();
    // The art was drawn before and is not now: the picture is the look's, the pixels counted
    assert_eq!(pixel(&with_art.picture, 41, 51), [0, 0, 255]);
    assert_eq!(pixel(&counted.picture, 41, 51), [0, 0xFF, 0]);
    assert_eq!(renderer.tally().art_pixels(), 256);
}

/// A pack with one background layer: BG4 of mode 0 (map `$0400`, characters `$1000`, one screen), the
/// tile rows 0 to 4, whose art is the colour (game x, game y, 0) of each game pixel at 4x.
fn background_pack(dir: &std::path::Path, vram: &[u16], fill: bool) -> Arc<Pack> {
    let key = crate::bgart::layer_key(vram, 0x0400, 0x1000, 0, false, 2, [0, 4]);
    write_png(&dir.join("art/bg/test.png"), 1024, 128, |x, y| {
        [((x / 4) % 256) as u8, ((y / 4) % 256) as u8, 0, 255]
    });
    write_png(&dir.join("art/bg/test-src.png"), 256, 32, |_, _| [255, 0, 0, 255]);
    let mut pack = Arc::try_unwrap(pack_of(dir, Vec::new(), ArtIndex::default())).unwrap();
    pack.bg = crate::bgart::Index {
        layer: vec![crate::bgart::Entry {
            id: "test".to_owned(),
            key: format!("{key:016x}"),
            bg: 3,
            mode: 0,
            map_base: 0x0400,
            char_base: 0x1000,
            size: 0,
            rows: [0, 4],
            fill,
            art: "art/bg/test.png".to_owned(),
            source: "art/bg/test-src.png".to_owned(),
            palette_key: String::new(),
            palette: String::new(),
        }],
    };
    Arc::new(pack)
}

/// BG4 (mode 0) showing the solid tile 1 on every tile of its map, in red.
fn background_frame(h_scroll: u16) -> FrameLog {
    let mut regs = regs();
    regs.bg_mode = 0;
    regs.main_bg = [false, false, false, true];
    regs.main_obj = false;
    regs.bg_map_base = [0, 0, 0, 0x0400];
    regs.bg_char_base = [0, 0, 0, 0x1000];
    regs.bg_h_scroll[3] = h_scroll;
    let mut log = log(regs);
    for word in 0..8 {
        log.vram[0x1000 + 8 + word] = 0x00FF;
    }
    for tile in 0..1024 {
        log.vram[0x0400 + tile] = 1;
    }
    log.cgram_start[0x61] = 0x001F;
    log
}

#[test]
fn a_background_layer_is_drawn_from_its_art_at_the_scroll_of_each_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = background_frame(10);
    let pack = background_pack(dir.path(), &log.vram, true);
    let mut renderer = Renderer::new();
    renderer.set_art(Some(Arc::clone(&pack)));
    let picture = renderer.render_hd(&log, options(4)).unwrap();
    // The first frame row is scanline 1: the layer's row 1; the pixel 0 shows the layer's pixel 10
    assert_eq!(pixel(&picture, 0, 0), [10, 1, 0]);
    assert_eq!(pixel(&picture, 3, 3), [10, 1, 0]);
    assert_eq!(pixel(&picture, 4, 4), [11, 2, 0]);
    // Past the art's rows (32 pixels) the tiles are drawn as the look does
    let below = pixel(&picture, 8, 40 * 4);
    assert_eq!(below, [255, 0, 0]);
    // The scroll wraps with the layer, and each line has its own: line 100 scrolls by 250
    for entry in &mut log.entries[99..] {
        entry.regs.bg_h_scroll[3] = 250;
    }
    let picture = renderer.render_hd(&log, options(4)).unwrap();
    assert_eq!(pixel(&picture, 0, 0), [10, 1, 0]);
    assert_eq!(renderer.art_stats().bg_pixels, 0, "the layer is not a battle background");
    // A pixel at line 10 (frame row 9)
    let again = pixel(&picture, 8, 9 * 4);
    assert_eq!(again, [12, 10, 0]);
    // The pixel after the wrap: x 6 of a line that scrolls 250 shows the layer's pixel 0
    let mut wrapped = background_frame(250);
    wrapped.vram.copy_from_slice(&log.vram);
    let picture = renderer.render_hd(&wrapped, options(4)).unwrap();
    assert_eq!(pixel(&picture, 6 * 4, 0), [0, 1, 0]);
}

#[test]
fn a_faded_palette_dims_the_background_art_by_the_same_ratio() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = background_frame(0);
    let pack = background_pack(dir.path(), &log.vram, true);
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    // The game's red is half as bright (the art was made for full red): art colour (100, 0, 0)
    log.cgram_start[0x61] = 0x000F;
    let picture = renderer.render_hd(&log, options(4)).unwrap();
    // game x 100, row 1: art (100, 1, 0); the source red 255 became 123: a fade, drawn as a plain
    // blend of the art towards black
    let [r, g, _] = pixel(&picture, 100 * 4, 0);
    assert!((46..=50).contains(&r), "red {r}");
    assert!(g <= 1, "green {g}");
}

#[test]
fn art_made_for_the_palette_in_use_is_bound_before_the_art_of_the_content() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = background_frame(0);
    let mut pack = Arc::try_unwrap(background_pack(dir.path(), &log.vram, true)).unwrap();
    // A variant of the same content made for the green palette: its art is blue
    write_png(&dir.path().join("art/bg/green.png"), 1024, 128, |_, _| [0, 0, 200, 255]);
    write_png(&dir.path().join("art/bg/green-src.png"), 256, 32, |_, _| [0, 255, 0, 255]);
    let key = |log: &FrameLog| {
        crate::bgart::palette_key(
            &log.vram,
            &log.cgram_start,
            3,
            0,
            0x0400,
            0,
            false,
            2,
            [0, 4],
            true,
        )
    };
    let mut green_log = log.clone();
    green_log.cgram_start[0x61] = 0x03E0;
    let mut variant = pack.bg.layer[0].clone();
    variant.id = "green".to_owned();
    variant.art = "art/bg/green.png".to_owned();
    variant.source = "art/bg/green-src.png".to_owned();
    variant.palette_key = format!("{:016x}", key(&green_log));
    // (the variant is listed first: the order of the index does not decide)
    pack.bg.layer.insert(0, variant);
    let mut renderer = Renderer::new();
    renderer.set_art(Some(Arc::new(pack)));
    // The green palette: the variant's art as it is
    let picture = renderer.render_hd(&green_log, options(4)).unwrap();
    assert_eq!(pixel(&picture, 100 * 4, 0), [0, 0, 200]);
    // The palette of the base art: the base art
    let picture = renderer.render_hd(&log, options(4)).unwrap();
    assert_eq!(pixel(&picture, 100 * 4, 0), [100, 1, 0]);
    // A palette nobody made art for: the base art, in the colours in use (not the variant's)
    log.cgram_start[0x61] = 0x7C00;
    let picture = renderer.render_hd(&log, options(4)).unwrap();
    let [r, _, b] = pixel(&picture, 100 * 4, 0);
    assert!(b > r, "blue palette, base art recoloured: {r} {b}");
    assert!(b != 200);
}

#[test]
fn a_palette_the_art_was_made_in_recolours_it_as_the_source_picture_does() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = background_frame(0);
    let with_source = background_pack(dir.path(), &log.vram, true);
    // The same pack with no source picture, but the CGRAM the art was made in: entry 0x61 red
    let mut pack = Arc::try_unwrap(background_pack(dir.path(), &log.vram, true)).unwrap();
    pack.bg.layer[0].source = String::new();
    let mut made_in = [0_u16; 256];
    made_in[0x61] = 0x001F;
    pack.bg.layer[0].palette = crate::bgart::palette_text(&made_in);
    assert_eq!(crate::bgart::parse_palette(&pack.bg.layer[0].palette), Some(made_in));
    let mut by_source = Renderer::new();
    by_source.set_art(Some(with_source));
    let mut by_palette = Renderer::new();
    by_palette.set_art(Some(Arc::new(pack)));
    // The palette the art was made in, a faded one and a flash to another hue: the same pictures
    for word in [0x001F_u16, 0x000F, 0x7C00, 0x03E0] {
        log.cgram_start[0x61] = word;
        let a = by_source.render_hd(&log, options(4)).unwrap();
        let b = by_palette.render_hd(&log, options(4)).unwrap();
        assert_eq!(a.rgba, b.rgba, "palette word {word:04X}");
    }
}

#[test]
fn a_palette_text_is_256_words_of_four_hex_digits() {
    let mut colours = [0_u16; 256];
    colours[3] = 0x7FFF;
    colours[255] = 0x0123;
    assert_eq!(crate::bgart::parse_palette(&crate::bgart::palette_text(&colours)), Some(colours));
    assert_eq!(crate::bgart::parse_palette(""), None);
    assert_eq!(crate::bgart::parse_palette(&"G".repeat(1024)), None);
    assert_eq!(crate::bgart::parse_palette(&"0".repeat(1020)), None);
}

#[test]
fn a_background_pack_without_a_source_picture_draws_the_art_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = background_frame(0);
    let mut pack = Arc::try_unwrap(background_pack(dir.path(), &log.vram, true)).unwrap();
    pack.bg.layer[0].source = String::new();
    let mut renderer = Renderer::new();
    renderer.set_art(Some(Arc::new(pack)));
    // The palette the art was made for, and a faded one: no source, so no recolouring either way
    let picture = renderer.render_hd(&log, options(4)).unwrap();
    assert_eq!(pixel(&picture, 100 * 4, 0), [100, 1, 0]);
    log.cgram_start[0x61] = 0x000F;
    let picture = renderer.render_hd(&log, options(4)).unwrap();
    assert_eq!(pixel(&picture, 100 * 4, 0), [100, 1, 0]);
}

#[test]
fn a_frame_whose_layer_is_not_in_the_index_is_drawn_as_the_look() {
    let dir = tempfile::tempdir().unwrap();
    let log = background_frame(0);
    let pack = background_pack(dir.path(), &log.vram, true);
    let mut other = log.clone();
    other.vram[0x0400] = 0; // another map: another content
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    let picture = renderer.render_hd(&other, options(4)).unwrap();
    assert_eq!(pixel(&picture, 40, 40), [255, 0, 0]);
    // and in the same renderer the known layer is still found
    let picture = renderer.render_hd(&log, options(4)).unwrap();
    assert_eq!(pixel(&picture, 40, 40), [10, 11, 0]);
}

/// A Mode 7 frame: BG1 alone, the identity matrix, every tile of the plane the tile 1 whose pixels
/// are the colour index 5 (CGRAM 5 is `0x001F`, full red).
fn plane_frame() -> FrameLog {
    let mut regs = regs();
    regs.bg_mode = 7;
    regs.main_obj = false;
    regs.m7_a = 0x0100;
    regs.m7_d = 0x0100;
    let mut log = log(regs);
    for word in &mut log.vram[..128 * 128] {
        *word = 1;
    }
    for pixel in 0..64 {
        log.vram[64 + pixel] = 5 << 8;
    }
    log.cgram_start[5] = 0x001F;
    log
}

/// A pack with the art of the plane of `vram`: the colour (game x / 4, game y / 4, 77) of each game
/// pixel at 4x, made in the colour 5 = full red.
fn plane_pack(dir: &std::path::Path, vram: &[u16]) -> Arc<Pack> {
    plane_pack_with(dir, vram, |_, _| 255)
}

/// [`plane_pack`] where the art of the tile `(column, row)` has the alpha `alpha`.
fn plane_pack_with(
    dir: &std::path::Path,
    vram: &[u16],
    alpha: impl Fn(u32, u32) -> u8,
) -> Arc<Pack> {
    use crate::mode7art;
    for ty in 0..4_u32 {
        for tx in 0..4_u32 {
            write_png(&dir.join(format!("art/mode7/test/t{tx}_{ty}.png")), 1024, 1024, |x, y| {
                [
                    (((tx * 1024 + x) / 16) % 256) as u8,
                    (((ty * 1024 + y) / 16) % 256) as u8,
                    77,
                    alpha(tx, ty),
                ]
            });
        }
    }
    let mut pack = Arc::try_unwrap(pack_of(dir, Vec::new(), ArtIndex::default())).unwrap();
    pack.mode7 = mode7art::Index {
        plane: vec![mode7art::Entry {
            id: "test".to_owned(),
            key: format!("{:016x}", mode7art::plane_key(vram)),
            tiles: "test/t{x}_{y}.png".to_owned(),
            palette: vec!["FF0000".to_owned(); 256],
        }],
    };
    Arc::new(pack)
}

#[test]
fn the_mode_7_plane_is_drawn_from_its_art_through_the_matrix_of_each_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = plane_frame();
    let pack = plane_pack(dir.path(), &log.vram);
    let mut renderer = Renderer::new();
    renderer.set_art(Some(Arc::clone(&pack)));
    let picture = renderer.render_hd(&log, options(4)).unwrap();
    // Picture pixel (36, 40): the plane's pixel (9, 11) (the first frame row is scanline 1): the art
    // there is (9 * 4 / 16, 11 * 4 / 16) = (2, 2) in a block of 16 art pixels
    assert_eq!(pixel(&picture, 36, 40), [2, 2, 77]);
    assert_eq!(pixel(&picture, 100, 100), [6, 6, 77]);
    // A line that zooms out by two (a = d = 0x200) from line 100 shows the plane twice as far along
    for entry in &mut log.entries[99..] {
        entry.regs.m7_a = 0x0200;
        entry.regs.m7_d = 0x0200;
    }
    let picture = renderer.render_hd(&log, options(4)).unwrap();
    assert_eq!(pixel(&picture, 36, 40), [2, 2, 77], "the lines before the change are as they were");
    // Row 400 is line 101: plane pixel (2 * 25, 2 * 101) at picture column 100
    assert_eq!(pixel(&picture, 100, 400), [12, 50, 77]);
}

#[test]
fn a_faded_plane_dims_the_art_and_an_unknown_plane_is_the_look() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = plane_frame();
    let pack = plane_pack(dir.path(), &log.vram);
    let mut renderer = Renderer::new();
    renderer.set_art(Some(Arc::clone(&pack)));
    // The game's red is half as bright: the art (2, 2, 77) takes the same ratio
    log.cgram_start[5] = 0x000F;
    let [r, g, b] = pixel(&renderer.render_hd(&log, options(4)).unwrap(), 36, 40);
    assert!((38..=44).contains(&b), "blue {b}");
    assert!(r < 140 && g <= 4, "{r} {g}");
    // Another plane: the colour the game shows (red)
    let mut other = plane_frame();
    other.vram[0] = 2;
    let picture = renderer.render_hd(&other, options(4)).unwrap();
    assert_eq!(pixel(&picture, 36, 40), [255, 0, 0]);
}

#[test]
fn a_flight_continues_the_plane_art_past_the_window_in_the_wide_view() {
    let dir = tempfile::tempdir().unwrap();
    let log = plane_frame();
    let pack = plane_pack(dir.path(), &log.vram);
    let mut wide = WideRenderer::new();
    wide.set_art(Some(pack));
    let options_wide = WideOptions { width: 372, hud: HudPlacement::Window };
    let view = wide.render_hd(Rom(&[]), 0x07, None, &log, options_wide, options(2)).unwrap();
    assert_eq!(view.scene, crate::world::Scene::Flight);
    // The window starts at column 58 of the wide frame (116 picture pixels). Picture column 160 is
    // window pixel 22 (2 picture pixels each): the art (22 * 4 / 16 = 5, 5) is at row 40 (plane y 21)
    assert_eq!(pixel(&view.picture, 160, 40), [5, 5, 77]);
    // Column 20 is window pixel -48: the plane wraps around (1024 - 48 = 976: 976 * 4 / 16 = 244)
    assert_eq!(pixel(&view.picture, 20, 40), [244, 5, 77]);
}

#[test]
fn a_texel_of_the_plane_without_art_is_the_look() {
    let dir = tempfile::tempdir().unwrap();
    let log = plane_frame();
    // The tile 0 of the art (plane pixels 0 to 255) has no art: the game's red shows there
    let pack =
        plane_pack_with(dir.path(), &log.vram, |tx, ty| if (tx, ty) == (0, 0) { 0 } else { 255 });
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    assert_eq!(pixel(&renderer.render_hd(&log, options(4)).unwrap(), 36, 40), [255, 0, 0]);
}

/// A fighter of one 16 x 16 sprite whose tiles hold the left four columns of colour 2 (so its
/// pose is not its own mirror image), drawn flipped left to right or not.
fn lopsided_fighter(flipped: bool) -> (FrameLog, Pose) {
    let (mut log, _) = fighter_log(40, 50);
    // The left tiles hold four columns, the right ones two
    for (tile, word) in [(0_usize, 0xF000_u16), (16, 0xF000), (1, 0xC000), (17, 0xC000)] {
        for row in 0..8 {
            log.vram[tile * 16 + row] = word;
        }
    }
    if flipped {
        log.oam_low[3] |= 0x40;
    }
    let sprites = poses::sprites(&log);
    let found: Vec<Pose> =
        poses::poses(&log, &sprites).into_iter().filter(|pose| !pose.field).collect();
    assert_eq!(found.len(), 1);
    (log, found.into_iter().next().unwrap())
}

#[test]
fn a_fighter_drawn_flipped_is_drawn_from_the_art_of_its_mirror_image_mirrored() {
    let dir = tempfile::tempdir().unwrap();
    let (plain_log, plain) = lopsided_fighter(false);
    let (flipped_log, flipped) = lopsided_fighter(true);
    assert_ne!(plain.key, flipped.key, "the two facings are two poses");
    assert_eq!(plain.mirror_key(), Some(flipped.key));
    // The art of the pose facing right: red on its left half, blue on its right half
    let half = (plain.w * 4 / 2) as u32;
    write_png(&dir.path().join("art/o.png"), (plain.w * 4) as u32, (plain.h * 4) as u32, |x, _| {
        if x < half { [255, 0, 0, 255] } else { [0, 0, 255, 255] }
    });
    let mut held = object(&plain, "art/o.png", false);
    held.palette = Palette { mask: 0b100, ..Palette::default() };
    held.palette.colors[2] = [0, 255, 0];
    held.mask = vec![0xFF; 32];
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![held]));
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));

    // The pose as the index has it: its left columns show the art's left (red)
    let drawn = renderer.render_hd(&plain_log, options(4)).unwrap();
    assert_eq!(pixel(&drawn, 40 * 4 + 1, 55 * 4 + 1), [255, 0, 0]);
    assert_eq!(renderer.art_stats().poses[0].source, crate::ArtSource::Exact);

    // Flipped, it has no art of its own: the mirrored art is drawn, its columns where the
    // flipped sprite's are (the box starts at the art's right end: blue, and ends red)
    let drawn = renderer.render_hd(&flipped_log, options(4)).unwrap();
    let stats = renderer.art_stats();
    assert!(stats.poses[0].matched);
    assert_eq!(stats.poses[0].source, crate::ArtSource::Mirror);
    assert_eq!(pixel(&drawn, 46 * 4 + 1, 55 * 4 + 1), [0, 0, 255]);
    assert_eq!(pixel(&drawn, 52 * 4 + 1, 55 * 4 + 1), [255, 0, 0]);
}

#[test]
fn units_of_one_pose_are_one_unit_in_a_frame() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/a.png"), 64, 64, |_, _| [255, 0, 0, 255]);
    write_png(&dir.path().join("art/b.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    // Two fighters of the same pixels, one in each fighter palette (a formation of one enemy
    // whose units are told apart by the colours)
    let (mut frame, pose) = fighter_log(40, 50);
    frame.oam_low[4..8].copy_from_slice(&[120, 50, 0, 2 << 1]);
    frame.oam_high[0] = 0b1010;
    frame.cgram_start[0x80 + 32 + 2] = 0x7D00; // palette 2, colour 2: blue-ish
    let unit = |id: &str, art: &str, colour: [u8; 3]| {
        let mut held = object(&pose, art, false);
        held.id = id.to_owned();
        held.palette = Palette { mask: 0b100, ..Palette::default() };
        held.palette.colors[2] = colour;
        held.mask = vec![0xFF; 32];
        held
    };
    // Unit a was made for green and unit b for blue: the first fighter (green) is a's by colour,
    // the second (blue) is b's by colour, and the frame draws both from the unit that fits best
    let index = ArtIndex::new(
        Vec::new(),
        vec![
            unit("battle-a-000", "art/a.png", [0, 255, 0]),
            unit("battle-b-000", "art/b.png", [0, 0, 255]),
        ],
    );
    let pack = pack_of(dir.path(), Vec::new(), index);
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    renderer.render_hd(&frame, options(4)).unwrap();
    let stats = renderer.art_stats();
    let fighters: Vec<_> = stats.poses.iter().filter(|p| p.fighter).collect();
    assert_eq!(fighters.len(), 2);
    assert_eq!(fighters[0].art, Some(0), "unit a's art");
    assert_eq!(
        fighters[0].art, fighters[1].art,
        "the two fighters are drawn from the art of one unit"
    );
    assert!(fighters[1].recoloured, "in the colours of its own palette");
}

#[test]
fn a_fighter_drawn_with_a_later_palette_is_still_a_fighter_in_a_frame_of_fighters() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/o.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    // The game draws the third enemy of a formation with the palette after the two fighter ones
    let (mut frame, pose) = fighter_log(40, 50);
    frame.oam_low[3] = 3 << 1;
    frame.cgram_start[0x80 + 3 * 16 + 2] = 0x03E0;
    let mut held = object(&pose, "art/o.png", false);
    held.palette = Palette { mask: 0b100, ..Palette::default() };
    held.palette.colors[2] = [0, 255, 0];
    held.mask = vec![0xFF; 32];
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![held]));
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    // Alone, a group of that palette is the field's (or an effect's), not a fighter
    renderer.render_hd(&frame, options(4)).unwrap();
    assert!(!renderer.art_stats().poses.iter().any(|p| p.fighter && p.palette == 3));
    // Beside a fighter of palette 1 that has art, it is the formation's third unit
    frame.oam_low[4..8].copy_from_slice(&[120, 50, 0, 1 << 1]);
    frame.oam_high[0] = 0b1010;
    frame.cgram_start[0x80 + 16 + 2] = 0x03E0;
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 55 * 4 + 1), [0, 0, 255]);
    let stats = renderer.art_stats();
    assert!(
        stats.poses.iter().any(|p| p.fighter && p.palette == 3 && p.matched),
        "{:?}",
        stats.poses
    );
}

/// A frame of 239 lines (the overscan some scenes use) with one effect sprite (palette 4) of 8 x 8
/// pixels in colour 1 (white) at `(x, y)`, the OAM `y` being 224 or more.
fn overscan_log(x: u8, y: u8) -> FrameLog {
    let mut log = log(regs());
    log.height = 239;
    log.entries = (1..=239_u16)
        .map(|scanline| LineEntry {
            scanline,
            from_pixel: 0,
            regs: regs(),
            sprites: SpriteLine { eval_line: scanline - 1, rebuilt: true, ..SpriteLine::default() },
        })
        .collect();
    log.oam_low[0] = x;
    log.oam_low[1] = y;
    log.oam_low[3] = 4 << 1;
    for row in 0..8 {
        log.vram[row] = 0x00FF;
    }
    log.cgram_start[0x80 + 4 * 16 + 1] = 0x7FFF;
    log
}

#[test]
fn the_sprites_under_line_224_are_poses_of_their_own_found_by_their_exact_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let frame = overscan_log(40, 232);
    assert!(poses::sprites(&frame).is_empty(), "the capture kept no sprite under line 224");
    let low = poses::overscan_sprites_in(&frame, &poses::View::window());
    assert_eq!(low.len(), 1);
    let found = poses::overscan_effects(&frame, &low);
    assert_eq!(found.len(), 1);
    let pose = &found[0];
    assert_eq!((pose.w, pose.h, pose.org), (8, 8, (40, 233)));
    write_png(&dir.path().join("art/l.png"), 32, 32, |_, _| [0, 0, 255, 255]);
    let pack = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(Vec::new(), vec![object(pose, "art/l.png", false)]),
    );
    let mut renderer = Renderer::new();
    let plain = renderer.render_hd(&frame, options(4)).unwrap();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&plain, 41 * 4 + 1, 234 * 4 + 1), [255, 255, 255]);
    assert_eq!(
        pixel(&drawn, 41 * 4 + 1, 234 * 4 + 1),
        [0, 0, 255],
        "the band is drawn from the art"
    );
    assert_eq!(pixel(&drawn, 5, 5), pixel(&plain, 5, 5));
    // Art for other pixels leaves the sprite as the game draws it
    let mut other = object(pose, "art/l.png", false);
    other.key ^= 1;
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![other]));
    renderer.set_art(Some(pack));
    assert_eq!(renderer.render_hd(&frame, options(4)).unwrap(), plain);
}

#[test]
fn a_field_figure_that_runs_under_line_224_is_drawn_whole_from_the_art_of_its_pieces_together() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/b.png"), 32, 72, |_, _| [0, 0, 255, 255]);
    // Two solid 8 x 8 pieces of one figure: one above line 224, one wholly under it (OAM y 224)
    let mut frame = overscan_log(40, 215);
    frame.oam_low[4..8].copy_from_slice(&[40, 224, 0, 4 << 1]);
    let sprites = poses::sprites(&frame);
    let band = poses::overscan_sprites_in(&frame, &poses::View::window());
    assert_eq!((sprites.len(), band.len()), (1, 1));
    let view = poses::View::window();
    let found = poses::poses(&frame, &sprites);
    assert_eq!(found.iter().filter(|pose| pose.field).count(), 1);
    // The art is made for the two pieces together
    let whole: Vec<&poses::Sprite> = sprites.iter().chain(band.iter()).collect();
    let whole = poses::field_pose(&whole, &frame, &view, 1).unwrap();
    let mut art = object(&whole, "art/b.png", false);
    art.id = "field-band-000".to_owned();
    let found =
        poses::join_band(found, &sprites, &band, &frame, &view, &mut |pose| pose.key == art.key);
    let joined: Vec<_> = found.iter().filter(|pose| pose.field).collect();
    assert_eq!(joined.len(), 1);
    assert_eq!(joined[0].sprites, 0b11, "the piece under the line is the figure's");
    // Without art for the whole, the pieces stay apart
    let found = poses::poses(&frame, &sprites);
    let found = poses::join_band(found, &sprites, &band, &frame, &view, &mut |_| false);
    assert_eq!(found.iter().find(|pose| pose.field).unwrap().sprites, 0b1);
    // The renderer draws both pieces from the art
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![art]));
    let mut renderer = Renderer::new();
    let plain = renderer.render_hd(&frame, options(4)).unwrap();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&plain, 41 * 4 + 1, 228 * 4 + 1), [255, 255, 255]);
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 228 * 4 + 1), [0, 0, 255]);
}

#[test]
fn an_effect_piece_under_line_224_is_placed_beside_the_band_pieces_a_figure_took() {
    let dir = tempfile::tempdir().unwrap();
    write_png(&dir.path().join("art/f.png"), 32, 72, |_, _| [0, 0, 255, 255]);
    write_png(&dir.path().join("art/e.png"), 32, 64, |_, _| [255, 0, 0, 255]);
    // A figure of palette 4 (a piece above line 224 and one under it) and a piece of palette 3
    // under it, within 4 pixels of the figure's: the effect cluster holds both lower pieces
    let mut frame = overscan_log(40, 215);
    frame.oam_low[4..8].copy_from_slice(&[40, 224, 0, 4 << 1]);
    frame.oam_low[8..12].copy_from_slice(&[40, 234, 0, 3 << 1]);
    frame.cgram_start[0x80 + 3 * 16 + 1] = 0x7FFF;
    let view = poses::View::window();
    let sprites = poses::sprites(&frame);
    let band = poses::overscan_sprites_in(&frame, &view);
    let whole: Vec<&poses::Sprite> = sprites.iter().chain(band.iter().take(1)).collect();
    let whole = poses::field_pose(&whole, &frame, &view, 1).unwrap();
    let mut figure = object(&whole, "art/f.png", false);
    figure.id = "field-band-001".to_owned();
    let effects = poses::overscan_effects(&frame, &band);
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].sprites, 0b110, "the cluster holds the figure's lower piece");
    let effect = object(&effects[0], "art/e.png", false);
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), vec![figure, effect]));
    let mut renderer = Renderer::new();
    let plain = renderer.render_hd(&frame, options(4)).unwrap();
    renderer.set_art(Some(pack));
    let drawn = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 228 * 4 + 1), [0, 0, 255], "the figure's piece");
    assert_eq!(pixel(&plain, 41 * 4 + 1, 236 * 4 + 1), [255, 255, 255]);
    assert_eq!(pixel(&drawn, 41 * 4 + 1, 236 * 4 + 1), [255, 0, 0], "the effect's own piece");
}

/// A pack of two portrait streams, each with one picture over its first 16 tiles (the keys of the
/// tiles are told apart by `seed`), and the frame whose BG1 shows a 4 x 4 block of characters at
/// map cell (2, 2): the tile of cell `n` is `tiles(n)`, a stream and a tile number.
fn portrait_block(
    dir: &std::path::Path,
    tiles: impl Fn(usize) -> (usize, usize),
) -> (Arc<Pack>, FrameLog) {
    write_png(&dir.join("art/a.png"), 128, 128, |_, _| [10, 0, 0, 255]);
    write_png(&dir.join("art/b.png"), 128, 128, |_, _| [0, 10, 0, 255]);
    let character_of = |stream: usize, tile: usize| {
        let mut chars = character();
        chars[1] = (stream * 100 + tile + 1) as u16;
        chars
    };
    let asset = |stream: usize, id: &str| Asset {
        id: id.to_owned(),
        bpp: 4,
        mode: Mode::Indexed,
        columns: 16,
        keys: (0..16).map(|tile| word_key(&character_of(stream, tile))).collect(),
        art: String::new(),
    };
    let assets = vec![asset(0, "portrait/16:8A9B/4bpp"), asset(1, "portrait/16:A780/4bpp")];
    let picture = |name: &str, asset: &str, art: &str| Picture {
        name: name.to_owned(),
        asset: asset.to_owned(),
        tiles: std::array::from_fn(|n| n as u16),
        palette: Palette::default(),
        art: art.to_owned(),
    };
    let art = ArtIndex::new(
        vec![
            picture("face-8A9B-1", "portrait/16:8A9B/4bpp", "art/a.png"),
            picture("face-A780-1", "portrait/16:A780/4bpp", "art/b.png"),
        ],
        Vec::new(),
    );
    let mut log = log(regs());
    for cell in 0..16_usize {
        let (stream, tile) = tiles(cell);
        // Character `cell + 1` of the base holds the tile
        let at = 0x1000 + (cell + 1) * 16;
        log.vram[at..at + 16].copy_from_slice(&character_of(stream, tile));
        log.vram[0x0400 + (2 + cell / 4) * 32 + 2 + cell % 4] = (cell + 1) as u16;
    }
    (pack_of(dir, assets, art), log)
}

#[test]
fn a_portrait_drawn_from_one_stream_is_whole() {
    let dir = tempfile::tempdir().unwrap();
    let (pack, log) = portrait_block(dir.path(), |cell| (0, cell));
    let tally = tallied(pack, &log);
    assert_eq!(tally.portraits.len(), 1, "{:?}", tally.portraits);
    let portrait = &tally.portraits[0];
    assert_eq!(portrait.cells, 0xFFFF);
    assert_eq!(portrait.fault(), None);
}

#[test]
fn a_portrait_whose_halves_are_two_streams_is_drawn_whole_from_one() {
    let dir = tempfile::tempdir().unwrap();
    // The top two rows of one character, the bottom two of another: the game's VRAM mixed two faces
    let (pack, log) = portrait_block(dir.path(), |cell| (usize::from(cell >= 8), cell));
    let tally = tallied(pack, &log);
    assert_eq!(tally.portraits.len(), 1, "{:?}", tally.portraits);
    let portrait = &tally.portraits[0];
    assert_eq!(portrait.pictures.len(), 1, "one picture draws all of it: {portrait:?}");
    assert_eq!(portrait.cells, 0xFFFF, "every cell");
    assert_eq!(portrait.bound_assets.len(), 2, "the characters were of two streams");
    assert_eq!(portrait.rows.len(), 1);
    // (the audit still says what the game showed)
    let fault = portrait.fault().unwrap();
    assert!(fault.contains("characters are of") && fault.contains("16:A780"), "{fault}");
}

#[test]
fn a_portrait_whose_halves_use_two_palette_rows_is_drawn_in_one() {
    let dir = tempfile::tempdir().unwrap();
    let (pack, mut log) = portrait_block(dir.path(), |cell| (usize::from(cell >= 8), cell));
    // The top two rows are also in the palette of another row (BG palette 2)
    for cell in 0..8_usize {
        log.vram[0x0400 + (2 + cell / 4) * 32 + 2 + cell % 4] |= 2 << 10;
    }
    let tally = tallied(pack, &log);
    assert_eq!(tally.portraits.len(), 1, "{:?}", tally.portraits);
    let portrait = &tally.portraits[0];
    assert_eq!(portrait.pictures.len(), 1, "{portrait:?}");
    assert_eq!(portrait.rows.len(), 1, "one row: {portrait:?}");
}

/// Work RAM with the Kaioken on (`$0C36` bit 7) and Super Saiyan Goku in party record `slot`.
fn wram_with_kaioken(on: bool, slot: usize) -> Vec<u8> {
    let mut wram = vec![0_u8; 0x20000];
    wram[0x0C36] = if on { 0x80 } else { 0x40 };
    wram[0x0B00 + 0x20 * slot + 1] = 0x3B;
    wram
}

#[test]
fn the_kaioken_condition_is_the_bit_of_the_state_and_goku_in_a_party_record() {
    let on = Conditions::of_work_ram(&wram_with_kaioken(true, 3));
    assert_eq!(on, Conditions { kaioken: true, super_saiyan_goku: true });
    let off = Conditions::of_work_ram(&wram_with_kaioken(false, 3));
    assert_eq!(off, Conditions { kaioken: false, super_saiyan_goku: true }, "Goku without it");
    let mut no_goku = wram_with_kaioken(true, 3);
    no_goku[0x0B00 + 0x20 * 3 + 1] = 0x39;
    assert_eq!(Conditions::of_work_ram(&no_goku), Conditions::default(), "Goku's unit is $3B");
    assert_eq!(Conditions::of_work_ram(&[]), Conditions::default(), "no work RAM says no");
}

#[test]
fn super_saiyan_goku_is_drawn_from_his_own_art_for_the_pixels_he_shares_with_the_normal_goku() {
    let dir = tempfile::tempdir().unwrap();
    // The 16 x 16 fighter of `fighter_log`: the normal Goku's art for its pixels is blue, Super Saiyan
    // Goku's red. The normal unit has a second pose (other pixels), so it is the unit with the most
    // poses: the one `unit_for` gave before the party's Super Saiyan Goku was taken into account.
    let (frame, pose) = fighter_log(40, 50);
    write_png(&dir.path().join("art/normal.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    write_png(&dir.path().join("art/ssj.png"), 64, 64, |_, _| [255, 0, 0, 255]);
    write_png(&dir.path().join("art/other.png"), 64, 64, |_, _| [0, 255, 0, 255]);
    let goku = |id: &str, key: u64, art: &str| Object {
        id: id.to_owned(),
        key,
        ..object(&pose, art, false)
    };
    let objects = vec![
        goku("battle-goku-000", pose.key, "art/normal.png"),
        goku("battle-goku-001", pose.key ^ 1, "art/other.png"),
        goku("battle-goku-ssj-000", pose.key, "art/ssj.png"),
    ];
    let pack = pack_of(dir.path(), Vec::new(), ArtIndex::new(Vec::new(), objects));
    let mut renderer = Renderer::new();
    renderer.set_art(Some(pack));
    // A pixel of the sprite
    let (x, y) = (41 * 4 + 1, 51 * 4 + 1);
    renderer.set_conditions(Conditions::default());
    let without = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&without, x, y), [0, 0, 255], "no Super Saiyan Goku: the normal unit");
    // Goku with the Kaioken off: `wram_with_kaioken(false, 0)` has his unit in the first party record
    renderer.set_conditions(Conditions::of_work_ram(&wram_with_kaioken(false, 0)));
    let with = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&with, x, y), [255, 0, 0], "Super Saiyan Goku: his own art");
    // The close-ups of the special attacks are another set of Goku's art: its Super Saiyan set is drawn the same way
    let gallery = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(
            Vec::new(),
            vec![
                goku("gallery-party-goku-u00-000", pose.key, "art/normal.png"),
                goku("gallery-party-goku-u00-001", pose.key ^ 1, "art/other.png"),
                goku("gallery-party-goku-ssj-000", pose.key, "art/ssj.png"),
            ],
        ),
    );
    renderer.set_art(Some(gallery));
    renderer.set_conditions(Conditions::of_work_ram(&wram_with_kaioken(false, 0)));
    let close_up = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&close_up, x, y), [255, 0, 0], "Super Saiyan Goku's close-up: his own art");
    // A pose that the Super Saiyan art has no picture of (its pixels are the other ones) stays the normal art
    let normal_only = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(
            Vec::new(),
            vec![
                goku("battle-goku-000", pose.key, "art/normal.png"),
                goku("battle-goku-001", pose.key ^ 1, "art/other.png"),
                goku("battle-goku-ssj-001", pose.key ^ 1, "art/ssj.png"),
            ],
        ),
    );
    renderer.set_art(Some(normal_only));
    renderer.set_conditions(Conditions::of_work_ram(&wram_with_kaioken(false, 0)));
    let kept = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&kept, x, y), [0, 0, 255], "no Super Saiyan art for the pose: the normal one");
}

/// A pack with the picture `art/o.png` of the fighter of [`fighter_log`] (blue) and the pose variant `art/v.png` (red, `size`
/// art pixels) of the same object, drawn while the Kaioken is on; the variant's margin is 2 game pixels.
fn variant_pack(dir: &tempfile::TempDir, pose: &Pose, variant: Option<(u32, u32)>) -> Arc<Pack> {
    write_png(&dir.path().join("art/o.png"), 64, 64, |_, _| [0, 0, 255, 255]);
    let mut pack = pack_of(
        dir.path(),
        Vec::new(),
        ArtIndex::new(Vec::new(), vec![object(pose, "art/o.png", false)]),
    );
    if let Some((width, height)) = variant {
        write_png(&dir.path().join("art/v.png"), width, height, |_, _| [255, 0, 0, 255]);
        let file = dir.path().join("art/v.png").to_string_lossy().into_owned();
        let variants = vec![PoseVariant {
            art: "battle-x-000".to_owned(),
            when: ssd_mod::pose::Condition::Kaioken,
            margin: 2,
            file,
        }];
        Arc::get_mut(&mut pack).unwrap().variants = variants;
    }
    pack
}

#[test]
fn a_pose_variant_is_drawn_over_its_box_and_its_margin_while_the_kaioken_is_on() {
    let dir = tempfile::tempdir().unwrap();
    // The 16 x 16 fighter at (40, 50), the variant 4 game pixels larger (2 on each side): 80 x 80 art pixels
    let (frame, pose) = fighter_log(40, 50);
    let pack = variant_pack(&dir, &pose, Some((80, 80)));
    let mut renderer = Renderer::new();
    let plain = renderer.render_hd(&frame, options(4)).unwrap();
    renderer.set_art(Some(pack));
    let red = [255, 0, 0];
    // Off: the object's own picture, and nothing past the sprite
    renderer.set_conditions(Conditions::of_work_ram(&wram_with_kaioken(false, 0)));
    let off = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&off, 41 * 4 + 1, 51 * 4 + 1), [0, 0, 255], "the object's picture");
    assert_eq!(pixel(&off, 38 * 4 + 1, 48 * 4 + 1), pixel(&plain, 38 * 4 + 1, 48 * 4 + 1));
    // On: the variant covers the sprite and its margin (frame 38 to 57, 48 to 67), and nothing past it
    renderer.set_conditions(Conditions::of_work_ram(&wram_with_kaioken(true, 0)));
    let on = renderer.render_hd(&frame, options(4)).unwrap();
    assert_eq!(pixel(&on, 41 * 4 + 1, 51 * 4 + 1), red, "the sprite's pixels");
    assert_eq!(pixel(&on, 38 * 4, 48 * 4), red, "the margin's first pixel");
    assert_eq!(pixel(&on, 57 * 4 + 3, 67 * 4 + 3), red, "the margin's last pixel");
    assert_ne!(pixel(&on, 37 * 4 + 3, 50 * 4), red, "nothing past the margin on the left");
    assert_ne!(pixel(&on, 40 * 4, 47 * 4 + 3), red, "nor above it");
    // Away from the pose nothing changes
    assert_eq!(pixel(&on, 5, 5), pixel(&plain, 5, 5));
}

#[test]
fn a_pose_variant_that_is_missing_or_of_another_size_is_not_drawn_and_the_object_stays() {
    let dir = tempfile::tempdir().unwrap();
    let (frame, pose) = fighter_log(40, 50);
    let on = Conditions::of_work_ram(&wram_with_kaioken(true, 0));
    // The variant's picture must be the object's (16 + 2 x 2 game pixels, four art pixels to one): 80 x 80
    for variant in [Some((64, 64)), None] {
        let pack = variant_pack(&dir, &pose, variant);
        let mut renderer = Renderer::new();
        renderer.set_art(Some(pack));
        renderer.set_conditions(on);
        let drawn = renderer.render_hd(&frame, options(4)).unwrap();
        assert_eq!(pixel(&drawn, 41 * 4 + 1, 51 * 4 + 1), [0, 0, 255], "{variant:?}");
        assert_ne!(pixel(&drawn, 38 * 4, 48 * 4), [255, 0, 0], "{variant:?}");
    }
}
