//! The cheats of Enhanced mode (M5): the cheat block the wrappers read, the wrappers of the routines the
//! cheats change, the host events of the settings and the scripted-loss gate. Research and the seen effects:
//! `docs/re/data/battle.md` and `cards.md`.
//!
//! The settings reach the game only as host events: [`block_events`] and [`settings_events`] write them
//! (recorded in a format 2 replay), so a replay with cheats plays to the same state. The wrappers write only
//! the runtime bytes of the block (the gate, the random-encounter mark, the generator) and the game's own
//! state their cheat asks for, and only while a cheat is on: a neutral block changes nothing, and each wrapper
//! is then the original routine. A wrapper that has nothing to do for the call (a neutral block, a gate that is
//! shut, a call outside the cheat's case) declines: it ends with `Ctx::resume_at` its own entry, so the original
//! runs inline, as after a timing fallback, and nothing of the wrapper is left on the stack. A wrapper that
//! acts after the original (`call_original`) costs zero of its own and is gated (`mark_gated`), because the
//! original's time is not a bound the window of `Rust` mode can check. Declining matters for the identity of
//! the neutral block: `Machine::call_original` pushes a trap return below the caller's stack pointer, which the
//! lockstep and standalone comparisons would see as stack bytes only one machine wrote.
//!
//! The battle cheats (damage, defence, EXP, enemy power and count) stop for a scripted fight (the gate). The
//! other cheats (the Z cards of the dealt cards, the max zeni, the endless items and the random encounters) act
//! on the map and in every fight, as the settings say.

use crate::data as d;
use crate::mods::state::{
    BLOCK as MODS_BLOCK, ENEMY_HIGH as MODS_ENEMY_HIGH, MARKER as MODS_MARKER,
    MARKER_AT as MODS_MARKER_AT,
};
use ssd_menus::cheats::{
    Cheats, ENDLESS, GAIN, GENERATOR, GENERATOR_SEED, MARKER, MARKER_BYTES, MAX_ZENI, MONEY,
    Multiplier, RANDOM_OFF, RANDOM_ONLY, SUSPENDED, Setting, Z_VALUE, ZCARDS, ZCARDS_FIGHTER, ZENI,
    ZTarget,
};
use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, HookTable, HostEvent, ReturnKind};

/// The cheat block in work RAM: [`BLOCK_LEN`] bytes at `$7F:FF00`, which no reference replay writes (the scan
/// of the 28 replays, `docs/re/data/battle.md`, "The cheat block"). It lies in the upper work RAM, outside the
/// save image (`$7E:2800`), and the game clears the lower work RAM as it boots, so the block is written after boot.
pub const BLOCK: u32 = 0x7F_FF00;

/// The application event that carries the run's cheats in a format 2 replay: its payload is [`Cheats::to_bytes`].
/// A change of the cheats records it with the block's writes. A replay that plays it sets the run's cheats (the
/// settings a Save writes to the sidecar, as the bookkeeping is kept on playback), and nothing else: the emulator
/// does not act on application events, and the sidecar file is never written by a replay. The bag's event is
/// `keeper::BIND_TAG` (1).
pub const CHEATS_TAG: u8 = 2;

/// The groups of the scripted fights the party is meant to lose (`encounter_group_index`): with a
/// cheat on, the battle cheats are off for such a fight (the scripted-loss gate). `$57` is Nail alone
/// against Frieza (`secret-nail-porunga`).
pub const SCRIPTED_LOSS: &[u8] = &[0x57];

/// The bus address of the first byte of work RAM.
const WRAM_BASE: u32 = 0x7E_0000;
/// The game's mode byte (`$24`): 2 and 4 are battles.
const GAME_MODE: u32 = 0x7E_0024;
/// The modes a battle goes through: 2 and 4, the states with the flag bit `$80` set (`$82`, `$84`, `$8C`), and
/// `$0C`, the one frame between the two phases of a scripted fight (`secret-nail-porunga`). Found with the group
/// spike (`docs/re/data/battle.md`).
const BATTLE_MODES: [u8; 6] = [0x02, 0x04, 0x82, 0x84, 0x8C, 0x0C];
/// The map: the mode a battle ends in.
const MAP_MODE: u8 = 0;
/// The scripted-loss gate's states: armed by the group pick, in battle once a battle mode is seen, and back to
/// zero on the map. A nonzero gate suspends the battle cheats.
pub const GATE_ARMED: u8 = 1;
pub const GATE_IN_BATTLE: u8 = 2;
/// The largest 24-bit value, the saturation of the damage, the EXP and the BP.
const MAX24: u32 = 0x00FF_FFFF;
/// The largest 16-bit value, the saturation of the HP.
const MAX16: u32 = 0xFFFF;

/// The enemy slots (`$0E00`, one record of `$20` bytes each, five slots).
const ENEMY_SLOTS: u16 = 0x0E00;
const ENEMY_STRIDE: u16 = 0x20;
const ENEMY_SLOT_COUNT: u16 = 5;
/// The flag byte of a slot in use (`+00`).
const ENEMY_LIVE: u8 = 0x40;
/// The unit id of a slot (`+01`) and its copy (`+1C`).
const ENEMY_ID: u16 = 0x01;
const ENEMY_ID_COPY: u16 = 0x1C;
/// The HP word (`+03`), the maximum HP word (`+0A`) and the BP (`+07..+09`) of a slot.
const ENEMY_HP: u16 = 0x03;
const ENEMY_MAX_HP: u16 = 0x0A;
const ENEMY_BP: u16 = 0x07;

/// The enemy slots a group of `n` enemies takes, by `n` - 1 (`01:B8DE`'s formation): one uses slot 0,
/// two use 1 and 2, three use 0, 3 and 4, four use 1 to 4, five use all.
const FORMATION: [&[u16]; 5] = [&[0], &[1, 2], &[0, 3, 4], &[1, 2, 3, 4], &[0, 1, 2, 3, 4]];

/// The damage the routines work on (`battle_damage`, a 24-bit word) and the EXP of the battle.
const BATTLE_DAMAGE: u16 = (d::BATTLE_DAMAGE & 0xFFFF) as u16;
const BATTLE_EXP: u16 = (d::BATTLE_EXP_AWARD & 0xFFFF) as u16;
/// The side that acts (`active_side`, bit 0): 0 the party, 1 the enemies.
const ACTIVE_SIDE: u32 = d::ACTIVE_SIDE;
/// The status byte that redirects or drops the damage (`$1700`, bits 0 and 1).
const DAMAGE_REDIRECT: u16 = 0x1700;
const ENEMY_COUNT: u16 = (d::ENEMY_COUNT & 0xFFFF) as u16;
const GROUP_INDEX: u16 = (d::ENCOUNTER_GROUP_INDEX & 0xFFFF) as u16;

/// The party's side record (side A, the fighter whose turn it is or who is attacked): its attack number at `+14`,
/// its defence number at `+15`, and its character id at `+1C`.
const SIDE_A: u32 = d::SIDE_RECORD_A;
const SIDE_ATTACK: u32 = 0x14;
const SIDE_DEFENCE: u32 = 0x15;
const SIDE_CHARACTER: u32 = 0x1C;

