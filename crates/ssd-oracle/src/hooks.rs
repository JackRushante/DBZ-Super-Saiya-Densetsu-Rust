//! Hook table: which original routines are replaced or checked, and how (spec §5.2).

use crate::{Ctx, Registers};
use snes_core::api::HookBitmap;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

/// Physical hook key of a 24-bit `PBR << 16 | PC`: `& $7FFFFF` (bank bit 7 selects a mirror).
#[must_use]
pub fn hook_key(full_pc: u32) -> u32 {
    HookBitmap::key(full_pc)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookMode {
    /// Not hooked: the original routine runs and no hit is reported.
    Original,
    /// The Rust function replaces the original routine.
    Rust,
    /// The original routine runs and the Rust function is checked against it.
    Verify,
    /// The Rust function patches the machine's state at an instruction boundary and takes no time: it ends with
    /// `Ctx::resume_at` its own key (the original goes on from there) or with the entry's return, and never calls the
    /// original. Having no cost, it runs whatever the timing window, so it never falls back: for data redirectors and
    /// story fixes, which must act on every call.
    Patch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReturnKind {
    Rts,
    Rtl,
}

impl ReturnKind {
    /// Performs the return on `registers` as the native-mode instruction does: RTS pulls PC,
    /// RTL pulls PC then PBR, and PC is the pulled address + 1. Each pull increments S (16-bit,
    /// wrapping) and reads bank 0 at S with `read`.
    pub(crate) fn perform(self, registers: &mut Registers, mut read: impl FnMut(u32) -> u8) {
        let mut s = registers.s;
        let mut pull = || {
            s = s.wrapping_add(1);
            read(u32::from(s))
        };
        let pc = u16::from_le_bytes([pull(), pull()]).wrapping_add(1);
        let pbr = match self {
            Self::Rts => registers.pbr,
            Self::Rtl => pull(),
        };
        registers.s = s;
        registers.pc = pc;
        registers.pbr = pbr;
    }
}

/// A Rust reimplementation of an original routine.
pub type RoutineFn = fn(&mut Ctx<'_>);

/// A routine's cost bound for one call: the most CPU-busy master cycles the original can spend
/// from the hit to the return, from the entry state alone. It reads the machine through the
/// read-only side of [`Ctx`] (registers, flags and the `peek` methods, which have no side
/// effects) and must be cheap: it runs at every hit of a routine in `Rust` mode and of one in
/// `Verify`. See [`HookEntry::bound`].
pub type BoundFn = fn(&Ctx<'_>) -> u64;

/// The Verify budget of a routine without `budget_frames` ([`HookEntry::budget_frames`]), in
/// frames of master cycles.
pub const DEFAULT_ROUTINE_BUDGET_FRAMES: u64 = 2;

#[derive(Debug, Clone, Copy)]
pub enum HookAction {
    /// A routine with its Rust reimplementation and the way the original returns.
    Routine { func: RoutineFn, ret: ReturnKind },
    /// An address reported for synchronisation (lockstep); the original code always resumes.
    SyncPoint,
}

#[derive(Debug, Clone)]
pub struct HookEntry {
    pub name: String,
    /// Physical key (see [`hook_key`]).
    pub key: u32,
    pub mode: HookMode,
    pub action: HookAction,
    /// Required accumulator width at entry (`Some(true)` = 8-bit); `None` accepts both. A call
    /// in emulation mode never meets the contract, whatever the widths.
    pub entry_m8: Option<bool>,
    /// Required index width at entry (`Some(true)` = 8-bit); `None` accepts both. A call in
    /// emulation mode never meets the contract, whatever the widths.
    pub entry_x8: Option<bool>,
    /// Flags compared in Verify, bit order NVMXDIZC.
    pub flags_mask: u8,
    /// Verify checks one call in `verify_sample`.
    pub verify_sample: u32,
    /// The routine's maximum cost over its paths, in CPU-busy master cycles from the hit to the
    /// return (even, as all master-cycle costs), including the time of the original code it
    /// calls ([`Ctx::call_original`]). In [`HookMode::Rust`] a call runs in Rust only if no
    /// timing event can come within this cost (else the original runs: `timing_fallback`), and
    /// an entry without it never does (`cost_missing`) unless it has a [`HookEntry::bound`]. The exact cost of each call is the one
    /// the routine declares ([`Ctx::set_cost`]) plus the time of the code it calls; a routine
    /// that declares none is charged this in all. A call that takes longer counts in
    /// [`HookStats::cost_over_bound`]. A [`HookEntry::bound`] replaces it per call.
    ///
    /// The Rust parts of a routine run atomically, so the whole call must be clear of timing
    /// events, the code it calls included: a routine whose calls of original code cannot be
    /// bounded needs `cycles` covering their worst case, or no `cycles` (it then never runs in
    /// Rust mode).
    pub cycles: Option<u64>,
    /// The cost bound of one call, computed from the entry state ([`BoundFn`]). For a routine
    /// whose cost follows a count or a data-dependent path, the static maximum `cycles` would be
    /// so large that the timing window never opens. With a bound, `Rust` mode uses it in place
    /// of `cycles` for the window check, charges it when the routine declares no cost, and
    /// `cost_over_bound` counts the calls whose declared cost goes above it; `Verify` counts
    /// `cost_over_bound` against it too. `cycles` is then optional (it stays the fallback of an
    /// entry without a bound). The bound has to be sound: a call that costs more than it
    /// returned lands in `cost_over_bound`, and in `Rust` mode its timing was not checked for
    /// the extra time.
    pub bound: Option<BoundFn>,
    /// How many frames of master cycles the original of a `Verify` call, and the original code
    /// a Rust routine runs with [`Ctx::call_original`], may take before the call counts as not
    /// returned (`None`: [`DEFAULT_ROUTINE_BUDGET_FRAMES`]). `Rust` mode is unchanged by it: a
    /// call longer than the timing window still runs the original.
    pub budget_frames: Option<u64>,
}

impl HookEntry {
    /// The frames of master cycles a `Verify` call or a [`Ctx::call_original`] of this routine
    /// may take.
    #[must_use]
    pub fn budget(&self) -> u64 {
        self.budget_frames.unwrap_or(DEFAULT_ROUTINE_BUDGET_FRAMES)
    }

    /// A sync point named `name` at a 24-bit address or key (stored as the physical key): every
    /// arrival is reported and the original code resumes; [`Machine::run_until_sync`] turns the
    /// first one after each NMI into a [`SyncEvent`]. Hooked in [`HookMode::Verify`] (any mode but
    /// [`HookMode::Original`] reports arrivals); no contract, no cost.
    ///
    /// [`Machine::run_until_sync`]: crate::Machine::run_until_sync
    /// [`SyncEvent`]: crate::SyncEvent
    #[must_use]
    pub fn sync_point(name: impl Into<String>, key: u32) -> Self {
        Self {
            name: name.into(),
            key: hook_key(key),
            mode: HookMode::Verify,
            action: HookAction::SyncPoint,
            entry_m8: None,
            entry_x8: None,
            flags_mask: 0xFF,
            verify_sample: 1,
            bound: None,
            budget_frames: None,
            cycles: None,
        }
    }

    /// Whether the CPU meets the entry contract: native mode, and the accumulator and index
    /// widths the entry requires.
    pub(crate) fn contract_holds(&self, registers: &Registers) -> bool {
        let p = u8::from(registers.p);
        !registers.emulation_mode
            && self.entry_m8.is_none_or(|m8| m8 == (p & P_M != 0))
            && self.entry_x8.is_none_or(|x8| x8 == (p & P_X != 0))
    }

    /// Whether `other` is the same routine: same name and same action (function and return).
    /// Mode, contract and costs may differ. Functions are compared by address, and the compiler
    /// or linker may give identical functions one address: such functions count as the same.
    pub(crate) fn same_routine(&self, other: &HookEntry) -> bool {
        self.name == other.name
            && match (self.action, other.action) {
                (HookAction::SyncPoint, HookAction::SyncPoint) => true,
                (
                    HookAction::Routine { func, ret },
                    HookAction::Routine { func: other_func, ret: other_ret },
                ) => ret == other_ret && std::ptr::fn_addr_eq(func, other_func),
                _ => false,
            }
    }
}

/// How a call's cost goes beyond its bounds, if it does (see [`HookStats::cost_over_bound`]):
/// `own` is the cost the routine declared for itself ([`Ctx::set_cost`], `None` if none),
/// `callees` the CPU-busy master cycles of the original code it called ([`Ctx::call_original`]),
/// `spent` all the CPU-busy master cycles from the hit to the return (`callees` included, the
/// final charge of `Rust` mode excluded) and `max_cost` the entry's [`HookEntry::cycles`].
///
/// The call's total is `own + callees`, or `max_cost` if the routine declared nothing (`Rust`
/// mode charges that). It is over bound if the total is above `max_cost`, or if `spent` is
/// above the total (the routine charged more than it declared).
pub(crate) fn cost_over_bound(
    own: Option<u64>,
    callees: u64,
    spent: u64,
    max_cost: Option<u64>,
) -> Option<String> {
    let total = own.map(|own| own + callees).or(max_cost)?;
    if let (Some(own), Some(max)) = (own, max_cost)
        && total > max
    {
        let calls = if callees == 0 {
            String::new()
        } else {
            format!(" + {callees} in called original code = {total}")
        };
        return Some(format!("cost over bound: declared {own}{calls}, above the maximum {max}"));
    }
    (spent > total).then(|| match own {
        Some(own) => {
            let charged = spent - callees;
            format!("cost over bound: charged {charged} before the return, declared {own}")
        }
        None => {
            format!("cost over bound: spent {spent} before the return, above the maximum {total}")
        }
    })
}

/// Splits a charge of CPU-busy master cycles into held CPU cycles: `(fast, slow)` of 6 and 8
/// master cycles. Every even number from 12 up is exact (as are 0, 6 and 8); any other value is
/// charged as the nearest exact one, ties going to the lower: 2 and 3 → 0, 4 and 5 → 6, 7 → 6,
/// 9 and 10 → 8, 11 → 12, and an odd value from 13 up → itself − 1.
pub(crate) fn split_charge(master_cycles: u64) -> (u64, u64) {
    let exact = |t: u64| matches!(t, 0 | 6 | 8) || (t >= 12 && t.is_multiple_of(2));
    let t = (0..=3)
        .flat_map(|distance| {
            [master_cycles.checked_sub(distance), master_cycles.checked_add(distance)]
        })
        .flatten()
        .find(|&t| exact(t))
        .expect("an exact value lies within 3 of any value");
    // 6 fast + 8 slow = t: slow ≡ t/2 (mod 3), and the smallest such slow count fits.
    let slow = (t / 2) % 3;
    ((t - 8 * slow) / 6, slow)
}

/// Accumulator width flag (bit order NVMXDIZC).
pub(crate) const P_M: u8 = 0x20;
/// Index width flag.
pub(crate) const P_X: u8 = 0x10;

/// Per-entry counters of the dispatch.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HookStats {
    /// `Verify` calls left uncompared because the Rust function drops its caller's return
    /// address (`PLA PLA`: the routine is an early exit of its caller, not a unit).
    pub early_exit: u64,
    /// Arrivals dispatched, in every mode.
    pub calls: u64,
    /// Calls the Rust function replaced (`Rust` mode).
    pub rust_runs: u64,
    /// `Verify` calls whose Rust result matched the original's.
    pub verified_ok: u64,
    /// `Verify` calls whose Rust result differed (all of them: `Machine::divergences` keeps the
    /// first 32).
    pub divergences: u64,
    /// `Verify` calls an interrupt entered (not compared).
    pub interrupted: u64,
    /// Checked `Verify` calls during which HDMA copied data, while the original ran or while
    /// the Rust function ran on the snapshot (also counted as verified, diverged or
    /// interrupted).
    pub hdma_crossed: u64,
    /// Checked `Verify` calls whose original moved the stack pointer by more than 4 in one step
    /// outside interrupt handling (`TCS`/`TXS`, e.g. to fill a buffer with pushes): their dead
    /// stack bytes are only the return address (also counted as verified, diverged or
    /// interrupted).
    pub stack_switched: u64,
    /// Divergence reports that could not be saved (a warning is logged; the divergence is
    /// kept).
    pub report_errors: u64,
    /// `Verify` calls whose original did not return within the budget.
    pub no_return: u64,
    /// Calls that resumed the original because the CPU did not meet the entry contract.
    pub contract_mismatch: u64,
    /// `Verify` calls resumed unchecked by sampling.
    pub sampled_out: u64,
    /// `Verify` calls dispatched inside the original of another `Verify` call, which reached
    /// this entry (also counted as checked, sampled out or contract mismatch).
    pub nested: u64,
    /// Arrivals at this `Verify` entry inside the original of a `Verify` call while 16 were in
    /// progress: the original resumed unchecked (also counted in `calls`).
    pub nested_overflow: u64,
    /// Arrivals at this `Verify` entry while its own call's original runs, with S below the S of
    /// that call's hit: a recursion, or an interrupt handler calling the routine it interrupted.
    /// The original runs through them unchecked, as part of that call (also counted in
    /// `calls`). A loop back to the entry at the S of the hit is part of the call, uncounted.
    pub reentered: u64,
    /// `Rust` calls that resumed the original because a timing event could come within the
    /// routine's maximum cost (see [`HookEntry::cycles`]).
    pub timing_fallback: u64,
    /// Compared `Verify` calls (not the interrupted ones) whose declared cost
    /// ([`Ctx::set_cost`], plus the time of the original code the routine called) differs from
    /// the original's measured cost. Calls HDMA crossed are compared too: both sides count
    /// CPU-busy master cycles, which leave out the time HDMA and the memory refresh take.
    pub cost_mismatch: u64,
    /// Calls without a cost: `Rust` calls of an entry without `cycles` (the original resumed),
    /// and calls whose Rust function declared none (`Rust`: charged the entry's `cycles`;
    /// `Verify`: not checked for `cost_mismatch`).
    pub cost_missing: u64,
    /// `Rust` calls, and compared `Verify` calls (on the snapshot), whose cost went beyond its
    /// bounds: the declared cost plus the time of the original code the routine called
    /// ([`Ctx::call_original`]) is above the entry's `cycles` (the maximum the timing window
    /// assumed), or the routine spent more before returning ([`Ctx::charge`], transfers) than
    /// it declared. A `Verify` divergence of such a call says so in its text.
    pub cost_over_bound: u64,
    /// `Verify` calls that returned with no interrupt and whose real cost (the original's
    /// CPU-busy master cycles) had no timing event inside its window at the entry: no GPDMA
    /// requested, and the next event ([`Machine::master_cycles_until_timing_event`]) beyond
    /// [`call_fits_window`]. The share of the checked calls (`verified_ok + divergences +
    /// interrupted + no_return`) is the most a per-call bound can unlock, with an exact bound:
    /// the other calls span a timing event and fall back to the original in `Rust` mode.
    ///
    /// [`Machine::master_cycles_until_timing_event`]: crate::Machine::master_cycles_until_timing_event
    pub window_fit: u64,
    /// Mean CPU-busy master cycles of the original per call, from the hit to the return (its
    /// nested `Verify` calls included), measured in the compared `Verify` calls (verified or
    /// diverged); 0 until then.
    pub mean_cpu_cycles: f64,
    /// Mean instructions (opcode fetches) of the original per call, from the hit to the return
    /// (its nested `Verify` calls included: they run inside it, so a routine that does its
    /// callee's work inline stands for those instructions too), measured in the same compared
    /// `Verify` calls as [`HookStats::mean_cpu_cycles`] (verified or diverged, the ones HDMA
    /// crossed included: it steals bus time, not instructions; not the interrupted ones, whose
    /// count would hold the interrupt handler's instructions); 0 until then. The coverage metric
    /// multiplies it by the `Rust` runs ([`Machine::coverage`]); a machine that runs in `Rust`
    /// takes the means a `Verify` run learned from [`Machine::set_mean_instructions`].
    ///
    /// [`Machine::coverage`]: crate::Machine::coverage
    /// [`Machine::set_mean_instructions`]: crate::Machine::set_mean_instructions
    pub mean_instructions: f64,
    /// `Rust` runs that declared their own instructions ([`Ctx::set_instructions`]): the
    /// coverage metric counts those exactly, the others at [`HookStats::mean_instructions`].
    pub declared_runs: u64,
    /// The instructions of the original those declared runs stand for, summed: the declared own
    /// instructions plus the instructions of the code each ran through [`Ctx::call_original`]
    /// (the latter also in [`HookStats::callee_instructions`], which the estimate takes off).
    pub declared_instructions: f64,
    /// Compared `Verify` calls whose declared instructions ([`Ctx::set_instructions`], plus the
    /// instructions of the original code the routine called) differ from the original's count.
    /// Hooked routines that ran in `Rust` on the snapshot inside the called code count at their
    /// means, so the sum is rounded.
    pub instructions_mismatch: u64,
    /// Instructions that this routine's `Rust` runs had run elsewhere, summed: through
    /// [`Ctx::call_original`] a Rust routine runs code of the original, hooked or not. The mean
    /// of the routine holds those instructions (the original ran them inside it), but the
    /// machine fetches them itself (original code) or counts them under the runs of the
    /// routines that replaced them (hooked in `Rust` mode), so the estimate takes them off
    /// ([`HookStats::estimated_rust_instructions`]). Counted in `Rust` mode only.
    pub callee_instructions: f64,
    /// Sync events a dispatch delayed: arrivals at this sync point inside a dispatch (the
    /// original of a `Verify` call, original code a Rust routine calls) that would have been sync
    /// events at the top level (an NMI came since the last event), at most one per NMI, the first
    /// after it (at any sync point). A dispatch runs to its end, so the event comes at the first
    /// arrival after it ([`Machine::run_until_sync`]); lockstep did not compare there.
    ///
    /// [`Machine::run_until_sync`]: crate::Machine::run_until_sync
    pub sync_inside_dispatch: u64,
}

/// The 16-bit address of the trap of every bank: original code called from Rust
/// ([`Ctx::call_original`]) returns to `bank:21FF`. In banks `$00–$3F` and `$80–$BF` it is an
/// unused B-bus address (open bus, read without side effects) where no code runs; in banks
/// `$40–$7F` it can be memory, so an arrival there is the return only with the stack pointer
/// of the call.
pub(crate) const TRAP_PC: u16 = 0x21FF;

/// The trap key of `bank` (mirrors share it).
pub(crate) fn trap_key(bank: u8) -> u32 {
    hook_key((u32::from(bank) << 16) | u32::from(TRAP_PC))
}

/// Whether `key` (a 24-bit address or key) is the trap of its bank.
fn is_trap(key: u32) -> bool {
    key & 0xFFFF == u32::from(TRAP_PC)
}

/// Hook entries by physical key, plus the bitmap of the hooked ones (every mode but
/// [`HookMode::Original`]) that the core checks at each opcode fetch. The bitmap also holds the
/// trap of every bank (`$21FF`, see [`Ctx::call_original`]), whatever the entries.
#[derive(Clone)]
pub struct HookTable {
    entries: BTreeMap<u32, HookEntry>,
    bitmap: Arc<HookBitmap>,
}

impl HookTable {
    /// An empty table: only the traps are hooked. An arrival at a trap with no call of original
    /// code in flight resumes the original.
    #[must_use]
    pub fn new() -> Self {
        let mut bitmap = HookBitmap::new();
        for bank in 0..=0x7F {
            bitmap.set(trap_key(bank));
        }
        Self { entries: BTreeMap::new(), bitmap: Arc::new(bitmap) }
    }

    /// Adds `entry` under `hook_key(entry.key)` (its `key` is normalised to that), returning the
    /// entry it replaces.
    pub fn insert(&mut self, mut entry: HookEntry) -> Option<HookEntry> {
        entry.key = hook_key(entry.key);
        let bitmap = Arc::make_mut(&mut self.bitmap);
        if entry.mode != HookMode::Original {
            bitmap.set(entry.key);
        } else if !is_trap(entry.key) {
            bitmap.clear(entry.key);
        }
        self.entries.insert(entry.key, entry)
    }

    /// The entry for a 24-bit address or key.
    #[must_use]
    pub fn get(&self, key: u32) -> Option<&HookEntry> {
        self.entries.get(&hook_key(key))
    }

    /// Entries in key order.
    pub fn entries(&self) -> impl Iterator<Item = &HookEntry> {
        self.entries.values()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether arrivals at this address or key are reported (always at a trap).
    #[must_use]
    pub fn is_hooked(&self, key: u32) -> bool {
        self.bitmap.contains(key)
    }

    /// Sets the entry to [`HookMode::Original`] and stops reporting its arrivals (unless it is at
    /// a trap). Returns `false` if there is no entry for `key`. A bitmap already installed in a
    /// core is never changed in place: install the table again for the demotion to take effect
    /// there.
    pub fn demote(&mut self, key: u32) -> bool {
        let Some(entry) = self.entries.get_mut(&hook_key(key)) else {
            return false;
        };
        entry.mode = HookMode::Original;
        if !is_trap(entry.key) {
            Arc::make_mut(&mut self.bitmap).clear(entry.key);
        }
        true
    }

    /// The bitmap to install in the core, or `None` when no entry is hooked (the core then skips
    /// hook detection entirely: no routine runs, so none calls original code).
    pub(crate) fn installed_bitmap(&self) -> Option<Arc<HookBitmap>> {
        self.entries
            .values()
            .any(|entry| entry.mode != HookMode::Original)
            .then(|| Arc::clone(&self.bitmap))
    }
}

/// The entries by key, the keys in hex; the bitmap (a megabyte of bits) is left out.
impl fmt::Debug for HookTable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        struct Key(u32);
        impl fmt::Debug for Key {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "${:06X}", self.0)
            }
        }
        f.debug_map().entries(self.entries.iter().map(|(&key, entry)| (Key(key), entry))).finish()
    }
}

