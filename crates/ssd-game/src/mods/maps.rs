//! The pack maps (`ssd_mod::map`): the redirectors that give the original's map code a pack map's header, grid, new block
//! definitions, objects, zones, palettes and start, read from the directory of the image (`docs/re/data/maps.md`,
//! sections 1 to 10). Every wrapper declines (`resume_at` its own key, nothing written) for an id below
//! [`ssd_mod::map::FIRST_ID`], so an original map runs the original's code.
//!
//! The keys, and what each does for a pack map (id `64` to `127`):
//!
//! - `00:8E26`, the map header lookup: writes the height, the width, the grid's 24-bit address (`$0704-$0706`) and the
//!   music byte (`$01E7`, from the original's `02:E41B` at the pack's music map), and returns without the original.
//! - `06:F0E6`, the block word of `06:F089` (the pixel to tile word): a block id of `$78` or more gets the word the
//!   table `06:F1EE` would hold, `(bank << 8) | middle byte` of its definition, which the original then reads.
//! - `03:93FA`, the object loader: fills the 16 slots of `$1180` from the pack's objects and returns.
//! - `02:D38B`, the encounter zone: tests the pack's zones (the first whose blocks hold the camera), and sets the zone
//!   and the encounter level the way the original does; `02:D428`, the post-load reset, does not zero them for a pack
//!   map (the original zeroes every id from `$20` up).
//! - `02:D379`, the group of a random encounter (the original's `STA` of the group): the zone's group.
//! - the palettes: the original reads the palette indexes and the background palette of the map id; for a pack map the
//!   read sees the pack's `palette_like` instead (`00:877E` twice, `00:8E01` with the palette map's `$03`, `00:86DB`).
//!   `06:EB31` (the same palette read) has no JSL caller in the ROM and is not hooked.
//! - `00:9345` and `00:935E`, the start of the mode 6 init (`00:933A`): the pack's start x and y.
//! - `03:F198`, the per-map trigger dispatch: a pack map runs no original trigger routine; its triggers are the pack's
//!   (`mods::triggers`, at `03:F163`).
//! - `03:92CF`, the entrance entities: a pack map has none (no landing spot), so its entity slots are cleared.

use super::redirect::overlay;
use ssd_mod::Family;
use ssd_mod::map::{FIRST_ID, NEW_BLOCK_BASE};
use ssd_oracle::Ctx;

/// The header lookup's entry (`00:8E26`, a whole routine that returns with RTL).
pub(super) const HEADER_KEY: u32 = 0x00_8E26;
/// `06:F0E6`, the `STA $08` after the block word is loaded (16-bit A).
pub(super) const BLOCK_KEY: u32 = 0x06_F0E6;
/// The object loader's entry (`03:93FA`).
pub(super) const OBJECTS_KEY: u32 = 0x03_93FA;
/// The encounter zone's entry (`02:D38B`) and the post-load reset's (`02:D428`).
pub(super) const ZONE_KEY: u32 = 0x02_D38B;
pub(super) const RESET_KEY: u32 = 0x02_D428;
/// `02:D379`, the `STA $0EA3` of the group pick (8-bit A).
pub(super) const GROUP_KEY: u32 = 0x02_D379;
/// The palette keys: `00:879A` and `00:87C6` (the `ASL` after the map id is loaded in `00:877E`), `00:8E01` (the `STA $03`
/// of `00:8DFA`) and `00:86DB` (the `LSR` of `00:86C8`).
pub(super) const PALETTE_KEYS: [u32; 4] = [0x00_879A, 0x00_87C6, 0x00_8E01, 0x00_86DB];
/// `00:9345` and `00:935E`, the loads of the start's x and y word in `00:933A` (16-bit A).
pub(super) const START_X_KEY: u32 = 0x00_9345;
pub(super) const START_Y_KEY: u32 = 0x00_935E;
/// `03:F198`, the `LDA #$00` that starts the dispatch of the per-map triggers (the field, not flying).
pub(super) const TRIGGER_KEY: u32 = 0x03_F198;
/// `03:92CF`, the entrance entities' entry (a whole routine that returns with RTL).
pub(super) const ENTRANCE_KEY: u32 = 0x03_92CF;
/// `06:EC12`, the field's per-map list of the mode 0 init (a whole routine that returns with RTL).
pub(super) const LIST_KEY: u32 = 0x06_EC12;