/// The hand of the cards: five cards of six bytes from `$7E:0140` (the flag, bit 7 set when the card was played;
/// the position word; the attack value at `+3`, the defence value at `+4`; the symbol at `+5`; `cards.md`).
const HAND: u32 = 0x7E_0140;
const HAND_CARDS: u32 = 5;
const CARD_BYTES: u32 = 6;
const CARD_ATTACK: u32 = 3;
const CARD_DEFENCE: u32 = 4;
/// The corners of a card as the Z flags of the settings: bit 0 attack, bit 1 defence.
const Z_ATTACK: u8 = 1;
const Z_DEFENCE: u8 = 2;

/// The encounter timer (`$AA`, one count a roll interval, reloaded with 59 by `02:D1F6`) and the chain of
/// battles queued in the air (`$01BF`, bit 7).
const ENCOUNTER_TIMER: u32 = d::ENCOUNTER_TIMER;
const ENCOUNTER_RELOAD: u8 = 59;
const CHAIN: u32 = 0x7E_01BF;
/// The map (`$01A0`) and the battle counter (`$01B0`, a byte the story reads). Maps 22 and 24 are the story's
/// Frieza maps, whose forced fights ride the random roll and count the battles of the roll.
const MAP_ID: u32 = d::MAP_ID;
const BATTLE_COUNTER: u32 = d::BATTLE_COUNTER;
const STORY_MAP_22: u8 = 0x16;
const STORY_MAP_24: u8 = 0x18;

/// The item list (`$0100`, two bytes an entry: the object, then its attribute), its count, the object of the
/// Item menu (`$0D7C`), and the attribute table (`item_attributes`, by object).
const ITEM_LIST: u32 = d::ITEM_LIST;
const ITEM_COUNT: u32 = d::ITEM_COUNT;
const MENU_ITEM_ID: u32 = d::MENU_ITEM_ID;
const ITEM_ATTRIBUTES: u32 = d::ITEM_ATTRIBUTES;
/// The return addresses (after the `JSL 01:C639`) of the effect tails that use an object: the common tail
/// (`01:DDF3`), Vegeta (`01:EB33`), Escape (`01:EBB3`), the end of Porunga's wishes (`01:E35E`) and `01:DD8A`.
const EFFECT_TAILS: [u32; 5] = [0x01_DD8A, 0x01_DDF3, 0x01_E35E, 0x01_EB33, 0x01_EBB3];

/// The routines the wrappers hook (bank:address, the entry of each).
const DAMAGE_ENTRY: u32 = 0x05_BDDC;
const EXP_ENTRY: u32 = 0x1D_8C33;
const GROUP_ENTRY: u32 = 0x01_F692;
const POWER_ENTRY: u32 = 0x01_F719;
const FORMATION_ENTRY: u32 = 0x01_B8DE;
const PICK_ENTRY: u32 = 0x02_D34A;
const DEAL_ENTRY: u32 = 0x01_C257;
const FIGHTER_ENTRY_B801: u32 = 0x05_B801;
const FIGHTER_ENTRY_B7FC: u32 = 0x05_B7FC;
const ROLL_ENTRY: u32 = 0x02_D289;
const ITEM_ENTRY: u32 = 0x01_C639;
/// The story event that gives zeni (`01:D7F9`, a target of the event table at `01:D456`): it adds the word `$0D53` to the zeni.
const MONEY_ENTRY: u32 = 0x01_D7F9;
/// The amount the money event adds (`$0D53`, a word); set by the script that makes the event, read only by the event.
const MONEY_AMOUNT: u32 = 0x7E_0D53;
/// The shop's sale (`03:A790`, `rtl`): it adds the price of the object sold to the zeni, capped at 999 (`menus.md`).
const SALE_ENTRY: u32 = 0x03_A790;
/// The body of the level deadline of map 5 (Earth, the Saiyans' chapter, `03:F320`): the map's routine (`03:F300`, run every
/// frame the party is free) branches to it when Piccolo reaches level 9, or Gohan level 10 without Piccolo. It sets flag `0x1D`,
/// which stops the random battles, and kills the fighters not yet recruited (Krillin, Yamcha, Tien, Chiaotzu): the story moves
/// on a level, which the EXP cheat or long training reach before the recruits are found. A Remastered story fix: the wrapper
/// returns at once, as the routine does for a lower level, so the recruits wait until the party walks into the Saiyans (the
/// game's other deadline, which stays).
pub const LEVEL_DEADLINE_ENTRY: u32 = 0x03_F320;
/// Porunga's wishes in a battle (`docs/re/data/menus.md`, "Porunga"): the window lists the three wishes on the tile rows 4, 6
/// and 8 of the English text, but the original puts the cursor (`01:F2E4`, a word table at `01:F2F6`) and the marks of the
/// wishes used (`01:B205`) on the rows 4, 8 and 10, the layout of a first wish of two lines: the cursor on the second wish
/// is drawn on the third, the one on the third on an empty row. A Remastered fix: the cursor and the marks go on the rows of
/// the text, each one two tile rows (`$80` bytes of the tile map) below the one before. Only Porunga's effect reaches them.
/// The `tax` after the table's read: A is the tile-map offset of the wish under the cursor (`map_entry_value`).
const WISH_CURSOR: u32 = 0x01_F2F2;
/// The `jsl` that draws the mark of the second wish used: X is its tile-map offset.
const WISH_MARK_SECOND: u32 = 0x01_B247;
/// The `jsl` that draws the mark of the third wish used.
const WISH_MARK_THIRD: u32 = 0x01_B252;
/// The original's table of the wishes' offsets: its first word is the first wish's, which is right.
const WISH_ROWS: u32 = 0x01_F2F6;
/// Two tile rows of the tile map, in bytes: the distance between two lines of the wishes' text.
const WISH_ROW_STEP: u16 = 0x80;
/// The object the shop's list is on (`menu_item_id`), which the sale reads.
const SALE_OBJECT: u32 = 0x7E_0D7C;
/// The price of each object id (`item_prices`, 40 bytes, `$FF` = none), which the sale adds.
const ITEM_PRICES: u32 = 0x01_CB8B;

