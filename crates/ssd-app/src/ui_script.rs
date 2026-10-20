//! The script of a UI snapshot (`--ui-script`): one action per line, applied in order before the
//! frame is drawn. Each line is an action the window's controls and panel make, or a target a
//! panel field takes. Blank lines and lines starting with `#` are skipped.
//!
//! ```text
//! panel open|close
//! tab routines|memory|debugger|replays
//! pause
//! step into|over|out
//! frames <n>
//! select <symbol or address>
//! breakpoint <symbol or address>
//! mode <routine> original|rust|verify
//! menu open
//! menu key up|down|left|right|a|b|l|r|y|select
//! menu show field|menu|items|cheats|options
//! menu battle-cheats
//! menu press <key>|esc|backspace|pad <button>
//! menu tab all|allies|tools
//! touch menu|options|edit|hidden|play|minimal|full|stick|dpad|gamepad|nogamepad|select <group>|press <buttons>|release
//! ```
//!
//! The `menu` lines are for `--mode enhanced`, and play the HD menu as a player does: `menu open`
//! presses A on the field, `menu key` gives the HD a key (`a` confirms, `b` cancels), and the
//! presses of the choice it starts run to their end before the next line. `menu show` checks that
//! the HD shows that screen (`field` is the command window, `menu` the Menu list, `items` the item
//! list).

use crate::debugger::StepKind;
use crate::panel::Tab;
use ssd_menus::Key;
use ssd_menus::options::controls::{self, Capture, Device};
use ssd_menus::view::Tab as BagTab;
use ssd_oracle::HookMode;
use std::fmt;

/// One action of a script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Opens or closes the panel (nothing when it is so already).
    Panel { open: bool },
    /// Opens the panel on a tab.
    Tab(Tab),
    /// Pauses the game (nothing when it is paused already).
    Pause,
    /// A step of the debugger, as its button makes it (while paused).
    Step(StepKind),
    /// Runs this many frames, as a window does while it runs: a breakpoint ends them early.
    Frames(u64),
    /// Selects a symbol or an address in the Memory tab, as its "Go to" field does.
    Select(String),
    /// Adds a breakpoint at a routine or an address, as the Debugger's field does.
    Breakpoint(String),
    /// Sets the mode of one routine, as its row in the Routines tab does.
    Mode { routine: String, mode: HookMode },
    /// A line of the HD menu (Enhanced mode).
    Menu(MenuCommand),
    /// A line of the touch controls (`--touch-ui`): see [`crate::touch::Touch::script`].
    Touch(String),
}

/// What a `menu` line does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuCommand {
    /// The player presses A on the field: the menu opens.
    Open,
    /// A navigation key of the HD menu.
    Key(Key),
    /// Checks that the HD shows this screen.
    Show(MenuShow),
    /// The bag's Item screen shows this tab.
    Tab(BagTab),
    /// A raw key (or, with `pad`, button) press while the Controls page waits for one.
    Press(Capture),
    /// The Cheats screen over the battle the game is in (F3, the touch MENU's Cheats).
    BattleCheats,
}

/// The HD screen a `menu show` line asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuShow {
    /// The command window of the field (or of a town).
    Field,
    /// The Menu list.
    Menu,
    /// The item list.
    Items,
    /// The cheats screen.
    Cheats,
    /// The Options screen.
    Options,
}

/// A line of a script, with its number (from 1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub number: usize,
    pub command: Command,
}

/// A line that is not an action of the script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ScriptError {}

/// Parses a whole script.
///
/// # Errors
///
/// The first line that is not an action, with the reason.
pub fn parse(text: &str) -> Result<Vec<Line>, ScriptError> {
    let mut lines = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let number = index + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let command =
            parse_command(trimmed).map_err(|message| ScriptError { line: number, message })?;
        lines.push(Line { number, command });
    }
    Ok(lines)
}