impl Default for HookTable {
    fn default() -> Self {
        Self::new()
    }
}

impl FromIterator<HookEntry> for HookTable {
    fn from_iter<I: IntoIterator<Item = HookEntry>>(entries: I) -> Self {
        let mut table = Self::new();
        for entry in entries {
            table.insert(entry);
        }
        table
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: u32, mode: HookMode) -> HookEntry {
        HookEntry {
            name: format!("routine_{key:06x}"),
            key,
            mode,
            action: HookAction::SyncPoint,
            entry_m8: None,
            entry_x8: None,
            flags_mask: 0xFF,
            verify_sample: 1,
            bound: None,
            budget_frames: None,
            cycles: None,
        }
    }

    #[test]
    fn hook_key_drops_the_mirror_bit() {
        assert_eq!(hook_key(0x80_8040), 0x00_8040);
        assert_eq!(hook_key(0xC1_2345), 0x41_2345);
        assert_eq!(hook_key(0x7E_1234), 0x7E_1234);
    }

    #[test]
    fn only_entries_not_in_original_mode_are_hooked() {
        let table: HookTable = [
            entry(0x00_8040, HookMode::Rust),
            entry(0x01_0000, HookMode::Verify),
            entry(0x02_0000, HookMode::Original),
        ]
        .into_iter()
        .collect();
        assert_eq!(table.len(), 3);
        assert!(table.is_hooked(0x00_8040));
        assert!(table.is_hooked(0x80_8040), "mirrors share the key");
        assert!(table.is_hooked(0x01_0000));
        assert!(!table.is_hooked(0x02_0000));
        assert!(!table.is_hooked(0x00_8041));
        assert!(table.installed_bitmap().is_some());
    }

