//! The debugger's state and logic, without drawing (`tab_debugger.rs` draws it): the stop banner,
//! the breakpoints (by symbol or address), the registers and flags as text, and the recent steps.
//! The machine itself is changed by `app.rs`, which reads what these functions decide.

use ssd_oracle::{HookMode, Registers, Stepped, Symbols, hook_key};
use std::collections::VecDeque;

/// Lines kept in the recent-steps log.
pub const LOG_LINES: usize = 8;

/// The flags of the status register, in the order the panel shows them (bit 7 down to bit 0).
pub const FLAG_NAMES: [char; 8] = ['N', 'V', 'M', 'X', 'D', 'I', 'Z', 'C'];

/// Which step a button asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepKind {
    Into,
    Over,
    Out,
}

impl StepKind {
    /// The steps in the controls bar's order.
    pub const ALL: [StepKind; 3] = [StepKind::Into, StepKind::Over, StepKind::Out];

    /// The name the controls bar shows.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            StepKind::Into => "Into",
            StepKind::Over => "Over",
            StepKind::Out => "Out",
        }
    }
}

/// The stop banner: where the run stopped, and what the next step does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Banner {
    pub label: String,
    pub title: String,
    pub detail: String,
}

/// One line of the recent-steps log: a kind (`step`, `routine`, `stop`, ...) and its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogLine {
    pub kind: &'static str,
    pub text: String,
}

/// A breakpoint of the list: its hook key (the `PBR:PC` address), its name, whether it is set on
/// the machine, and how many times it stopped the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Breakpoint {
    pub key: u32,
    pub name: String,
    pub enabled: bool,
    pub hits: u64,
}

/// What the debugger tab shows and remembers.
#[derive(Debug, Default)]
pub struct DebugState {
    /// Whether a step over or out ends at an NMI or IRQ handler ([`ssd_oracle::StepOptions`]).
    pub stop_at_interrupts: bool,
    /// The text of the field the breakpoints are added from: a symbol or an address.
    pub target: String,
    pub breakpoints: Vec<Breakpoint>,
    pub banner: Option<Banner>,
    pub log: VecDeque<LogLine>,
    /// Why the last action did nothing (an unknown symbol, a breakpoint already set), if it did.
    pub notice: Option<String>,
}

impl DebugState {
    /// Adds an enabled breakpoint at `key`, named `name`; an error if one is set there already.
    pub fn add(&mut self, key: u32, name: String) -> Result<(), String> {
        if self.breakpoints.iter().any(|b| b.key == key) {
            return Err(format!("there is a breakpoint at {name} already"));
        }
        self.push_log("break", format!("set at {name}"));
        self.breakpoints.push(Breakpoint { key, name, enabled: true, hits: 0 });
        Ok(())
    }

    /// Flips breakpoint `index` between enabled and disabled; returns its key and the new state.
    pub fn toggle(&mut self, index: usize) -> Option<(u32, bool)> {
        let breakpoint = self.breakpoints.get_mut(index)?;
        breakpoint.enabled = !breakpoint.enabled;
        Some((breakpoint.key, breakpoint.enabled))
    }

    /// Removes breakpoint `index`; returns its key and whether it was enabled.
    pub fn remove(&mut self, index: usize) -> Option<(u32, bool)> {
        if index >= self.breakpoints.len() {
            return None;
        }
        let breakpoint = self.breakpoints.remove(index);
        self.push_log("break", format!("removed {}", breakpoint.name));
        Some((breakpoint.key, breakpoint.enabled))
    }

    /// The keys of the breakpoints that are enabled.
    #[must_use]
    pub fn enabled_keys(&self) -> Vec<u32> {
        self.breakpoints.iter().filter(|b| b.enabled).map(|b| b.key).collect()
    }