/// The hook entries of the cheats, in Rust mode. They replace any hook at the same address (the recompiled
/// routine, which is the original's code) and are installed on top of the table [`install`] is given.
#[must_use]
pub fn hook_entries() -> Vec<HookEntry> {
    let entry = |name: &str, key: u32, func: ssd_oracle::RoutineFn, ret: ReturnKind| HookEntry {
        name: name.to_owned(),
        key,
        mode: HookMode::Rust,
        action: HookAction::Routine { func, ret },
        entry_m8: None,
        entry_x8: None,
        flags_mask: 0,
        verify_sample: 1,
        // A wrapper's own cost is zero: the timing window is checked for the entry alone (see the
        // module documentation).
        cycles: Some(0),
        bound: None,
        budget_frames: None,
    };
    vec![
        entry("cheat_damage", DAMAGE_ENTRY, damage_wrapper, ReturnKind::Rts),
        entry("cheat_exp", EXP_ENTRY, exp_wrapper, ReturnKind::Rtl),
        entry("cheat_group", GROUP_ENTRY, group_wrapper, ReturnKind::Rtl),
        entry("cheat_power", POWER_ENTRY, power_wrapper, ReturnKind::Rtl),
        entry("cheat_pick", PICK_ENTRY, pick_wrapper, ReturnKind::Rtl),
        entry("cheat_deal", DEAL_ENTRY, deal_wrapper, ReturnKind::Rtl),
        entry("cheat_fighter_b801", FIGHTER_ENTRY_B801, fighter_b801_wrapper, ReturnKind::Rtl),
        entry("cheat_fighter_b7fc", FIGHTER_ENTRY_B7FC, fighter_b7fc_wrapper, ReturnKind::Rtl),
        entry("cheat_roll", ROLL_ENTRY, roll_wrapper, ReturnKind::Rtl),
        entry("cheat_item", ITEM_ENTRY, item_wrapper, ReturnKind::Rtl),
        // A jump target: nothing returns from it, the wrapper always ends by resuming the original.
        entry("cheat_money", MONEY_ENTRY, money_wrapper, ReturnKind::Rtl),
        entry("cheat_sale", SALE_ENTRY, sale_wrapper, ReturnKind::Rtl),
        HookEntry {
            mode: HookMode::Patch,
            ..entry("story_porunga_wish_cursor", WISH_CURSOR, wish_cursor_wrapper, ReturnKind::Rtl)
        },
        HookEntry {
            mode: HookMode::Patch,
            ..entry(
                "story_porunga_wish_mark_2",
                WISH_MARK_SECOND,
                wish_mark_second_wrapper,
                ReturnKind::Rtl,
            )
        },
        HookEntry {
            mode: HookMode::Patch,
            ..entry(
                "story_porunga_wish_mark_3",
                WISH_MARK_THIRD,
                wish_mark_third_wrapper,
                ReturnKind::Rtl,
            )
        },
        // A branch target inside `03:F300`: returning here is the routine's own `rtl` for a level below the deadline.
        // A story fix must act on every call: a patch never falls back to the original for timing.
        HookEntry {
            mode: HookMode::Patch,
            ..entry(
                "story_level_deadline",
                LEVEL_DEADLINE_ENTRY,
                level_deadline_wrapper,
                ReturnKind::Rtl,
            )
        },
    ]
}

/// The addresses of the routines the wrappers hook. The recompiler (`xtask recompile`) must not run through them:
/// generated code calls its callees directly, so a routine it ran inline would never reach its wrapper, and the
/// cheat would be silently skipped in the recompiled game. Each is a stop of the generator: the generated code
/// hands over to the original in front of it, where the wrapper's hook is.
#[must_use]
pub fn entry_points() -> Vec<u32> {
    hook_entries().into_iter().map(|entry| entry.key).collect()
}

/// Installs the cheat wrappers into `table`, replacing the entries at their addresses.
pub fn install(table: &mut HookTable) {
    for entry in hook_entries() {
        table.insert(entry);
    }
}

/// Whether the cheat wrappers are in `table` (the entry of the damage routine is the cheat's). A table that
/// was replaced by another one (a change of the hook modes) does not have them.
#[must_use]
pub fn installed_in(table: Option<&HookTable>) -> bool {
    table
        .and_then(|table| table.get(DAMAGE_ENTRY))
        .is_some_and(|entry| entry.name == "cheat_damage")
}

/// The host events that write the whole block for `cheats`: once, at the start of a run.
#[must_use]
pub fn block_events(cheats: &Cheats) -> Vec<HostEvent> {
    vec![HostEvent::Wram { address: BLOCK, bytes: cheats.to_block().to_vec() }]
}

/// The host event that writes the money multiplier of `cheats` (its pair lies after the marker, outside the settings).
#[must_use]
pub fn money_event(cheats: &Cheats) -> HostEvent {
    HostEvent::Wram { address: BLOCK + MONEY as u32, bytes: cheats.money_bytes().to_vec() }
}

/// The host event that writes the zeni-gained multiplier of `cheats` (its pair lies after the money's).
#[must_use]
pub fn gain_event(cheats: &Cheats) -> HostEvent {
    HostEvent::Wram { address: BLOCK + GAIN as u32, bytes: cheats.gain_bytes().to_vec() }
}

/// The host event that writes the settings of `cheats` (the first bytes of the block: the multipliers, the
/// switches and the generator, reseeded; the runtime bytes stay as the wrappers left them).
#[must_use]
pub fn settings_event(cheats: &Cheats) -> HostEvent {
    HostEvent::Wram { address: BLOCK, bytes: cheats.settings_bytes().to_vec() }
}

/// The host events of a change of the settings, given the work RAM before the frame: the settings, and the
/// Z corners of the hand ([`hand_events`]).
#[must_use]
pub fn settings_events(cheats: &Cheats, wram: &[u8]) -> Vec<HostEvent> {
    let mut events = vec![settings_event(cheats), money_event(cheats), gain_event(cheats)];
    events.extend(hand_events(cheats, wram));
    events
}

/// The writes that give the cards already in the hand the Z corners `cheats` asks for, when the Z cards are
/// for the whole party (the cards dealt later are Z by the dealer's wrapper). A corner is only ever set, never
/// cleared: the cards keep what they show, and a corner that is Z already is not written.
#[must_use]
pub fn hand_events(cheats: &Cheats, wram: &[u8]) -> Vec<HostEvent> {
    let mode = cheats.z_mode();
    if cheats.z_target() != ZTarget::Party || (!mode.attack() && !mode.defence()) {
        return Vec::new();
    }
    let mut events = Vec::new();
    for card in 0..HAND_CARDS {
        let base = HAND - WRAM_BASE + card * CARD_BYTES;
        if wram[base as usize] & 0x80 != 0 {
            continue; // played: the slot is refilled by the dealer, which makes the card Z itself
        }
        for (corner, on) in [(CARD_ATTACK, mode.attack()), (CARD_DEFENCE, mode.defence())] {
            let at = base + corner;
            if on && wram[at as usize] != Z_VALUE {
                events.push(HostEvent::Wram { address: WRAM_BASE + at, bytes: vec![Z_VALUE] });
            }
        }
    }
    events
}

/// The host events due at a frame boundary, given the work RAM before the frame. The scripted-loss gate moves
/// from armed to in battle when the first battle mode shows, and back to zero when the game is on the map again
/// (the gate stays through the states a battle passes through). With the max zeni on, the zeni is put back to
/// its maximum whenever it differs (a purchase, or a sale, lowers it). With a faster encounter rate, the encounter
/// timer is capped where the frame that follows counts it down (a reload of the timer is at most the cap then; the
/// routine that reloads it runs once a frame and needs no hook of its own).
#[must_use]
pub fn frame_events(wram: &[u8]) -> Vec<HostEvent> {
    let at = (BLOCK - WRAM_BASE) as usize;
    if wram[at + MARKER..at + MARKER + MARKER_BYTES.len()] != MARKER_BYTES {
        return Vec::new();
    }
    let mut events = Vec::new();
    events.extend(gate_event(wram, at));
    let zeni_at = (d::ZENI - WRAM_BASE) as usize;
    let zeni = u16::from_le_bytes([wram[zeni_at], wram[zeni_at + 1]]);
    if wram[at + ZENI] != 0 && zeni != MAX_ZENI {
        events.push(HostEvent::Wram { address: d::ZENI, bytes: MAX_ZENI.to_le_bytes().to_vec() });
    }
    let rate_at = at + Setting::Encounters.offset();
    let (num, den) = (wram[rate_at], wram[rate_at + 1]);
    let rate = if num == 0 || den == 0 { Multiplier::new(1, 1) } else { Multiplier::new(num, den) };
    if let Some(cap) = faster_cap(rate) {
        let timer = wram[(ENCOUNTER_TIMER - WRAM_BASE) as usize];
        if timer < 0x80 && timer > cap {
            events.push(HostEvent::Wram { address: ENCOUNTER_TIMER, bytes: vec![cap] });
        }
    }
    events
}