    #[test]
    fn a_table_prints_its_entries_by_key_and_not_its_bitmap() {
        let table: HookTable =
            [entry(0x01_9000, HookMode::Verify), entry(0x80_8040, HookMode::Rust)]
                .into_iter()
                .collect();
        let text = format!("{table:?}");
        let (first, second) = (text.find("$008040: "), text.find("$019000: "));
        assert!(first < second && first.is_some(), "the entries in key order: {text}");
        assert!(text.contains("routine_808040") && text.contains("Rust"), "{text}");
        assert!(text.len() < 1000, "no bitmap in the output: {} bytes", text.len());
        assert_eq!(format!("{:?}", HookTable::new()), "{}");
    }

    #[test]
    fn entries_are_stored_under_their_physical_key() {
        let mut table = HookTable::new();
        assert!(table.insert(entry(0x80_8040, HookMode::Rust)).is_none());
        assert_eq!(table.get(0x00_8040).map(|e| e.key), Some(0x00_8040));
        assert_eq!(table.get(0x80_8040).map(|e| e.key), Some(0x00_8040));

        let replaced = table.insert(entry(0x00_8040, HookMode::Original));
        assert_eq!(replaced.map(|e| e.mode), Some(HookMode::Rust));
        assert!(!table.is_hooked(0x00_8040), "the replacement is not hooked");
        assert_eq!(table.len(), 1);
    }