/// Work RAM (`$7E:xxxx`).
const MAP_ID: u32 = 0x7E_01A0;
/// The list `06:EC12` copies for the map (`$074C`, words, `$FFFF` ends it).
const MAP_LIST: u32 = 0x7E_074C;
const COPY_SELECT: u32 = 0x7E_01E7;
const MAP_HEIGHT: u32 = 0x7E_0702;
const MAP_WIDTH: u32 = 0x7E_0703;
const MAP_GRID: u32 = 0x7E_0704;
/// The block id `06:F089` stores before it reads the block word (`$71`).
const BLOCK_ID: u32 = 0x7E_0071;
/// The camera's block coordinates (the high bytes of the camera, `$34` and `$36`).
const CAMERA_X: u32 = 0x7E_0034;
const CAMERA_Y: u32 = 0x7E_0036;
/// The game's state byte the zone test reads (`$28`: bit 7 stops the encounters).
const STATE_28: u32 = 0x7E_0028;
/// The encounter flags (`$01BC`: bit 0 flying, bit 6 the second bonus) and the forced-encounter counter (`$01BF`).
const GATE_FLAGS: u32 = 0x7E_01BC;
const FORCED: u32 = 0x7E_01BF;
/// The encounter timer (`$AA`), the encounter zone (`$0D8B`) and the encounter level (`$0D8A`).
const TIMER: u32 = 0x7E_00AA;
const ZONE: u32 = 0x7E_0D8B;
const LEVEL: u32 = 0x7E_0D8A;
/// The object slots of the loader (`$1180`, 7 bytes each, 16 slots) and the entrance entities (`$1000`, 6 bytes, 32).
pub(super) const OBJECT_SLOTS: u32 = 0x7E_1180;
const ENTITIES: u32 = 0x7E_1000;
/// The original's tables the wrappers read: the music byte of a map (`02:E41B`) and the object records (`02:EB67`).
const MUSIC_BYTES: u32 = 0x02_E41B;
const OBJECT_RECORDS: u32 = 0x02_EB67;

/// The length of an object slot and of the object record of a pack map (`MapObjects`: object, x, y, message).
const SLOT_LEN: u32 = 7;
const PACK_OBJECT_LEN: u32 = 7;
/// The number of entrance entities the original keeps (`$1000`, 6 bytes each).
const ENTITY_SLOTS: u32 = 32;
const ENTITY_LEN: u32 = 6;
/// The bytes of a block definition: 256 cells of one word.
const BLOCK_LEN: u32 = 512;
/// The zone record of a pack map (`MapZones`): x0, x1, y0, y1, level, group.
const ZONE_LEN: u32 = 6;
/// The encounter level the forced encounter uses (the original's `$0F`).
const FORCED_LEVEL: u8 = 0x0F;
/// The timer the forced encounter sets (the original's `$5B`).
const FORCED_TIMER: u8 = 0x5B;

/// Whether `map` is a pack map (its id is one of the pack's).
pub(super) fn is_pack(map: u8) -> bool {
    map >= FIRST_ID
}

/// `map` as the directory's id.
pub(super) fn slot(map: u8) -> u16 {
    u16::from(map)
}

/// The header lookup (`00:8E26`): a pack map gets its size, its grid and its music byte from the pack, and the routine
/// returns without the original (which would read a header of the original table, past its end).
pub(super) fn header_wrapper(ctx: &mut Ctx<'_>) {
    let map = ctx.peek8(MAP_ID);
    let header = overlay(ctx, Family::MapHeader, slot(map));
    let grid = overlay(ctx, Family::MapBlocks, slot(map));
    let (Some(header), Some(grid), true) = (header, grid, is_pack(map)) else {
        ctx.resume_at(HEADER_KEY);
        return;
    };
    let width = ctx.peek8(header);
    let height = ctx.peek8(header + 1);
    let music = ctx.peek8(header + 2);
    ctx.write8(MAP_WIDTH, width);
    ctx.write8(MAP_HEIGHT, height);
    for (k, byte) in pointer_bytes(grid).into_iter().enumerate() {
        ctx.write8(MAP_GRID + k as u32, byte);
    }
    let copy = ctx.peek8(MUSIC_BYTES + u32::from(music));
    ctx.write8(COPY_SELECT, copy);
}

