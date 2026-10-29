use crate::compare::Divergence;
use crate::coverage::FrameStats;
use crate::ctx::Finished;
use crate::hooks::{
    BoundFn, DEFAULT_ROUTINE_BUDGET_FRAMES, HookAction, HookEntry, HookMode, HookStats, HookTable,
    ReturnKind, RoutineFn, TRAP_PC, cost_over_bound, hook_key, split_charge, trap_key,
};
use crate::input::{FramePoller, InputCache};
use crate::savestore::{MemorySaveStore, SaveStore, SaveWriterAdapter};
use crate::sinks::{AudioAdapter, AudioSink, FrameCapture};
use crate::sync::StackWatch;
use crate::verify::{self, VerifyCall};
use crate::{Ctx, Registers, Symbols, SyncEvent, report, state, timing};
use jgenesis_common::frontend::{Color, EmulatorTrait, FrameSize, RenderFrameOptions, TickEffect};
use snes_core::api::{CoprocessorRoms, HookBitmap, SnesEmulator, SnesEmulatorConfig};
use snes_core::linelog::FrameLog;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

pub type Joypad = snes_config::SnesJoypadState;

/// Seed for the deterministic power-on WRAM fill used by [`Machine::new`].
pub const DEFAULT_WRAM_SEED: u64 = 0x5EED_D82B_55D0_0001;

/// Master cycles in one scanline.
const MCLK_PER_LINE: u64 = 1364;
/// A tick that moves S by more than this outside interrupt handling switches the stack
/// (`TCS`, `TXS`): pushes and pulls move it by a few bytes at most.
pub(crate) const MAX_STACK_MOVE: u16 = 4;
/// `Verify` calls in progress at most: inside the original of the 16th, an arrival at another
/// `Verify` entry resumes the original (see [`Machine::run_frame`]).
const MAX_VERIFY_DEPTH: usize = 16;

/// Instructions a generated function may run on a `Verify` snapshot before the call is known
/// (afterwards: a multiple of the original's count).
const DEFAULT_RUNAWAY_LIMIT: u64 = 50_000_000;

#[derive(Debug, thiserror::Error)]
pub enum OracleError {
    #[error("cannot load ROM: {0}")]
    Load(String),
    #[error("emulation error: {0}")]
    Run(String),
    #[error("cannot load save state: {0}")]
    State(String),
    /// A replay that cannot be read, recorded or played as asked (the message names the
    /// mismatch).
    #[error("replay: {0}")]
    Replay(String),
    /// A host write to work RAM that is not in work RAM (see [`Machine::write_wram`]).
    #[error("work RAM write: {0}")]
    WramWrite(String),
    /// A lockstep run that cannot be set up as asked (the message says why).
    #[error("lockstep: {0}")]
    Lockstep(String),
    /// WRAM freezes that cannot be set (the message says why).
    #[error("freeze: {0}")]
    Freeze(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Bytes of WRAM the machine holds at a value: a cheat or a debugger pin
/// ([`Machine::set_freezes`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Freeze {
    /// The first byte: `$7E0000-$7FFFFF`, or the low-RAM mirror of `$7E0000-$7E1FFF` in banks
    /// `$00-$3F` and `$80-$BF` (`$0000-$1FFF`). [`Machine::freezes`] gives it as `$7E0000 + ...`.
    pub address: u32,
    /// The values the bytes from `address` on are held at.
    pub bytes: Vec<u8>,
}

pub struct Frame<'a> {
    pub pixels: &'a [Color],
    pub size: FrameSize,
    pub options: RenderFrameOptions,
}

/// What one [`Machine::tick`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TickEvent {
    /// The core ticked; nothing to report.
    None,
    /// The core ticked and rendered a frame.
    Frame,
    /// The CPU is about to fetch the opcode at a hooked address (an entry, a trap or a
    /// breakpoint), given as the 24-bit `PBR << 16 | PC`; nothing ticked.
    Hook(u32),
}

/// Where [`Machine::run`] stops, besides its frame budget: each flag set is a stop, and the
/// first one the run reaches wins. With none set, the run goes on until its budget.
///
/// Stops are checked where they can happen, never per tick unless asked for: `frame_end` after
/// a tick or a dispatch that renders a frame; `breakpoint`, `sync`, `after_dispatch` and `call`
/// at the arrivals the run reports at its top level (an arrival inside a dispatch never stops:
/// a dispatched routine runs to its end, atomic); `interrupt`, `instructions`, `stack_above` and
/// `returned_above` before each tick, only when one of them is set (see [`Machine::run`] for the
/// order of stops reached at the same point).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stops {
    /// After the tick or the dispatch that rendered a frame ([`Stopped::FrameEnd`]).
    pub frame_end: bool,
    /// After the dispatch of an arrival at this hooked address or a mirror of it
    /// ([`Stopped::Call`]).
    pub call: Option<u32>,
    /// At a sync event, before the arrival is dispatched ([`Stopped::Sync`]; see
    /// [`Machine::run_until_sync`]).
    pub sync: bool,
    /// Before the dispatch of an arrival at a breakpoint ([`Machine::set_breakpoint`],
    /// [`Stopped::Breakpoint`]). Only arrivals at the top level of the run stop: a breakpoint in
    /// the code a dispatch runs (the original of a `Verify` call, original code a `Rust` routine
    /// calls) is passed, and one in the body of a routine running in `Rust` is never reached.
    pub breakpoint: bool,
    /// Right after the dispatch of an arrival at a hooked entry of the installed table (not in
    /// [`HookMode::Original`]), whatever the dispatch did ([`Stopped::Dispatched`]).
    pub after_dispatch: bool,
    /// At the first instruction boundary at which the CPU has started at least this many
    /// instructions (opcode fetches) since the run started ([`Stopped::Instructions`]): at an
    /// opcode fetch, never in the middle of an instruction or of an interrupt entry, so with an
    /// interrupt pending the run goes on to the handler's first instruction. The instructions of
    /// the original code a dispatch runs count too, and a dispatch is never cut short, so the
    /// count can overshoot after one; a `Rust` run counts none of its own.
    pub instructions: Option<u64>,
    /// At the first instruction boundary (as for `instructions`) with S above this
    /// ([`Stopped::StackAbove`]).
    pub stack_above: Option<u16>,
    /// At the first instruction boundary (as for `instructions`) with S above this that a return
    /// led to ([`Stopped::Returned`]): right after an `RTS`, `RTL` or `RTI` the run executed, or
    /// after the dispatch of a routine that ran (it ends with the routine's return). An interrupt
    /// entry between the return and the boundary leaves it out (the handler's own `RTI` is a
    /// return in turn). A push and a pull do not count (`PHX … PLX` raises S without a return).
    /// The opcode of each instruction is read where it lies, without side effects, from the ROM
    /// image (`LoROM` mapping) or WRAM; code elsewhere counts as no return.
    pub returned_above: Option<u16>,
    /// At the first instruction boundary after the CPU entered an NMI or IRQ handler, its first
    /// instruction ([`Stopped::Interrupt`]). An interrupt taken inside a dispatch does not count:
    /// the routine runs to its end.
    pub interrupt: bool,
}

/// Why [`Machine::run`] stopped.
#[derive(Debug, Clone, PartialEq)]
pub enum Stopped {
    /// A frame was rendered ([`Stops::frame_end`]).
    FrameEnd,
    /// The call of [`Stops::call`] was dispatched; `stats` are its routine's counters after it,
    /// and `rendered` tells whether the dispatch rendered a frame (no [`Stopped::FrameEnd`]
    /// follows for it).
    Call { stats: HookStats, rendered: bool },
    /// A sync event ([`Stops::sync`]): the CPU is about to fetch the sync point's opcode.
    Sync(SyncEvent),
    /// The CPU is about to fetch the opcode at the breakpoint under `key` ([`Stops::breakpoint`]):
    /// the arrival is not dispatched yet. The next run or step goes on from there, without
    /// stopping at this breakpoint again (see [`Machine::set_breakpoint`]).
    Breakpoint { key: u32 },
    /// An arrival at the entry `name` was dispatched ([`Stops::after_dispatch`]). `ran`: the
    /// routine ran (a `Rust` run, a checked `Verify` call) and the CPU is past it, usually at its
    /// return address; otherwise the original resumes (a sync point, a timing fallback, a
    /// contract mismatch, a sampled-out `Verify` call, a routine already running) and the CPU is
    /// still about to fetch the entry's opcode. `rendered`: the dispatch rendered a frame.
    Dispatched { name: String, ran: bool, rendered: bool },
    /// [`Stops::instructions`] were run.
    Instructions,
    /// S rose above [`Stops::stack_above`].
    StackAbove,
    /// A return took S above [`Stops::returned_above`].
    Returned,
    /// The CPU entered an interrupt handler ([`Stops::interrupt`]): it is about to fetch the
    /// handler's first opcode.
    Interrupt,
    /// The frame budget ran out first.
    Budget,
}

/// What [`Machine::dispatch`] did with an arrival.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Dispatch {
    /// The routine ran as one unit (a `Rust` run, or the original of a checked `Verify` call run
    /// to its return or for its whole budget): the CPU is past the entry. `rendered`: a frame
    /// was rendered meanwhile.
    Ran { rendered: bool },
    /// Nothing ran: the original resumes, the CPU still at the entry's opcode fetch (a sync
    /// point, a timing fallback, a contract mismatch, a missing cost, a sampled-out `Verify`
    /// call, an arrival at a routine already running, or an address without a routine).
    Resumed,
}

impl Dispatch {
    pub(crate) fn ran(self) -> bool {
        matches!(self, Self::Ran { .. })
    }

    pub(crate) fn rendered(self) -> bool {
        matches!(self, Self::Ran { rendered: true })
    }
}

/// What the instruction boundaries of a [`Machine::run`] with fine stops need to know.
struct Fine {
    /// Opcode fetches when the run started.
    fetches_start: u64,
    /// For [`Stops::returned_above`]: the opcode fetches at the last boundary checked and the
    /// opcode there (`None` if it could not be read).
    last: Option<(u64, Option<u8>)>,
    /// A return led to the current boundary (see [`Stops::returned_above`]).
    returned: bool,
    /// The CPU entered an interrupt handler since the last boundary checked.
    interrupted: bool,
}

impl Fine {
    /// After a tick: `handling` tells whether the CPU is in an interrupt entry. An entry cancels
    /// a return not yet stopped at: the CPU is not at the instruction the return led to, and the
    /// handler's own RTI is the return that brings it there.
    #[inline(always)]
    fn interrupt_entry(&mut self, handling: bool) {
        if handling {
            self.interrupted = true;
            self.returned = false;
        }
    }
}

pub struct Machine {
    emu: SnesEmulator,
    saves: Box<dyn SaveStore>,
    capture: FrameCapture,
    /// Frames rendered since power-on (or as the loaded state says). A tick belongs to the frame
    /// this counts when it starts: its joypad is looked up by it (see [`crate::input`]).
    frames: u64,
    /// The joypad of the frame the core polls, looked up once per frame and run.
    input_cache: InputCache,
    rom_crc32: u32,
    /// CRC32 of the image the machine runs: the ROM's own for [`Machine::with_wram_seed`], a pack
    /// set's image for [`Machine::with_image`]. The save states are keyed on it.
    image_crc32: u32,
    /// The ROM image as the core maps it (copier header stripped, mirrored to a power of two),
    /// for reading code without side effects ([`Machine::peek_code`]).
    rom: Arc<[u8]>,
    /// Seed of the power-on WRAM fill.
    wram_seed: u64,
    /// Output sample rate requested by the host; a loaded state must not revert it.
    audio_hz: Option<u64>,
    hooks: Option<HookTable>,
    /// Whether the core has a hook bitmap installed (a hooked entry or a breakpoint); without
    /// one, `run_frame` takes the plain tick loop.
    hooked: bool,
    /// Whether the installed table has an entry not in [`HookMode::Original`].
    table_hooked: bool,
    /// Breakpoints by hook key ([`Machine::set_breakpoint`]), hooked in the core's bitmap besides
    /// the table's entries.
    breakpoints: BTreeSet<u32>,
    /// The breakpoint that the first top-level arrival does not stop at, with the opcode fetches
    /// at which it was set: the arrival a breakpoint stop (or a step) left pending, while the CPU
    /// has not moved.
    skip_breakpoint: Option<(u32, u64)>,
    /// Dispatch counters by hook key, for the routine installed under the key.
    stats: BTreeMap<u32, HookStats>,
    /// Where divergence reports go, if anywhere.
    divergence_dir: Option<PathBuf>,
    /// The `Verify` divergences kept, in order.
    divergences: Vec<Divergence>,
    /// Names for divergence reports.
    symbols: Option<Arc<Symbols>>,
    /// The CPU-write log setting chosen with [`Machine::set_cpu_write_log`]; `Verify` turns the
    /// core's log on while it records a call and restores this.
    cpu_write_log: bool,
    /// Writes of the user's log that `Verify` took out of the core, oldest first; they come
    /// before the core's in [`Machine::take_cpu_writes`].
    held_cpu_writes: Vec<(u32, u8)>,
    /// `Verify` calls recording their CPU writes, nested ones included (see
    /// [`Machine::start_cpu_write_capture`]).
    capture_depth: usize,
    /// The CPU writes taken out of the core since the outermost of them started, oldest first.
    captured_writes: Vec<(u32, u8)>,
    /// [`Machine::nmi_count`] at the last sync event (at power-on or the last load if none since):
    /// the next event needs a higher count.
    last_sync_nmi: u64,
    /// [`Machine::nmi_count`] at the last arrival counted in
    /// [`HookStats::sync_inside_dispatch`]: one count per NMI at most.
    last_inside_nmi: u64,
    /// Whether this is the snapshot a `Verify` call runs its Rust function on.
    is_verify_snapshot: bool,
    /// What the runs of [`Machine::run_until`] in progress have seen; a nested run adds its own
    /// to its caller's when it ends.
    marks: RunMarks,
    /// Hook keys of the Rust routines running, outermost first: an arrival at one of them is
    /// not dispatched (see [`Machine::dispatch`]).
    in_flight: Vec<u32>,
    /// Hook keys of the `Verify` calls whose original is running, outermost first, with the S of
    /// their hit: an arrival at one of them resumes the original (see
    /// [`Machine::dispatch_nested`]).
    verifying: Vec<(u32, u16)>,
    /// The stack lockstep watches for its dead stack, if it does (see [`Machine::watch_stack`]).
    stack_watch: Option<StackWatch>,
    /// What the `Rust` runs so far stand for, in instructions: for each, its routine's mean
    /// instructions less what it ran itself as original code or as the runs of other routines
    /// ([`HookStats::callee_instructions`]). A Rust routine that calls original code reads it
    /// before and after to know what the code it called accounts for.
    rust_instructions_run: f64,
    /// Opcode fetches of the original code by 23-bit address (`xtask causes`; empty unless
    /// enabled with [`Machine::enable_pc_histogram`]).
    #[cfg(feature = "histogram")]
    histogram: Vec<u64>,
    /// Declined arrivals by log2 of the cycles to the next timing event (`xtask causes`).
    #[cfg(feature = "histogram")]
    until_declined: [u64; 40],
    /// Arrivals at routine entries dispatched ([`FrameStats::dispatches`]); a counter, not state.
    dispatch_count: u64,
    /// The counters at the last frame boundary; the next frame's statistics are the difference.
    frame_base: FrameCounters,
    last_frame: Option<FrameStats>,
    /// WRAM freezes ([`Machine::set_freezes`]), canonical addresses.
    freezes: Vec<Freeze>,
    /// The last hook hit taken (24-bit address, opcode fetches then): the CPU still there means
    /// the arrival was handled and none is pending ([`Machine::snapshot`]).
    last_hit: Option<(u32, u64)>,
    /// The rest of a `Rust` call's final charge, when the CPU was held across a frame boundary
    /// and the run stops at frame ends: the held ticks go on, one tick at a time, from the next
    /// tick of any run ([`PendingHold`]).
    pending_hold: Option<PendingHold>,
    /// Set by the top level of [`Machine::run`] for the dispatch of one arrival: that dispatch
    /// may leave the end of a `Rust` call's charge pending ([`Machine::pending_hold`]). Taken by
    /// [`Machine::dispatch`], so nothing it runs or dispatches inherits it.
    may_defer_hold: bool,
    /// Whether the generated code's reads are logged ([`Machine::log_reads`]).
    log_reads: bool,
    /// Instructions after which a generated function on this `Verify` snapshot counts as running
    /// away ([`Ctx::runaway_limit`]).
    runaway_limit: u64,
    /// The addresses read since the log was last taken (24-bit), in order; filled by
    /// [`Ctx::read8`] while `log_reads` is set.
    read_log: Vec<u32>,
    /// The reads of the Rust function of the last `Verify` call that ran one, outermost call
    /// last ([`Machine::take_check_reads`]); recorded while `log_reads` is set.
    check_reads: Vec<u32>,
}