/// The gate's event for the frame: the gate moves on when its state changes (see [`frame_events`]).
fn gate_event(wram: &[u8], at: usize) -> Option<HostEvent> {
    let mode = wram[(GAME_MODE - WRAM_BASE) as usize];
    let gate = wram[at + SUSPENDED];
    let next = match gate {
        GATE_ARMED if BATTLE_MODES.contains(&mode) => GATE_IN_BATTLE,
        GATE_IN_BATTLE if mode == MAP_MODE => 0,
        _ => return None,
    };
    Some(HostEvent::Wram { address: BLOCK + SUSPENDED as u32, bytes: vec![next] })
}

/// The multiplier of `setting` in the block. A pair with a zero is read as neutral (an unset block).
fn multiplier_in(ctx: &mut Ctx<'_>, setting: Setting) -> Multiplier {
    if !live(ctx) {
        return Multiplier::new(1, 1);
    }
    let at = BLOCK + setting.offset() as u32;
    let (num, den) = (ctx.read8(at), ctx.read8(at + 1));
    if num == 0 || den == 0 { Multiplier::new(1, 1) } else { Multiplier::new(num, den) }
}

/// Whether `setting` is on in the block, and does something: not neutral.
fn is_on(multiplier: Multiplier) -> bool {
    !multiplier.is_identity()
}

/// Whether any battle cheat is on in the block.
fn any_on(ctx: &mut Ctx<'_>) -> bool {
    Setting::ALL.iter().any(|&setting| is_on(multiplier_in(ctx, setting)))
}

/// Whether the battle cheats are off for this fight (the scripted-loss gate).
fn suspended(ctx: &mut Ctx<'_>) -> bool {
    live(ctx) && ctx.read8(BLOCK + SUSPENDED as u32) != 0
}

/// Whether the block was written by a run (its marker is there); a block without it is neutral.
fn live(ctx: &mut Ctx<'_>) -> bool {
    MARKER_BYTES.iter().enumerate().all(|(k, &byte)| ctx.read8(BLOCK + (MARKER + k) as u32) == byte)
}

/// The byte of the block at `offset` (zero when the block is not live: a neutral block).
fn switch(ctx: &mut Ctx<'_>, offset: usize) -> u8 {
    if live(ctx) { ctx.read8(BLOCK + offset as u32) } else { 0 }
}

/// The Z corners the block asks for (the bits of [`ZCARDS`]).
fn z_corners(ctx: &mut Ctx<'_>) -> u8 {
    switch(ctx, ZCARDS)
}

/// The character the Z cards are for, or `None` for the whole party (the block holds the id plus one).
fn z_fighter(ctx: &mut Ctx<'_>) -> Option<u8> {
    match switch(ctx, ZCARDS_FIGHTER) {
        0 => None,
        id => Some(id - 1),
    }
}

/// The 24-bit word at `offset` of the direct data bank (`battle_damage` and the like).
fn read24(ctx: &mut Ctx<'_>, offset: u16) -> u32 {
    let low = u32::from(ctx.read_db8(offset));
    let mid = u32::from(ctx.read_db8(offset + 1));
    let high = u32::from(ctx.read_db8(offset + 2));
    low | mid << 8 | high << 16
}

fn write24(ctx: &mut Ctx<'_>, offset: u16, value: u32) {
    ctx.write_db8(offset, value as u8);
    ctx.write_db8(offset + 1, (value >> 8) as u8);
    ctx.write_db8(offset + 2, (value >> 16) as u8);
}

/// `05:BDDC`, the step that takes the damage off the defender's HP (`rts`). The damage the party deals
/// (the defender is the enemy side) is multiplied; the damage the enemies deal to the party is divided
/// by the defence. Only when the damage is not redirected by `$1700` (the original's own cases).
fn damage_wrapper(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    if !suspended(ctx) && ctx.read_db8(DAMAGE_REDIRECT) & 0x03 == 0 {
        let party_acts = ctx.read_db8(ACTIVE_SIDE as u16) & 1 == 0;
        let setting = if party_acts { Setting::Damage } else { Setting::Defence };
        let multiplier = multiplier_in(ctx, setting);
        if is_on(multiplier) {
            let damage = read24(ctx, BATTLE_DAMAGE);
            let scaled = if party_acts {
                multiplier.scale(damage, MAX24)
            } else {
                multiplier.divide(damage, MAX24)
            };
            write24(ctx, BATTLE_DAMAGE, scaled);
        }
    }
    ctx.resume_at(DAMAGE_ENTRY);
}

/// `1D:8C33`, the EXP of one defeated enemy is added to the battle's EXP (`rtl`). The award is the
/// difference over the original's step, multiplied.
fn exp_wrapper(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    let multiplier = multiplier_in(ctx, Setting::Exp);
    if suspended(ctx) || !is_on(multiplier) {
        ctx.resume_at(EXP_ENTRY);
        return;
    }
    let before = read24(ctx, BATTLE_EXP);
    let Ok(()) = ctx.call_original(EXP_ENTRY, ReturnKind::Rtl) else { return };
    let award = read24(ctx, BATTLE_EXP).wrapping_sub(before) & MAX24;
    let total = before.saturating_add(multiplier.scale(award, MAX24));
    write24(ctx, BATTLE_EXP, total.min(MAX24));
    ctx.set_cost(0);
}

/// A num/den pair of the block at `offset`, x1 when the block is not live or the pair holds a zero (an older run's block).
fn block_pair(ctx: &mut Ctx<'_>, offset: usize) -> Multiplier {
    if !live(ctx) {
        return Multiplier::new(1, 1);
    }
    let (num, den) = (ctx.read8(BLOCK + offset as u32), ctx.read8(BLOCK + offset as u32 + 1));
    if num == 0 || den == 0 { Multiplier::new(1, 1) } else { Multiplier::new(num, den) }
}

/// `01:D7F9`, the story event that gives zeni: it adds `$0D53` to the zeni (capped at 999). The amount is scaled by the money
/// multiplier (the Difficulty's) and then by the zeni gained (the Cheats') before the original runs, and the original goes
/// on inline (a jump target, not a call: nothing returns). Both at x1 declines and changes nothing. A battle itself pays no
/// zeni (`battle.md`), so this and selling ([`sale_wrapper`]) are the places the game hands money out.
fn money_wrapper(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    let money = block_pair(ctx, MONEY);
    let gain = block_pair(ctx, GAIN);
    if !is_on(money) && !is_on(gain) {
        ctx.resume_at(MONEY_ENTRY);
        return;
    }
    let amount = u32::from(ctx.read8(MONEY_AMOUNT)) | u32::from(ctx.read8(MONEY_AMOUNT + 1)) << 8;
    let cap = u32::from(MAX_ZENI);
    let scaled = gain.scale(money.scale(amount, cap), cap) as u16;
    ctx.write8(MONEY_AMOUNT, (scaled & 0xFF) as u8);
    ctx.write8(MONEY_AMOUNT + 1, (scaled >> 8) as u8);
    ctx.resume_at(MONEY_ENTRY);
}