    fn registers(s: u16, p: u8) -> Registers {
        Registers {
            a: 0,
            x: 0,
            y: 0,
            s,
            d: 0,
            pbr: 0x05,
            pc: 0x9000,
            dbr: 0,
            p: crate::StatusRegister::from(p),
            emulation_mode: false,
        }
    }

    /// Performs `kind` with S = `s` over a 64 KiB bank-0 stack holding `bytes` at S + 1 onwards;
    /// returns the registers and the addresses read.
    fn perform(kind: ReturnKind, s: u16, bytes: &[u8]) -> (Registers, Vec<u32>) {
        let mut stack = vec![0xEE_u8; 0x1_0000];
        for (i, &byte) in bytes.iter().enumerate() {
            stack[usize::from(s.wrapping_add(1 + i as u16))] = byte;
        }
        let mut reads = Vec::new();
        let mut regs = registers(s, 0x30);
        kind.perform(&mut regs, |address| {
            reads.push(address);
            stack[address as usize]
        });
        (regs, reads)
    }

    #[test]
    fn rts_pulls_the_return_address_plus_one() {
        let (regs, reads) = perform(ReturnKind::Rts, 0x1FF0, &[0x09, 0x80]);
        assert_eq!((regs.pbr, regs.pc, regs.s), (0x05, 0x800A, 0x1FF2), "PBR unchanged");
        assert_eq!(reads, [0x1FF1, 0x1FF2]);
    }

