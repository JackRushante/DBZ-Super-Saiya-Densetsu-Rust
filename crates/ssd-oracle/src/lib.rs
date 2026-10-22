//! Safe wrapper over the vendored jgenesis SNES core (spec §5.1 `ssd-oracle`).

mod compare;
mod coverage;
mod ctx;
mod debugger;
mod hooks;
pub mod input;
mod lockstep;
mod machine;
mod replay;
pub mod report;
pub mod repro;
mod savestore;
mod sinks;
mod state;
mod symbols;
mod sync;
mod timing;
mod verify;

pub use compare::{Difference, Divergence, Outcome, compare};
pub use coverage::{Coverage, FrameStats};
pub use ctx::Ctx;
pub use debugger::{StepOptions, Stepped};
pub use hooks::{
    BoundFn, DEFAULT_ROUTINE_BUDGET_FRAMES, HookAction, HookEntry, HookMode, HookStats, HookTable,
    ReturnKind, RoutineFn, hook_key,
};
pub use input::{joypad_bits, joypad_from_bits};
pub use jgenesis_common::frontend::{Color, FrameSize, RenderFrameOptions};
pub use lockstep::{
    DEFAULT_BUDGET_FRAMES, LockstepConfig, LockstepDivergence, LockstepReport, LockstepRun,
    LockstepStep, Verdict, run_lockstep,
};
pub use machine::{
    DEFAULT_WRAM_SEED, Frame, Freeze, Joypad, Machine, OracleError, RustRun, Stopped, Stops,
    TickEvent,
};
pub use replay::{
    CORE_VERSION, HostEvent, MODE_CLASSIC, MODE_ENHANCED, ORACLE_REVISION, PackTag, Recorder,
    Replay, apply_events,
};
pub use savestore::{FsSaveStore, MemorySaveStore, SaveStore};
pub use sinks::{AudioSink, NullAudio};
pub use snes_core::linelog::{
    CgramWrite, FrameLog, LineEntry, LineRegs, OamWrite, SpriteLine, VramWrite,
};
pub use symbols::{Routine, Symbols, SymbolsError, SyncPoint, Table, Timing, Variable};
pub use sync::SyncEvent;
pub use timing::call_fits_window;
pub use wdc65816_emu::core::{Registers, StatusRegister};