/// The action of one line, which is not blank nor a comment.
fn parse_command(text: &str) -> Result<Command, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let Some((&action, args)) = words.split_first() else {
        return Err("empty line".to_owned());
    };
    match (action, args) {
        ("panel", ["open"]) => Ok(Command::Panel { open: true }),
        ("panel", ["close"]) => Ok(Command::Panel { open: false }),
        ("panel", _) => Err("panel takes open or close".to_owned()),
        ("tab", [name]) => tab_named(name).map(Command::Tab),
        ("tab", _) => Err("tab takes one of routines, memory, debugger, replays".to_owned()),
        ("pause", []) => Ok(Command::Pause),
        ("pause", _) => Err("pause takes no argument".to_owned()),
        ("step", [name]) => step_named(name).map(Command::Step),
        ("step", _) => Err("step takes one of into, over, out".to_owned()),
        ("frames", [count]) => count
            .parse::<u64>()
            .map(Command::Frames)
            .map_err(|_| format!("frames takes a number of frames, not `{count}`")),
        ("frames", _) => Err("frames takes one number of frames".to_owned()),
        ("select", [target]) => Ok(Command::Select((*target).to_owned())),
        ("breakpoint", [target]) => Ok(Command::Breakpoint((*target).to_owned())),
        ("select" | "breakpoint", _) => Err(format!("{action} takes one symbol or address")),
        ("mode", [routine, mode]) => {
            Ok(Command::Mode { routine: (*routine).to_owned(), mode: mode_named(mode)? })
        }
        ("mode", _) => Err("mode takes a routine and one of original, rust, verify".to_owned()),
        ("menu", ["open"]) => Ok(Command::Menu(MenuCommand::Open)),
        ("menu", ["battle-cheats"]) => Ok(Command::Menu(MenuCommand::BattleCheats)),
        ("menu", ["key", key]) => {
            menu_key_named(key).map(|key| Command::Menu(MenuCommand::Key(key)))
        }
        ("menu", ["show", screen]) => {
            menu_screen_named(screen).map(|screen| Command::Menu(MenuCommand::Show(screen)))
        }
        ("menu", ["tab", tab]) => {
            menu_tab_named(tab).map(|tab| Command::Menu(MenuCommand::Tab(tab)))
        }
        ("menu", ["press", "pad", name @ ..]) if !name.is_empty() => {
            menu_press_named(Device::Gamepad, &name.join(" "))
                .map(|capture| Command::Menu(MenuCommand::Press(capture)))
        }
        ("menu", ["press", name]) => menu_press_named(Device::Keyboard, name)
            .map(|capture| Command::Menu(MenuCommand::Press(capture))),
        ("menu", _) => Err(MENU_USAGE.to_owned()),
        ("touch", rest) if !rest.is_empty() => Ok(Command::Touch(rest.join(" "))),
        ("touch", _) => Err("touch takes a word, such as menu, edit or press up,a".to_owned()),
        _ => Err(format!(
            "unknown action `{action}`: expected panel, tab, pause, step, frames, select, \
             breakpoint, mode, menu or touch"
        )),
    }
}

/// What a `menu` line takes, for the error message.
const MENU_USAGE: &str = "menu takes open, key up|down|left|right|a|b, show field|menu|items|cheats, press <key>, or tab all|allies|tools";

fn menu_tab_named(name: &str) -> Result<BagTab, String> {
    match name {
        "all" => Ok(BagTab::All),
        "allies" => Ok(BagTab::Allies),
        "tools" => Ok(BagTab::Tools),
        _ => Err(format!("unknown tab `{name}`: expected all, allies or tools")),
    }
}

fn menu_key_named(name: &str) -> Result<Key, String> {
    match name {
        "up" => Ok(Key::Up),
        "down" => Ok(Key::Down),
        "left" => Ok(Key::Left),
        "right" => Ok(Key::Right),
        "a" => Ok(Key::Confirm),
        "b" => Ok(Key::Cancel),
        "l" => Ok(Key::TabBack),
        "r" => Ok(Key::Tab),
        "y" => Ok(Key::ResetPage),
        "select" => Ok(Key::ResetAll),
        _ => Err(format!(
            "unknown menu key `{name}`: expected up, down, left, right, a, b, l, r, y or select"
        )),
    }
}