    #[test]
    fn rtl_pulls_the_return_address_plus_one_and_the_bank() {
        let (regs, reads) = perform(ReturnKind::Rtl, 0x1FF0, &[0x09, 0x80, 0x12]);
        assert_eq!((regs.pbr, regs.pc, regs.s), (0x12, 0x800A, 0x1FF3));
        assert_eq!(reads, [0x1FF1, 0x1FF2, 0x1FF3]);
    }

    #[test]
    fn returns_wrap_the_stack_and_the_pc_in_16_bits() {
        let (regs, reads) = perform(ReturnKind::Rtl, 0xFFFE, &[0xFF, 0xFF, 0x7E]);
        assert_eq!((regs.pbr, regs.pc, regs.s), (0x7E, 0x0000, 0x0001));
        assert_eq!(reads, [0xFFFF, 0x0000, 0x0001], "bank 0 only");
        // Other registers are untouched.
        let regs = perform(ReturnKind::Rts, 0x01FD, &[0x00, 0x90]).0;
        assert_eq!((regs.a, regs.x, regs.y, regs.d, regs.dbr), (0, 0, 0, 0, 0));
        assert_eq!(u8::from(regs.p), 0x30);
    }

    #[test]
    fn the_contract_checks_widths_and_native_mode() {
        let mut e = entry(0x00_8040, HookMode::Rust);
        let m8x16 = registers(0x1FF0, P_M);
        assert!(e.contract_holds(&m8x16), "no requirement");
        e.entry_m8 = Some(true);
        e.entry_x8 = Some(false);
        assert!(e.contract_holds(&m8x16));
        assert!(!e.contract_holds(&registers(0x1FF0, 0x00)), "16-bit A");
        assert!(!e.contract_holds(&registers(0x1FF0, P_M | P_X)), "8-bit X/Y");
        let mut emulation = m8x16;
        emulation.emulation_mode = true;
        assert!(!e.contract_holds(&emulation), "emulation mode never meets a contract");
    }