/// The part of a `Rust` call's charge not yet held: `fast` CPU cycles of 6 master cycles and `slow`
/// ones of 8 (see [`Machine::hold_cpu_cycles`]), and whether the interrupt lines are sampled when
/// it is over. The routine has run and returned already; only its time is still to pass.
///
/// A call whose charge spans a frame boundary (a decompression with the screen off, a few frames
/// long) renders that frame inside the dispatch. Left at that, the run would go on to the end of
/// the call and return several frames on, and everything that counts calls as frames (the
/// joypad and the host events of a replay, a recorder, a harness comparing frame by frame) would
/// meet the machine at the wrong frame. So a run that stops at frame ends returns right after the
/// frame rendered and leaves the rest of the charge here; the following ticks hold the CPU until
/// it has passed, then the original goes on at the routine's return address. Every machine
/// state a harness can see in between is the original's at that master cycle except for what the
/// routine has already written, which the original writes over the rest of the charge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PendingHold {
    fast: u64,
    slow: u64,
    poll: bool,
}

/// A `Rust` call at its entry: the hooked routine, its Rust function and return, the bound on its
/// cost, and whether its charge may be left partly pending ([`PendingHold`]).
#[derive(Clone, Copy)]
struct RustCall {
    key: u32,
    func: RoutineFn,
    ret: ReturnKind,
    max_cost: u64,
    defer_hold: bool,
}

/// What [`Machine::run_rust_unchecked`] saw the Rust function do.
#[derive(Debug, Clone, Copy)]
pub struct RustRun {
    /// The instructions it declared (its own, and those of the code it called in Rust).
    pub instructions: Option<u64>,
    /// Where the original goes on if the function handed over instead of returning.
    pub resume: Option<u32>,
    /// The cost it declared, in CPU-busy master cycles.
    pub cost: Option<u64>,
    /// It gave up as a runaway.
    pub runaway: bool,
    /// It dropped its caller's return address.
    pub dropped: bool,
}

/// The cumulative counters [`FrameStats`] are differences of.
#[derive(Debug, Clone, Copy, Default)]
struct FrameCounters {
    original: u64,
    rust: u64,
    master_cycles: u64,
    nmis: u64,
    dispatches: u64,
}

/// Where [`Machine::run_until`] ends, and which arrivals it dispatches meanwhile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RunEnd {
    /// The CPU is about to fetch the opcode at `key`, a hook key the installed table reports
    /// (such as a trap), with S = `s`: that arrival is taken, not dispatched. Every other
    /// arrival is dispatched ([`Machine::dispatch`]), an arrival at `key` with another S
    /// included.
    Arrival { key: u32, s: u16 },
    /// The routine whose opcode fetch the CPU is at returns: an instruction boundary with S above
    /// its value at the start, even with an interrupt pending. Only arrivals at other `Verify`
    /// routines are dispatched, as nested `Verify` calls ([`Machine::dispatch_nested`]); the
    /// others resume the original.
    Return,
}

/// What the runs of [`Machine::run_until`] in progress have seen, nested runs included.
#[derive(Debug, Clone, Copy)]
struct RunMarks {
    /// The lowest stack pointer (for a nested run that switched stacks, its start S).
    stack_low: u16,
    /// The CPU entered an NMI or IRQ handler.
    interrupted: bool,
}

impl RunMarks {
    const NONE: Self = Self { stack_low: u16::MAX, interrupted: false };
}

/// What [`Machine::run_until`] did, the calls it dispatched included.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RunOutcome {
    /// The run reached its [`RunEnd`] within the budget.
    pub reached: bool,
    /// A frame was rendered meanwhile.
    pub rendered: bool,
    /// CPU-busy master cycles of the run.
    pub cpu_cycles: u64,
    /// Instructions started (opcode fetches) in the run, the calls dispatched meanwhile included.
    pub instructions: u64,
    /// HDMA copied data meanwhile.
    pub hdma_crossed: bool,
    /// The lowest stack pointer seen, nested runs included; after a stack switch, the stack
    /// pointer at the start (the switched stack may lie in a buffer the code filled).
    pub min_s: u16,
    /// The stack pointer moved by more than [`MAX_STACK_MOVE`] in one tick outside interrupt
    /// handling and dispatches.
    pub stack_switched: bool,
    /// The CPU entered an NMI or IRQ handler meanwhile, nested runs included.
    pub interrupted: bool,
}

impl Machine {
    /// A machine at power-on, its WRAM filled from [`DEFAULT_WRAM_SEED`].
    ///
    /// # Errors
    ///
    /// Returns [`OracleError::Io`] if existing SRAM cannot be read, and [`OracleError::Load`] if
    /// the core rejects the ROM image.
    pub fn new(rom: Vec<u8>, saves: Box<dyn SaveStore>) -> Result<Self, OracleError> {
        Self::with_wram_seed(rom, saves, DEFAULT_WRAM_SEED)
    }

    /// A machine at power-on, its WRAM filled from `wram_seed` (the same bytes for the same
    /// seed).
    ///
    /// # Errors
    ///
    /// As for [`Machine::new`].
    pub fn with_wram_seed(
        rom: Vec<u8>,
        saves: Box<dyn SaveStore>,
        wram_seed: u64,
    ) -> Result<Self, OracleError> {
        let rom_crc32 = ssd_rom::crc32(&rom);
        Self::create(rom, rom_crc32, rom_crc32, saves, wram_seed)
    }

    /// A machine at power-on that runs `image` instead of a ROM: a pack set's image (`ssd_mod::build`),
    /// whose first 1 MiB is the ROM with CRC32 `original_crc32`. [`Machine::rom_crc32`] reports that CRC32,
    /// [`Machine::image_crc32`] the image's, and the save states are keyed on the image's. Its WRAM fills
    /// from `wram_seed` as [`Machine::with_wram_seed`] does.
    ///
    /// # Errors
    ///
    /// As for [`Machine::new`].
    pub fn with_image(
        image: Vec<u8>,
        original_crc32: u32,
        saves: Box<dyn SaveStore>,
        wram_seed: u64,
    ) -> Result<Self, OracleError> {
        let image_crc32 = ssd_rom::crc32(&image);
        Self::create(image, original_crc32, image_crc32, saves, wram_seed)
    }

    /// The machine of [`Machine::with_wram_seed`] and [`Machine::with_image`] for the bytes `rom`
    /// (the ROM or the image) with the given CRC32s.
    fn create(
        rom: Vec<u8>,
        rom_crc32: u32,
        image_crc32: u32,
        mut saves: Box<dyn SaveStore>,
        wram_seed: u64,
    ) -> Result<Self, OracleError> {
        // The core treats any SRAM read error as "no save" and would later overwrite the real file
        // with blank SRAM, so surface read errors here before the core is created.
        saves.load("sav")?;
        let image = rom_image(&rom);
        // Changing this configuration changes what replays play: bump `ORACLE_REVISION`.
        let config = SnesEmulatorConfig {
            audio_60hz_hack: false,
            wram_fill_seed: Some(wram_seed),
            ..SnesEmulatorConfig::default()
        };
        let emu = SnesEmulator::create(
            rom,
            config,
            CoprocessorRoms::none(),
            &mut SaveWriterAdapter(saves.as_mut()),
        )
        .map_err(|e| OracleError::Load(e.to_string()))?;
        let capture = FrameCapture {
            pixels: vec![Color::BLACK; 256 * 224],
            size: Some(FrameSize { width: 256, height: 224 }),
            ..FrameCapture::default()
        };
        Ok(Self {
            emu,
            saves,
            capture,
            frames: 0,
            input_cache: InputCache::default(),
            rom_crc32,
            image_crc32,
            rom: image,
            wram_seed,
            audio_hz: None,
            hooks: None,
            hooked: false,
            table_hooked: false,
            breakpoints: BTreeSet::new(),
            skip_breakpoint: None,
            stats: BTreeMap::new(),
            divergence_dir: None,
            divergences: Vec::new(),
            symbols: None,
            cpu_write_log: false,
            held_cpu_writes: Vec::new(),
            capture_depth: 0,
            captured_writes: Vec::new(),
            last_sync_nmi: 0,
            last_inside_nmi: 0,
            is_verify_snapshot: false,
            marks: RunMarks::NONE,
            in_flight: Vec::new(),
            verifying: Vec::new(),
            stack_watch: None,
            rust_instructions_run: 0.0,
            #[cfg(feature = "histogram")]
            histogram: Vec::new(),
            #[cfg(feature = "histogram")]
            until_declined: [0; 40],
            dispatch_count: 0,
            frame_base: FrameCounters::default(),
            last_frame: None,
            freezes: Vec::new(),
            last_hit: None,
            pending_hold: None,
            may_defer_hold: false,
            log_reads: false,
            runaway_limit: DEFAULT_RUNAWAY_LIMIT,
            read_log: Vec::new(),
            check_reads: Vec::new(),
        })
    }