/// The block word of `06:F089` (`06:F0E6`): a block id of `$78` or more of a pack map takes the word of its definition in
/// the pack's data (see [`definition_word`]). Any other block is left to the original.
pub(super) fn block_wrapper(ctx: &mut Ctx<'_>) {
    let map = ctx.peek8(MAP_ID);
    let block = ctx.peek8(BLOCK_ID);
    if !is_pack(map) || block < NEW_BLOCK_BASE {
        ctx.resume_at(BLOCK_KEY);
        return;
    }
    let grid = u32::from(ctx.peek16(MAP_GRID)) | u32::from(ctx.peek8(MAP_GRID + 2)) << 16;
    let cells = usize::from(ctx.peek8(MAP_WIDTH)) * usize::from(ctx.peek8(MAP_HEIGHT));
    ctx.set_a(definition_word(grid, cells, block));
    ctx.resume_at(BLOCK_KEY);
}

/// The word `06:F1EE` would hold for block `block` of a pack map whose grid is at `grid` and holds `cells` block ids:
/// the bank in the high byte and the middle byte of the definition in the low byte. The definitions follow the grid,
/// padded to 256 bytes, 512 bytes each, from the first new block (`$78`); the pack's image aligns them to 256.
fn definition_word(grid: u32, cells: usize, block: u8) -> u16 {
    let padded = cells.div_ceil(256) * 256;
    let address = grid + padded as u32 + u32::from(block - NEW_BLOCK_BASE) * BLOCK_LEN;
    ((address >> 16) as u16) << 8 | ((address >> 8) & 0xFF) as u16
}

/// The object loader (`03:93FA`): a pack map's objects fill the slots, and the routine returns without the original.
pub(super) fn objects_wrapper(ctx: &mut Ctx<'_>) {
    let map = ctx.peek8(MAP_ID);
    let Some(list) = overlay(ctx, Family::MapObjects, slot(map)).filter(|_| is_pack(map)) else {
        ctx.resume_at(OBJECTS_KEY);
        return;
    };
    // The original clears the 112 bytes of the slots first (and writes the terminator at the end of the list).
    for k in 0..SLOT_LEN * ssd_mod::map::MAX_OBJECTS as u32 {
        ctx.write8(OBJECT_SLOTS + k, 0);
    }
    let count = usize::from(ctx.peek8(list)).min(ssd_mod::map::MAX_OBJECTS);
    for index in 0..count {
        let record = list + 1 + PACK_OBJECT_LEN * index as u32;
        let object = ctx.peek8(record);
        let (x, y) = (ctx.peek16(record + 1), ctx.peek16(record + 3));
        let kind = ctx.peek8(OBJECT_RECORDS + 5 * u32::from(object));
        let at = OBJECT_SLOTS + SLOT_LEN * index as u32;
        ctx.write8(at, 0x80 | (kind & 0x7F));
        ctx.write8(at + 1, object.wrapping_add(0x30));
        ctx.write16(at + 2, x);
        ctx.write16(at + 4, y);
        // The slot's flag is 2 when the original's record has its bit 7 (the object's own flag).
        ctx.write8(at + 6, if kind & 0x80 != 0 { 2 } else { 0 });
    }
    ctx.write8(OBJECT_SLOTS + SLOT_LEN * count as u32, 0);
}