/// The press a `menu press` line gives: Esc and Backspace are the page's own, the rest a key or button by its name.
fn menu_press_named(device: Device, name: &str) -> Result<Capture, String> {
    if device == Device::Keyboard {
        match name.to_ascii_lowercase().as_str() {
            "esc" | "escape" => return Ok(Capture::Cancel),
            "backspace" => return Ok(Capture::Clear),
            _ => {}
        }
    }
    controls::code_named(device, name)
        .map(Capture::Code)
        .ok_or_else(|| format!("unknown key or button `{name}`"))
}

fn menu_screen_named(name: &str) -> Result<MenuShow, String> {
    match name {
        "field" => Ok(MenuShow::Field),
        "menu" => Ok(MenuShow::Menu),
        "items" => Ok(MenuShow::Items),
        "cheats" => Ok(MenuShow::Cheats),
        "options" => Ok(MenuShow::Options),
        _ => Err(format!(
            "unknown menu screen `{name}`: expected field, menu, items, cheats or options"
        )),
    }
}

fn tab_named(name: &str) -> Result<Tab, String> {
    match name {
        "routines" => Ok(Tab::Routines),
        "memory" => Ok(Tab::Memory),
        "debugger" => Ok(Tab::Debugger),
        "replays" => Ok(Tab::Replays),
        _ => Err(format!("unknown tab `{name}`: expected routines, memory, debugger or replays")),
    }
}

fn step_named(name: &str) -> Result<StepKind, String> {
    match name {
        "into" => Ok(StepKind::Into),
        "over" => Ok(StepKind::Over),
        "out" => Ok(StepKind::Out),
        _ => Err(format!("unknown step `{name}`: expected into, over or out")),
    }
}

