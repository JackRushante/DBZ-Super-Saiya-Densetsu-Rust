//! The wide BP in the damage routine (M10, Task 9, part C.1): see [`super::state`] for the layout.
//!
//! The damage body runs from `05:B804`, common to its entries `05:B7FC` (Y = 3) and `05:B801` (Y = 2), to its return. It
//! reads a side's BP only through `05:B6F2`, which loads the 24 bits at `+07..+09` of the side record and adds the class
//! bonus. Its last such read is the `JSL` at `05:B8D8`, which loads the BP of the other side. `05:B8DC` is the first
//! instruction after it: from there the body works on the values it has loaded, and no BP is read between it and the
//! divider (`05:B917`). The callees on the body's paths do not read those bytes (checked by listing).
//!
//! The two hooks bracket the body. At `05:B804`, when the block is live and a side has a high part, both sides' low parts
//! are replaced by their wide BPs shifted right by one common amount ([`super::state::damage_shift`]), so that the body's
//! 24-bit arithmetic holds. The body's damage is a ratio of its BPs (degree zero in them), so the shift keeps it up to
//! rounding. The original low parts are kept in the block's scratch. At `05:B8DC` they go back into the records, which the
//! body does not read again, and the scratch is cleared.
//!
//! The scratch is cleared when the state is applied ([`super::state::apply_events`]), so while the block is live it holds
//! zero except between the entry and the exit of a call that shifted the sides: the exit acts only on that shift. A neutral
//! block is written by neither hook, so a run without state is the original's.
//!
//! Both hooks use bus addresses only, so they do not depend on the widths of M and X or on D and DBR, and they change no
//! register. The damage word `$14CF..$14D1` is written by the body after the exit.

use super::state::{
    BLOCK, MARKER, MARKER_AT, SCRATCH_SHIFT, SCRATCH_SIDE_A_LOW, SCRATCH_SIDE_B_LOW, SIDE_HIGH,
    damage_shift, side_record, wide,
};
use ssd_oracle::{Ctx, HookAction, HookEntry, HookMode, ReturnKind, RoutineFn};

/// `05:B804`: the body's first instruction (`PHY`), where the sides are shifted (see [`shift_wrapper`]).
pub(super) const ENTRY_KEY: u32 = 0x05_B804;
/// `05:B8DC`: the instruction after the last read of a side record (the `JSL` at `05:B8D8`), where they are restored (see
/// [`restore_wrapper`]).
pub(super) const EXIT_KEY: u32 = 0x05_B8DC;

/// The offset of a side record's 24-bit BP (`+07..+09`).
const BP_OFFSET: u32 = 0x07;

/// The hook entries of the damage: the shift at `05:B804` and the restore at `05:B8DC`, both `Patch`.
pub(super) fn hook_entries() -> Vec<HookEntry> {
    let patch = |name: &str, key: u32, func: RoutineFn| HookEntry {
        name: name.to_owned(),
        key,
        mode: HookMode::Patch,
        action: HookAction::Routine { func, ret: ReturnKind::Rtl },
        entry_m8: None,
        entry_x8: None,
        flags_mask: 0,
        verify_sample: 1,
        cycles: Some(0),
        bound: None,
        budget_frames: None,
    };
    vec![
        patch("mod_damage_shift", ENTRY_KEY, shift_wrapper),
        patch("mod_damage_restore", EXIT_KEY, restore_wrapper),
    ]
}

/// The shift of a damage pair, from the low and high parts of its two BPs (`a` and `d`): `None` when neither has a high
/// part, so the pair keeps its 24-bit values. Otherwise the [`damage_shift`] of the two wide BPs, which is at least one
/// when a high part is nonzero.
#[must_use]
pub(super) fn shift_for(a_low: u32, a_high: u16, d_low: u32, d_high: u16) -> Option<u32> {
    (a_high != 0 || d_high != 0).then(|| damage_shift(wide(a_low, a_high), wide(d_low, d_high)))
}