    #[test]
    fn charges_split_exactly_into_fast_and_slow_held_cycles() {
        for t in (12..=200).step_by(2).chain([0, 6, 8, 400_000]) {
            let (fast, slow) = split_charge(t);
            assert_eq!(6 * fast + 8 * slow, t, "{t}");
            assert!(slow < 3, "{t}: as few slow cycles as possible");
        }
        // Not exact: the nearest exact value, ties going to the lower.
        for (t, charged) in
            [(1, 0), (2, 0), (3, 0), (4, 6), (5, 6), (7, 6), (9, 8), (10, 8), (11, 12), (13, 12)]
        {
            let (fast, slow) = split_charge(t);
            assert_eq!(6 * fast + 8 * slow, charged, "{t}");
        }
    }

    #[test]
    fn same_routine_compares_name_and_action() {
        fn other(ctx: &mut Ctx<'_>) {
            ctx.set_c(true);
        }
        fn routine(name: &str, func: RoutineFn, ret: ReturnKind) -> HookEntry {
            let mut e = entry(0x00_8040, HookMode::Rust);
            e.name = name.to_owned();
            e.action = HookAction::Routine { func, ret };
            e
        }
        let base = routine("a", noop, ReturnKind::Rts);
        let mut verify = base.clone();
        verify.mode = HookMode::Verify;
        verify.cycles = Some(80);
        assert!(base.same_routine(&verify), "mode and costs do not matter");
        assert!(!base.same_routine(&routine("b", noop, ReturnKind::Rts)));
        assert!(!base.same_routine(&routine("a", other, ReturnKind::Rts)));
        assert!(!base.same_routine(&routine("a", noop, ReturnKind::Rtl)));
        let mut sync_point = entry(0x00_8040, HookMode::Rust);
        sync_point.name = "a".to_owned();
        assert!(!base.same_routine(&sync_point), "a sync point is another action");
        assert!(entry(1, HookMode::Rust).same_routine(&entry(1, HookMode::Verify)));
    }