/// The zone test (`02:D38B`): a pack map's zones are tested with the camera's block, as the original tests its own
/// rectangles (`02:D3A9..D3D7`); see [`zone_at`] and [`encounter_level`] for the rules, which the original's code
/// follows. The routine returns without the original.
pub(super) fn zone_wrapper(ctx: &mut Ctx<'_>) {
    let map = ctx.peek8(MAP_ID);
    if !is_pack(map) {
        ctx.resume_at(ZONE_KEY);
        return;
    }
    let (x, y) = (ctx.peek8(CAMERA_X), ctx.peek8(CAMERA_Y));
    let zones = read_zones(ctx, map);
    let Some((number, zone)) = zone_at(&zones, x, y) else {
        // No zone holds the camera: this map has no random battles here.
        ctx.write8(ZONE, 0);
        ctx.write8(LEVEL, 0);
        return;
    };
    ctx.write8(ZONE, number);
    // The original's tail after the match: an encounter with a zone's level unless the state stops them, the timer of
    // the forced encounter otherwise (level 15).
    if ctx.peek8(STATE_28) & 0x80 != 0 {
        return;
    }
    if ctx.peek8(FORCED) & 0x80 == 0 {
        let flags = ctx.peek8(GATE_FLAGS);
        ctx.write8(LEVEL, encounter_level(zone.level, flags));
        return;
    }
    ctx.write8(TIMER, 0);
    let counter = ctx.peek8(FORCED).wrapping_sub(1);
    ctx.write8(FORCED, counter);
    if counter & 0x80 == 0 {
        ctx.write8(FORCED, 0);
        ctx.write8(TIMER, FORCED_TIMER);
    }
    ctx.write8(LEVEL, FORCED_LEVEL);
}

/// The post-load reset (`02:D428`): the original zeroes the zone and the level of every id from `$20` up, which would
/// take the pack's zones away, so a pack map keeps them and the routine returns.
pub(super) fn reset_wrapper(ctx: &mut Ctx<'_>) {
    if is_pack(ctx.peek8(MAP_ID)) {
        return;
    }
    ctx.resume_at(RESET_KEY);
}

/// The group of a random encounter (`02:D379`): the zone's group, in place of the original's list (which a pack map has
/// not). A pack map with no zone keeps the accumulator.
pub(super) fn group_wrapper(ctx: &mut Ctx<'_>) {
    let map = ctx.peek8(MAP_ID);
    if is_pack(map) {
        let number = ctx.peek8(ZONE);
        if let Some(zone) = zone_record(ctx, map, number) {
            ctx.set_al(zone.group);
        }
    }
    ctx.resume_at(GROUP_KEY);
}

/// A palette read of the original, at `key`: a pack map reads the palette of its `palette_like` map (the byte the header
/// keeps at offset 3), in place of its own id, which the original would look up past its table.
fn palette_wrapper(ctx: &mut Ctx<'_>, key: u32) {
    let map = ctx.peek8(MAP_ID);
    if is_pack(map)
        && let Some(header) = overlay(ctx, Family::MapHeader, slot(map))
    {
        let like = ctx.peek8(header + 3);
        ctx.set_al(like);
    }
    ctx.resume_at(key);
}

/// `00:879A`, the palette indexes of `00:877E` (the field's `ASL` of the map id).
pub(super) fn palette_879a(ctx: &mut Ctx<'_>) {
    palette_wrapper(ctx, PALETTE_KEYS[0]);
}

/// `00:87C6`, the second palette indexes of `00:877E`.
pub(super) fn palette_87c6(ctx: &mut Ctx<'_>) {
    palette_wrapper(ctx, PALETTE_KEYS[1]);
}

/// `00:8E01`, the `STA $03` of `00:8DFA`: the map whose background palette the copy of `00:88AE` reads.
pub(super) fn palette_8e01(ctx: &mut Ctx<'_>) {
    palette_wrapper(ctx, PALETTE_KEYS[2]);
}

/// `00:86DB`, the `LSR` of `00:86C8`: the face palette's index.
pub(super) fn palette_86db(ctx: &mut Ctx<'_>) {
    palette_wrapper(ctx, PALETTE_KEYS[3]);
}