    /// Run the emulator until it renders one frame, one [`Machine::tick`] at a time, dispatching
    /// every hook hit (spec §5.3): [`Machine::run`] with [`Stops::frame_end`].
    ///
    /// Sync points resume the original routine (the CPU fetches the hooked opcode on the next
    /// tick). So does an arrival at a routine already running in Rust (called back through
    /// [`Ctx::call_original`]; not counted in its `calls`), and a routine when the CPU does not
    /// meet its entry contract (native mode, `entry_m8`, `entry_x8`; counted in
    /// `contract_mismatch`). In [`HookMode::Rust`], so does
    /// a routine whose entry has no `cycles` (its maximum cost; counted in `cost_missing`), and
    /// one that a timing event could reach within that cost: an NMI, an IRQ or HDMA within
    /// `cycles` plus the memory refreshes it can span plus 8 master cycles
    /// ([`Machine::master_cycles_until_timing_event`]), or a GPDMA requested right before the
    /// call (counted in `timing_fallback`): the Rust function, atomic at the entry, could not
    /// reproduce it. Otherwise:
    ///
    /// 1. the Rust function runs on the live machine through [`Ctx`] (time passes only through
    ///    [`Ctx::charge`] and the original code it runs with [`Ctx::call_original`], whose hook
    ///    hits are dispatched as here);
    /// 2. the return (RTS/RTL) is performed on the emulated stack;
    /// 3. the call is charged its cost, the own cost the function declared with
    ///    [`Ctx::set_cost`] plus the time of the original code it called (else the entry's
    ///    `cycles` in all, counted in `cost_missing`): the CPU is held until
    ///    [`Machine::cpu_busy_cycles`] has grown by exactly that much since the hit (held CPU
    ///    cycles of 6 and 8 master cycles; the few costs that cannot be exact are rounded, see
    ///    `split_charge`). With no timing event in the window, the machine is then where the
    ///    original would be, cycle for cycle. A call whose cost is above `cycles`, or that
    ///    spent more before its return than it declared, counts in `cost_over_bound`;
    /// 4. a return to a hooked address is reported like any other arrival.
    ///
    /// In [`HookMode::Verify`], the first of every `verify_sample` calls is checked and the
    /// others resume the original (`sampled_out`). A checked call:
    ///
    /// 1. snapshots the machine without its frame capture (and the snapshot again, as the entry
    ///    state for a report, if a divergence directory is set and the routine has fewer than
    ///    32 divergences);
    /// 2. runs the original on the live machine, one tick at a time, until it returns (an
    ///    instruction boundary with S above its entry value, even with an interrupt pending; a
    ///    nested call can return for it, as after a tail call). An arrival inside it at the
    ///    entry of another `Verify` routine is a nested `Verify` call, dispatched as here
    ///    (counted in `nested`, with its own snapshot, budget and comparison) unless 16 `Verify`
    ///    calls are in progress (the original resumes, counted in `calls` and
    ///    `nested_overflow`). An arrival at the entry of a `Verify` call in progress runs as
    ///    original, as part of that call: counted in `calls` and `reentered` if S is below the S
    ///    of that call's hit (a recursion, or an interrupt handler calling the routine it
    ///    interrupted), uncounted otherwise (a loop back to the entry). Other arrivals inside it
    ///    run as original, uncounted: sync points (but in `sync_inside_dispatch`, see
    ///    [`Machine::run_until_sync`]) and routines in other modes. Calls made by an
    ///    interrupt handler inside it are nested calls like any other: the interrupt is the
    ///    outer call's, not theirs. An arrival at the return address is reported like any
    ///    other. The CPU-busy master cycles and the instructions (opcode fetches) from the hit to
    ///    the return, nested calls included, update `mean_cpu_cycles` and `mean_instructions`
    ///    (calls that an interrupt entered excepted). An original still
    ///    running after its budget (2 frames of master cycles, or `budget_frames`) counts as `no_return`: the entry is demoted to
    ///    [`HookMode::Original`] (a warning is logged) and the live machine runs on (a nested
    ///    call's 2 frames end a few cycles after its callers', so they do not return either);
    /// 3. counts `hdma_crossed` if HDMA copied data during the run, `stack_switched` if the
    ///    original moved S by more than 4 in one tick outside interrupt handling and nested
    ///    calls (`TCS`, `TXS`), and `interrupted` if the CPU entered an NMI or IRQ handler (in a
    ///    nested call too): such a call is not compared;
    /// 4. runs the Rust function on the snapshot as in `Rust` mode (a GPDMA requested right
    ///    before the call drained first, the function and the original code it calls, the
    ///    return; no final charge). Hook hits in that code are dispatched on the snapshot,
    ///    `Verify` ones as in `Rust` mode; their counters are discarded with the snapshot. HDMA
    ///    copying data meanwhile on the snapshot (the time the function charges) counts in
    ///    `hdma_crossed` like HDMA during the original's run. It compares the cost the function
    ///    declared plus the time of the code it called with the original's (`cost_mismatch` if
    ///    they differ, HDMA-crossed calls included: both are CPU-busy cycles, which leave out
    ///    the DMA and refresh time; `cost_missing` if it declared none; `cost_over_bound` as in
    ///    `Rust` mode): a nested call is part of the original's own time, so the function
    ///    declares it in its own cost unless it runs that routine with
    ///    [`Ctx::call_original`]. It compares the outcomes with
    ///    [`compare`](crate::compare): registers (P under `flags_mask`), WRAM except the stack
    ///    bytes left dead (pushed during the call, by the original and its nested calls or by
    ///    the code the Rust function called, and below S after the original's return; after a
    ///    stack switch, its return address only), SRAM and, unless HDMA crossed the call, VRAM,
    ///    CGRAM, OAM and the CPU writes to observable registers (a nested call's included).
    ///    `verified_ok`, or `divergences` and, for the first 32 of the routine, a [`Divergence`]
    ///    (with at most 256 differences, and a note if the cost went beyond its bounds) named
    ///    after the variables of [`Machine::set_symbols`] and saved with [`report::save`] in the
    ///    divergence directory, if one is set. A report that cannot be saved logs a warning and
    ///    counts in `report_errors`; the run goes on.
    ///
    /// The live machine keeps the original's result, and its timing is exactly the original's.
    /// Both runs record their CPU writes whatever [`Machine::set_cpu_write_log`] says; the user's
    /// log ends up as without hooks. A Verify call is over when its dispatch ends: nothing of it
    /// is in flight across `run_frame` calls or a [`Machine::load_state`].
    ///
    /// If a frame was rendered during a dispatch, `run_frame` returns right after it. In `Rust`
    /// mode that is the first frame rendered while the call's charge is held: the cycles of the
    /// charge still to hold stay pending ([`Machine::charge_pending`]) and the ticks of the next
    /// runs hold them before the original goes on at the routine's return address. So a call
    /// whose charge spans several frames (a decompression with the screen off) does not make a
    /// run render several: `run_frame` renders one frame, as without hooks, and the frame count
    /// is the number of runs. (Original code a Rust routine calls, and a `Verify` call, render
    /// their frames inside the dispatch.) With no hooked entry and no breakpoint installed this
    /// is the plain core loop, cycle for cycle, without hit checks. Sync events are reported by the runs with [`Stops::sync`] only
    /// ([`Machine::run_until_sync`], or [`Machine::run`] with it): `run_frame` passes sync points
    /// without recording an event, and breakpoints without stopping.
    ///
    /// `input` is held on every tick of the call, the ticks of a dispatch that runs on after the
    /// frame it renders included (they belong to the next frame, whose input this call does not
    /// know): an auto-joypad read among them latches `input`, where a run without hooks would
    /// latch the next call's. [`Machine::run_frame_with`] looks the input of every tick up by
    /// its frame instead; playing a replay with hooks needs it.
    ///
    /// # Errors
    ///
    /// Returns [`OracleError::Run`] if the core fails (for example while persisting SRAM), or if
    /// original code called from a Rust function does not return within its budget
    /// ([`Ctx::call_original`]). If that happens during a Rust call, the return has been
    /// performed and the call is not charged. The snapshot a `Verify` call runs the Rust function
    /// on cannot fail but through such a call in practice (in-memory saves, infallible frame
    /// capture and audio); if it does, its error is returned with the original's call complete
    /// and the call not counted.
    pub fn run_frame(
        &mut self,
        input: Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<(), OracleError> {
        self.run_frame_with(&mut |_| input, audio)
    }

    /// [`Machine::run_frame`] with the joypad of each tick looked up by the frame the tick belongs
    /// to: `input(f)` is held from the tick after frame `f` is rendered to the tick that renders
    /// frame `f + 1` (see [`crate::input`]), in dispatches too. A dispatch that renders frame
    /// `f + 1` and runs on feeds `input(f + 1)` after the render, as a run without hooks does.
    /// Without hooks, the run is tick for tick the one of `run_frame(input(f))`.
    ///
    /// `input` is asked when a frame's first tick is polled, and may be asked again for the same
    /// frame (by a later call, or a `Verify` snapshot): it must give the same joypad for the same
    /// frame.
    ///
    /// # Errors
    ///
    /// As for [`Machine::run_frame`].
    pub fn run_frame_with(
        &mut self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<(), OracleError> {
        // No budget: the frame ends the run.
        let stops = Stops { frame_end: true, ..Stops::default() };
        self.run(&stops, input, audio, u64::MAX).map(drop)
    }

    /// Runs like [`Machine::run_frame_with`] (hook hits dispatched as described in
    /// [`Machine::run_frame`]) until the first hit at the hooked `address` (or a mirror of it) has
    /// been dispatched, and returns `true`: the call is over, whatever the dispatch did (a
    /// `Verify` check, a `Rust` run, or the original resumed), and its counters
    /// ([`Machine::hook_stats`]) tell which. Returns `false` once `budget_frames` frames have been
    /// rendered since the call without such a hit (at once for a budget of 0). A machine loaded
    /// with [`Machine::load_state`] at a hooked opcode fetch hits it first thing, so this replays
    /// that call and nothing else; that is how a divergence report is reproduced
    /// ([`report::save`]). [`Machine::run`] with [`Stops::call`].
    ///
    /// Hits are counted only at the top level of this loop, not inside the dispatch of another
    /// routine: a call of `address` made by the original code of a `Verify` call, or by a Rust
    /// routine through [`Ctx::call_original`], is dispatched there and does not end this call.
    /// So the routine's counters hold this one call only if no other hooked routine calls it.
    ///
    /// # Errors
    ///
    /// As for [`Machine::run_frame`].
    pub fn run_until_call(
        &mut self,
        address: u32,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
        budget_frames: u64,
    ) -> Result<bool, OracleError> {
        let stops = Stops { call: Some(address), ..Stops::default() };
        let stopped = self.run(&stops, input, audio, budget_frames)?;
        Ok(matches!(stopped, Stopped::Call { .. }))
    }

    /// Runs the emulator one [`Machine::tick`] at a time, dispatching every hook hit as described
    /// in [`Machine::run_frame`], until one of `stops` (see [`Stops`]) or until `budget_frames`
    /// frames have been rendered since the call ([`Stopped::Budget`]; at once for a budget of 0).
    /// The loop of [`Machine::run_frame_with`] ([`Stops::frame_end`]),
    /// [`Machine::run_until_call`] ([`Stops::call`]) and [`Machine::run_until_sync`]
    /// ([`Stops::sync`]).
    ///
    /// Stops reached at the same point come in this order:
    ///
    /// - at an arrival the run reports (at its top level, never inside a dispatch): a
    ///   breakpoint, before anything else (the hit is left pending: the next run or step
    ///   dispatches it, see [`Machine::set_breakpoint`]); a sync event, before the arrival is
    ///   dispatched (the hit is taken: the next tick fetches the opcode); then the arrival is
    ///   dispatched; then `after_dispatch`, then the call, then a frame the dispatch rendered,
    ///   then the budget;
    /// - before each tick, at an instruction boundary: an interrupt entry, then `instructions`,
    ///   then `stack_above`, then `returned_above`. They are checked before a pending hit is
    ///   taken: the next run handles that arrival. An interrupt comes first because `instructions`
    ///   is reached at the handler's first instruction too when the interrupt followed the last
    ///   instruction counted.
    ///
    /// Each tick holds the input of the frame it belongs to, `input(frame)`, a dispatch that
    /// renders a frame and runs on included (see [`Machine::run_frame_with`]). The stack lockstep
    /// watches, if any, follows every tick and dispatch (and every tick of the original code a
    /// dispatch runs). With no hooked entry and no breakpoint installed, no stack watched, and
    /// only `frame_end` to stop at, this is the plain core loop, cycle for cycle, without hit
    /// checks.
    ///
    /// # Errors
    ///
    /// As for [`Machine::run_frame`].
    pub fn run(
        &mut self,
        stops: &Stops,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
        budget_frames: u64,
    ) -> Result<Stopped, OracleError> {
        self.input_cache.clear();
        let end = self.frames.saturating_add(budget_frames);
        if self.frames >= end {
            return Ok(Stopped::Budget);
        }
        let mut audio = AudioAdapter(audio);
        let fine = stops.interrupt
            || stops.instructions.is_some()
            || stops.stack_above.is_some()
            || stops.returned_above.is_some();
        let watch = self.stack_watch.is_some();
        if stops.frame_end && !self.hooked && !fine && !watch && self.pending_hold.is_none() {
            // Nothing but the frame's end can stop this run.
            while !self.tick_with(input, &mut audio)? {}
            self.apply_freezes();
            return Ok(Stopped::FrameEnd);
        }
        match (watch, fine) {
            (false, false) => self.run_loop::<false, false>(stops, end, input, &mut audio),
            (false, true) => self.run_loop::<false, true>(stops, end, input, &mut audio),
            (true, false) => self.run_loop::<true, false>(stops, end, input, &mut audio),
            (true, true) => self.run_loop::<true, true>(stops, end, input, &mut audio),
        }
    }

    /// The loop of [`Machine::run`] until the frame count `end`. `WATCH` updates the stack watch
    /// after every tick and dispatch, `FINE` checks `interrupt`, `instructions`, `stack_above`
    /// and `returned_above` before every tick; without them, a tick costs its event and nothing
    /// else.
    fn run_loop<const WATCH: bool, const FINE: bool>(
        &mut self,
        stops: &Stops,
        end: u64,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut AudioAdapter<'_>,
    ) -> Result<Stopped, OracleError> {
        let mut fine = Fine {
            fetches_start: self.emu.opcode_fetches(),
            last: None,
            returned: false,
            interrupted: false,
        };
        loop {
            if FINE && let Some(stopped) = self.boundary_stop(stops, &mut fine) {
                return Ok(stopped);
            }
            match self.tick_event(input, audio)? {
                TickEvent::None => {
                    if WATCH {
                        self.watch_tick(false);
                    }
                    if FINE {
                        fine.interrupt_entry(self.emu.is_handling_interrupt());
                    }
                }
                TickEvent::Frame => {
                    self.apply_freezes();
                    if WATCH {
                        self.watch_tick(false);
                    }
                    if FINE {
                        fine.interrupt_entry(self.emu.is_handling_interrupt());
                    }
                    if stops.frame_end {
                        return Ok(Stopped::FrameEnd);
                    }
                    if self.frames >= end {
                        return Ok(Stopped::Budget);
                    }
                }
                TickEvent::Hook(full_pc) => {
                    let (stopped, dispatch) =
                        self.arrival(full_pc, stops, end, WATCH, input, &mut *audio.0)?;
                    if FINE && dispatch.ran() {
                        // The routine's return brought the CPU where it is, with no fetch of its
                        // own in a `Rust` run.
                        fine.returned = true;
                        fine.last = None;
                    }
                    if let Some(stopped) = stopped {
                        return Ok(stopped);
                    }
                }
            }
        }
    }

    /// The `interrupt`, `instructions`, `stack_above` or `returned_above` stop of a run, if the CPU
    /// is at an opcode fetch and one is reached (`fine`: what the run's boundaries saw so far).
    /// Checking the same boundary again (after an arrival that resumed the original) gives the
    /// same answer.
    fn boundary_stop(&self, stops: &Stops, fine: &mut Fine) -> Option<Stopped> {
        if !self.emu.at_fetch_boundary() {
            return None;
        }
        let fetches = self.emu.opcode_fetches();
        // One instruction since the opcode read at the last boundary: it brought the CPU here.
        if let Some((at, opcode)) = fine.last
            && fetches == at + 1
        {
            fine.returned = !fine.interrupted && opcode.is_some_and(is_return);
        }
        let interrupted = std::mem::take(&mut fine.interrupted);
        if stops.interrupt && interrupted {
            return Some(Stopped::Interrupt);
        }
        if stops.instructions.is_some_and(|n| fetches - fine.fetches_start >= n) {
            return Some(Stopped::Instructions);
        }
        let registers = self.emu.cpu_registers();
        if stops.stack_above.is_some_and(|s| registers.s > s) {
            return Some(Stopped::StackAbove);
        }
        if let Some(s) = stops.returned_above {
            if fine.returned && registers.s > s {
                return Some(Stopped::Returned);
            }
            fine.last = Some((fetches, self.peek_code(full_pc_of(&registers))));
        }
        None
    }

    /// An arrival [`Machine::run`] reports at its top level: its stops, in the order given there,
    /// around its dispatch (`watch`: the stack watch follows the dispatch), and what the dispatch
    /// did ([`Dispatch::Resumed`] if the run stopped before it). Kept out of line, as the
    /// dispatch.
    #[cold]
    #[inline(never)]
    fn arrival(
        &mut self,
        full_pc: u32,
        stops: &Stops,
        end: u64,
        watch: bool,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<(Option<Stopped>, Dispatch), OracleError> {
        let key = hook_key(full_pc);
        // The first arrival ends the skip; it is skipped if the CPU has not moved since.
        let skip = self.skip_breakpoint.take();
        if stops.breakpoint
            && self.breakpoints.contains(&key)
            && skip != Some((key, self.emu.opcode_fetches()))
        {
            // The CPU has not moved: the hit is reported again by the next tick.
            self.last_hit = None;
            self.emu.detect_hook_hit();
            self.skip_breakpoint = Some((key, self.emu.opcode_fetches()));
            return Ok((Some(Stopped::Breakpoint { key }), Dispatch::Resumed));
        }
        if stops.sync
            && let Some(event) = self.take_sync_event(full_pc)
        {
            return Ok((Some(Stopped::Sync(event)), Dispatch::Resumed));
        }
        // A run that stops at frame ends and at nothing finer leaves the end of a `Rust` call's
        // charge pending when a frame is rendered inside it (see `PendingHold`).
        self.may_defer_hold = stops.frame_end
            && !stops.interrupt
            && stops.instructions.is_none()
            && stops.stack_above.is_none()
            && stops.returned_above.is_none();
        let dispatch = self.dispatch(full_pc, input, audio)?;
        if watch {
            self.watch_tick(true);
        }
        let rendered = dispatch.rendered();
        if rendered {
            self.apply_freezes();
        }
        if stops.after_dispatch
            && let Some(entry) = hooked_entry(self.hooks.as_ref(), key)
        {
            let name = entry.name.clone();
            return Ok((
                Some(Stopped::Dispatched { name, ran: dispatch.ran(), rendered }),
                dispatch,
            ));
        }
        if stops.call.is_some_and(|address| hook_key(address) == key) {
            let stats = self.stats.get(&key).copied().unwrap_or_default();
            return Ok((Some(Stopped::Call { stats, rendered }), dispatch));
        }
        if rendered && stops.frame_end {
            return Ok((Some(Stopped::FrameEnd), dispatch));
        }
        Ok(((self.frames >= end).then_some(Stopped::Budget), dispatch))
    }

    /// Handles a hook hit as described in [`Machine::run_frame`]; returns whether the routine ran
    /// (and whether a frame was rendered meanwhile) or the original resumes. An arrival at a
    /// routine already running in Rust (its own entry, called back through
    /// [`Ctx::call_original`] to run a path in its original code, or a recursion) resumes the
    /// original, uncounted, and so does an arrival at an address without an entry (a
    /// breakpoint, a trap with no call of original code in flight). Inside the original of a
    /// `Verify` call, arrivals go through [`Machine::dispatch_nested`] first: only nested
    /// `Verify` calls get here. Kept out of line so the per-tick loop stays small.
    #[cold]
    #[inline(never)]
    pub(crate) fn dispatch(
        &mut self,
        full_pc: u32,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<Dispatch, OracleError> {
        let key = hook_key(full_pc);
        let defer_hold = std::mem::take(&mut self.may_defer_hold);
        if self.in_flight.contains(&key) {
            return Ok(Dispatch::Resumed);
        }
        let on_verify_snapshot = self.is_verify_snapshot();
        let Some(entry) = hooked_entry(self.hooks.as_ref(), key) else {
            return Ok(Dispatch::Resumed);
        };
        let stats = self.stats.entry(key).or_default();
        stats.calls += 1;
        // Sync points resume the original.
        let HookAction::Routine { func, ret } = entry.action else {
            return Ok(Dispatch::Resumed);
        };
        self.dispatch_count += 1;
        if !entry.contract_holds(&self.emu.cpu_registers()) {
            stats.contract_mismatch += 1;
            return Ok(Dispatch::Resumed);
        }
        // A Verify snapshot computes what the Rust routines leave: its Verify hooks run as Rust.
        let mode = match entry.mode {
            HookMode::Verify if on_verify_snapshot => HookMode::Rust,
            mode => mode,
        };
        match mode {
            HookMode::Patch => {
                let call = RustCall { key, func, ret, max_cost: 0, defer_hold };
                match self.dispatch_rust(&call, input, audio)? {
                    Some(rendered) => Ok(Dispatch::Ran { rendered }),
                    None => Ok(Dispatch::Resumed),
                }
            }
            HookMode::Rust => {
                let (cycles, bound) = (entry.cycles, entry.bound);
                // The bound of this call, from the entry state, in place of the static maximum.
                let max_cost = match bound {
                    Some(bound) => Some(self.cost_bound(key, bound, input, audio)),
                    None => cycles,
                };
                let Some(max_cost) = max_cost else {
                    self.stats_mut(key).cost_missing += 1;
                    return Ok(Dispatch::Resumed);
                };
                if !timing::window_is_clear(self, max_cost) {
                    #[cfg(feature = "histogram")]
                    {
                        let until = self.master_cycles_until_timing_event();
                        let bucket = if self.emu.gpdma_requested() {
                            39
                        } else {
                            (64 - until.leading_zeros() as usize).min(38)
                        };
                        self.until_declined[bucket] += 1;
                    }
                    self.stats_mut(key).timing_fallback += 1;
                    return Ok(Dispatch::Resumed);
                }
                let call = RustCall { key, func, ret, max_cost, defer_hold };
                match self.dispatch_rust(&call, input, audio)? {
                    Some(rendered) => Ok(Dispatch::Ran { rendered }),
                    None => {
                        self.stats_mut(key).timing_fallback += 1;
                        Ok(Dispatch::Resumed)
                    }
                }
            }
            HookMode::Verify => {
                let call = VerifyCall {
                    key,
                    func,
                    ret,
                    flags_mask: entry.flags_mask,
                    verify_sample: entry.verify_sample,
                    cycles: entry.cycles,
                    bound: entry.bound,
                    budget_frames: entry.budget(),
                };
                verify::verify(self, &call, input, audio)
            }
            // Not a hooked entry (see `hooked_entry`).
            HookMode::Original => Ok(Dispatch::Resumed),
        }
    }

    /// A `Rust` call: the routine runs in Rust and is charged the own cost it declares plus the
    /// time of the original code it called (else `max_cost`, the entry's `cycles`, in all) in
    /// CPU-busy master cycles.
    fn dispatch_rust(
        &mut self,
        call: &RustCall,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<Option<bool>, OracleError> {
        let RustCall { key, func, ret, max_cost, defer_hold } = *call;
        let run = self.run_routine(key, func, ret, input, audio)?;
        // What it pushed counts for the run it is part of.
        if let Some(low) = run.lowest_s {
            self.marks.stack_low = self.marks.stack_low.min(low);
        }
        if run.spent == 0 && run.resume.is_some_and(|pc| hook_key(pc) == key) {
            // The routine declined at its own entry before it spent any time (a recompiled
            // routine whose first stretch does not fit the window): the original runs.
            return Ok(None);
        }
        let stats = self.stats_mut(key);
        stats.cost_missing += u64::from(run.cost.is_none());
        let bound = if run.gated {
            None
        } else {
            cost_over_bound(run.cost, run.callee_cycles, run.spent, Some(max_cost))
        };
        stats.cost_over_bound += u64::from(bound.is_some());
        stats.callee_instructions += run.callee_instructions;
        if let Some(own) = run.instructions {
            stats.declared_runs += 1;
            stats.declared_instructions += own as f64 + run.callee_instructions;
        }
        // Counted before the hold, which can render a frame: the run's callee instructions are
        // already charged, and the frame's Rust estimate must not dip below the previous one's.
        stats.rust_runs += 1;
        let total = run.cost.map_or(max_cost, |own| own + run.callee_cycles);
        let (fast, slow) = split_charge(total.saturating_sub(run.spent));
        let (held_rendered, rest) =
            self.hold_cpu_cycles_to(input, audio, fast, slow, defer_hold)?;
        let rendered = run.rendered | held_rendered;
        if let Some((fast, slow)) = rest {
            // A frame was rendered with charge left: the run returns at the frame, and the next
            // ticks hold the rest, then do what follows.
            self.pending_hold = Some(PendingHold { fast, slow, poll: !run.no_poll });
        } else {
            // The held ticks neither polled the interrupt lines nor detected the arrival at the
            // return address.
            if !run.no_poll {
                self.emu.poll_cpu_interrupt_lines();
            }
            self.emu.detect_hook_hit();
        }

        let stats = self.stats_mut(key);
        // What this run stands for: its routine's mean, less what it ran or had run elsewhere.
        let own_instructions = match run.instructions {
            Some(own) => own as f64,
            None => stats.mean_instructions - run.callee_instructions,
        };
        self.rust_instructions_run += own_instructions.max(0.0);
        Ok(Some(rendered))
    }

    /// What the `Rust` runs so far stand for, in instructions (see the field of that name).
    pub(crate) fn rust_instructions_run(&self) -> f64 {
        self.rust_instructions_run
    }

    /// Runs `func` in place of the routine under `key` whose opcode fetch the CPU is at: a
    /// GPDMA requested right before the call runs to its end (CPU held), `func` runs through
    /// [`Ctx`] (`key` in flight meanwhile), and the return is performed. Returns what `func`
    /// left, with the CPU-busy master cycles spent from the end of that transfer (the hit, in
    /// `Rust` mode, which never has one): the transfer is the original's DMA time, not its
    /// cost.
    ///
    /// The return is performed even if a tick made for `func` failed, so the CPU is never left
    /// inside a routine that already ran; the error is returned after it.
    pub(crate) fn run_routine(
        &mut self,
        key: u32,
        func: RoutineFn,
        ret: ReturnKind,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<Finished, OracleError> {
        let drained = self.drain_gpdma(input, audio)?;
        let busy_start = self.emu.cpu_busy_cycles();
        self.in_flight.push(key);
        let mut ctx = Ctx::new(self, key, input, audio);
        func(&mut ctx);
        let ran = ctx.finish();
        self.in_flight.pop();
        let mut registers = self.emu.cpu_registers();
        match ran.as_ref().ok().and_then(|finished| finished.resume) {
            Some(pc) => {
                // The routine did not return: the original goes on from `pc`.
                registers.pbr = (pc >> 16) as u8;
                registers.pc = pc as u16;
            }
            None => ret.perform(&mut registers, |address| self.emu.bus_read(address)),
        }
        self.emu.set_cpu_registers(registers);
        let mut finished = ran?;
        finished.rendered |= drained;
        finished.spent = self.emu.cpu_busy_cycles() - busy_start;
        Ok(finished)
    }

    /// Runs the original code at the 24-bit `address` for the Rust routine under `routine` (see
    /// [`Ctx::call_original`]): pushes the return to the trap of `address`'s bank, starts the
    /// code there, runs until the CPU arrives at the trap (2-frame budget) and puts PBR and PC
    /// back. The arrival at the trap is the return only with S back where it was before the
    /// return address was pushed: in banks `$40–$7F`, `$21FF` can be memory that code runs
    /// through. On error the registers are put back as before the call, once the instruction in
    /// progress is over (see [`Machine::finish_instruction`]).
    pub(crate) fn call_original(
        &mut self,
        routine: u32,
        address: u32,
        ret: ReturnKind,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<RunOutcome, OracleError> {
        let address = address & 0xFF_FFFF;
        let bank = (address >> 16) as u8;
        let saved = self.emu.cpu_registers();
        let s_return = saved.s;
        let mut registers = saved.clone();
        if ret == ReturnKind::Rtl {
            self.push8(&mut registers.s, bank);
        }
        let [low, high] = (TRAP_PC - 1).to_le_bytes();
        self.push8(&mut registers.s, high);
        self.push8(&mut registers.s, low);
        registers.pbr = bank;
        registers.pc = address as u16;
        self.emu.set_cpu_registers(registers);
        // The time charged so far stands for the code before the call, whose last instruction
        // samples the interrupt lines; a hooked entry is an arrival like any other.
        self.emu.poll_cpu_interrupt_lines();
        self.emu.detect_hook_hit();

        let budget_frames = self.budget_frames(routine);
        let budget = self.frames_mclk(budget_frames);
        let end = RunEnd::Arrival { key: trap_key(bank), s: s_return };
        let run = self.run_until(end, input, audio, budget);
        match run {
            Ok(run) if run.reached => {
                let mut registers = self.emu.cpu_registers();
                (registers.pbr, registers.pc) = (saved.pbr, saved.pc);
                self.emu.set_cpu_registers(registers);
                Ok(run)
            }
            Ok(_) => {
                let finished = self.finish_instruction(input, audio);
                self.emu.set_cpu_registers(saved);
                finished?;
                let callee = self.hooks.as_ref().and_then(|table| table.get(address));
                let callee = callee.map(|entry| format!(" ({})", entry.name)).unwrap_or_default();
                Err(OracleError::Run(format!(
                    "{}: the original code it called at ${address:06X}{callee} did not return \
                     within {budget_frames} frames",
                    self.hook_name(routine)
                )))
            }
            Err(error) => {
                self.emu.set_cpu_registers(saved);
                Err(error)
            }
        }
    }

    /// Ticks (hook hits not dispatched) until the CPU is between instructions, for at most a
    /// frame of master cycles: CPU registers set between two cycles of an instruction would
    /// meet the rest of its micro-operations.
    fn finish_instruction(
        &mut self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<(), OracleError> {
        let deadline = self.emu.total_master_cycles() + self.frames_mclk(1);
        let mut audio = AudioAdapter(audio);
        while self.emu.is_mid_instruction() && self.emu.total_master_cycles() < deadline {
            self.tick_event(input, &mut audio)?;
        }
        Ok(())
    }

    /// Pushes `value` as the CPU does in native mode: writes it at `s` in bank 0, then
    /// decrements `s` (wrapping in 16 bits).
    fn push8(&mut self, s: &mut u16, value: u8) {
        self.emu.bus_write(u32::from(*s), value);
        *s = s.wrapping_sub(1);
    }

    /// Runs the original of the `Verify` call under `key`, whose opcode fetch the CPU is at,
    /// until it returns ([`RunEnd::Return`]) or `budget_frames` frames of master cycles have passed. The call
    /// is in progress meanwhile: an arrival at its entry resumes the original (see
    /// [`Machine::dispatch_nested`]).
    pub(crate) fn run_original(
        &mut self,
        key: u32,
        budget_frames: u64,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<RunOutcome, OracleError> {
        self.verifying.push((key, self.emu.cpu_registers().s));
        let budget = self.frames_mclk(budget_frames);
        let run = self.run_until(RunEnd::Return, input, audio, budget);
        self.verifying.pop();
        run
    }

    /// Runs one tick at a time until `end` or until `budget` master cycles have passed,
    /// dispatching the arrivals `end` says. Tracks the frames rendered, the CPU-busy master
    /// cycles, HDMA copies, the lowest stack pointer, stack switches and interrupts, the calls
    /// dispatched meanwhile included (a dispatched call moves S by a whole call: no switch).
    /// Every tick also updates the stack watch, if any ([`Machine::watch_stack`]). An arrival at
    /// a sync point that would be a sync event counts in its `sync_inside_dispatch`
    /// ([`Machine::count_sync_inside_dispatch`]).
    pub(crate) fn run_until(
        &mut self,
        end: RunEnd,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
        budget: u64,
    ) -> Result<RunOutcome, OracleError> {
        let deadline = self.emu.total_master_cycles().saturating_add(budget);
        let busy_start = self.emu.cpu_busy_cycles();
        let fetches_start = self.emu.opcode_fetches();
        let hdma_start = self.emu.hdma_copy_units();
        let s_start = self.emu.cpu_registers().s;
        let fresh = RunMarks { stack_low: s_start, interrupted: false };
        let caller = std::mem::replace(&mut self.marks, fresh);
        let mut run = RunOutcome {
            reached: false,
            rendered: false,
            cpu_cycles: 0,
            instructions: 0,
            hdma_crossed: false,
            min_s: s_start,
            stack_switched: false,
            interrupted: false,
        };
        let ticked = self.tick_until(end, deadline, input, audio, &mut run);
        run.cpu_cycles = self.emu.cpu_busy_cycles() - busy_start;
        run.instructions = self.emu.opcode_fetches() - fetches_start;
        run.hdma_crossed = self.emu.hdma_copy_units() != hdma_start;
        run.interrupted = self.marks.interrupted;
        run.min_s = if run.stack_switched { s_start } else { self.marks.stack_low };
        self.marks = RunMarks {
            stack_low: caller.stack_low.min(run.min_s),
            interrupted: caller.interrupted | run.interrupted,
        };
        ticked.map(|()| run)
    }

    /// The loop of [`Machine::run_until`], which sets `run` and `self.marks` up and completes
    /// them.
    fn tick_until(
        &mut self,
        end: RunEnd,
        deadline: u64,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
        run: &mut RunOutcome,
    ) -> Result<(), OracleError> {
        let s_start = self.emu.cpu_registers().s;
        let mut audio = AudioAdapter(audio);
        let mut previous_s = s_start;
        // For the end of a call: the instruction that brought the CPU to the boundary must be a
        // return. A routine that pulls its caller's return address (a dispatcher that reads the
        // table after its `JSR`, an early exit of the caller) raises S above its start without
        // having returned.
        let mut fine = Fine { fetches_start: 0, last: None, returned: false, interrupted: false };
        if end == RunEnd::Return && self.emu.at_fetch_boundary() {
            let registers = self.emu.cpu_registers();
            fine.last = Some((self.emu.opcode_fetches(), self.peek_code(full_pc_of(&registers))));
        }
        while self.emu.total_master_cycles() < deadline {
            let mut ran = false;
            let dispatched = match self.tick_event(input, &mut audio)? {
                TickEvent::None => false,
                TickEvent::Frame => {
                    run.rendered = true;
                    false
                }
                TickEvent::Hook(full_pc) => match end {
                    RunEnd::Arrival { key, s }
                        if hook_key(full_pc) == key && s == self.emu.cpu_registers().s =>
                    {
                        run.reached = true;
                        return Ok(());
                    }
                    RunEnd::Arrival { .. } => {
                        self.count_sync_inside_dispatch(full_pc);
                        run.rendered |= self.dispatch(full_pc, input, &mut *audio.0)?.rendered();
                        true
                    }
                    RunEnd::Return => {
                        self.count_sync_inside_dispatch(full_pc);
                        let nested = self.dispatch_nested(full_pc, input, &mut *audio.0)?;
                        run.rendered |= nested.rendered();
                        ran = nested.ran();
                        true
                    }
                },
            };
            let handling_interrupt = self.emu.is_handling_interrupt();
            self.marks.interrupted |= handling_interrupt;
            let s = self.emu.cpu_registers().s;
            run.stack_switched |=
                !dispatched && !handling_interrupt && s.abs_diff(previous_s) > MAX_STACK_MOVE;
            previous_s = s;
            self.marks.stack_low = self.marks.stack_low.min(s);
            if let Some(watch) = &mut self.stack_watch {
                if dispatched {
                    watch.dispatched(s);
                } else {
                    watch.ticked(s, handling_interrupt);
                }
            }
            // Checked after a dispatched call too, which can return for the routine (a tail
            // call).
            if end == RunEnd::Return {
                // Right after the instruction (or the nested call) that took S above its start,
                // before an interrupt entry: it must be a return.
                if s > s_start && !self.emu.is_mid_instruction() {
                    let by_opcode = fine.last.is_some_and(|(at, opcode)| {
                        self.emu.opcode_fetches() == at + 1 && opcode.is_some_and(is_return)
                    });
                    if ran || by_opcode {
                        run.reached = true;
                        return Ok(());
                    }
                }
                if self.emu.at_fetch_boundary() {
                    let registers = self.emu.cpu_registers();
                    fine.last =
                        Some((self.emu.opcode_fetches(), self.peek_code(full_pc_of(&registers))));
                }
            }
        }
        Ok(())
    }

    /// An arrival inside the original of a `Verify` call ([`RunEnd::Return`]): at the entry of
    /// another `Verify` routine, a nested `Verify` call, dispatched as usual (counted in
    /// `nested`) unless 16 are in progress (counted in `calls` and `nested_overflow`; the
    /// original resumes). At the entry of a `Verify` call in progress, the original resumes: as
    /// part of that call, counted in `calls` and `reentered` if S is below the S of its hit (a
    /// recursion, or an interrupt handler calling the routine it interrupted), uncounted
    /// otherwise (a loop back to the entry). Any other arrival resumes the original, uncounted:
    /// sync points (but see [`Machine::count_sync_inside_dispatch`]), routines in other modes and
    /// breakpoints. Returns what the dispatch did.
    #[cold]
    #[inline(never)]
    fn dispatch_nested(
        &mut self,
        full_pc: u32,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<Dispatch, OracleError> {
        let key = hook_key(full_pc);
        let entry = self.hooks.as_ref().and_then(|table| table.get(key));
        let is_verify_routine = entry.is_some_and(|entry| {
            entry.mode == HookMode::Verify && matches!(entry.action, HookAction::Routine { .. })
        });
        if !is_verify_routine {
            return Ok(Dispatch::Resumed);
        }
        if let Some(&(_, s_hit)) = self.verifying.iter().find(|&&(running, _)| running == key) {
            if self.emu.cpu_registers().s < s_hit {
                let stats = self.stats_mut(key);
                stats.calls += 1;
                stats.reentered += 1;
            }
            return Ok(Dispatch::Resumed);
        }
        let overflow = self.verifying.len() >= MAX_VERIFY_DEPTH;
        let stats = self.stats_mut(key);
        if overflow {
            stats.calls += 1;
            stats.nested_overflow += 1;
            return Ok(Dispatch::Resumed);
        }
        stats.nested += 1;
        self.dispatch(full_pc, input, audio)
    }

    /// Starts watching the stack ([`StackWatch`]) from the current S: after every tick and
    /// dispatch of [`Machine::run`] and every tick of the original code its dispatches run.
    pub(crate) fn watch_stack(&mut self) {
        self.stack_watch = Some(StackWatch::new(self.emu.cpu_registers().s));
    }

    /// The floor of the watched stack, if the stack is watched.
    pub(crate) fn stack_floor(&self) -> Option<u16> {
        self.stack_watch.map(StackWatch::floor)
    }

    /// Raises the floor of the watched stack (see [`StackWatch::raise_floor`]).
    pub(crate) fn raise_stack_floor(&mut self, floor: u16) {
        if let Some(watch) = &mut self.stack_watch {
            watch.raise_floor(floor);
        }
    }

    /// Updates the stack watch, if any, with the S a tick left (`dispatched`: a dispatch, whose
    /// own ticks were watched).
    fn watch_tick(&mut self, dispatched: bool) {
        if let Some(watch) = &mut self.stack_watch {
            let s = self.emu.cpu_registers().s;
            if dispatched {
                watch.dispatched(s);
            } else {
                watch.ticked(s, self.emu.is_handling_interrupt());
            }
        }
    }

    /// Master cycles in `frames` frames (262 lines of 1364 in NTSC, 312 in PAL).
    pub(crate) fn frames_mclk(&self, frames: u64) -> u64 {
        let lines = if self.target_fps() < 55.0 { 312 } else { 262 };
        frames * lines * MCLK_PER_LINE
    }

    /// Counts an arrival at `full_pc` inside a dispatch in the `sync_inside_dispatch` of its sync
    /// point if it would be a sync event at the top level (see [`Machine::take_sync_event`]): a
    /// sync point, with an NMI since the last event. It is no event: the dispatch runs on, and the
    /// event comes at the first arrival after it. At most one arrival counts per NMI (one event
    /// delayed): the first after it.
    #[cold]
    #[inline(never)]
    fn count_sync_inside_dispatch(&mut self, full_pc: u32) {
        let key = hook_key(full_pc);
        let Some(entry) = hooked_entry(self.hooks.as_ref(), key) else { return };
        let nmi_count = self.emu.nmi_count();
        if matches!(entry.action, HookAction::SyncPoint)
            && nmi_count > self.last_sync_nmi.max(self.last_inside_nmi)
        {
            self.last_inside_nmi = nmi_count;
            self.stats_mut(key).sync_inside_dispatch += 1;
        }
    }

    /// The sync event of an arrival at `full_pc` (see [`Machine::run_until_sync`]), counted in
    /// the sync point's calls; `None` if `full_pc` is no sync point or no NMI came since the last
    /// event (the arrival is then left to [`Machine::dispatch`]).
    pub(crate) fn take_sync_event(&mut self, full_pc: u32) -> Option<SyncEvent> {
        let key = hook_key(full_pc);
        let entry = hooked_entry(self.hooks.as_ref(), key)?;
        let nmi_count = self.emu.nmi_count();
        if !matches!(entry.action, HookAction::SyncPoint) || nmi_count <= self.last_sync_nmi {
            return None;
        }
        self.last_sync_nmi = nmi_count;
        self.stats.entry(key).or_default().calls += 1;
        Some(SyncEvent {
            key,
            frame: self.frames,
            nmi_count,
            master_cycles: self.emu.total_master_cycles(),
        })
    }

    /// Whether an arrival at `full_pc` would be a sync event now, or a hand-written routine hooks it in place
    /// of the generated code (see [`Ctx::sync_due`]).
    pub(crate) fn sync_due(&self, full_pc: u32) -> bool {
        hooked_entry(self.hooks.as_ref(), hook_key(full_pc)).is_some_and(|entry| {
            match entry.action {
                HookAction::SyncPoint => self.emu.nmi_count() > self.last_sync_nmi,
                // A routine that is not generated code (a stop of the generator that a wrapper hooks): generated
                // code calls its callees directly, so it hands over in front of it, where the hook is.
                HookAction::Routine { .. } => {
                    !entry.name.starts_with("rc_") && !entry.name.starts_with("rm_")
                }
            }
        })
    }

    /// The dispatch counters of `key`.
    pub(crate) fn stats_mut(&mut self, key: u32) -> &mut HookStats {
        self.stats.entry(key).or_default()
    }

    /// The frames of master cycles the original of a `Verify` call of the routine under `key`, or
    /// the original code that routine calls, may take (see [`HookEntry::budget_frames`]).
    pub(crate) fn budget_frames(&self, key: u32) -> u64 {
        self.hooks
            .as_ref()
            .and_then(|table| table.get(key))
            .map_or(DEFAULT_ROUTINE_BUDGET_FRAMES, HookEntry::budget)
    }

    /// The cost bound of the call of the routine under `key` that is about to start ([`BoundFn`]),
    /// from the machine as it is now.
    pub(crate) fn cost_bound(
        &mut self,
        key: u32,
        bound: BoundFn,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> u64 {
        bound(&Ctx::new(self, key, input, audio))
    }

    /// The name of the entry under `key`.
    pub(crate) fn hook_name(&self, key: u32) -> String {
        self.hooks
            .as_ref()
            .and_then(|table| table.get(key))
            .map_or_else(|| format!("${key:06X}"), |entry| entry.name.clone())
    }

    /// Sets the entry under `key` to [`HookMode::Original`] and installs the table again, so its
    /// arrivals are no longer reported (a hit of another entry stays pending). Its counters are
    /// kept.
    pub(crate) fn demote_hook(&mut self, key: u32) {
        if let Some(table) = &mut self.hooks {
            table.demote(key);
        }
        self.install_hooks();
    }

    pub(crate) fn divergence_dir(&self) -> Option<&Path> {
        self.divergence_dir.as_deref()
    }

    pub(crate) fn symbols(&self) -> Option<&Symbols> {
        self.symbols.as_deref()
    }

    /// Keeps a divergence and, given the machine at the call's entry and a divergence directory,
    /// saves its report there. The divergence is kept even if saving fails.
    pub(crate) fn record_divergence(
        &mut self,
        divergence: Divergence,
        entry_state: Option<&Machine>,
    ) -> std::io::Result<()> {
        let saved = match (&self.divergence_dir, entry_state) {
            (Some(dir), Some(entry_state)) => report::save(dir, &divergence, entry_state).map(drop),
            _ => Ok(()),
        };
        self.divergences.push(divergence);
        saved
    }

    /// Starts recording the CPU writes of a `Verify` call, whatever the user's log setting;
    /// returns where they start, for [`Machine::finish_cpu_write_capture`]. Captures nest: the
    /// writes of a nested call are also its caller's. The outermost one keeps the writes the
    /// user's log recorded so far for [`Machine::take_cpu_writes`] and turns the core's log on.
    pub(crate) fn start_cpu_write_capture(&mut self) -> usize {
        let recorded = self.emu.take_cpu_writes();
        if self.capture_depth == 0 {
            self.held_cpu_writes.extend(recorded);
            self.emu.set_cpu_write_log(true);
        } else {
            self.captured_writes.extend(recorded);
        }
        self.capture_depth += 1;
        self.captured_writes.len()
    }

    /// Ends the capture that started at `start`: returns the writes recorded since. The
    /// outermost one restores the user's setting, and the user's log, if enabled, keeps every
    /// write the captures recorded, as without hooks.
    pub(crate) fn finish_cpu_write_capture(&mut self, start: usize) -> Vec<(u32, u8)> {
        let recorded = self.emu.take_cpu_writes();
        self.captured_writes.extend(recorded);
        self.capture_depth -= 1;
        if self.capture_depth > 0 {
            return self.captured_writes[start..].to_vec();
        }
        debug_assert_eq!(start, 0, "the outermost capture starts the log");
        self.emu.set_cpu_write_log(self.cpu_write_log);
        let writes = std::mem::take(&mut self.captured_writes);
        if self.cpu_write_log {
            self.held_cpu_writes.extend_from_slice(&writes);
        }
        writes
    }

    /// One tick with the CPU held; a CPU cycle lasts 8 master cycles if `slow`, else 6. Returns
    /// whether it rendered a frame (counted in [`Machine::frame_count`]).
    fn hold_tick(
        &mut self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut AudioAdapter<'_>,
        slow: bool,
    ) -> Result<bool, OracleError> {
        let rendered = self
            .emu
            .hold_tick(
                &mut self.capture,
                audio,
                &mut FramePoller {
                    cache: &mut self.input_cache,
                    frame: self.frames,
                    lookup: input,
                },
                &mut SaveWriterAdapter(self.saves.as_mut()),
                slow,
            )
            .map_err(|e| OracleError::Run(e.to_string()))?;
        if rendered {
            self.end_frame();
        }
        Ok(rendered)
    }

    /// Ticks with the CPU held (6-master-cycle CPU cycles), one tick at a time, until `done`
    /// (checked before each tick); returns whether a frame was rendered.
    fn hold_until(
        &mut self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
        mut done: impl FnMut(&Self) -> bool,
    ) -> Result<bool, OracleError> {
        let mut audio = AudioAdapter(audio);
        let mut rendered = false;
        let deadline = self.emu.total_master_cycles().saturating_add(self.frames_mclk(4));
        while !done(self) {
            if self.emu.total_master_cycles() > deadline {
                return Err(OracleError::Run(
                    "the CPU held for four frames without reaching an instruction boundary"
                        .to_owned(),
                ));
            }
            rendered |= self.hold_tick(input, &mut audio, false)?;
        }
        Ok(rendered)
    }

    /// Holds the CPU for `fast` CPU cycles of 6 master cycles and `slow` ones of 8 (refresh and
    /// DMA ticks in between do not count): [`Machine::cpu_busy_cycles`] grows by exactly
    /// `6 * fast + 8 * slow`. Returns whether a frame was rendered.
    pub(crate) fn hold_cpu_cycles(
        &mut self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
        fast: u64,
        slow: u64,
    ) -> Result<bool, OracleError> {
        self.hold_cpu_cycles_to(input, audio, fast, slow, false).map(|(rendered, _)| rendered)
    }

    /// [`Machine::hold_cpu_cycles`] that, with `stop_at_frame`, stops at the tick that renders a
    /// frame if cycles remain to be held: returns whether a frame was rendered and the cycles
    /// left (`fast`, `slow`), `None` when the hold is complete.
    fn hold_cpu_cycles_to(
        &mut self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
        mut fast: u64,
        mut slow: u64,
        stop_at_frame: bool,
    ) -> Result<(bool, Option<(u64, u64)>), OracleError> {
        let mut audio = AudioAdapter(audio);
        let mut rendered = false;
        while fast + slow > 0 {
            let use_slow = slow > 0;
            let busy = self.emu.cpu_busy_cycles();
            let frame = self.hold_tick(input, &mut audio, use_slow)?;
            rendered |= frame;
            if self.emu.cpu_busy_cycles() != busy {
                if use_slow {
                    slow -= 1;
                } else {
                    fast -= 1;
                }
            }
            if frame && stop_at_frame && fast + slow > 0 {
                return Ok((true, Some((fast, slow))));
            }
        }
        Ok((rendered, None))
    }

    /// One tick of the charge left by a `Rust` call ([`PendingHold`]): the CPU held; when the
    /// charge is over, the interrupt lines are sampled and the arrival at the return address is
    /// detected, as at the end of the call. Returns the tick's event as [`Machine::tick_event`]
    /// does.
    #[inline(never)]
    fn pending_hold_tick(
        &mut self,
        mut hold: PendingHold,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut AudioAdapter<'_>,
    ) -> Result<TickEvent, OracleError> {
        let use_slow = hold.slow > 0;
        let busy = self.emu.cpu_busy_cycles();
        let rendered = self.hold_tick(input, audio, use_slow)?;
        if self.emu.cpu_busy_cycles() != busy {
            if use_slow {
                hold.slow -= 1;
            } else {
                hold.fast -= 1;
            }
        }
        if hold.fast + hold.slow == 0 {
            self.pending_hold = None;
            if hold.poll {
                self.emu.poll_cpu_interrupt_lines();
            }
            self.emu.detect_hook_hit();
        } else {
            self.pending_hold = Some(hold);
        }
        Ok(if rendered { TickEvent::Frame } else { TickEvent::None })
    }

    /// Samples the interrupt lines (see [`Ctx::poll_lines`]).
    pub(crate) fn poll_interrupt_lines(&mut self) {
        self.emu.poll_cpu_interrupt_lines();
    }

    /// Lets a memory refresh (or DMA step) that came due at the end of the held cycles happen
    /// now, as the original's next cycle would find it done: at most two held ticks. Returns
    /// whether a frame was rendered.
    pub(crate) fn settle(
        &mut self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<bool, OracleError> {
        let mut audio = AudioAdapter(audio);
        let mut rendered = false;
        let mut ticks = 0;
        while !self.emu.at_fetch_boundary() && ticks < 2 {
            rendered |= self.hold_tick(input, &mut audio, false)?;
            ticks += 1;
        }
        Ok(rendered)
    }

    /// If a GPDMA is requested, holds the CPU until it is over: `!gpdma_requested()` and
    /// `at_fetch_boundary()` (the channel flags clear one DMA tick before the DMA unit is idle).
    /// Returns whether a frame was rendered.
    pub(crate) fn drain_gpdma(
        &mut self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<bool, OracleError> {
        if !self.emu.gpdma_requested() {
            return Ok(false);
        }
        self.hold_until(input, audio, |m| !m.emu.gpdma_requested() && m.emu.at_fetch_boundary())
    }

    /// One unit of emulation. A pending hook hit is returned first, without ticking, if the CPU
    /// is still at that opcode fetch (else it is dropped): the CPU is then about to fetch the
    /// opcode at the hooked address. Otherwise the core ticks once (one CPU cycle, DMA step or
    /// memory refresh) and the result tells whether it rendered a frame. A tick that renders a
    /// frame and also hits returns [`TickEvent::Frame`]; the [`TickEvent::Hook`] comes on the next
    /// call. `tick` never dispatches: ticking on after a hit runs the original routine
    /// ([`Machine::run_frame`] dispatches).
    ///
    /// # Errors
    ///
    /// Returns [`OracleError::Run`] if the core fails (for example while persisting SRAM).
    pub fn tick(
        &mut self,
        input: Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<TickEvent, OracleError> {
        self.input_cache.clear();
        self.tick_event(&mut |_| input, &mut AudioAdapter(audio))
    }

    /// [`Machine::tick`] with the input lookup and audio adapter of the caller's loop (the
    /// caller clears the input cache when its lookup starts).
    /// Inlined: otherwise the per-tick call makes [`Machine::run`] about 8% slower.
    #[inline(always)]
    pub(crate) fn tick_event(
        &mut self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut AudioAdapter<'_>,
    ) -> Result<TickEvent, OracleError> {
        if let Some(hold) = self.pending_hold {
            return self.pending_hold_tick(hold, input, audio);
        }
        // A hit goes stale when the CPU was moved off its fetch (`set_cpu_registers`).
        if let Some(full_pc) = self.emu.take_hook_hit()
            && self.emu.at_fetch_boundary()
            && full_pc_of(&self.emu.cpu_registers()) == full_pc
        {
            self.last_hit = Some((full_pc, self.emu.opcode_fetches()));
            return Ok(TickEvent::Hook(full_pc));
        }
        Ok(if self.tick_with(input, audio)? { TickEvent::Frame } else { TickEvent::None })
    }

    /// A frame was rendered: takes its statistics ([`Machine::last_frame_stats`]) and counts it.
    /// Out of line, once per frame.
    #[cold]
    #[inline(never)]
    fn end_frame(&mut self) {
        let now = self.frame_counters();
        let base = self.frame_base;
        self.last_frame = Some(FrameStats {
            frame: self.frames,
            original_instructions: now.original.saturating_sub(base.original),
            rust_instructions_estimated: now.rust.saturating_sub(base.rust),
            master_cycles: now.master_cycles.saturating_sub(base.master_cycles),
            nmis: now.nmis.saturating_sub(base.nmis),
            dispatches: now.dispatches.saturating_sub(base.dispatches),
        });
        self.frame_base = now;
        self.frames += 1;
    }

    fn frame_counters(&self) -> FrameCounters {
        FrameCounters {
            original: self.emu.opcode_fetches(),
            rust: self.rust_instructions_estimated(),
            master_cycles: self.emu.total_master_cycles(),
            nmis: self.emu.nmi_count(),
            dispatches: self.dispatch_count,
        }
    }

    /// `Σ` [`HookStats::estimated_rust_instructions`] over the routines of the installed table.
    pub(crate) fn rust_instructions_estimated(&self) -> u64 {
        self.hooks
            .iter()
            .flat_map(HookTable::entries)
            .filter_map(|entry| self.stats.get(&entry.key))
            .map(HookStats::estimated_rust_instructions)
            .sum()
    }

    /// Whether a `Rust` call's charge is only partly held: the run that dispatched it stopped at
    /// the first frame rendered inside the charge, and the ticks that follow hold the CPU for the
    /// rest. The routine has run and returned (its writes are made and the CPU is at its return
    /// address), but the time it stands for has not all passed: what the original would still be
    /// writing is already written, so work RAM and the registers are the routine's, the rest of
    /// the machine (picture, timers, sound) the original's at this master cycle. See
    /// [`Machine::run_frame`].
    #[must_use]
    pub fn charge_pending(&self) -> bool {
        self.pending_hold.is_some()
    }

    /// Holds WRAM bytes at values: replaces the freezes with `freezes` (empty: none). Each is
    /// written again at the top level of [`Machine::run`] (so of [`Machine::run_frame`] and the
    /// runs built on it) right after the tick or the dispatch that rendered a frame, in order
    /// (the later of two overlapping freezes wins), before the NMI handler's frame task reads
    /// them: the game's own writes in between are undone at every frame boundary.
    ///
    /// A write in flight at the render tick (the CPU instruction that was running, a DMA) can
    /// land after the freeze, so a byte can hold another value for one frame. Nothing else
    /// applies freezes: not [`Machine::tick`], not the ticks inside a dispatch (`Verify` snapshots
    /// included), not [`Machine::advance_idle`]. The writes are made with the CPU-write log off,
    /// so [`Machine::take_cpu_writes`] shows only the game's. Freezes are not part of a save
    /// state ([`Machine::load_state`] keeps them) and are copied by [`Machine::snapshot`]. A
    /// machine with freezes cannot be recorded ([`Recorder`](crate::Recorder)); playing a replay
    /// on one is allowed, but the result is not reproducible across hook modes, since a dispatch
    /// that runs on past a render applies the freezes at another point of the frame than a
    /// machine without hooks.
    ///
    /// # Errors
    ///
    /// [`OracleError::Freeze`] if a freeze has no bytes or does not lie wholly in WRAM (its
    /// mirror: `$0000-$1FFF` of a low bank); nothing changes then.
    pub fn set_freezes(&mut self, freezes: Vec<Freeze>) -> Result<(), OracleError> {
        let mut canonical = Vec::with_capacity(freezes.len());
        for Freeze { address, bytes } in freezes {
            let end = bytes.len();
            let offset = wram_offset(address).filter(|&offset| match address >> 16 {
                0x7E | 0x7F => offset + end <= 0x2_0000,
                _ => offset + end <= 0x2000,
            });
            let Some(offset) = offset.filter(|_| end > 0) else {
                return Err(OracleError::Freeze(format!(
                    "{end} byte(s) at ${address:06X} do not lie in WRAM ($7E0000-$7FFFFF or the low \
                     RAM mirror $0000-$1FFF of banks $00-$3F and $80-$BF)"
                )));
            };
            canonical.push(Freeze { address: 0x7E_0000 + offset as u32, bytes });
        }
        self.freezes = canonical;
        Ok(())
    }

    /// The freezes set, with their canonical addresses (`$7E0000 + offset`).
    #[must_use]
    pub fn freezes(&self) -> &[Freeze] {
        &self.freezes
    }

    /// Writes the freezes, after a render at the top level of a run.
    #[inline(always)]
    fn apply_freezes(&mut self) {
        if !self.freezes.is_empty() {
            self.write_freezes();
        }
    }

    #[cold]
    #[inline(never)]
    fn write_freezes(&mut self) {
        let freezes = std::mem::take(&mut self.freezes);
        // The log's setting is the user's; these writes are not the game's.
        self.emu.set_cpu_write_log(false);
        for freeze in &freezes {
            for (address, &byte) in (freeze.address..).zip(&freeze.bytes) {
                self.emu.bus_write(address, byte);
            }
        }
        self.emu.set_cpu_write_log(self.cpu_write_log);
        self.freezes = freezes;
    }

    /// The statistics of the frame the machine rendered last, or `None` before its first render
    /// (and after [`Machine::load_state`]).
    #[must_use]
    pub fn last_frame_stats(&self) -> Option<FrameStats> {
        self.last_frame
    }

    /// One core tick; returns whether it rendered a frame (counted in [`Machine::frame_count`]).
    #[inline(always)]
    fn tick_with(
        &mut self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut AudioAdapter<'_>,
    ) -> Result<bool, OracleError> {
        #[cfg(feature = "histogram")]
        let before = (!self.histogram.is_empty()).then(|| {
            let r = self.emu.cpu_registers();
            (self.emu.opcode_fetches(), (u32::from(r.pbr) << 16 | u32::from(r.pc)) & 0x7F_FFFF)
        });
        let effect = self
            .emu
            .tick(
                &mut self.capture,
                audio,
                &mut FramePoller {
                    cache: &mut self.input_cache,
                    frame: self.frames,
                    lookup: input,
                },
                &mut SaveWriterAdapter(self.saves.as_mut()),
            )
            .map_err(|e| OracleError::Run(e.to_string()))?;
        #[cfg(feature = "histogram")]
        if let Some((fetches, pc)) = before
            && self.emu.opcode_fetches() != fetches
        {
            self.histogram[pc as usize] += 1;
        }
        let rendered = effect == TickEffect::FrameRendered;
        if rendered {
            self.end_frame();
        }
        Ok(rendered)
    }

    /// Starts counting the opcode fetches of the original code by address (23 bits).
    #[cfg(feature = "histogram")]
    pub fn enable_pc_histogram(&mut self) {
        self.histogram = vec![0; 1 << 23];
    }

    /// Declined arrivals by the bit length of the cycles to the next timing event (index 39: a
    /// GPDMA was requested).
    #[cfg(feature = "histogram")]
    #[must_use]
    pub fn until_declined(&self) -> &[u64; 40] {
        &self.until_declined
    }

    /// The opcode fetches counted since [`Machine::enable_pc_histogram`], by 23-bit address.
    #[cfg(feature = "histogram")]
    #[must_use]
    pub fn pc_histogram(&self) -> &[u64] {
        &self.histogram
    }

    /// Installs a hook table (`None` removes it). Arrivals at the opcode fetch of every entry not
    /// in [`HookMode::Original`] are reported by [`Machine::tick`]; a pending hit stays pending if
    /// the new table still hooks its address (or a breakpoint is there). The table stays
    /// installed across [`Machine::load_state`] and is copied by [`Machine::snapshot`]. Ticks of
    /// [`Machine::advance_idle`] hold the CPU and report nothing. The breakpoints stay.
    ///
    /// The dispatch counters of a key ([`Machine::hook_stats`]) are kept if the new table has
    /// the same routine under it (same name and action; mode, contract and costs may change),
    /// and reset otherwise.
    pub fn set_hook_table(&mut self, table: Option<HookTable>) {
        let (old, new) = (self.hooks.as_ref(), table.as_ref());
        self.stats.retain(|&key, _| {
            matches!(
                (old.and_then(|t| t.get(key)), new.and_then(|t| t.get(key))),
                (Some(old), Some(new)) if old.same_routine(new)
            )
        });
        self.hooks = table;
        self.install_hooks();
        self.frame_base.rust = self.rust_instructions_estimated();
    }

    /// The installed hook table, if any.
    #[must_use]
    pub fn hook_table(&self) -> Option<&HookTable> {
        self.hooks.as_ref()
    }

    /// Gives the core the bitmap of the current table with the breakpoints added (`None` when
    /// nothing is hooked, which keeps the core's no-hook path). With breakpoints, the bitmap is a
    /// copy (a megabyte): installs are rare.
    fn install_hooks(&mut self) {
        let mut bitmap = self.hooks.as_ref().and_then(HookTable::installed_bitmap);
        self.table_hooked = bitmap.is_some();
        if !self.breakpoints.is_empty() {
            let bitmap = Arc::make_mut(bitmap.get_or_insert_with(|| Arc::new(HookBitmap::new())));
            for &key in &self.breakpoints {
                bitmap.set(key);
            }
        }
        self.hooked = bitmap.is_some();
        self.emu.set_hooks(bitmap);
    }

    /// Sets a breakpoint at a 24-bit address (its hook key: a mirror stops there too): a run with
    /// [`Stops::breakpoint`] stops at every arrival there at its top level, before anything
    /// else ([`Stopped::Breakpoint`]), with the CPU about to fetch the opcode. Arrivals inside a
    /// dispatch never stop: a routine in `Rust` or `Verify` mode runs as one unit, so a
    /// breakpoint in its body is hit only when its original runs at the top level (in
    /// [`HookMode::Original`], or a call whose dispatch resumed the original: a timing
    /// fallback, a sampled-out `Verify` call).
    ///
    /// A breakpoint stop leaves the arrival pending: the next run or step goes on from there
    /// without stopping at the breakpoint again (it dispatches the routine there, reports the
    /// sync event there), as long as the CPU has not moved ([`Machine::load_state`] and
    /// [`Machine::set_cpu_registers`] end that). So do the debugger steps
    /// ([`Machine::step_into`]) for the instruction they start from and the one they end at, and
    /// setting a breakpoint where the CPU is about to fetch an opcode: the next run goes on from
    /// there without stopping.
    ///
    /// Breakpoints are the machine's, not the hook table's: they stay across
    /// [`Machine::set_hook_table`] and [`Machine::load_state`], [`Machine::snapshot`] copies them,
    /// and they never count in [`Machine::hook_stats`]. A breakpoint at a hooked entry in
    /// [`HookMode::Original`] does not make it a hooked entry. Runs without
    /// [`Stops::breakpoint`] pass them, cycle for cycle as without them.
    pub fn set_breakpoint(&mut self, address: u32) {
        if self.breakpoints.insert(hook_key(address)) {
            self.install_hooks();
            // The CPU may sit there, an arrival pending (a run stopped at an instruction count
            // or a frame's end): that one is not stopped at, or the next run would stop at once
            // with no progress.
            self.skip_breakpoint_here();
        }
    }

    /// Removes the breakpoint at a 24-bit address (its hook key), if there is one. An arrival a
    /// breakpoint stop left pending there is dropped unless an entry is hooked there.
    pub fn clear_breakpoint(&mut self, address: u32) {
        let key = hook_key(address);
        if self.breakpoints.remove(&key) {
            if self.skip_breakpoint.is_some_and(|(skip, _)| skip == key) {
                self.skip_breakpoint = None;
            }
            self.install_hooks();
        }
    }

    /// The breakpoints, by hook key.
    #[must_use]
    pub fn breakpoints(&self) -> &BTreeSet<u32> {
        &self.breakpoints
    }

    /// Lets the first top-level arrival go on without a breakpoint stop if the CPU is about to
    /// fetch an opcode at a breakpoint and does not move before it: the debugger steps start and
    /// end there (see [`Machine::set_breakpoint`]).
    pub(crate) fn skip_breakpoint_here(&mut self) {
        if !self.emu.at_fetch_boundary() {
            return;
        }
        let key = hook_key(full_pc_of(&self.emu.cpu_registers()));
        if self.breakpoints.contains(&key) {
            self.skip_breakpoint = Some((key, self.emu.opcode_fetches()));
        }
    }

    /// The byte at a 24-bit address, read without side effects: from the ROM image (`LoROM`
    /// mapping: `$8000-$FFFF` of every bank but `$7E-$7F`, all of `$40-$6F` and `$C0-$EF`) or
    /// WRAM (`$7E-$7F`, and `$0000-$1FFF` of banks `$00-$3F` and `$80-$BF`, the low-RAM mirror).
    /// `None` elsewhere: registers, SRAM, open bus. Unlike [`Machine::bus_read`] it changes
    /// nothing (no open-bus latch, no WRAM port step), so a view can show memory while the game
    /// runs. The map is the `LoROM` one whatever the cartridge (see [`Machine::peek_code`]).
    #[must_use]
    pub fn peek(&self, address: u32) -> Option<u8> {
        self.peek_code(address)
    }

    /// The byte at a 24-bit address, read without side effects, as [`Machine::peek`] gives it.
    /// For code: what an opcode fetch there would read.
    ///
    /// Assumes the `LoROM` map of [`Machine::peek`] whatever the cartridge is: a cartridge mapped
    /// otherwise (`HiROM`, a coprocessor) reads wrong bytes or `None`, and the steps that decode
    /// opcodes with it ([`Machine::step_over`], [`Machine::step_out`]) misjudge there. The game
    /// this crate is for is `LoROM`.
    pub(crate) fn peek_code(&self, address: u32) -> Option<u8> {
        let bank = (address >> 16) as u8;
        let offset = address as u16;
        match (bank, offset) {
            (0x7E..=0x7F, _) => {
                self.wram().get((usize::from(bank & 1) << 16) | usize::from(offset)).copied()
            }
            (0x00..=0x3F | 0x80..=0xBF, 0x0000..=0x1FFF) => {
                self.wram().get(usize::from(offset)).copied()
            }
            (0x40..=0x6F | 0xC0..=0xEF, _) | (_, 0x8000..=0xFFFF) => {
                // As the core maps it: A23 and A15 ignored, A16-A22 shifted down; the image is a
                // power of two (at least 32 KiB) long.
                let offset = ((address & 0x7F_0000) >> 1) | (address & 0x7FFF);
                self.rom.get(offset as usize & (self.rom.len() - 1)).copied()
            }
            _ => None,
        }
    }

    /// Dispatch counters of every entry of the installed table, in key order. They are kept
    /// while the same routine stays installed under its key (see [`Machine::set_hook_table`]),
    /// across [`Machine::load_state`]; [`Machine::snapshot`] copies them.
    #[must_use]
    pub fn hook_stats(&self) -> Vec<(String, HookStats)> {
        self.hooks
            .iter()
            .flat_map(HookTable::entries)
            .map(|entry| {
                (entry.name.clone(), self.stats.get(&entry.key).copied().unwrap_or_default())
            })
            .collect()
    }

    /// The counters of the routine installed under `key` (an address or a hook key); all zero when
    /// it has not run. [`Machine::hook_stats`] for one routine, with no copy of the others.
    #[must_use]
    pub fn hook_stat(&self, key: u32) -> HookStats {
        self.stats.get(&hook_key(key)).copied().unwrap_or_default()
    }

    /// The counters of the routines that have run (or have a measured mean), by hook key: what
    /// [`Machine::hook_stats`] holds without the names and the routines that never ran.
    pub fn hook_counters(&self) -> impl Iterator<Item = (u32, &HookStats)> {
        self.stats.iter().map(|(key, stats)| (*key, stats))
    }

    /// Sets [`HookStats::mean_instructions`] of the routines of the installed table, by name, from
    /// `means` (pairs of a routine name and its mean; the names that are not in the table are
    /// ignored): how a machine in `Rust` mode takes the means a `Verify` run of the same replay
    /// learned, for [`Machine::coverage`]. The mean stays with the routine as the other counters
    /// do (see [`Machine::set_hook_table`]); a `Verify` call on this machine updates it again.
    pub fn set_mean_instructions(&mut self, means: &[(String, f64)]) {
        let Some(table) = &self.hooks else { return };
        for entry in table.entries() {
            if let Some((_, mean)) = means.iter().find(|(name, _)| *name == entry.name) {
                self.stats.entry(entry.key).or_default().mean_instructions = *mean;
            }
        }
        self.frame_base.rust = self.rust_instructions_estimated();
    }

    /// Sets the directory for divergence reports (`None`: no reports): the first 32 divergences
    /// of each routine are saved there with [`report::save`]. While one is set, each checked
    /// `Verify` call of a routine with fewer than 32 divergences also keeps a snapshot of its
    /// entry state. The directory is created when the first report is saved; a report that
    /// cannot be saved is logged and counted in [`HookStats::report_errors`].
    pub fn set_divergence_dir(&mut self, dir: Option<PathBuf>) {
        self.divergence_dir = dir;
    }

    /// Sets the symbols whose `[[variable]]` entries name WRAM addresses in divergences (`None`:
    /// addresses only). [`Machine::snapshot`] shares them.
    pub fn set_symbols(&mut self, symbols: Option<Arc<Symbols>>) {
        self.symbols = symbols;
    }

    /// The `Verify` divergences kept, in order: the first 32 of each routine (the others are
    /// only counted in [`HookStats::divergences`]). Kept across [`Machine::load_state`]; a
    /// [`Machine::snapshot`] starts with none.
    #[must_use]
    pub fn divergences(&self) -> &[Divergence] {
        &self.divergences
    }

    /// The last frame the core rendered (black 256x224 before the first frame).
    ///
    /// # Panics
    ///
    /// Never in practice: the frame size is set at construction.
    #[must_use]
    pub fn frame(&self) -> Frame<'_> {
        Frame {
            pixels: &self.capture.pixels,
            size: self.capture.size.expect("frame size set at creation"),
            options: self.capture.options,
        }
    }

    #[must_use]
    pub fn frame_count(&self) -> u64 {
        self.frames
    }

    /// CRC32 of the ROM the machine runs (of the original ROM, for a machine on a pack set's image).
    #[must_use]
    pub fn rom_crc32(&self) -> u32 {
        self.rom_crc32
    }

    /// CRC32 of the image the machine runs: [`Machine::rom_crc32`] unless it runs a pack set's image
    /// ([`Machine::with_image`]).
    #[must_use]
    pub fn image_crc32(&self) -> u32 {
        self.image_crc32
    }

    /// Seed of the machine's power-on WRAM fill ([`Machine::with_wram_seed`]).
    #[must_use]
    pub fn wram_seed(&self) -> u64 {
        self.wram_seed
    }

    /// Whether the installed hook table has an entry not in [`HookMode::Original`]: only then
    /// can a run dispatch anything. Breakpoints do not count: they change no run but the ones
    /// that stop at them.
    pub(crate) fn is_hooked(&self) -> bool {
        self.table_hooked
    }

    /// Runs the system with the CPU held (DMA, refresh, PPU, APU and timers advance; the CPU
    /// executes nothing) until at least `master_cycles` have elapsed. Returns whether a frame was
    /// rendered; every rendered frame is counted in [`Machine::frame_count`].
    ///
    /// Held ticks report no hook hit and do not poll the interrupt lines. Before resuming a
    /// hooked opcode fetch after a hold, hold until [`Machine::at_fetch_boundary`] is true:
    /// otherwise the tick that completes a pending refresh or DMA reports the same arrival again.
    ///
    /// # Errors
    ///
    /// Returns [`OracleError::Run`] if the core fails (for example while persisting SRAM).
    pub fn advance_idle(
        &mut self,
        input: Joypad,
        audio: &mut dyn AudioSink,
        master_cycles: u64,
    ) -> Result<bool, OracleError> {
        // One held tick at a time, as the core's `advance_idle` ticks (fast CPU cycles), so every
        // tick polls the input with the frame count of its start.
        self.input_cache.clear();
        let target = self.emu.total_master_cycles().saturating_add(master_cycles);
        self.hold_until(&mut |_| input, audio, |m| m.emu.total_master_cycles() >= target)
    }

    #[must_use]
    pub fn cpu_registers(&self) -> Registers {
        self.emu.cpu_registers()
    }

    /// Sets the CPU registers. A pending hook hit goes stale if the CPU no longer is at its opcode
    /// fetch, and the next run stops at a breakpoint here again (see
    /// [`Machine::set_breakpoint`]).
    pub fn set_cpu_registers(&mut self, registers: Registers) {
        self.skip_breakpoint = None;
        self.emu.set_cpu_registers(registers);
    }

    /// Starts or stops logging the addresses the generated code reads through [`Ctx::read8`]
    /// (the original code's reads are not logged). `Verify` calls then keep the reads of the
    /// Rust function they ran, see [`Machine::take_check_reads`].
    pub fn log_reads(&mut self, on: bool) {
        self.log_reads = on;
        if !on {
            self.read_log.clear();
        }
    }

    /// The addresses (24-bit, in order, repeats included) the Rust function of the last `Verify`
    /// call read, nested calls included; empty if no call ran a Rust function or the log is off.
    pub fn take_check_reads(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.check_reads)
    }

    pub(crate) fn set_runaway_limit(&mut self, instructions: u64) {
        self.runaway_limit = instructions;
    }

    pub(crate) fn runaway_limit(&self) -> u64 {
        self.runaway_limit
    }

    pub(crate) fn note_read(&mut self, address: u32) {
        if self.log_reads && self.read_log.len() < (1 << 22) {
            self.read_log.push(address);
        }
    }

    pub(crate) fn take_read_log(&mut self) -> Vec<u32> {
        std::mem::take(&mut self.read_log)
    }

    pub(crate) fn set_check_reads(&mut self, reads: Vec<u32>) {
        if self.log_reads {
            self.check_reads = reads;
        }
    }

    /// Reads a 24-bit address as the CPU would, with the same side effects (read-to-clear
    /// registers, open bus, WRAM port increments).
    pub fn bus_read(&mut self, address: u32) -> u8 {
        self.emu.bus_read(address)
    }

    /// Writes a 24-bit address as the CPU would, immediately; recorded in the CPU-write log when
    /// it is enabled.
    pub fn bus_write(&mut self, address: u32, value: u8) {
        self.emu.bus_write(address, value);
    }

    /// Records every CPU write (CPU instructions and [`Machine::bus_write`], never DMA) while
    /// enabled. [`Machine::load_state`] and [`Machine::snapshot`] keep the setting and start with
    /// an empty log. `Verify` calls record their writes for themselves and leave this log as it
    /// would be without hooks.
    pub fn set_cpu_write_log(&mut self, enabled: bool) {
        self.cpu_write_log = enabled;
        self.emu.set_cpu_write_log(enabled);
    }

    /// CPU writes recorded since the last call, in program order, as `(address, value)`.
    pub fn take_cpu_writes(&mut self) -> Vec<(u32, u8)> {
        let recorded = self.emu.take_cpu_writes();
        if self.held_cpu_writes.is_empty() {
            return recorded;
        }
        let mut writes = std::mem::take(&mut self.held_cpu_writes);
        writes.extend(recorded);
        writes
    }

    /// Writes `bytes` into work RAM at the bus address `address` (`$7E:0000` onwards), between two
    /// frames: the host's write, not the game's (it is not logged as a CPU write). A replay records
    /// it as a host event ([`crate::HostEvent`]).
    ///
    /// # Errors
    ///
    /// [`OracleError::WramWrite`] if the bytes do not lie in work RAM (`$7E:0000` to `$7E:FFFF`).
    pub fn write_wram(&mut self, address: u32, bytes: &[u8]) -> Result<(), OracleError> {
        let end = u64::from(address) + bytes.len() as u64;
        if !(0x7E_0000..=0x7F_FFFF).contains(&address) || end > 0x80_0000 {
            return Err(OracleError::WramWrite(format!(
                "{} bytes at {address:06X} are not in work RAM",
                bytes.len()
            )));
        }
        self.emu.set_cpu_write_log(false);
        for (address, &byte) in (address..).zip(bytes) {
            self.emu.bus_write(address, byte);
        }
        self.emu.set_cpu_write_log(self.cpu_write_log);
        Ok(())
    }

    /// Writes `bytes` into the cartridge's SRAM from `offset` (`$70:0000` onwards on the bus), between two frames: the host's
    /// write (the save editor's), not the game's. The next [`Machine::flush_saves`] stores the SRAM. Not for a replay: a replay
    /// starts from the SRAM it recorded.
    ///
    /// # Errors
    ///
    /// The cartridge has no SRAM, or the bytes do not fit it.
    pub fn write_sram(&mut self, offset: usize, bytes: &[u8]) -> Result<(), String> {
        let size = self.emu.peek_sram().map_or(0, <[u8]>::len);
        if offset + bytes.len() > size {
            return Err(format!(
                "{} bytes at {offset:04X} do not fit the {size} bytes of SRAM",
                bytes.len()
            ));
        }
        self.emu.set_cpu_write_log(false);
        for (address, &byte) in (0x70_0000_u32 + offset as u32..).zip(bytes) {
            self.emu.bus_write(address, byte);
        }
        self.emu.set_cpu_write_log(self.cpu_write_log);
        Ok(())
    }

    /// Work RAM, 128 KiB (`$7E:0000` is index 0).
    #[must_use]
    pub fn wram(&self) -> &[u8] {
        self.emu.peek_wram()
    }

    /// Cartridge SRAM, if the cartridge has any.
    #[must_use]
    pub fn sram(&self) -> Option<&[u8]> {
        self.emu.peek_sram()
    }

    /// Video RAM, 32 Ki words.
    #[must_use]
    pub fn vram(&self) -> &[u16] {
        self.emu.peek_vram()
    }

    /// Palette RAM, 256 words.
    #[must_use]
    pub fn cgram(&self) -> &[u16] {
        self.emu.peek_cgram()
    }

    /// Sprite attribute RAM as (low table 512 bytes, high table 32 bytes).
    #[must_use]
    pub fn oam(&self) -> (&[u8], &[u8]) {
        self.emu.peek_oam()
    }

    /// Turns the PPU's log of what each frame's render read on or off ([`Machine::frame_log`]).
    /// Output only: the machine runs, saves and compares identically either way, and the setting
    /// survives a state load. A frame is logged from its start, so the log of the frame in
    /// progress is incomplete.
    pub fn set_line_log(&mut self, on: bool) {
        self.emu.set_ppu_line_log(on);
    }

    /// Whether the PPU's render log is on ([`Machine::set_line_log`]).
    #[must_use]
    pub fn line_log_enabled(&self) -> bool {
        self.emu.ppu_line_log_enabled()
    }

    /// Turns the drawing of the PPU's own pixels on or off ([`Machine::frame`]). Everything the
    /// game can observe is the same either way (the CPU, the APU, DMA, the sprite flags, the
    /// counters, the save state apart from the frame buffer), but with it off the frame is not
    /// drawn in the 256-pixel progressive modes: set it only for frames that are shown by
    /// drawing them again from the render log ([`Machine::frame_log`]) or not at all, and between
    /// frames. On by default, and it survives a state load.
    pub fn set_compose_pixels(&mut self, on: bool) {
        self.emu.set_ppu_compose_pixels(on);
        // (the frame is not copied for [`Machine::frame`] either while the PPU leaves it undrawn)
        self.capture.skip_pixels = !on;
    }

    /// Turns the resampling of the audio on or off. With it off the sound chip runs as ever but
    /// no sample reaches the [`AudioSink`]: for frames whose sound is thrown away. On by default,
    /// and it survives a state load.
    pub fn set_audio_resampling(&mut self, on: bool) {
        self.emu.set_audio_resampling(on);
    }

    /// What the PPU read to render the last frame it completed: per scanline registers, the
    /// CGRAM writes, and VRAM and OAM as they were when the display ended. `None` with the log
    /// off, or before a frame was logged from its start. Read it right after the run that
    /// rendered the frame ([`Stops::frame_end`]): the next frame replaces it.
    #[must_use]
    pub fn frame_log(&self) -> Option<&FrameLog> {
        self.emu.ppu_frame_log()
    }

    /// Master cycles since power-on (part of the save state).
    #[must_use]
    pub fn total_master_cycles(&self) -> u64 {
        self.emu.total_master_cycles()
    }

    /// Master cycles spent on CPU cycles (everything but DMA and memory refresh; cycles held by
    /// [`Machine::advance_idle`] included). A counter outside the save state: use differences.
    #[must_use]
    pub fn cpu_busy_cycles(&self) -> u64 {
        self.emu.cpu_busy_cycles()
    }

    /// Instructions started (opcode fetches). A counter outside the save state: use differences.
    #[must_use]
    pub fn opcode_fetches(&self) -> u64 {
        self.emu.opcode_fetches()
    }

    /// Whether the CPU is at an opcode fetch (no refresh, DMA or interrupt entry pending). When
    /// true, the next tick that does not hold the CPU (a [`Machine::run_frame`] tick, not
    /// [`Machine::advance_idle`]) is the opcode fetch; DMA that becomes pending on that tick
    /// starts on a later one.
    #[must_use]
    pub fn at_fetch_boundary(&self) -> bool {
        self.emu.at_fetch_boundary()
    }

    /// Whether the CPU is between two cycles of an instruction or of an interrupt entry.
    #[must_use]
    pub fn is_mid_instruction(&self) -> bool {
        self.emu.is_mid_instruction()
    }

    /// Whether the CPU is in an NMI or IRQ entry sequence (pushing the return state and loading
    /// the vector).
    #[must_use]
    pub fn is_handling_interrupt(&self) -> bool {
        self.emu.is_handling_interrupt()
    }

    /// Whether HDMA is pending, reloading or transferring. The reload check at the start of
    /// every frame makes it true briefly even when no HDMA channel is enabled.
    #[must_use]
    pub fn hdma_busy(&self) -> bool {
        self.emu.hdma_busy()
    }

    /// HDMA copy units performed: one per active channel on each scanline that transfers data.
    /// A counter outside the save state: use differences.
    #[must_use]
    pub fn hdma_copy_units(&self) -> u64 {
        self.emu.hdma_copy_units()
    }

    /// NMIs the CPU has taken since power-on (one per NMI entry, nested ones included). A
    /// counter outside the save state ([`Machine::load_state`] keeps it, [`Machine::snapshot`]
    /// copies it): use differences.
    #[must_use]
    pub fn nmi_count(&self) -> u64 {
        self.emu.nmi_count()
    }

    /// Master cycles until the next timing event the running code does not start itself: the
    /// vblank NMI edge (if NMI is enabled), the next H/V IRQ (if enabled and the CPU's I flag is
    /// clear) and HDMA taking the bus (the frame-start reload and the per-line transfers, if a
    /// channel is enabled); 0 if an NMI or IRQ is pending or HDMA is busy or due on the next
    /// tick, `u64::MAX` if none can come. None of them comes sooner. Read-only.
    #[must_use]
    pub fn master_cycles_until_timing_event(&self) -> u64 {
        self.emu.master_cycles_until_timing_event()
    }

    /// Whether bytes of a GPDMA requested through `$420B` remain to be copied. The channel flags
    /// clear one DMA tick before the DMA unit is idle: with the CPU held at an opcode fetch, the
    /// transfer is over once this is false and [`Machine::at_fetch_boundary`] is true again.
    #[must_use]
    pub fn gpdma_requested(&self) -> bool {
        self.emu.gpdma_requested()
    }

    /// Persists SRAM now if it changed since the last write. The core only checks every 30
    /// frames, so call this before exiting or the last half second of saves can be lost.
    ///
    /// # Errors
    ///
    /// Returns [`OracleError::Io`] if the save store cannot persist the data.
    pub fn flush_saves(&mut self) -> Result<(), OracleError> {
        self.emu.flush_sram(&mut SaveWriterAdapter(self.saves.as_mut()))?;
        Ok(())
    }

    /// Runnable copy of the machine: running both with the same inputs keeps them identical.
    ///
    /// The ROM is shared, the core's internal frame buffer starts blank (the last captured frame
    /// is copied, so [`Machine::frame`] matches), and saves go to a fresh in-memory store so the
    /// copy never touches the original's save files. The hook table, its counters, the
    /// breakpoints, the divergence directory, the symbols and the NMI count of the last sync
    /// event are copied; the list of divergences starts empty, and so does the CPU-write log (its
    /// setting is kept).
    ///
    /// A snapshot taken while the CPU waits at the entry of a hooked routine (paused at a
    /// breakpoint or after a step onto it, the arrival pending) keeps that arrival: the core's
    /// snapshot drops it, so the machine, which knows whether it took the arrival, reports it
    /// again on the copy (`detect_hook_hit`).
    #[must_use]
    pub fn snapshot(&self) -> Self {
        let mut copy = self.copy_with_capture(self.capture.clone());
        // An arrival the original has not taken yet (a breakpoint stop, a step onto an entry)
        // is pending in the copy too.
        if self.arrival_pending() {
            copy.emu.detect_hook_hit();
        }
        copy
    }

    /// For tools that look into a recompiled routine: the machine after the Rust function of the
    /// routine hooked at the entry the CPU is at ran on a copy of this machine, as a `Verify`
    /// call runs it (no timing window), with what it reported.
    ///
    /// # Errors
    ///
    /// No routine is hooked at the CPU's address, or the copy's core fails.
    pub fn run_rust_unchecked(
        &self,
        input: &mut dyn FnMut(u64) -> Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<(Self, RustRun), OracleError> {
        let mut snap = self.verify_snapshot();
        let key = hook_key(full_pc_of(&snap.emu.cpu_registers()));
        let Some(HookAction::Routine { func, ret }) =
            hooked_entry(snap.hooks.as_ref(), key).map(|entry| entry.action)
        else {
            return Err(OracleError::Run(format!("no routine is hooked at ${key:06X}")));
        };
        let finished = snap.run_routine(key, func, ret, input, audio)?;
        let run = RustRun {
            instructions: finished.instructions,
            resume: finished.resume,
            cost: finished.cost,
            runaway: finished.runaway,
            dropped: finished.dropped,
        };
        Ok((snap, run))
    }

    /// Whether the CPU is at an opcode fetch of a hooked address whose arrival has not been
    /// taken: the next run dispatches it (or stops at its breakpoint).
    fn arrival_pending(&self) -> bool {
        if !self.hooked || self.pending_hold.is_some() || !self.emu.at_fetch_boundary() {
            return false;
        }
        let full_pc = full_pc_of(&self.emu.cpu_registers());
        self.last_hit != Some((full_pc, self.emu.opcode_fetches()))
    }

    /// Reports the arrival at the opcode the CPU is about to fetch, if that address is hooked or
    /// a breakpoint: for a state made by setting the registers ([`Machine::set_cpu_registers`])
    /// to a routine's entry, whose next run then dispatches it.
    pub fn arm_arrival(&mut self) {
        self.last_hit = None;
        self.emu.detect_hook_hit();
    }

    /// [`Machine::snapshot`] for `Verify`, without copying the frame capture: nothing reads
    /// the copy's [`Machine::frame`] (it is empty until the copy renders one). On the copy,
    /// [`Machine::is_verify_snapshot`] is true.
    pub(crate) fn verify_snapshot(&self) -> Self {
        let capture = FrameCapture {
            pixels: Vec::new(),
            size: self.capture.size,
            options: self.capture.options,
            skip_pixels: self.capture.skip_pixels,
        };
        Self { is_verify_snapshot: true, ..self.copy_with_capture(capture) }
    }

    /// Whether this is the snapshot a `Verify` call runs its Rust function on: there, the
    /// original code the function calls dispatches `Verify` hooks as in `Rust` mode.
    pub(crate) fn is_verify_snapshot(&self) -> bool {
        self.is_verify_snapshot
    }

    fn copy_with_capture(&self, capture: FrameCapture) -> Self {
        Self {
            emu: self.emu.snapshot(),
            saves: Box::new(MemorySaveStore::default()),
            capture,
            frames: self.frames,
            input_cache: InputCache::default(),
            rom_crc32: self.rom_crc32,
            image_crc32: self.image_crc32,
            rom: Arc::clone(&self.rom),
            wram_seed: self.wram_seed,
            audio_hz: self.audio_hz,
            hooks: self.hooks.clone(),
            hooked: self.hooked,
            table_hooked: self.table_hooked,
            breakpoints: self.breakpoints.clone(),
            skip_breakpoint: None,
            stats: self.stats.clone(),
            divergence_dir: self.divergence_dir.clone(),
            divergences: Vec::new(),
            symbols: self.symbols.clone(),
            cpu_write_log: self.cpu_write_log,
            held_cpu_writes: Vec::new(),
            capture_depth: 0,
            captured_writes: Vec::new(),
            last_sync_nmi: self.last_sync_nmi,
            last_inside_nmi: self.last_inside_nmi,
            is_verify_snapshot: false,
            marks: RunMarks::NONE,
            in_flight: self.in_flight.clone(),
            verifying: Vec::new(),
            stack_watch: None,
            rust_instructions_run: self.rust_instructions_run,
            #[cfg(feature = "histogram")]
            histogram: Vec::new(),
            #[cfg(feature = "histogram")]
            until_declined: [0; 40],
            dispatch_count: self.dispatch_count,
            frame_base: self.frame_base,
            last_frame: self.last_frame,
            freezes: self.freezes.clone(),
            last_hit: self.last_hit,
            pending_hold: self.pending_hold,
            may_defer_hold: false,
            log_reads: self.log_reads,
            runaway_limit: DEFAULT_RUNAWAY_LIMIT,
            read_log: Vec::new(),
            check_reads: Vec::new(),
        }
    }

    #[must_use]
    pub fn target_fps(&self) -> f64 {
        self.emu.target_fps()
    }

    pub fn set_audio_frequency(&mut self, hz: u64) {
        self.audio_hz = Some(hz);
        self.emu.update_audio_output_frequency(hz);
    }

    /// # Panics
    ///
    /// Never in practice: encoding an in-memory state into a `Vec` cannot fail.
    #[must_use]
    pub fn save_state(&self) -> Vec<u8> {
        let mut payload = bincode::encode_to_vec(self.emu.to_save_state(), state::STATE_CONFIG)
            .expect("encoding an in-memory state cannot fail");
        // The charge a `Rust` call has left to hold is not the core's: a state made in the
        // middle of one carries it after the core's, and a state without one is unchanged.
        if let Some(hold) = self.pending_hold {
            payload.extend_from_slice(PENDING_HOLD_MAGIC);
            payload.extend_from_slice(&hold.fast.to_le_bytes());
            payload.extend_from_slice(&hold.slow.to_le_bytes());
            payload.push(u8::from(hold.poll));
        }
        state::wrap(SnesEmulator::save_state_version(), self.image_crc32, self.frames, &payload)
    }

    /// Validates everything before touching the running machine: on error it is left unchanged.
    ///
    /// # Errors
    ///
    /// Returns [`OracleError::State`] if the bytes are not a valid state for this ROM and core.
    pub fn load_state(&mut self, bytes: &[u8]) -> Result<(), OracleError> {
        let (header, payload) =
            state::unwrap(bytes, SnesEmulator::save_state_version(), self.image_crc32)
                .map_err(OracleError::State)?;
        let (decoded, used): (SnesEmulator, usize) =
            bincode::decode_from_slice(payload, state::STATE_CONFIG)
                .map_err(|e| OracleError::State(e.to_string()))?;
        let pending_hold = parse_pending_hold(&payload[used..])?;
        self.emu.load_state(decoded);
        self.pending_hold = pending_hold;
        self.frames = header.frames;
        // As in the core, writes recorded before the load belong to the discarded timeline, and
        // so does the stack a lockstep run was watching.
        self.held_cpu_writes.clear();
        self.stack_watch = None;
        // The NMI count is carried over; whether the loaded state's last NMI had its sync event
        // is unknown, so the next event follows the next NMI.
        self.last_sync_nmi = self.emu.nmi_count();
        // Hooks are not part of the state: the current table and breakpoints apply to the loaded
        // one, and an arrival at a hooked opcode fetch (or a breakpoint) in the loaded state is
        // reported by the next tick, a breakpoint stopping there first thing.
        self.skip_breakpoint = None;
        self.install_hooks();
        // The counters carry on; the first frame after a load has no statistics of the old ones.
        self.frame_base = self.frame_counters();
        self.last_frame = None;
        // The state carries the audio rate it was saved at (resampler ratio); keep the host's rate.
        if let Some(hz) = self.audio_hz {
            self.emu.update_audio_output_frequency(hz);
        }
        // The frame capture is not part of the state: refresh it so `frame()` matches the new state.
        let Ok(()) = self.emu.force_render(&mut self.capture);
        Ok(())
    }
}

/// Marks the charge a save state carries after the core's state ([`PendingHold`]).
const PENDING_HOLD_MAGIC: &[u8; 4] = b"HOLD";

/// The charge a save state carries after the core's state, `None` if it has none.
fn parse_pending_hold(rest: &[u8]) -> Result<Option<PendingHold>, OracleError> {
    if rest.is_empty() {
        return Ok(None);
    }
    let Some(body) = rest.strip_prefix(PENDING_HOLD_MAGIC.as_slice()).filter(|b| b.len() == 17)
    else {
        return Err(OracleError::State("trailing bytes in save state payload".into()));
    };
    let word = |at: usize| u64::from_le_bytes(body[at..at + 8].try_into().expect("8 bytes"));
    Ok(Some(PendingHold { fast: word(0), slow: word(8), poll: body[16] != 0 }))
}

/// The 24-bit `PBR << 16 | PC`.
fn full_pc_of(registers: &Registers) -> u32 {
    (u32::from(registers.pbr) << 16) | u32::from(registers.pc)
}

/// The WRAM offset a 24-bit address maps to: `$7E0000-$7FFFFF`, or the low RAM mirror of
/// `$7E0000-$7E1FFF` (`$0000-$1FFF` of banks `$00-$3F` and `$80-$BF`).
fn wram_offset(address: u32) -> Option<usize> {
    let (bank, offset) = (address >> 16, (address & 0xFFFF) as usize);
    match bank {
        0x7E | 0x7F => Some((usize::from(bank == 0x7F) << 16) | offset),
        0x00..=0x3F | 0x80..=0xBF if offset < 0x2000 => Some(offset),
        _ => None,
    }
}

/// The entry of `table` under `key` if it is hooked (not in [`HookMode::Original`]): an arrival
/// at an entry in `Original` mode (at a breakpoint there) is no arrival at the entry.
fn hooked_entry(table: Option<&HookTable>, key: u32) -> Option<&HookEntry> {
    table?.get(key).filter(|entry| entry.mode != HookMode::Original)
}

/// Whether `opcode` is a return: `RTS`, `RTL` or `RTI`.
fn is_return(opcode: u8) -> bool {
    matches!(opcode, 0x60 | 0x6B | 0x40)
}

/// The ROM image as the core maps it: a 512-byte copier header stripped, then mirrored to the
/// next power of two.
fn rom_image(rom: &[u8]) -> Arc<[u8]> {
    let mut image = if rom.len() & 0x7FFF == 0x0200 { rom[0x200..].to_vec() } else { rom.to_vec() };
    jgenesis_common::rom::mirror_to_next_power_of_two(&mut image);
    image.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interrupt_entry_cancels_a_return_not_yet_stopped_at() {
        // A dispatched routine ran (returned) and an interrupt entry follows before the boundary:
        // the boundary is the handler's first instruction, no return led there.
        let mut fine = Fine { fetches_start: 0, last: None, returned: true, interrupted: false };
        fine.interrupt_entry(false);
        assert!(fine.returned && !fine.interrupted);
        fine.interrupt_entry(true);
        assert!(!fine.returned && fine.interrupted);
        // The entry takes several ticks; the flags stay.
        fine.interrupt_entry(true);
        fine.interrupt_entry(false);
        assert!(!fine.returned && fine.interrupted);
    }

    /// A 32 KiB `LoROM` whose reset vector runs `STP`: a machine that does nothing.
    fn idle_rom() -> Vec<u8> {
        let mut rom = vec![0xFF_u8; 0x8000];
        rom[0] = 0xDB; // STP
        rom[0x7FC0..0x7FD5].copy_from_slice(b"SSD UNIT TEST        ");
        rom[0x7FD5] = 0x20; // LoROM, SlowROM
        rom[0x7FD7] = 0x05; // 32 KiB
        rom[0x7FD9] = 0x01; // North America -> NTSC
        rom[0x7FFC..0x7FFE].copy_from_slice(&0x8000_u16.to_le_bytes());
        ssd_rom::fix_lorom_checksum(&mut rom);
        rom
    }

    #[test]
    fn a_loaded_state_drops_the_stack_watch_of_the_discarded_timeline() {
        let mut machine = Machine::new(idle_rom(), Box::new(MemorySaveStore::default())).unwrap();
        assert_eq!(machine.stack_floor(), None);
        machine.watch_stack();
        assert!(machine.stack_floor().is_some());

        let state = machine.save_state();
        machine.load_state(&state).unwrap();
        assert_eq!(machine.stack_floor(), None);
    }

    #[test]
    fn code_peeks_read_rom_and_wram_as_the_bus_does() {
        let mut rom = idle_rom();
        for (i, byte) in rom.iter_mut().enumerate().take(0x7FC0).skip(1) {
            *byte = (i * 7 + i / 256) as u8;
        }
        ssd_rom::fix_lorom_checksum(&mut rom);
        let mut machine = Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap();
        for (i, address) in
            [0x7E_0010, 0x7E_1FF0, 0x7F_FFFF, 0x00_0010, 0x80_1FFF].into_iter().enumerate()
        {
            machine.bus_write(address, 0xA0 + i as u8);
        }
        let mapped = [
            0x00_8000, 0x00_8123, 0x00_FFFF, 0x80_8123, 0x01_8123, 0x3F_C000, 0x40_0123, 0x6F_F123,
            0xC0_1234, 0xFF_8123, 0x7E_0010, 0x7E_1FF0, 0x7F_FFFF, 0x00_0010, 0x80_1FFF, 0x3F_1FF0,
        ];
        for address in mapped {
            let peeked = machine.peek_code(address);
            assert_eq!(peeked, Some(machine.bus_read(address)), "${address:06X}");
        }
        // Registers, SRAM and expansion areas: no code to read without side effects.
        for address in [0x00_2100, 0x00_4210, 0x80_6000, 0x70_0000, 0xF0_7FFF] {
            assert_eq!(machine.peek_code(address), None, "${address:06X}");
        }
    }
}