/// The tile-map offset of Porunga's wish `row` (0 to 2) on the English text: the first wish's, then one line further each.
fn wish_row(ctx: &mut Ctx<'_>, row: u16) -> u16 {
    let first = u16::from(ctx.read8(WISH_ROWS)) | u16::from(ctx.read8(WISH_ROWS + 1)) << 8;
    first + WISH_ROW_STEP * row
}

/// [`WISH_CURSOR`]: the offset of the wish under the cursor is the row of its text.
fn wish_cursor_wrapper(ctx: &mut Ctx<'_>) {
    let row = u16::from(ctx.read8(d::MAP_ENTRY_VALUE));
    if row < 3 {
        let offset = wish_row(ctx, row);
        ctx.set_a(offset);
    }
    ctx.resume_at(WISH_CURSOR);
}

/// [`WISH_MARK_SECOND`]: the second wish's mark on the second line of the text.
fn wish_mark_second_wrapper(ctx: &mut Ctx<'_>) {
    let offset = wish_row(ctx, 1);
    ctx.set_x(offset);
    ctx.resume_at(WISH_MARK_SECOND);
}

/// [`WISH_MARK_THIRD`]: the third wish's mark on the third line of the text.
fn wish_mark_third_wrapper(ctx: &mut Ctx<'_>) {
    let offset = wish_row(ctx, 2);
    ctx.set_x(offset);
    ctx.resume_at(WISH_MARK_THIRD);
}

/// [`LEVEL_DEADLINE_ENTRY`]: returns (`rtl`) without the deadline, always (a story fix, not a cheat).
fn level_deadline_wrapper(ctx: &mut Ctx<'_>) {
    ctx.set_cost(0);
}

/// `03:A790`, the shop's sale (`rtl`): the original adds the price of the object sold to the zeni and caps it at 999. With the
/// zeni gained above x1 the rest of the price times the multiplier is added first, capped the same way, and the original goes
/// on: the zeni grows by the price times the multiplier, at most 999. An object without a price (`$FF`) is left to the
/// original.
fn sale_wrapper(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    let gain = block_pair(ctx, GAIN);
    let object = ctx.read8(SALE_OBJECT);
    let price = ctx.read8(ITEM_PRICES + u32::from(object));
    if is_on(gain) && price != 0xFF {
        let price = u32::from(price);
        let extra = gain.scale(price, u32::MAX).saturating_sub(price);
        let zeni = u32::from(ctx.read8(d::ZENI)) | u32::from(ctx.read8(d::ZENI + 1)) << 8;
        let total = (zeni + extra).min(u32::from(MAX_ZENI)) as u16;
        ctx.write8(d::ZENI, (total & 0xFF) as u8);
        ctx.write8(d::ZENI + 1, (total >> 8) as u8);
    }
    ctx.resume_at(SALE_ENTRY);
}

/// `01:F692`, the enemy group of a battle is set up (`rtl`). A group of a scripted fight turns the
/// battle cheats off for it. A random encounter's group is rebuilt with the enemies-per-battle
/// multiplier, once the original has set it up.
fn group_wrapper(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    let group = ctx.read_db8(GROUP_INDEX);
    // Any cheat that acts in a fight arms the gate, the Z cards too (a scripted loss is not a place for them).
    if (any_on(ctx) || z_corners(ctx) != 0) && SCRIPTED_LOSS.contains(&group) {
        ctx.write8(BLOCK + SUSPENDED as u32, GATE_ARMED);
    }
    let random = live(ctx) && ctx.read8(BLOCK + RANDOM_ONLY as u32) != 0;
    if !random {
        ctx.resume_at(GROUP_ENTRY);
        return;
    }
    let Ok(()) = ctx.call_original(GROUP_ENTRY, ReturnKind::Rtl) else { return };
    // The mark belongs to this battle only.
    ctx.write8(BLOCK + RANDOM_ONLY as u32, 0);
    if !suspended(ctx) {
        let multiplier = multiplier_in(ctx, Setting::Enemies);
        if is_on(multiplier) {
            rebuild_group(ctx, group, multiplier);
        }
    }
    ctx.set_cost(0);
}

/// Rebuilds the enemies of `group` with `multiplier` times as many (at least one, at most five),
/// repeating the group's units in order, and sets the formation for the new count.
fn rebuild_group(ctx: &mut Ctx<'_>, group: u8, multiplier: Multiplier) {
    let row = d::ENCOUNTER_GROUPS + u32::from(group) * 5;
    let units: Vec<u8> = (0..5).map(|k| ctx.peek8(row + k)).filter(|&id| id != 0xFF).collect();
    if units.is_empty() {
        return;
    }
    let count = multiplier.scale(units.len() as u32, u32::from(ENEMY_SLOT_COUNT)) as u8;
    let positions = FORMATION[usize::from(count) - 1];
    for slot in 0..ENEMY_SLOT_COUNT {
        let base = ENEMY_SLOTS + slot * ENEMY_STRIDE;
        match positions.iter().position(|&at| at == slot) {
            Some(k) => {
                let id = units[k % units.len()];
                ctx.write_db8(base, ENEMY_LIVE);
                ctx.write_db8(base + ENEMY_ID, id);
                ctx.write_db8(base + ENEMY_ID_COPY, id);
            }
            None => ctx.write_db8(base, 0),
        }
    }
    ctx.write_db8(ENEMY_COUNT, count);
    let Ok(()) = ctx.call_original(FORMATION_ENTRY, ReturnKind::Rtl) else { return };
}

/// The largest BP of a wide record: 40 bits (`mods::state`).
const MAX40: u64 = (1 << 40) - 1;

/// Whether the mod state block is live (its marker is in place): only then are the high parts of a wide BP valid.
fn mod_block_live(ctx: &mut Ctx<'_>) -> bool {
    MODS_MARKER.iter().enumerate().all(|(k, &byte)| ctx.read8(MODS_MARKER_AT + k as u32) == byte)
}

/// Multiplies the BP of enemy slot `slot`, whose record is at `base`. With the mod state block live a BP is 40 bits, the
/// record's 24 and the 16 of the slot's high part, and it is multiplied whole and put back in both places (an enemy that
/// `enemy_scaling` made stronger than 24 bits is multiplied as it is). Without the block it is the record's 24 bits, saturating
/// at them.
fn multiply_enemy_bp(ctx: &mut Ctx<'_>, slot: u16, base: u16, multiplier: Multiplier) {
    let low = read24(ctx, base + ENEMY_BP);
    if !mod_block_live(ctx) {
        write24(ctx, base + ENEMY_BP, multiplier.scale(low, MAX24));
        return;
    }
    let high_at = MODS_BLOCK + (MODS_ENEMY_HIGH + 2 * usize::from(slot)) as u32;
    let wide = u64::from(ctx.read16(high_at)) << 24 | u64::from(low);
    let scaled = if wide == 0 {
        0
    } else {
        (wide * u64::from(multiplier.num) / u64::from(multiplier.den)).clamp(1, MAX40)
    };
    write24(ctx, base + ENEMY_BP, (scaled & u64::from(MAX24)) as u32);
    ctx.write16(high_at, (scaled >> 24) as u16);
}