/// The start of the mode 6 init at `key`: a pack map starts where its header says (`offset` is the x word at 4, the y
/// word at 6).
fn start_wrapper(ctx: &mut Ctx<'_>, key: u32, offset: u32) {
    let map = ctx.peek8(MAP_ID);
    if is_pack(map)
        && let Some(header) = overlay(ctx, Family::MapHeader, slot(map))
    {
        let value = ctx.peek16(header + offset);
        ctx.set_a(value);
    }
    ctx.resume_at(key);
}

/// `00:9345`, the load of the start's x word in `00:933A`.
pub(super) fn start_x_wrapper(ctx: &mut Ctx<'_>) {
    start_wrapper(ctx, START_X_KEY, 4);
}

/// `00:935E`, the load of the start's y word in `00:933A`.
pub(super) fn start_y_wrapper(ctx: &mut Ctx<'_>) {
    start_wrapper(ctx, START_Y_KEY, 6);
}

/// The trigger dispatch (`03:F198`): a pack map runs no original trigger routine, so the dispatch returns (the original would
/// index its table by the pack's id, past the table). The pack's triggers run in `mods::triggers`, before this dispatch.
pub(super) fn trigger_wrapper(ctx: &mut Ctx<'_>) {
    if is_pack(ctx.peek8(MAP_ID)) {
        return;
    }
    ctx.resume_at(TRIGGER_KEY);
}

/// The per-map list of the mode 0 init (`06:EC12`): the original copies the list its table `06:FA2D` gives for the map
/// id, which a pack map's id is past (the copy runs on through work RAM). A pack map has no such list: the terminator
/// is written, and the routine returns.
pub(super) fn list_wrapper(ctx: &mut Ctx<'_>) {
    if !is_pack(ctx.peek8(MAP_ID)) {
        ctx.resume_at(LIST_KEY);
        return;
    }
    ctx.write16(MAP_LIST, 0xFFFF);
}

/// The entrance entities (`03:92CF`): a pack map has no entrances, so the entity slots are emptied (the original's
/// loop reads the list of the id from its table, past the end for a pack map).
pub(super) fn entrance_wrapper(ctx: &mut Ctx<'_>) {
    if !is_pack(ctx.peek8(MAP_ID)) {
        ctx.resume_at(ENTRANCE_KEY);
        return;
    }
    for k in 0..ENTITY_SLOTS {
        ctx.write8(ENTITIES + ENTITY_LEN * k, 0);
    }
}

/// The zone records of pack map `map` (count, then records), empty when it has none.
fn read_zones(ctx: &mut Ctx<'_>, map: u8) -> Vec<Zone> {
    let Some(list) = overlay(ctx, Family::MapZones, slot(map)) else { return Vec::new() };
    let count = ctx.peek8(list);
    (0..u32::from(count))
        .map(|k| {
            let at = list + 1 + ZONE_LEN * k;
            Zone {
                x0: ctx.peek8(at),
                x1: ctx.peek8(at + 1),
                y0: ctx.peek8(at + 2),
                y1: ctx.peek8(at + 3),
                level: ctx.peek8(at + 4),
                group: ctx.peek8(at + 5),
            }
        })
        .collect()
}

/// The record of zone `number` (1-based) of pack map `map`.
fn zone_record(ctx: &mut Ctx<'_>, map: u8, number: u8) -> Option<Zone> {
    let zones = read_zones(ctx, map);
    zones.get(usize::from(number).checked_sub(1)?).copied()
}

/// A zone of a pack map: the rectangle of blocks (inclusive) of the camera's block, the encounter level and the group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Zone {
    pub x0: u8,
    pub x1: u8,
    pub y0: u8,
    pub y1: u8,
    pub level: u8,
    pub group: u8,
}

/// The first zone whose rectangle holds the camera's block `(x, y)`, with its number (1-based, the zone the original
/// stores in `$0D8B`).
pub(super) fn zone_at(zones: &[Zone], x: u8, y: u8) -> Option<(u8, Zone)> {
    zones
        .iter()
        .enumerate()
        .find(|(_, zone)| zone.x0 <= x && x <= zone.x1 && zone.y0 <= y && y <= zone.y1)
        .map(|(index, zone)| ((index + 1) as u8, *zone))
}