    /// A breakpoint at `key` stopped the run. `mode` is the mode of the routine there, if the machine
    /// dispatches it (`None` for original code): a routine in Rust, or verified, runs as one step.
    pub fn stopped_at_breakpoint(&mut self, key: u32, name: &str, mode: Option<HookMode>) {
        if let Some(breakpoint) = self.breakpoints.iter_mut().find(|b| b.key == key) {
            breakpoint.hits += 1;
        }
        let detail = match mode {
            Some(HookMode::Rust | HookMode::Patch) => {
                "Next step runs it in Rust as one step (a Rust routine cannot stop half-way)."
            }
            Some(HookMode::Verify) => {
                "Next step verifies it as one step: the original runs, then the Rust function is \
                 checked against it."
            }
            Some(HookMode::Original) | None => {
                "Next step runs the original code one instruction at a time."
            }
        };
        self.banner = Some(Banner {
            label: "Stopped before".to_owned(),
            title: name.to_owned(),
            detail: detail.to_owned(),
        });
        self.push_log("stop", format!("breakpoint {name}"));
    }

    /// A step ended as `stepped` says; `name_of` names a key (for the breakpoint it may stop at).
    pub fn stepped(&mut self, stepped: &Stepped, name_of: impl Fn(u32) -> String) {
        let (kind, banner) = describe_step(stepped, name_of);
        self.push_log(kind, format!("{} {}", banner.title, banner.detail).trim_end().to_owned());
        self.banner = Some(banner);
    }

    /// Appends a line to the recent-steps log, dropping the oldest beyond [`LOG_LINES`].
    pub fn push_log(&mut self, kind: &'static str, text: impl Into<String>) {
        if self.log.len() == LOG_LINES {
            self.log.pop_front();
        }
        self.log.push_back(LogLine { kind, text: text.into() });
    }

    /// Takes the stop banner away (the run goes on).
    pub fn clear_banner(&mut self) {
        self.banner = None;
    }
}

/// The address a target text names: a routine or sync point by name, else an address (hex, with
/// an optional `$`, of one to six digits, the bank included: `$008040`, `8040`).
///
/// # Errors
///
/// An empty text, or one that is neither a name in `symbols` nor an address.
pub fn parse_target(text: &str, symbols: &Symbols) -> Result<u32, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("type a routine name or an address".to_owned());
    }
    if let Some(routine) = symbols.routines().iter().find(|r| r.name == text) {
        return Ok(hook_key(routine.address));
    }
    if let Some(point) = symbols.sync_points().iter().find(|p| p.name == text) {
        return Ok(hook_key(point.address));
    }
    let digits = text.strip_prefix('$').unwrap_or(text);
    let hex = (1..=6).contains(&digits.len()) && digits.bytes().all(|b| b.is_ascii_hexdigit());
    if hex && let Ok(address) = u32::from_str_radix(digits, 16) {
        return Ok(hook_key(address));
    }
    Err(format!("no routine or address named `{text}`"))
}

/// The name of the routine or sync point at `key`, else its address (`$008040`).
#[must_use]
pub fn name_of(symbols: &Symbols, key: u32) -> String {
    let key = hook_key(key);
    if let Some(routine) = symbols.routine_for_key(key) {
        return routine.name.clone();
    }
    if let Some(point) = symbols.sync_points().iter().find(|p| hook_key(p.address) == key) {
        return point.name.clone();
    }
    format!("${key:06X}")
}

/// The registers as the panel lists them: name and value, in the panel's order.
#[must_use]
pub fn register_rows(registers: &Registers) -> Vec<(&'static str, String)> {
    vec![
        ("A", format!("{:04X}", registers.a)),
        ("X", format!("{:04X}", registers.x)),
        ("Y", format!("{:04X}", registers.y)),
        ("S", format!("{:04X}", registers.s)),
        ("D", format!("{:04X}", registers.d)),
        ("DBR", format!("{:02X}", registers.dbr)),
        ("PBR", format!("{:02X}", registers.pbr)),
        ("PC", format!("{:04X}", registers.pc)),
        ("E", u8::from(registers.emulation_mode).to_string()),
    ]
}

/// The flags of the status register, in [`FLAG_NAMES`]' order, each with whether it is set.
#[must_use]
pub fn flag_bits(registers: &Registers) -> [(char, bool); 8] {
    let p = u8::from(registers.p);
    std::array::from_fn(|i| (FLAG_NAMES[i], p & (0x80 >> i) != 0))
}