fn mode_named(name: &str) -> Result<HookMode, String> {
    match name {
        "original" => Ok(HookMode::Original),
        "rust" => Ok(HookMode::Rust),
        "verify" => Ok(HookMode::Verify),
        _ => Err(format!("unknown mode `{name}`: expected original, rust or verify")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn command(text: &str) -> Command {
        let lines = parse(text).unwrap();
        assert_eq!(lines.len(), 1, "one action in {text:?}");
        lines[0].command.clone()
    }

    fn error(text: &str) -> ScriptError {
        parse(text).expect_err("a bad script")
    }

    #[test]
    fn a_menu_press_line_gives_a_key_a_button_or_esc_and_backspace() {
        assert_eq!(command("menu press z"), Command::Menu(MenuCommand::Press(Capture::Code(29))));
        assert_eq!(command("menu press Esc"), Command::Menu(MenuCommand::Press(Capture::Cancel)));
        assert_eq!(
            command("menu press backspace"),
            Command::Menu(MenuCommand::Press(Capture::Clear))
        );
        assert_eq!(command("menu press f5"), Command::Menu(MenuCommand::Press(Capture::Code(62))));
        assert_eq!(
            command("menu press pad D-pad Up"),
            Command::Menu(MenuCommand::Press(Capture::Code(12)))
        );
        assert!(error("menu press nonsense").message.contains("unknown key or button"));
    }

    #[test]
    fn every_action_parses_to_its_command() {
        assert_eq!(command("panel open"), Command::Panel { open: true });
        assert_eq!(command("panel close"), Command::Panel { open: false });
        assert_eq!(command("tab memory"), Command::Tab(Tab::Memory));
        assert_eq!(command("tab replays"), Command::Tab(Tab::Replays));
        assert_eq!(command("pause"), Command::Pause);
        assert_eq!(command("step over"), Command::Step(StepKind::Over));
        assert_eq!(command("frames 240"), Command::Frames(240));
        assert_eq!(command("select side_record_a"), Command::Select("side_record_a".to_owned()));
        assert_eq!(
            command("breakpoint get_active_side_record"),
            Command::Breakpoint("get_active_side_record".to_owned())
        );
        assert_eq!(
            command("mode rng_next_a verify"),
            Command::Mode { routine: "rng_next_a".to_owned(), mode: HookMode::Verify }
        );
        assert_eq!(command("menu show field"), Command::Menu(MenuCommand::Show(MenuShow::Field)));
        assert_eq!(command("menu show items"), Command::Menu(MenuCommand::Show(MenuShow::Items)));
        assert_eq!(command("menu open"), Command::Menu(MenuCommand::Open));
        assert_eq!(command("menu key a"), Command::Menu(MenuCommand::Key(Key::Confirm)));
        assert_eq!(command("menu key b"), Command::Menu(MenuCommand::Key(Key::Cancel)));
        assert_eq!(command("menu key y"), Command::Menu(MenuCommand::Key(Key::ResetPage)));
        assert_eq!(command("menu key select"), Command::Menu(MenuCommand::Key(Key::ResetAll)));
        assert_eq!(command("menu key l"), Command::Menu(MenuCommand::Key(Key::TabBack)));
        assert_eq!(command("menu key r"), Command::Menu(MenuCommand::Key(Key::Tab)));
        assert_eq!(
            command("menu show options"),
            Command::Menu(MenuCommand::Show(MenuShow::Options))
        );
    }

    #[test]
    fn a_bad_menu_line_says_what_it_takes() {
        assert!(error("menu").message.contains("menu takes open"));
        assert!(error("menu show submenu").message.contains("unknown menu screen `submenu`"));
        assert_eq!(command("menu show cheats"), Command::Menu(MenuCommand::Show(MenuShow::Cheats)));
        assert_eq!(command("menu battle-cheats"), Command::Menu(MenuCommand::BattleCheats));
        assert!(error("menu key x").message.contains("unknown menu key `x`"));
    }

    #[test]
    fn spacing_is_free_and_addresses_are_kept_as_written() {
        assert_eq!(command("   tab    routines  "), Command::Tab(Tab::Routines));
        assert_eq!(command("select 7E:1400"), Command::Select("7E:1400".to_owned()));
        assert_eq!(command("breakpoint $00C0F0"), Command::Breakpoint("$00C0F0".to_owned()));
    }

    #[test]
    fn blank_lines_and_comments_are_skipped_but_counted() {
        let lines = parse("# the battle\n\n   \npanel open\n# done\ntab debugger\n").unwrap();
        let numbers: Vec<usize> = lines.iter().map(|line| line.number).collect();
        assert_eq!(numbers, [4, 6]);
        assert_eq!(lines[1].command, Command::Tab(Tab::Debugger));
        assert_eq!(parse("").unwrap(), Vec::<Line>::new());
    }

    #[test]
    fn an_unknown_action_is_an_error_on_its_line() {
        let error = error("panel open\n\nfly to the moon\n");
        assert_eq!(error.line, 3);
        assert!(error.message.contains("unknown action `fly`"), "{error}");
        assert!(error.to_string().starts_with("line 3: "), "{error}");
    }

    #[test]
    fn wrong_arguments_are_errors_that_say_what_is_expected() {
        assert!(error("panel").message.contains("open or close"));
        assert!(error("panel shut").message.contains("open or close"));
        assert!(error("tab inventory").message.contains("unknown tab `inventory`"));
        assert!(error("tab").message.contains("one of routines"));
        assert!(error("pause now").message.contains("no argument"));
        assert!(error("step sideways").message.contains("unknown step `sideways`"));
        assert!(error("step").message.contains("into, over, out"));
        assert!(error("frames many").message.contains("not `many`"));
        assert!(error("frames -3").message.contains("not `-3`"));
        assert!(error("frames 3 4").message.contains("one number"));
        assert!(error("select").message.contains("one symbol or address"));
        assert!(error("breakpoint a b").message.contains("one symbol or address"));
        assert!(error("mode rng_next_a").message.contains("routine and one of"));
        assert!(error("mode rng_next_a fast").message.contains("unknown mode `fast`"));
    }

    #[test]
    fn the_first_bad_line_is_the_one_reported() {
        let error = error("tab memory\nframes x\nfly\n");
        assert_eq!(error.line, 2);
    }
}