/// `01:F719`, the enemies' stats are filled (`rtl`). The HP, the maximum HP and the BP of every enemy
/// in use are multiplied by the enemy-power multiplier, after the numbers the original and the packs' hooks inside it
/// (the data tables, `enemy_scaling`) have written: the cheat acts on the scaled enemy.
fn power_wrapper(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    let multiplier = multiplier_in(ctx, Setting::Power);
    if suspended(ctx) || !is_on(multiplier) {
        ctx.resume_at(POWER_ENTRY);
        return;
    }
    let Ok(()) = ctx.call_original(POWER_ENTRY, ReturnKind::Rtl) else { return };
    {
        for slot in 0..ENEMY_SLOT_COUNT {
            let base = ENEMY_SLOTS + slot * ENEMY_STRIDE;
            if ctx.read_db8(base) & ENEMY_LIVE == 0 {
                continue;
            }
            let hp = u32::from(ctx.read_db8(base + ENEMY_HP))
                | u32::from(ctx.read_db8(base + ENEMY_HP + 1)) << 8;
            let max_hp = u32::from(ctx.read_db8(base + ENEMY_MAX_HP))
                | u32::from(ctx.read_db8(base + ENEMY_MAX_HP + 1)) << 8;
            let hp = multiplier.scale(hp, MAX16);
            let max_hp = multiplier.scale(max_hp, MAX16);
            ctx.write_db8(base + ENEMY_HP, hp as u8);
            ctx.write_db8(base + ENEMY_HP + 1, (hp >> 8) as u8);
            ctx.write_db8(base + ENEMY_MAX_HP, max_hp as u8);
            ctx.write_db8(base + ENEMY_MAX_HP + 1, (max_hp >> 8) as u8);
            multiply_enemy_bp(ctx, slot, base, multiplier);
        }
    }
    ctx.set_cost(0);
}

/// `02:D34A`, the group of a random encounter is picked (`rtl`). The group is marked as a random
/// encounter, so that its enemies may be rebuilt by [`group_wrapper`].
fn pick_wrapper(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    if !any_on(ctx) {
        ctx.resume_at(PICK_ENTRY);
        return;
    }
    let Ok(()) = ctx.call_original(PICK_ENTRY, ReturnKind::Rtl) else { return };
    ctx.write8(BLOCK + RANDOM_ONLY as u32, 1);
    ctx.set_cost(0);
}

/// `01:C257`, a card is dealt (`rtl`). The routine leaves the attack and defence values in `$10` and `$11`;
/// for the whole party the chosen corners become Z (8). One fighter's cards are changed in the battle instead
/// ([`fighter_wrapper`]), because the hand is shared.
fn deal_wrapper(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    let corners = z_corners(ctx);
    if suspended(ctx) || z_fighter(ctx).is_some() || corners & (Z_ATTACK | Z_DEFENCE) == 0 {
        ctx.resume_at(DEAL_ENTRY);
        return;
    }
    let Ok(()) = ctx.call_original(DEAL_ENTRY, ReturnKind::Rtl) else { return };
    if corners & Z_ATTACK != 0 {
        ctx.write_dp8(0x10, Z_VALUE);
    }
    if corners & Z_DEFENCE != 0 {
        ctx.write_dp8(0x11, Z_VALUE);
    }
    ctx.set_cost(0);
}

/// `05:B801` (`rtl`), the damage routine's entry for the party's attack and the counter-attacks: see
/// [`fighter_wrapper`].
fn fighter_b801_wrapper(ctx: &mut Ctx<'_>) {
    fighter_wrapper(ctx, FIGHTER_ENTRY_B801);
}

/// `05:B7FC` (`rtl`), the damage routine's other entry: see [`fighter_wrapper`].
fn fighter_b7fc_wrapper(ctx: &mut Ctx<'_>) {
    fighter_wrapper(ctx, FIGHTER_ENTRY_B7FC);
}

/// The damage routine's two entries (`rtl`), for the Z cards of one fighter. The party's fighter is side A: its
/// attack number is its record's `+14` (when it attacks) and its defence number is `+15` (when it is attacked, and
/// for a counter-attack). Its cards are changed in the record before the damage is computed, when its character
/// id (`+1C`) is the chosen one; the hand is not touched.
fn fighter_wrapper(ctx: &mut Ctx<'_>, entry: u32) {
    ctx.mark_gated();
    if !suspended(ctx)
        && let Some(fighter) = z_fighter(ctx)
        && ctx.read8(SIDE_A + SIDE_CHARACTER) == fighter
    {
        let corners = z_corners(ctx);
        if corners & Z_ATTACK != 0 {
            ctx.write8(SIDE_A + SIDE_ATTACK, Z_VALUE);
        }
        if corners & Z_DEFENCE != 0 {
            ctx.write8(SIDE_A + SIDE_DEFENCE, Z_VALUE);
        }
    }
    ctx.resume_at(entry);
}

/// `02:D289`, the battle-start decision of a roll (`rtl`). A normal battle (carry clear) is vetoed when random
/// battles are off, and with probability 1 - m when the encounter rate is m below x1. A veto sets carry (the
/// caller then starts nothing), drops a queued chain battle, and on maps 22 and 24 counts the battle in
/// `$01B0` as the original would have, so the story's counter still advances. Story battles (carry set) are
/// not touched.
fn roll_wrapper(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    if !rolls_are_changed(ctx) {
        ctx.resume_at(ROLL_ENTRY);
        return;
    }
    let Ok(()) = ctx.call_original(ROLL_ENTRY, ReturnKind::Rtl) else { return };
    if !ctx.c() && vetoed(ctx) {
        ctx.set_c(true);
        if ctx.read8(CHAIN) & 0x80 != 0 {
            ctx.write8(CHAIN, 0);
        }
        let map = ctx.read8(MAP_ID);
        if map == STORY_MAP_22 || map == STORY_MAP_24 {
            let count = ctx.read8(BATTLE_COUNTER);
            ctx.write8(BATTLE_COUNTER, count.wrapping_add(1));
        }
    }
    ctx.set_cost(0);
}

/// Whether the rolls of random battles are changed at all: random battles off, or an encounter rate below x1.
fn rolls_are_changed(ctx: &mut Ctx<'_>) -> bool {
    if !live(ctx) {
        return false;
    }
    let rate = multiplier_in(ctx, Setting::Encounters);
    ctx.read8(BLOCK + RANDOM_OFF as u32) != 0 || rate.num < rate.den
}

/// Whether the normal battle a roll starts is vetoed: always with random battles off, else with the encounter
/// rate below x1 with the chance the rate leaves (the block's generator draws it, once per roll).
fn vetoed(ctx: &mut Ctx<'_>) -> bool {
    if !live(ctx) {
        return false;
    }
    if ctx.read8(BLOCK + RANDOM_OFF as u32) != 0 {
        return true;
    }
    let rate = multiplier_in(ctx, Setting::Encounters);
    if rate.num >= rate.den {
        return false;
    }
    !kept(next_random(ctx), rate)
}