/// The kind of log line and the banner a step that ended as `stepped` shows.
#[must_use]
pub fn describe_step(stepped: &Stepped, name_of: impl Fn(u32) -> String) -> (&'static str, Banner) {
    let banner = |label: &str, title: &str, detail: &str| Banner {
        label: label.to_owned(),
        title: title.to_owned(),
        detail: detail.to_owned(),
    };
    match stepped {
        Stepped::Instruction => {
            ("step", banner("Stepped", "one instruction", "The CPU is at the next one."))
        }
        Stepped::Routine { name } => (
            "routine",
            banner(
                "Ran in Rust",
                name,
                "The routine ran as one step; the CPU is where it left it.",
            ),
        ),
        Stepped::Returned => (
            "return",
            banner(
                "Returned",
                "to the caller",
                "The CPU is at the instruction the return goes to.",
            ),
        ),
        Stepped::Interrupt => (
            "interrupt",
            banner(
                "Interrupt",
                "NMI or IRQ handler entered",
                "The CPU is at its first instruction.",
            ),
        ),
        Stepped::Breakpoint { key } => {
            ("stop", banner("Stopped before", &name_of(*key), "A breakpoint."))
        }
        Stepped::NoReturn => (
            "stop",
            banner("Did not return", "within the step's frames", "The CPU is where it got to."),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::sram_counter_machine;
    use std::path::Path;

    const SYMBOLS: &str = r#"
[[routine]]
name = "leaf"
address = 0x008040
returns = "rts"
timing = "pure"
mode = "rust"
cycles = 80

[[sync_point]]
name = "main_loop_idle"
address = 0x00C0F0
"#;

    fn symbols() -> Symbols {
        Symbols::parse(SYMBOLS, Path::new("symbols.toml")).unwrap()
    }

    fn registers() -> Registers {
        sram_counter_machine().0.cpu_registers()
    }

    #[test]
    fn a_target_is_a_symbol_or_an_address() {
        let symbols = symbols();
        assert_eq!(parse_target("leaf", &symbols), Ok(0x008040));
        assert_eq!(parse_target("  main_loop_idle ", &symbols), Ok(0x00C0F0));
        assert_eq!(parse_target("$008040", &symbols), Ok(0x008040));
        assert_eq!(parse_target("8040", &symbols), Ok(0x008040));
        assert_eq!(parse_target("$C0F0", &symbols), Ok(0x00C0F0));
        assert_eq!(parse_target("$05B6B2", &symbols), Ok(0x05B6B2));
        assert!(parse_target("", &symbols).is_err());
        assert!(parse_target("nothing_like_this", &symbols).unwrap_err().contains("nothing"));
        assert!(parse_target("$1234567", &symbols).is_err(), "seven digits");
        assert!(parse_target("$", &symbols).is_err());
    }

    #[test]
    fn names_come_from_the_symbols_or_the_address() {
        let symbols = symbols();
        assert_eq!(name_of(&symbols, 0x008040), "leaf");
        assert_eq!(name_of(&symbols, 0x80_8040), "leaf", "the bank's mirror bit is dropped");
        assert_eq!(name_of(&symbols, 0x00C0F0), "main_loop_idle");
        assert_eq!(name_of(&symbols, 0x00D000), "$00D000");
    }

    #[test]
    fn breakpoints_are_added_toggled_and_removed_by_position() {
        let mut state = DebugState::default();
        state.add(0x008040, "leaf".to_owned()).unwrap();
        assert!(state.add(0x008040, "leaf".to_owned()).unwrap_err().contains("leaf"));
        state.add(0x00C0F0, "main_loop_idle".to_owned()).unwrap();
        assert_eq!(state.enabled_keys(), [0x008040, 0x00C0F0]);

        assert_eq!(state.toggle(1), Some((0x00C0F0, false)));
        assert_eq!(state.enabled_keys(), [0x008040]);
        assert_eq!(state.toggle(9), None);
        assert_eq!(state.remove(0), Some((0x008040, true)));
        assert_eq!(state.remove(0), Some((0x00C0F0, false)), "a disabled one has no machine bit");
        assert_eq!(state.remove(0), None);
        assert_eq!(state.breakpoints.len(), 0);
    }

    #[test]
    fn a_breakpoint_stop_counts_a_hit_and_says_what_comes_next() {
        let mut state = DebugState::default();
        state.add(0x008040, "leaf".to_owned()).unwrap();
        state.stopped_at_breakpoint(0x008040, "leaf", Some(HookMode::Rust));
        assert_eq!(state.breakpoints[0].hits, 1);
        let banner = state.banner.clone().unwrap();
        assert_eq!((banner.label.as_str(), banner.title.as_str()), ("Stopped before", "leaf"));
        assert!(banner.detail.contains("in Rust as one step"), "{banner:?}");
        state.stopped_at_breakpoint(0x008040, "leaf", Some(HookMode::Verify));
        let detail = state.banner.clone().unwrap().detail;
        assert!(detail.contains("verifies it") && !detail.contains("in Rust"), "{detail}");
        state.stopped_at_breakpoint(0x00C0F0, "main_loop_idle", None);
        assert!(state.banner.clone().unwrap().detail.contains("one instruction at a time"));
        state.stopped_at_breakpoint(0x00C0F0, "main_loop_idle", Some(HookMode::Original));
        assert!(state.banner.clone().unwrap().detail.contains("one instruction at a time"));
        state.clear_banner();
        assert_eq!(state.banner, None);
    }

    #[test]
    fn the_log_keeps_the_last_lines_only() {
        let mut state = DebugState::default();
        for n in 0..10 {
            state.push_log("step", format!("line {n}"));
        }
        assert_eq!(state.log.len(), LOG_LINES);
        assert_eq!(state.log.front().map(|l| l.text.as_str()), Some("line 2"));
        assert_eq!(state.log.back().map(|l| l.text.as_str()), Some("line 9"));
    }

    #[test]
    fn steps_describe_themselves() {
        let name = |key: u32| format!("routine at {key:06X}");
        let (kind, banner) = describe_step(&Stepped::Instruction, name);
        assert_eq!((kind, banner.label.as_str()), ("step", "Stepped"));
        let (kind, banner) = describe_step(&Stepped::Routine { name: "leaf".to_owned() }, name);
        assert_eq!((kind, banner.title.as_str()), ("routine", "leaf"));
        let (kind, _) = describe_step(&Stepped::Interrupt, name);
        assert_eq!(kind, "interrupt");
        let (kind, banner) = describe_step(&Stepped::Breakpoint { key: 0x008040 }, name);
        assert_eq!((kind, banner.title.as_str()), ("stop", "routine at 008040"));
        let (kind, banner) = describe_step(&Stepped::NoReturn, name);
        assert_eq!((kind, banner.label.as_str()), ("stop", "Did not return"));

        let mut state = DebugState::default();
        state.stepped(&Stepped::Returned, name);
        assert_eq!(state.log.back().map(|l| l.kind), Some("return"));
        assert_eq!(state.banner.map(|b| b.label), Some("Returned".to_owned()));
    }

    #[test]
    fn registers_and_flags_are_written_as_the_panel_shows_them() {
        let mut regs = registers();
        regs.a = 0x12AB;
        regs.x = 0x0004;
        regs.s = 0x01FF;
        regs.pbr = 0x05;
        regs.pc = 0xB6B2;
        regs.emulation_mode = false;
        let rows = register_rows(&regs);
        assert_eq!(rows[0], ("A", "12AB".to_owned()));
        assert_eq!(rows[3], ("S", "01FF".to_owned()));
        assert_eq!(rows[6], ("PBR", "05".to_owned()));
        assert_eq!(rows[7], ("PC", "B6B2".to_owned()));
        assert_eq!(rows[8], ("E", "0".to_owned()));

        // N and C set: bits 7 and 0 of P.
        regs.p = ssd_oracle::StatusRegister::from(0b1000_0001);
        let flags = flag_bits(&regs);
        assert_eq!(flags.map(|(name, _)| name), FLAG_NAMES);
        assert_eq!(
            flags.map(|(_, set)| set),
            [true, false, false, false, false, false, false, true]
        );
    }

    #[test]
    fn the_step_buttons_are_named_as_the_bar_shows_them() {
        let labels: Vec<&str> = StepKind::ALL.iter().map(|k| k.label()).collect();
        assert_eq!(labels, ["Into", "Over", "Out"]);
    }
}