/// The low 24 bits a side record takes for its wide BP shifted right by `shift` bits.
#[must_use]
pub(super) fn shifted_low(low: u32, high: u16, shift: u32) -> u32 {
    (wide(low, high).checked_shr(shift).unwrap_or(0) & 0xFF_FFFF) as u32
}

/// The bus address of the block's byte at `offset`.
fn block_at(offset: usize) -> u32 {
    BLOCK + offset as u32
}

/// Whether the block is live: its marker is at `$7F:FAFC`, read as the bus reads it. A neutral block has no high parts.
fn block_is_live(ctx: &mut Ctx<'_>) -> bool {
    MARKER.iter().enumerate().all(|(k, &byte)| ctx.read8(MARKER_AT + k as u32) == byte)
}

/// Reads the 24 bits at the bus address `at`, low byte first.
fn read24(ctx: &mut Ctx<'_>, at: u32) -> u32 {
    u32::from(ctx.read16(at)) | (u32::from(ctx.read8(at + 2)) << 16)
}

/// Writes the low 24 bits of `value` at the bus address `at`, low byte first.
fn write24(ctx: &mut Ctx<'_>, at: u32, value: u32) {
    let [low, mid, high, _] = value.to_le_bytes();
    ctx.write8(at, low);
    ctx.write8(at + 1, mid);
    ctx.write8(at + 2, high);
}

/// `05:B804` (`PHY`): when the block is live and a side has a high part, both sides' low parts are replaced by their shifted
/// wide BPs, and the originals and the shift are kept in the scratch for [`restore_wrapper`]. Otherwise nothing is written.
/// The shift is at most 17, as a 40-bit BP shifts into 24 bits by that much, so it fits the scratch byte.
fn shift_wrapper(ctx: &mut Ctx<'_>) {
    if block_is_live(ctx) {
        let (a, d) = (side_record(false), side_record(true));
        let (a_low, d_low) = (read24(ctx, a + BP_OFFSET), read24(ctx, d + BP_OFFSET));
        let (a_high, d_high) =
            (ctx.read16(block_at(SIDE_HIGH)), ctx.read16(block_at(SIDE_HIGH + 2)));
        if let Some(shift) = shift_for(a_low, a_high, d_low, d_high) {
            ctx.write8(block_at(SCRATCH_SHIFT), shift as u8);
            write24(ctx, block_at(SCRATCH_SIDE_A_LOW), a_low);
            write24(ctx, block_at(SCRATCH_SIDE_B_LOW), d_low);
            write24(ctx, a + BP_OFFSET, shifted_low(a_low, a_high, shift));
            write24(ctx, d + BP_OFFSET, shifted_low(d_low, d_high, shift));
        }
    }
    ctx.resume_at(ENTRY_KEY);
}