/// The encounter level the original stores for a matched zone of level `level`: none for level 0; with the flying bit
/// (`$01BC` bit 0) one more, and one more again with bit 6 (both only while flying).
pub(super) fn encounter_level(level: u8, flags: u8) -> u8 {
    if level == 0 || flags & 1 == 0 {
        return level;
    }
    level.wrapping_add(1).wrapping_add(u8::from(flags & 0x40 != 0))
}

/// The bytes of a bus address, low byte first (the three bytes of a 24-bit pointer).
fn pointer_bytes(address: u32) -> [u8; 3] {
    [address as u8, (address >> 8) as u8, (address >> 16) as u8]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pack_map_is_an_id_from_64_and_an_original_one_is_not() {
        assert!(!is_pack(0));
        assert!(!is_pack(47));
        assert!(!is_pack(63));
        assert!(is_pack(64));
        assert!(is_pack(127));
    }

    #[test]
    fn the_definition_word_is_the_bank_and_the_middle_byte_of_the_block_after_the_padded_grid() {
        // A 2 x 2 grid pads to 256 bytes; the first new block then starts at grid + 256, the second 512 bytes on.
        assert_eq!(definition_word(0x20_9000, 4, NEW_BLOCK_BASE), 0x2091);
        assert_eq!(definition_word(0x20_9000, 4, NEW_BLOCK_BASE + 1), 0x2093);
        // A 16 x 16 grid (256 bytes) pads to exactly 256.
        assert_eq!(definition_word(0x21_8000, 256, NEW_BLOCK_BASE), 0x2181);
        // A 17 x 16 grid pads to 512.
        assert_eq!(definition_word(0x21_8000, 272, NEW_BLOCK_BASE), 0x2182);
    }

    #[test]
    fn the_first_zone_that_holds_the_camera_block_is_numbered_from_one() {
        let zones = [
            Zone { x0: 0, x1: 1, y0: 0, y1: 0, level: 5, group: 3 },
            Zone { x0: 1, x1: 3, y0: 0, y1: 2, level: 9, group: 7 },
        ];
        assert_eq!(zone_at(&zones, 0, 0).map(|(n, z)| (n, z.level)), Some((1, 5)));
        assert_eq!(
            zone_at(&zones, 1, 0).map(|(n, z)| (n, z.level)),
            Some((1, 5)),
            "the first wins"
        );
        assert_eq!(zone_at(&zones, 2, 2).map(|(n, z)| (n, z.group)), Some((2, 7)));
        assert_eq!(zone_at(&zones, 4, 0), None);
        assert_eq!(zone_at(&[], 0, 0), None, "a map without zones has no battles");
    }

    #[test]
    fn the_encounter_level_adds_the_flying_bonuses_as_the_original_does() {
        assert_eq!(encounter_level(0, 0xFF), 0, "level 0 is no battles");
        assert_eq!(encounter_level(7, 0x00), 7, "on foot");
        assert_eq!(encounter_level(7, 0x40), 7, "bit 6 alone adds nothing");
        assert_eq!(encounter_level(7, 0x01), 8, "flying");
        assert_eq!(encounter_level(7, 0x41), 9, "flying with bit 6");
    }

    #[test]
    fn the_pointer_is_low_byte_first_and_the_bank_last() {
        assert_eq!(pointer_bytes(0x20_9000), [0x00, 0x90, 0x20]);
    }

    #[test]
    fn the_keys_are_distinct_and_are_the_addresses_the_disassembly_gave() {
        let mut keys = vec![
            HEADER_KEY,
            BLOCK_KEY,
            OBJECTS_KEY,
            ZONE_KEY,
            RESET_KEY,
            GROUP_KEY,
            START_X_KEY,
            START_Y_KEY,
            TRIGGER_KEY,
            ENTRANCE_KEY,
            LIST_KEY,
        ];
        keys.extend(PALETTE_KEYS);
        let count = keys.len();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), count, "no key is used twice");
        assert_eq!(BLOCK_KEY, 0x06_F0E6);
        assert_eq!(GROUP_KEY, 0x02_D379);
    }
}