/// `01:C639`, the entry of a used object is compacted out of the list and the count goes down (`rtl`). With the
/// endless items on, an object used by one of the effect tails stays: the entry the effect cleared is put back,
/// the original runs (its registers and flags as ever) and the count is restored. Drop, the shop and the story's
/// removals are not effect tails and go on as the original.
fn item_wrapper(ctx: &mut Ctx<'_>) {
    ctx.mark_gated();
    if switch(ctx, ENDLESS) == 0 || !effect_tail(ctx) {
        ctx.resume_at(ITEM_ENTRY);
        return;
    }
    let entry = u32::from(ctx.x());
    let object = ctx.read8(MENU_ITEM_ID);
    let attribute = ctx.read8(ITEM_ATTRIBUTES + u32::from(object));
    let count = ctx.read8(ITEM_COUNT);
    ctx.write8(ITEM_LIST + entry, object);
    ctx.write8(ITEM_LIST + entry + 1, attribute);
    let Ok(()) = ctx.call_original(ITEM_ENTRY, ReturnKind::Rtl) else { return };
    ctx.write8(ITEM_COUNT, count);
    ctx.set_cost(0);
}

/// Whether the routine was called from an effect tail: the return address on the stack (the `JSL`'s, plus one)
/// is one of [`EFFECT_TAILS`]. The stack holds the return's low byte, high byte and bank from the stack pointer up.
fn effect_tail(ctx: &mut Ctx<'_>) -> bool {
    let stack = ctx.s();
    let byte = |ctx: &Ctx<'_>, k: u16| u32::from(ctx.peek8(u32::from(stack.wrapping_add(k))));
    let pushed = byte(ctx, 1) | byte(ctx, 2) << 8 | byte(ctx, 3) << 16;
    is_effect_tail(pushed.wrapping_add(1) & 0xFF_FFFF)
}

/// Whether `target`, a return address, is one of the effect tails.
fn is_effect_tail(target: u32) -> bool {
    EFFECT_TAILS.contains(&target)
}

/// The cap of the encounter timer for a rate above x1: `59 * den / num`, so the roll comes `num / den` times as
/// often. `None` for a rate of x1 or below.
fn faster_cap(rate: Multiplier) -> Option<u8> {
    if rate.num <= rate.den {
        return None;
    }
    Some((u32::from(ENCOUNTER_RELOAD) * u32::from(rate.den) / u32::from(rate.num)) as u8)
}

/// Whether a roll with `draw` (uniform in `0..den`) keeps the battle at the rate `rate` below x1: a draw below the
/// numerator, so the battle is kept `num / den` of the time.
fn kept(draw: u32, rate: Multiplier) -> bool {
    draw % u32::from(rate.den) < u32::from(rate.num)
}

/// One step of the block's generator, an xorshift32: the state is never zero (zero would stay zero).
fn xorshift(state: u32) -> u32 {
    let state = if state == 0 { GENERATOR_SEED } else { state };
    let state = state ^ (state << 13);
    let state = state ^ (state >> 17);
    state ^ (state << 5)
}