/// `05:B8DC` (`REP #$20`, after the last read of a side record): when the block is live and [`shift_wrapper`] shifted the
/// sides in this call, their original low parts go back into the records and the scratch is cleared. Registers and the
/// damage word are not touched.
fn restore_wrapper(ctx: &mut Ctx<'_>) {
    if block_is_live(ctx) && ctx.read8(block_at(SCRATCH_SHIFT)) != 0 {
        let a_low = read24(ctx, block_at(SCRATCH_SIDE_A_LOW));
        let d_low = read24(ctx, block_at(SCRATCH_SIDE_B_LOW));
        write24(ctx, side_record(false) + BP_OFFSET, a_low);
        write24(ctx, side_record(true) + BP_OFFSET, d_low);
        ctx.write8(block_at(SCRATCH_SHIFT), 0);
    }
    ctx.resume_at(EXIT_KEY);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::state::split;

    /// Side A's BP in the tests: 50 000 000, the high part 2 and the low part `0xFAF080`.
    const GOKU_LOW: u32 = 0xFAF080;
    const GOKU_HIGH: u16 = 2;

    #[test]
    fn a_pair_without_high_parts_is_declined() {
        assert_eq!(shift_for(GOKU_LOW, 0, 1_000, 0), None);
        assert_eq!(shift_for(0xFF_FFFF, 0, 0xFF_FFFF, 0), None, "24-bit BPs keep their values");
    }

    #[test]
    fn a_high_part_on_either_side_shifts_the_pair_by_at_least_one() {
        assert_eq!(
            shift_for(GOKU_LOW, GOKU_HIGH, 1_000, 0),
            Some(2),
            "50 M against 1 000: two shifts"
        );
        assert_eq!(
            shift_for(1_000, 0, GOKU_LOW, GOKU_HIGH),
            Some(2),
            "the same pair with the sides swapped"
        );
        assert_eq!(shift_for(0, 1, 0, 0), Some(1), "2^24 needs one shift for its class bonus");
        for high in [1, 2, 0x100, 0xFFFF] {
            for low in [0, 1, 0xFF_FFFF] {
                let shift = shift_for(low, high, 0, 0);
                assert!(shift.is_some_and(|s| s >= 1), "high {high:#x}, low {low:#x}: {shift:?}");
            }
        }
    }

    #[test]
    fn the_shifted_pair_keeps_the_ratio_and_fits_the_class_bonus() {
        let shift = shift_for(GOKU_LOW, GOKU_HIGH, 1_000, 0).unwrap();
        let a = shifted_low(GOKU_LOW, GOKU_HIGH, shift);
        let d = shifted_low(1_000, 0, shift);
        assert_eq!((a, d), (12_500_000, 250), "50 M and 1 000 after two shifts");
        assert_eq!(u64::from(a) * 1_000, 50_000_000 * u64::from(d), "the same ratio, 50 000");
        assert!(a + a / 4 <= 0xFF_FFFF, "the larger side with its class bonus fits 24 bits");
    }

    #[test]
    fn any_shifted_pair_keeps_its_ratio_up_to_rounding_and_fits_the_class_bonus() {
        // The shifted parts are the wide BPs shifted by `s` (no bit is lost to the mask). Each is a floor, so the cross
        // products differ from the true ones by at most the larger BP.
        let pairs = [
            (50_000_000_u64, 1_000_u64),
            (50_000_000, 4_000_000),
            (wide(0xFF_FFFF, 0xFFFF), 1),
            (1 << 24, 3),
            (12_345_678_901, 98_765),
        ];
        for (a, d) in pairs {
            let (a_low, a_high) = split(a);
            let (d_low, d_high) = split(d);
            let shift = shift_for(a_low, a_high, d_low, d_high).unwrap();
            let sa = u64::from(shifted_low(a_low, a_high, shift));
            let sd = u64::from(shifted_low(d_low, d_high, shift));
            assert_eq!(sa, a >> shift, "no bit is lost to the 24-bit mask");
            assert_eq!(sd, d >> shift, "no bit is lost to the 24-bit mask");
            assert!(
                sa + sa / 4 <= 0xFF_FFFF,
                "the larger side with its class bonus fits 24 bits: {a} {d}"
            );
            let error = (u128::from(sa) * u128::from(d)).abs_diff(u128::from(a) * u128::from(sd));
            assert!(error <= u128::from(a.max(d)), "the ratio is kept up to rounding: {a} {d}");
        }
    }

    #[test]
    fn shifting_by_nothing_gives_back_the_low_part_whatever_the_high_part() {
        assert_eq!(shifted_low(GOKU_LOW, GOKU_HIGH, 0), GOKU_LOW, "the restore's inverse");
        assert_eq!(shifted_low(0x12_3456, 0xFFFF, 0), 0x12_3456);
    }

    #[test]
    fn the_hooks_are_the_first_and_last_instructions_of_the_body_and_no_cheat_key() {
        assert_eq!(ENTRY_KEY, 0x05_B804);
        assert_eq!(EXIT_KEY, 0x05_B8DC);
        let keys: Vec<u32> = hook_entries().iter().map(|entry| entry.key).collect();
        assert_eq!(keys, [ENTRY_KEY, EXIT_KEY]);
        let cheats = crate::cheats::entry_points();
        for key in keys {
            assert!(!cheats.contains(&key), "${key:06X} is a cheat's key");
        }
    }

    /// The damage routine run headless on the ROM, from `$7F:FE00` as `05:B7FC` with Y = 3. Side A is the attacker (the
    /// active side) and side B the defender. The other fields the body reads are plausible values staged here, not the
    /// battle's: the test compares runs that stage the same values, so it does not depend on them.
    mod damage_rom {
        use super::{GOKU_HIGH, GOKU_LOW};
        use crate::mods::state::{BLOCK, SCRATCH_SHIFT, SIDE_HIGH, apply_events, side_record};
        use ssd_menus::mods_state::{HIGH_BYTES, ModState};
        use ssd_oracle::{
            HookTable, HostEvent, Joypad, Machine, MemorySaveStore, NullAudio, StatusRegister,
            Stopped, Stops,
        };

        /// The bus address of the call stub: `jsl $05:B7FC` is written here and run from `$7F:FE00`.
        const STUB: u32 = 0x7F_FE00;
        /// The address the stub returns to (after its four bytes).
        const BACK: u32 = STUB + 4;
        /// The scratch's power-on fill in the tests (the console's work RAM starts random): a value no shift writes, so an
        /// exit that trusted the scratch would corrupt the records.
        const FILL: u8 = 0xCB;

        /// What one call leaves: the damage word (`$14CF..$14D1`), both side records' low parts, and the scratch's shift.
        #[derive(Debug, PartialEq, Eq)]
        struct Outcome {
            damage: u32,
            side_a_low: u32,
            side_b_low: u32,
            shift: u8,
        }

        /// The 24 bits at the work RAM address `at` (`$7E:0000` is index 0), low byte first.
        fn read24(wram: &[u8], at: u32) -> u32 {
            let at = (at - 0x7E_0000) as usize;
            u32::from_le_bytes([wram[at], wram[at + 1], wram[at + 2], 0])
        }

        /// One call of the damage routine with side A's low part `a_low` and side B's `b_low`. `high` is the block's high
        /// parts of sides A and B, with the block live; `None` leaves the block neutral.
        fn call(a_low: u32, b_low: u32, high: Option<(u16, u16)>) -> Outcome {
            let raw = std::fs::read(std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH")).unwrap();
            let rom = ssd_rom::normalize(&raw, None).unwrap().bytes;
            let image = ssd_mod::build(&rom, &[]).unwrap().bytes;
            let mut machine = Machine::with_image(
                image,
                ssd_rom::crc32(&rom),
                Box::new(MemorySaveStore::default()),
                ssd_oracle::DEFAULT_WRAM_SEED,
            )
            .unwrap();
            let mut table = HookTable::new();
            crate::mods::install(&mut table);
            machine.set_hook_table(Some(table));
            // Boot: the title runs with no input, so the work RAM is the game's own before the records are staged.
            let frame = Stops { frame_end: true, ..Stops::default() };
            for _ in 0..300 {
                machine.run(&frame, &mut |_| Joypad::default(), &mut NullAudio, 1).unwrap();
            }
            // A live block is applied as a state is (`apply_events`: the markers, the high parts, and the scratch at zero). A
            // neutral one keeps the power-on fill in the scratch (`+40` to `+46`), which no hook may read or write.
            match high {
                Some((a_high, b_high)) => {
                    let mut bytes = vec![0; HIGH_BYTES];
                    bytes[SIDE_HIGH..SIDE_HIGH + 2].copy_from_slice(&a_high.to_le_bytes());
                    bytes[SIDE_HIGH + 2..SIDE_HIGH + 4].copy_from_slice(&b_high.to_le_bytes());
                    let state = ModState { flags: None, high: Some(bytes) };
                    for event in apply_events(&state) {
                        if let HostEvent::Wram { address, bytes } = event {
                            machine.write_wram(address, &bytes).unwrap();
                        }
                    }
                }
                None => machine.write_wram(BLOCK + SCRATCH_SHIFT as u32, &[FILL; 7]).unwrap(),
            }
            for (record, low) in [(side_record(false), a_low), (side_record(true), b_low)] {
                let [low_byte, mid, high_byte, _] = low.to_le_bytes();
                machine.write_wram(record + 0x07, &[low_byte, mid, high_byte]).unwrap();
                // The other fields the body reads: the attack and defence numbers, the classes, `+0A`, and the two values
                // that the multipliers of the body take from the records (`+17` of the defender, `+1E` of the attacker).
                machine.write_wram(record + 0x14, &[1]).unwrap();
                machine.write_wram(record + 0x15, &[1]).unwrap();
                machine.write_wram(record + 0x16, &[7]).unwrap();
                machine.write_wram(record + 0x1B, &[0]).unwrap();
                machine.write_wram(record + 0x0A, &[0x40]).unwrap();
                machine.write_wram(record + 0x17, &[0x0A]).unwrap();
                machine.write_wram(record + 0x1E, &[0x0A]).unwrap();
            }
            machine.write_wram(0x7E_14C2, &[0]).unwrap(); // the active side: A
            machine.write_wram(0x7E_14D3, &[0]).unwrap();
            machine.write_wram(0x7E_14DF, &[0]).unwrap(); // battle_repeat
            machine.write_wram(0x7E_14C7, &[0]).unwrap();
            machine.write_wram(STUB, &[0x22, 0xFC, 0xB7, 0x05]).unwrap();
            let mut registers = machine.cpu_registers();
            registers.pbr = (STUB >> 16) as u8;
            registers.pc = (STUB & 0xFFFF) as u16;
            registers.dbr = 0;
            registers.d = 0;
            registers.y = 3;
            registers.p = StatusRegister::from(0x24);
            machine.set_cpu_registers(registers);
            machine.set_breakpoint(BACK);
            let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
            let stopped =
                machine.run(&stops, &mut |_| Joypad::default(), &mut NullAudio, 2).unwrap();
            assert!(
                matches!(stopped, Stopped::Breakpoint { key } if key == BACK),
                "the routine did not return: {stopped:?}"
            );
            let wram = machine.wram();
            Outcome {
                damage: read24(wram, 0x7E_14CF),
                side_a_low: read24(wram, side_record(false) + 0x07),
                side_b_low: read24(wram, side_record(true) + 0x07),
                shift: wram[(BLOCK + SCRATCH_SHIFT as u32 - 0x7E_0000) as usize],
            }
        }

        #[test]
        #[ignore = "needs SSD_ROM_PATH"]
        fn a_wide_attacker_is_computed_from_its_shifted_pair_and_its_records_are_restored() {
            // Side A is 50 000 000 (high part 2) against 1 000. The same pair staged already shifted by two (12 500 000 and
            // 250, no block) must leave the same damage word, as the body then sees the same values. Without a shift the
            // records must come back as staged, with a neutral block and with a live block that has no high part.
            let wide = call(GOKU_LOW, 1_000, Some((GOKU_HIGH, 0)));
            let shifted = call(12_500_000, 250, None);
            let no_high = call(GOKU_LOW, 1_000, Some((0, 0)));
            assert_eq!(
                wide.damage, shifted.damage,
                "the body divides the shifted pair as it divides the same pair staged"
            );
            assert_eq!(
                (wide.side_a_low, wide.side_b_low),
                (GOKU_LOW, 1_000),
                "both records are restored"
            );
            assert_eq!(wide.shift, 0, "the scratch is cleared");
            assert_eq!(shifted.shift, FILL, "a neutral block is not written by the hooks");
            assert_eq!(
                (shifted.side_a_low, shifted.side_b_low),
                (12_500_000, 250),
                "a neutral block leaves the records as staged"
            );
            assert_eq!(
                (no_high.side_a_low, no_high.side_b_low),
                (GOKU_LOW, 1_000),
                "a live block without high parts leaves the records as staged"
            );
            assert_ne!(wide.damage, 0xFF_FFFF, "the damage is not saturated");
        }
    }
}