    fn noop(_: &mut Ctx<'_>) {}

    #[test]
    fn sync_point_entries_are_hooked_and_resume_the_original() {
        let e = HookEntry::sync_point("idle", 0x80_9000);
        assert_eq!(e.name, "idle");
        assert_eq!(e.key, 0x00_9000, "the physical key");
        assert!(matches!(e.action, HookAction::SyncPoint));
        assert_ne!(e.mode, HookMode::Original, "arrivals are reported");
        assert_eq!((e.entry_m8, e.entry_x8, e.cycles), (None, None, None));
        let table: HookTable = [e].into_iter().collect();
        assert!(table.is_hooked(0x00_9000));
    }

    #[test]
    fn the_trap_of_every_bank_is_hooked_for_good() {
        let mut table = HookTable::new();
        for bank in 0..=0xFF_u32 {
            assert!(table.is_hooked((bank << 16) | 0x21FF), "bank {bank:02X}");
            assert!(!table.is_hooked((bank << 16) | 0x21FE), "bank {bank:02X}");
        }
        assert_eq!(trap_key(0x83), 0x03_21FF, "mirrors share the key");
        assert!(table.is_empty() && table.installed_bitmap().is_none(), "no entry is hooked");
        // Neither an Original entry nor a demotion at a trap unhooks it.
        table.insert(entry(0x00_21FF, HookMode::Verify));
        assert!(table.demote(0x00_21FF));
        table.insert(entry(0x7E_21FF, HookMode::Original));
        assert!(table.is_hooked(0x00_21FF) && table.is_hooked(0x7E_21FF));
    }

    #[test]
    fn costs_over_bound() {
        // Within bounds: exact, below the maximum, or no maximum to check.
        assert_eq!(cost_over_bound(Some(126), 80, 126, Some(206)), None);
        assert_eq!(cost_over_bound(Some(100), 0, 40, Some(400)), None);
        assert_eq!(cost_over_bound(Some(100), 0, 100, None), None);
        assert_eq!(cost_over_bound(None, 0, 0, None), None);
        assert_eq!(cost_over_bound(None, 80, 300, Some(400)), None);
        // The declared total above the maximum, with or without callees.
        assert_eq!(
            cost_over_bound(Some(126), 80, 126, Some(150)).as_deref(),
            Some(
                "cost over bound: declared 126 + 80 in called original code = 206, \
                 above the maximum 150"
            )
        );
        assert_eq!(
            cost_over_bound(Some(174), 0, 0, Some(160)).as_deref(),
            Some("cost over bound: declared 174, above the maximum 160")
        );
        // Charged beyond the declared cost; callees do not count as charged.
        assert_eq!(
            cost_over_bound(Some(142), 80, 280, Some(400)).as_deref(),
            Some("cost over bound: charged 200 before the return, declared 142")
        );
        assert_eq!(cost_over_bound(Some(142), 80, 222, None), None);
        // Undeclared: the maximum is the total.
        assert_eq!(
            cost_over_bound(None, 300, 500, Some(400)).as_deref(),
            Some("cost over bound: spent 500 before the return, above the maximum 400")
        );
    }

    #[test]
    fn demote_unhooks_an_entry() {
        let mut table: HookTable = [entry(0x00_8040, HookMode::Verify)].into_iter().collect();
        let installed = table.installed_bitmap().unwrap();
        assert!(table.demote(0x80_8040));
        assert_eq!(table.get(0x00_8040).map(|e| e.mode), Some(HookMode::Original));
        assert!(!table.is_hooked(0x00_8040));
        assert!(installed.contains(0x00_8040), "an installed bitmap is never changed in place");
        assert!(table.installed_bitmap().is_none(), "nothing left to hook");
        assert!(!table.demote(0x00_9000));
    }
}