/// The block's generator: one step, its state kept in the block (little-endian at [`GENERATOR`]). Returns the
/// new state, which is a draw.
fn next_random(ctx: &mut Ctx<'_>) -> u32 {
    let at = BLOCK + GENERATOR as u32;
    let state = u32::from_le_bytes([
        ctx.read8(at),
        ctx.read8(at + 1),
        ctx.read8(at + 2),
        ctx.read8(at + 3),
    ]);
    let state = xorshift(state);
    for (k, byte) in state.to_le_bytes().into_iter().enumerate() {
        ctx.write8(at + k as u32, byte);
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use ssd_menus::cheats::BLOCK_LEN;

    #[test]
    fn the_block_is_in_the_upper_work_ram_outside_the_save_image() {
        assert_eq!(BLOCK & 0xFFFF_0000, 0x7F_0000, "bank $7F, above the save image at $7E:2800");
        assert_eq!(BLOCK + BLOCK_LEN as u32 - 1, 0x7F_FF1F, "the block ends at $7F:FF1F");
    }

    #[test]
    fn each_wrapper_has_its_routine_and_return_kind() {
        let entries = hook_entries();
        let mut keys: Vec<u32> = entries.iter().map(|e| e.key).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), entries.len(), "one wrapper per routine");
        for entry in &entries {
            let story = entry.name.starts_with("story_");
            assert_eq!(entry.mode, if story { HookMode::Patch } else { HookMode::Rust });
            assert_eq!(entry.cycles, Some(0));
            assert!(matches!(entry.action, HookAction::Routine { .. }));
            assert!(
                entry.name.starts_with("cheat_") || entry.name.starts_with("story_"),
                "{}",
                entry.name
            );
        }
        let rtl = entries
            .iter()
            .filter(|e| matches!(e.action, HookAction::Routine { ret: ReturnKind::Rtl, .. }))
            .count();
        assert_eq!(rtl, entries.len() - 1, "all but the damage routine return with rtl");
    }

    #[test]
    fn the_formation_puts_each_count_in_its_slots() {
        for (index, slots) in FORMATION.iter().enumerate() {
            assert_eq!(slots.len(), index + 1);
            assert!(slots.iter().all(|&slot| slot < ENEMY_SLOT_COUNT));
        }
    }

    #[test]
    fn a_block_without_the_marker_is_neutral_whatever_its_bytes() {
        let mut wram = vec![0x55_u8; 0x20000];
        let at = (BLOCK - WRAM_BASE) as usize;
        wram[(GAME_MODE - WRAM_BASE) as usize] = 2;
        wram[at + SUSPENDED] = GATE_ARMED;
        wram[at + ZENI] = 1;
        assert!(frame_events(&wram).is_empty(), "the power-on fill is not a gate or a cheat");
        wram[at + MARKER..at + MARKER + MARKER_BYTES.len()].copy_from_slice(&MARKER_BYTES);
        assert_eq!(
            frame_events(&wram),
            vec![
                HostEvent::Wram { address: BLOCK + SUSPENDED as u32, bytes: vec![GATE_IN_BATTLE] },
                HostEvent::Wram { address: d::ZENI, bytes: MAX_ZENI.to_le_bytes().to_vec() },
            ]
        );
    }

    #[test]
    fn the_gate_is_armed_then_in_battle_then_off_on_the_map() {
        let mut wram = vec![0_u8; 0x20000];
        let block = (BLOCK - WRAM_BASE) as usize;
        wram[block + MARKER..block + MARKER + MARKER_BYTES.len()].copy_from_slice(&MARKER_BYTES);
        let gate_at = block + SUSPENDED;
        let mode_at = (GAME_MODE - WRAM_BASE) as usize;
        let write =
            |byte: u8| HostEvent::Wram { address: BLOCK + SUSPENDED as u32, bytes: vec![byte] };
        wram[gate_at] = GATE_ARMED;
        wram[mode_at] = 0;
        assert!(frame_events(&wram).is_empty(), "armed, the battle has not started");
        wram[mode_at] = 2;
        assert_eq!(frame_events(&wram), vec![write(GATE_IN_BATTLE)], "the battle has started");
        wram[gate_at] = GATE_IN_BATTLE;
        for mode in [0x0C, 0x82, 0x84, 0x8C, 4] {
            wram[mode_at] = mode;
            assert!(frame_events(&wram).is_empty(), "mode {mode:#04x} is the battle going on");
        }
        wram[mode_at] = 0;
        assert_eq!(frame_events(&wram), vec![write(0)], "back on the map");
        wram[gate_at] = 0;
        assert!(frame_events(&wram).is_empty(), "nothing to clear");
    }

    #[test]
    fn the_zeni_is_put_back_to_the_maximum_only_when_the_cheat_is_on_and_it_differs() {
        let mut cheats = Cheats::neutral();
        cheats.set_zeni(true);
        let mut wram = vec![0_u8; 0x20000];
        let block = (BLOCK - WRAM_BASE) as usize;
        let zeni = (d::ZENI - WRAM_BASE) as usize;
        wram[block + MARKER..block + MARKER + MARKER_BYTES.len()].copy_from_slice(&MARKER_BYTES);
        wram[block..block + BLOCK_LEN].copy_from_slice(&cheats.to_block());
        wram[zeni..zeni + 2].copy_from_slice(&480_u16.to_le_bytes());
        let write = HostEvent::Wram { address: d::ZENI, bytes: vec![0xE7, 0x03] };
        assert_eq!(frame_events(&wram), vec![write.clone()], "480 is not the maximum");
        wram[zeni..zeni + 2].copy_from_slice(&999_u16.to_le_bytes());
        assert!(frame_events(&wram).is_empty(), "999 is the maximum: no write");
        wram[zeni..zeni + 2].copy_from_slice(&40_u16.to_le_bytes());
        wram[block + ZENI] = 0;
        assert!(frame_events(&wram).is_empty(), "the cheat is off");
    }

    #[test]
    fn the_encounter_timer_is_capped_once_a_frame_for_a_faster_rate_only() {
        let mut cheats = Cheats::neutral();
        cheats.set_index(Setting::Encounters, 3); // x2
        let mut wram = vec![0_u8; 0x20000];
        let block = (BLOCK - WRAM_BASE) as usize;
        wram[block..block + BLOCK_LEN].copy_from_slice(&cheats.to_block());
        let timer = (ENCOUNTER_TIMER - WRAM_BASE) as usize;
        let capped = HostEvent::Wram { address: ENCOUNTER_TIMER, bytes: vec![29] };
        wram[timer] = 59;
        assert_eq!(frame_events(&wram), vec![capped.clone()], "a reload of 59 is capped at 29");
        wram[timer] = 29;
        assert!(frame_events(&wram).is_empty(), "at the cap: nothing");
        wram[timer] = 0x9F;
        assert!(frame_events(&wram).is_empty(), "a negative timer is left to its reload");
        cheats.set_index(Setting::Encounters, 2); // x1
        wram[block..block + BLOCK_LEN].copy_from_slice(&cheats.to_block());
        wram[timer] = 59;
        assert!(frame_events(&wram).is_empty(), "x1 is not faster");
    }

    #[test]
    fn the_z_corners_of_the_hand_are_written_for_the_party_only_and_only_where_not_yet_z() {
        let mut cheats = Cheats::neutral();
        cheats.set_z_mode(ssd_menus::cheats::ZMode::Both);
        let mut wram = vec![0_u8; 0x20000];
        let hand = (HAND - WRAM_BASE) as usize;
        // Card 0 dealt with values 3 and 5, card 1 played (flag bit 7), card 2 dealt with 8 in attack, cards
        // 3 and 4 played as well.
        wram[hand] = 0x20;
        wram[hand + 3] = 3;
        wram[hand + 4] = 5;
        wram[hand + 6] = 0x80;
        wram[hand + 12] = 0x20;
        wram[hand + 15] = 8;
        wram[hand + 16] = 2;
        wram[hand + 18] = 0x80;
        wram[hand + 24] = 0x80;
        let events = hand_events(&cheats, &wram);
        let at = |offset: usize, value: u8| HostEvent::Wram {
            address: HAND + offset as u32,
            bytes: vec![value],
        };
        assert_eq!(
            events,
            vec![at(3, 8), at(4, 8), at(16, 8)],
            "the dealt cards 0 and 2 get the corners that are not Z yet; the played card 1 is left"
        );
        cheats.set_z_target(ZTarget::Fighter(0));
        assert!(hand_events(&cheats, &wram).is_empty(), "one fighter: the hand is not touched");
        assert!(hand_events(&Cheats::neutral(), &wram).is_empty(), "neutral: nothing");
    }

    #[test]
    fn the_settings_reseed_the_generator_and_the_hand_follows_a_z_change() {
        let mut cheats = Cheats::neutral();
        cheats.set_z_mode(ssd_menus::cheats::ZMode::Attack);
        let wram = vec![0_u8; 0x20000];
        let events = settings_events(&cheats, &wram);
        let HostEvent::Wram { address, bytes } = &events[0] else { panic!("the settings") };
        assert_eq!(*address, BLOCK);
        assert_eq!(&bytes[GENERATOR..GENERATOR + 4], &GENERATOR_SEED.to_le_bytes());
        assert_eq!(bytes.len(), ssd_menus::cheats::SETTINGS_LEN);
    }

    #[test]
    fn the_generator_never_stays_at_zero_and_does_not_repeat_at_once() {
        assert_ne!(xorshift(0), 0, "zero is reseeded");
        let first = xorshift(GENERATOR_SEED);
        let second = xorshift(first);
        assert_ne!(first, GENERATOR_SEED);
        assert_ne!(first, second);
    }

    #[test]
    fn a_rate_below_x1_keeps_its_share_of_the_draws_and_a_faster_one_caps_the_timer() {
        let quarter = Multiplier::new(1, 4);
        let kept_of_4 = (0..4).filter(|&draw| kept(draw, quarter)).count();
        assert_eq!(kept_of_4, 1, "x1/4 keeps one draw in four");
        let half = Multiplier::new(1, 2);
        assert_eq!((0..2).filter(|&draw| kept(draw, half)).count(), 1, "x1/2 keeps half");
        assert_eq!(faster_cap(Multiplier::new(2, 1)), Some(29), "x2 caps at 29, every 60 frames");
        assert_eq!(faster_cap(Multiplier::new(3, 1)), Some(19));
        assert_eq!(faster_cap(Multiplier::new(4, 1)), Some(14), "x4 caps at 14, every 30 frames");
        assert_eq!(faster_cap(Multiplier::new(1, 1)), None, "x1 is not faster");
        assert_eq!(faster_cap(half), None, "a slower rate is a veto, not a cap");
    }

    #[test]
    fn only_the_five_effect_tails_keep_an_object() {
        assert!(is_effect_tail(0x01_DDF3), "the common tail");
        assert!(is_effect_tail(0x01_EB33), "Vegeta");
        assert!(is_effect_tail(0x01_EBB3), "Escape");
        assert!(is_effect_tail(0x01_E35E), "Porunga");
        assert!(is_effect_tail(0x01_DD8A));
        assert!(!is_effect_tail(0x01_D844), "the story's removal stays original");
        assert!(!is_effect_tail(0x01_8E50), "Drop stays original");
    }

    #[test]
    fn a_neutral_block_writes_the_block_as_zero_runtime_and_identity_settings() {
        let events = block_events(&Cheats::neutral());
        let HostEvent::Wram { address, bytes } = &events[0] else { panic!("a Wram event") };
        assert_eq!(*address, BLOCK);
        assert_eq!(bytes.len(), BLOCK_LEN);
        assert!(
            bytes[SUSPENDED..MARKER].iter().all(|&byte| byte == 0),
            "the runtime bytes start at zero"
        );
        assert!(bytes[..RANDOM_OFF].chunks(2).all(|pair| pair[0] == pair[1]), "identity");
        assert!(bytes[RANDOM_OFF..GENERATOR].iter().all(|&byte| byte == 0), "no switch is on");
    }
}
