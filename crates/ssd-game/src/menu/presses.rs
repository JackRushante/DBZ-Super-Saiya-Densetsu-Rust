//! The presses of the HD menus: a plan is the original menu's own input, one button at a time, and
//! after each press the menu's state is checked against what the press should have done. The
//! original performs every consequence (its messages, its effects, its saves); the plan only says
//! which buttons to press, and when to stop.
//!
//! A press holds its button for two frames and releases it on the third, as a player does. The step
//! after it waits, within a budget of frames, for the state it expects. A step that does not get its
//! state aborts the plan: the menu is then closed with B presses, so the field is playable again.
//! [`Player::next`] is asked once per frame, after the frame before has run, and says which joypad
//! the next frame runs with, so the same plan runs in the window, in a UI snapshot and in a test.

use super::{MenuProbe, SAVE_DONE_STEP, SAVE_LIST_STEP, SAVE_SCREEN, order};
use ssd_oracle::Joypad;

/// The rows of a page of the original's item list (`map_entry_value` 0 to 7; `map_entry_0d63` is the page).
const LIST_ROWS: u8 = 8;

/// The buttons a plan presses: the menus read these, and nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Up,
    Down,
    Left,
    Right,
    A,
    B,
}

/// The state a step waits for, read from the menu state of the probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Until {
    /// The field is free, or the open menu is drawn and idle, so the next press is taken.
    Ready,
    /// The menu is open (`field_state` is `$0F` while it opens, 1 once it is open).
    Open,
    /// The menu shows this screen (`menu_screen`).
    Screen(u8),
    /// The menu shows the command window: the field's or a town's (screens 0 and 13).
    Command,
    /// The cursor of the open list is on this row (`map_entry_value`).
    Cursor(u8),
    /// The original took the press on its command window: a message or an effect runs (`menu_busy`), the menu moved
    /// on to a screen of its own or closed, or the field holds the party for the message (`field_state` 5 while a
    /// Talk or Look message waits for A) or for a scene. A press the original refuses (nobody to talk to, no place to
    /// land) leaves the command window idle, and is not this.
    Taken,
    /// The Item list is drawn and ready (screen 2, step 1).
    ItemList,
    /// The Use or Drop choice of an item is drawn (screen 2, step 2).
    ItemChoice,
    /// The party list of a use is drawn (screen 2, step 5), or the message after the use (step 3).
    ItemReply,
    /// The effect of a use runs (screen 2, step 4), or its result is shown (step 7).
    ItemEffect,
    /// The party list of a use is drawn (screen 2, step 5).
    ItemMembers,
    /// The original took the Use: it asks for a member or shows a message (screen 2, steps 5, 3
    /// and 9), its effect runs on the choice step (`effect_pending` bit 7: the Dragon Radar's own
    /// screen, Bubbles), or it is over already (the menu closed or is back on its command window).
    UseTaken,
    /// The Yes or No of a drop is drawn (screen 2, step 6).
    DropAsk,
    /// The field is free: the menu is closed.
    Closed,
    /// The original's answer to a use: the field is free, or its menu is idle on its command window or after an
    /// effect that was refused (the object stays, and the player decides what comes next).
    ItemAnswered,
    /// A message of a use waits for A (`menu_busy` 1 on the message steps 3, 4 and 7: the message after the use, the
    /// effect's own, and its result).
    ItemMessage,
    /// Not a message that waits for A any more (the state a press of A has moved on from).
    Moved,
    /// The field is free, or the menu is open on its command window (the field's or a town's).
    Settled,
    /// The original's Save shows its slot list (screen 10, step 1).
    SaveList,
    /// The original's Save wrote the slot and shows "Completed!" (screen 10, step 2).
    SaveDone,
    /// The command window of a battle is up and idle (screen 17 in a battle), where B from the battle's Item list
    /// goes.
    BattleCommand,
    /// The original's Order screen is on this step (screen 8): 1 takes the picks, 2 asks Yes or No.
    OrderStep(u8),
    /// The original's Order screen has filled this many slots (`$0D6B`), or more.
    Picked(u8),
    /// The cursor of the open list is not on this row any more (`map_entry_value`): a list whose cursor skips rows moved.
    CursorMoved(u8),
}

/// One step of a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Presses Down until the cursor is on `row` of a list of `rows` rows. Down wraps around, as the
    /// menu's own lists do, so pressing Down alone reaches every row.
    Row { row: u8, rows: u8 },
    /// Presses Left or Right until the cursor of a row of `columns` choices (the Text screen's five speeds) is on
    /// `column`. The list does not wrap, so the presses go the short way, one cell each.
    Column { column: u8, columns: u8 },
    /// Presses Down until the cursor of a list of `rows` rows is on `row`, one press at a time, each until the cursor has
    /// moved (the Order screen's cursor skips the rows already taken, so a press does not always move it by one row).
    Seek { row: u8, rows: u8 },
    /// Moves the cursor of the original's item list to its first entry (row 0 of page 0) with the original's own Up,
    /// after a stage put the chosen stack there. The original reads the id of the entry under its cursor
    /// (`menu_item_id`) when the cursor moves, not when A is pressed, so the cursor moves even when it is on the first
    /// entry already (Down, then Up), unless the list holds one entry.
    ListTop,
    /// Presses `button`, then waits for `until`.
    Press { button: Button, until: Until },
    /// Waits for `until` without pressing anything.
    Wait { until: Until },
    /// Stages the stack `id` as the first row of the original's list (a host write, made by the
    /// driver before the next frame). No frame is run for it.
    Stage { id: u32 },
    /// The party list of a use, if the original shows it: presses Down to `row` of `rows` and A.
    /// When the original goes on without it, the step does nothing.
    MemberIf { row: u8, rows: u8 },
    /// Runs `frames` frames with no press: the original settles after a menu closes.
    Pause { frames: u32 },
    /// The original draws the screens from here on: the curtain lifts, so the player sees what the
    /// original shows (its use window, its party list, its message, its effect). No frame is run for it.
    Lift,
    /// The original's answer after a use: each message that waits for A takes a press of A, and the step ends when
    /// the original is idle (see [`Until::ItemAnswered`]). Frames pass while its effect runs.
    Answer,
}

/// What a plan leaves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// Every step reached its state.
    Reached,
    /// A step did not reach its state in its budget, and the menu was closed again.
    Aborted { reason: &'static str },
    /// A step failed and the menu did not close: the game is not playable from the menu.
    Stuck { reason: &'static str },
}

/// What the next frame of a plan does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Runs one frame with this joypad, then asks again.
    Frame(Joypad),
    /// The plan is over: no further frame belongs to it.
    Done(Outcome),
    /// The driver stages the stack `id` (its writes go before the next frame), then asks again.
    Stage(u32),
    /// The driver lifts the curtain (the original draws what comes next), then asks again.
    Lift,
}

/// Frames a step waits for its state after a press: a press shows within a few frames, a screen or a
/// message within tens.
pub const SETTLE_FRAMES: u32 = 150;
/// Frames the menu may take to be ready for a press (its opening takes about 20).
pub const READY_FRAMES: u32 = 600;
/// Frames a press holds its button: two, then the release on the third frame.
const HELD: u8 = 2;
/// Times a press that does not reach its state is repeated before the plan aborts: the original
/// takes a press only some of the time just after a window is drawn.
const PRESS_RETRIES: u8 = 4;
/// Frames the plan lets pass after a press ends before it starts the next one: the original ignores a press that comes
/// within three frames of the one before (measured on the battle's Item list: an Up right after a Down, and an A right
/// after that Up, were lost, and each lost press cost [`SETTLE_FRAMES`] before it was repeated, so a use from a list of two
/// or more stacks froze the screen for 327 frames). Four frames are enough; eight leave room.
const PRESS_GAP: u32 = 8;
/// B presses a plan may spend closing the menu after an abort.
const CLOSE_PRESSES: u32 = 12;
/// Frames the close waits after each B press for the menu to close.
const CLOSE_SETTLE: u32 = 60;

/// Where the plan is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Before the first frame.
    Start,
    /// Waiting for the menu to take the press of the step at `index`.
    Ready { waited: u32 },
    /// Choosing what the step at `index` does now.
    Act,
    /// Holding `button` for the press of a step whose state is `until`; `held` frames so far.
    Holding { button: Button, until: Until, held: u8 },
    /// Waiting for `until` after a press or for a wait step.
    Settle { until: Until, waited: u32 },
    /// Closing the menu with B after an abort: `held` frames of the B press, or of the wait after it.
    Closing { presses: u32, held: u8, waited: u32 },
    /// A pause: `left` frames to run before the next step.
    Pausing { left: u32 },
    /// The plan is over.
    Finished(Outcome),
}

/// The plan of one menu action, run one frame at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Player {
    steps: Vec<Step>,
    index: usize,
    phase: Phase,
    /// Why the plan aborted, once it has.
    reason: &'static str,
    /// Repeats of the press of the current step so far.
    retries: u8,
    /// Frames the answer step has waited for the original's answer.
    answering: u32,
    /// The list's cursor has moved during the current [`Step::ListTop`].
    moved: bool,
    /// Presses the current [`Step::Seek`] has made.
    seeks: u8,
    /// Frames left of the gap after the last press ([`PRESS_GAP`]): the next press waits for it.
    cool: u32,
}

impl Player {
    /// A plan of `steps`, not started yet.
    #[must_use]
    pub fn new(steps: Vec<Step>) -> Self {
        Self {
            steps,
            index: 0,
            phase: Phase::Start,
            reason: "",
            retries: 0,
            answering: 0,
            moved: false,
            seeks: 0,
            cool: 0,
        }
    }

    /// The next frame of the plan, given the menu state after the frame before it.
    pub fn next(&mut self, probe: &MenuProbe<'_>) -> Next {
        let next = self.next_step(probe);
        if matches!(next, Next::Frame(_)) {
            self.cool = self.cool.saturating_sub(1);
        }
        next
    }

    /// Starts holding `button` for a press, or lets the gap after the last press pass first.
    fn hold(&mut self, button: Button, until: Until) {
        self.phase = if self.cool > 0 {
            Phase::Pausing { left: self.cool }
        } else {
            Phase::Holding { button, until, held: 0 }
        };
    }

    fn next_step(&mut self, probe: &MenuProbe<'_>) -> Next {
        loop {
            match self.phase {
                Phase::Finished(outcome) => return Next::Done(outcome),
                Phase::Start => self.phase = Phase::Ready { waited: 0 },
                Phase::Ready { waited } => {
                    if self.index == self.steps.len() {
                        self.phase = Phase::Finished(Outcome::Reached);
                    } else if !self.needs_ready() || probe.ready() {
                        self.phase = Phase::Act;
                    } else if waited >= READY_FRAMES {
                        self.abort("the menu is not ready for the next press");
                    } else {
                        self.phase = Phase::Ready { waited: waited + 1 };
                        return Next::Frame(pad(None));
                    }
                }
                Phase::Act => match self.steps[self.index] {
                    Step::Stage { id } => {
                        self.advance();
                        return Next::Stage(id);
                    }
                    Step::Pause { frames } => {
                        self.index += 1;
                        self.phase = Phase::Pausing { left: frames };
                    }
                    Step::Lift => {
                        self.advance();
                        return Next::Lift;
                    }
                    Step::Answer => {
                        if satisfied(probe, Until::ItemAnswered) {
                            self.advance();
                        } else if satisfied(probe, Until::ItemMessage) {
                            // A message that waits for A takes it; the answer is asked again after it.
                            let press = Step::Press { button: Button::A, until: Until::Moved };
                            self.steps.splice(self.index..=self.index, [press, Step::Answer]);
                            self.phase = Phase::Ready { waited: 0 };
                        } else if self.answering >= READY_FRAMES {
                            // The original keeps its menu on a state of its own: it is the player's to leave.
                            self.phase =
                                Phase::Finished(Outcome::Stuck { reason: "the use has no answer" });
                        } else {
                            // The effect runs: a frame, and the answer is asked again.
                            self.answering += 1;
                            self.phase = Phase::Pausing { left: 1 };
                        }
                    }
                    Step::MemberIf { row, rows } => {
                        let members = probe.screen() == Some(2) && probe.step() == Some(5);
                        // The party list's own steps replace this one: a row, then A.
                        let replaced: Vec<Step> = if members {
                            vec![
                                Step::Row { row, rows },
                                Step::Press { button: Button::A, until: Until::ItemReply },
                            ]
                        } else {
                            Vec::new()
                        };
                        self.steps.splice(self.index..=self.index, replaced);
                        self.phase = Phase::Ready { waited: 0 };
                    }
                    Step::ListTop => match (probe.cursor(), probe.page()) {
                        (Some(0), Some(0)) if self.moved || probe.item_count() <= 1 => {
                            self.advance();
                        }
                        (Some(0), Some(0)) => {
                            self.moved = true;
                            let until = Until::Cursor(1);
                            self.hold(Button::Down, until);
                        }
                        (Some(row), Some(_)) => {
                            self.moved = true;
                            // Up from a page's first row goes to the last row of the page before.
                            let until = Until::Cursor(row.checked_sub(1).unwrap_or(LIST_ROWS - 1));
                            self.hold(Button::Up, until);
                        }
                        _ => self.abort("the list has no cursor"),
                    },
                    Step::Column { column, columns } => match probe.cursor() {
                        Some(cursor) if cursor == column => self.advance(),
                        Some(_) if column >= columns => self.abort("the choice is not on the list"),
                        Some(cursor) if cursor < column => {
                            let until = Until::Cursor(cursor + 1);
                            self.hold(Button::Right, until);
                        }
                        Some(cursor) => {
                            let until = Until::Cursor(cursor - 1);
                            self.hold(Button::Left, until);
                        }
                        None => self.abort("the list has no cursor"),
                    },
                    Step::Seek { row, rows } => match probe.cursor() {
                        Some(cursor) if cursor == row => self.advance(),
                        // Every press moves the cursor to the next row left: more presses than rows means the row is
                        // not on the list.
                        Some(_) if self.seeks > rows.saturating_add(PRESS_RETRIES) => {
                            self.abort("the row is not on the list");
                        }
                        Some(cursor) => {
                            self.seeks += 1;
                            let until = Until::CursorMoved(cursor);
                            self.hold(Button::Down, until);
                        }
                        None => self.abort("the list has no cursor"),
                    },
                    Step::Row { row, rows } => match probe.cursor() {
                        Some(cursor) if cursor == row => self.advance(),
                        // The cursor moves down one row per press, and the list wraps.
                        Some(cursor) => {
                            let until = Until::Cursor((cursor + 1) % rows.max(1));
                            self.hold(Button::Down, until);
                        }
                        None => self.abort("the list has no cursor"),
                    },
                    // An A that waits for the menu to close is not pressed where the menu closes by itself:
                    // on the field it would open the menu, on the command window it would choose a row.
                    // A B that waits for it is not pressed on the field, where it does nothing.
                    Step::Press { button: Button::A, until: Until::Closed }
                        if probe.field_free() || matches!(probe.screen(), Some(0 | 13)) =>
                    {
                        self.phase = Phase::Settle { until: Until::Closed, waited: 0 };
                    }
                    Step::Press { button: Button::B, until: Until::Closed }
                        if probe.field_free() =>
                    {
                        self.advance();
                    }
                    // The effect of a use that is over already (the menu closed itself, or is back on its
                    // command window) takes no A: it would open the menu or choose a row.
                    Step::Press {
                        button: Button::A,
                        until: Until::UseTaken | Until::ItemEffect,
                    } if satisfied(probe, Until::Settled) => {
                        self.advance();
                    }
                    Step::Press { button, until } => {
                        self.hold(button, until);
                    }
                    Step::Wait { until } => self.phase = Phase::Settle { until, waited: 0 },
                },
                Phase::Holding { button, until, held } => {
                    if held < HELD {
                        self.phase = Phase::Holding { button, until, held: held + 1 };
                        return Next::Frame(pad(Some(button)));
                    }
                    // The release frame: the press is over, and its state is checked from the next frame.
                    self.cool = PRESS_GAP;
                    self.phase = Phase::Settle { until, waited: 0 };
                    return Next::Frame(pad(None));
                }
                Phase::Settle { until, waited } => {
                    if satisfied(probe, until) {
                        // A row step is not done until its cursor is on the row: check again.
                        if matches!(
                            self.steps[self.index],
                            Step::Row { .. }
                                | Step::Column { .. }
                                | Step::Seek { .. }
                                | Step::ListTop
                        ) {
                            self.phase = Phase::Ready { waited: 0 };
                        } else {
                            self.advance();
                        }
                    } else if waited >= SETTLE_FRAMES {
                        // A press on an Item sheet that the original did not take (it takes one only
                        // some of the time, just after its window is drawn) is repeated: no state
                        // changed, so the repeat is the same press.
                        match self.steps[self.index] {
                            Step::Press {
                                button,
                                until:
                                    again @ (Until::ItemChoice | Until::ItemReply | Until::UseTaken),
                            } if self.retries < PRESS_RETRIES && again == until => {
                                self.retries += 1;
                                self.hold(button, until);
                            }
                            // The list's cursor press is repeated the same way.
                            Step::ListTop | Step::Seek { .. } if self.retries < PRESS_RETRIES => {
                                self.retries += 1;
                                self.phase = Phase::Ready { waited: 0 };
                            }
                            // The original does not take the use (a refusal, such as a fighter at full KI): its
                            // menu stays open on its use window, for the player.
                            Step::Press { until: Until::UseTaken, .. } => {
                                self.phase = Phase::Finished(Outcome::Stuck {
                                    reason: "the original does not take the use",
                                });
                            }
                            _ => self.abort("a press did not reach the state it should"),
                        }
                    } else {
                        self.phase = Phase::Settle { until, waited: waited + 1 };
                        return Next::Frame(pad(None));
                    }
                }
                Phase::Pausing { left } => {
                    if left == 0 {
                        self.phase = Phase::Ready { waited: 0 };
                    } else {
                        self.phase = Phase::Pausing { left: left - 1 };
                        return Next::Frame(pad(None));
                    }
                }
                Phase::Closing { presses, held, waited } => {
                    if probe.field_free() {
                        self.phase = Phase::Finished(Outcome::Aborted { reason: self.reason });
                    } else if held < HELD {
                        self.phase = Phase::Closing { presses, held: held + 1, waited };
                        return Next::Frame(pad(Some(Button::B)));
                    } else if held == HELD {
                        self.phase = Phase::Closing { presses, held: HELD + 1, waited: 0 };
                        return Next::Frame(pad(None));
                    } else if waited < CLOSE_SETTLE {
                        self.phase = Phase::Closing { presses, held, waited: waited + 1 };
                        return Next::Frame(pad(None));
                    } else if presses + 1 >= CLOSE_PRESSES {
                        self.phase = Phase::Finished(Outcome::Stuck { reason: self.reason });
                    } else {
                        self.phase = Phase::Closing { presses: presses + 1, held: 0, waited: 0 };
                    }
                }
            }
        }
    }

    /// The step at `index` is done: the next one, or the end of the plan.
    fn advance(&mut self) {
        self.index += 1;
        self.retries = 0;
        self.moved = false;
        self.seeks = 0;
        self.phase = Phase::Ready { waited: 0 };
    }

    /// Stops the plan: the menu is closed with B presses, and the outcome says why.
    fn abort(&mut self, reason: &'static str) {
        self.reason = reason;
        self.phase = Phase::Closing { presses: 0, held: 0, waited: 0 };
    }

    /// Whether the step at `index` presses a button, and so needs the menu ready first.
    fn needs_ready(&self) -> bool {
        matches!(
            self.steps.get(self.index),
            Some(
                Step::Row { .. }
                    | Step::Column { .. }
                    | Step::Seek { .. }
                    | Step::ListTop
                    | Step::Press { .. }
            )
        )
    }
}

/// Whether the menu state of `probe` is the state `until` names.
fn satisfied(probe: &MenuProbe<'_>, until: Until) -> bool {
    match until {
        Until::Ready => probe.ready(),
        Until::Open => probe.menu_open(),
        Until::Screen(screen) => probe.menu_open() && probe.screen() == Some(screen),
        Until::Command => probe.menu_open() && matches!(probe.screen(), Some(0 | 13)),
        Until::Cursor(row) => probe.menu_open() && probe.cursor() == Some(row),
        Until::Taken => {
            probe.busy() || !probe.menu_open() || !matches!(probe.screen(), Some(0 | 13))
        }
        Until::ItemList => {
            probe.menu_open() && probe.screen() == Some(2) && probe.step() == Some(1)
        }
        Until::ItemChoice => {
            probe.menu_open() && probe.screen() == Some(2) && probe.step() == Some(2)
        }
        Until::ItemReply => {
            probe.menu_open() && probe.screen() == Some(2) && matches!(probe.step(), Some(3 | 5))
        }
        Until::ItemEffect => {
            (probe.menu_open() && probe.screen() == Some(2) && matches!(probe.step(), Some(4 | 7)))
                || satisfied(probe, Until::Settled)
        }
        Until::UseTaken => {
            probe.effect_runs()
                || probe.battle_effect()
                || (probe.menu_open()
                    && probe.screen() == Some(2)
                    && matches!(probe.step(), Some(3 | 5 | 9)))
                || satisfied(probe, Until::Settled)
        }
        Until::ItemMembers => {
            probe.menu_open() && probe.screen() == Some(2) && probe.step() == Some(5)
        }
        Until::DropAsk => probe.menu_open() && probe.screen() == Some(2) && probe.step() == Some(6),
        Until::Closed => probe.field_free(),
        Until::ItemAnswered => {
            probe.field_free()
                || probe.battle_command()
                || (probe.menu_open()
                    && !probe.busy()
                    && (matches!(probe.screen(), Some(0 | 13))
                        || (probe.screen() == Some(2) && probe.step() == Some(10))))
        }
        Until::ItemMessage => {
            probe.menu_open()
                && probe.screen() == Some(2)
                && matches!(probe.step(), Some(3 | 4 | 7))
                && probe.message_waits()
        }
        Until::Moved => !satisfied(probe, Until::ItemMessage),
        Until::Settled => {
            probe.field_free()
                || probe.battle_command()
                || (probe.menu_open() && matches!(probe.screen(), Some(0 | 13)))
        }
        Until::SaveList => {
            probe.menu_open()
                && probe.screen() == Some(SAVE_SCREEN)
                && probe.step() == Some(SAVE_LIST_STEP)
        }
        Until::SaveDone => {
            probe.menu_open()
                && probe.screen() == Some(SAVE_SCREEN)
                && probe.step() == Some(SAVE_DONE_STEP)
        }
        Until::BattleCommand => probe.battle_command(),
        Until::OrderStep(step) => {
            probe.menu_open() && probe.screen() == Some(order::SCREEN) && probe.step() == Some(step)
        }
        Until::Picked(count) => {
            probe.menu_open()
                && probe.screen() == Some(order::SCREEN)
                && probe.order_picked() >= count
        }
        Until::CursorMoved(from) => probe.menu_open() && probe.cursor() != Some(from),
    }
}

/// The joypad that holds `button` (or nothing).
fn pad(button: Option<Button>) -> Joypad {
    let mut joypad = Joypad::default();
    match button {
        Some(Button::Up) => joypad.up = true,
        Some(Button::Down) => joypad.down = true,
        Some(Button::Left) => joypad.left = true,
        Some(Button::Right) => joypad.right = true,
        Some(Button::A) => joypad.a = true,
        Some(Button::B) => joypad.b = true,
        None => {}
    }
    joypad
}

#[cfg(test)]
mod tests {
    use super::super::testing::{FakeMenu, held};
    use super::*;

    /// Runs `plan` on `menu` for at most `frames` frames, returning the joypads it ran with and the
    /// outcome (if the plan ended).
    fn run(plan: Vec<Step>, menu: &mut FakeMenu, frames: u32) -> (Vec<Joypad>, Option<Outcome>) {
        let labels = ssd_menus::Labels::default();
        let mut player = Player::new(plan);
        let mut pads = Vec::new();
        for _ in 0..frames {
            let wram = menu.wram();
            let probe = MenuProbe::new(&wram, &labels);
            match player.next(&probe) {
                Next::Frame(pad) => {
                    pads.push(pad);
                    menu.tick(pad);
                }
                Next::Done(outcome) => return (pads, Some(outcome)),
                Next::Stage(_) | Next::Lift => {}
            }
        }
        (pads, None)
    }

    /// The menu open on the field, idle, on its command window.
    fn open_menu() -> FakeMenu {
        let mut menu = FakeMenu::default();
        menu.field = 1;
        menu.tick(Joypad::default());
        menu
    }

    #[test]
    fn a_press_holds_its_button_for_two_frames_and_releases_on_the_third() {
        let mut menu = open_menu();
        let plan = vec![Step::Press { button: Button::Down, until: Until::Cursor(1) }];
        let (pads, outcome) = run(plan, &mut menu, 40);
        assert_eq!(outcome, Some(Outcome::Reached));
        assert_eq!(pads[0], held(Button::Down));
        assert_eq!(pads[1], held(Button::Down));
        assert_eq!(pads[2], Joypad::default(), "released on the third frame");
        assert_eq!(pads.len(), 3, "the press runs no frame after its state is there");
    }

    #[test]
    fn a_row_step_presses_down_until_the_cursor_is_on_its_row() {
        let mut menu = open_menu();
        let plan = vec![
            Step::Row { row: 3, rows: 5 },
            Step::Press { button: Button::A, until: Until::ItemList },
        ];
        let (pads, outcome) = run(plan, &mut menu, 200);
        assert_eq!(outcome, Some(Outcome::Reached));
        let downs = pads.iter().filter(|pad| pad.down).count() / 2;
        assert_eq!(downs, 3, "three presses of Down from Talk to Item");
        assert_eq!(menu.screen, 2, "the Item list is drawn");
    }

    #[test]
    fn down_wraps_so_a_row_behind_the_cursor_is_reached_the_short_way() {
        let mut menu = open_menu();
        menu.cursor = 4;
        let plan = vec![Step::Row { row: 0, rows: 5 }];
        let (pads, outcome) = run(plan, &mut menu, 40);
        assert_eq!(outcome, Some(Outcome::Reached));
        assert_eq!(pads.iter().filter(|pad| pad.down).count(), 2, "one press: 4 to 0 wraps");
    }

    #[test]
    fn a_plan_waits_for_a_busy_menu_before_it_presses() {
        let mut menu = open_menu();
        menu.busy = true;
        let plan = vec![Step::Press { button: Button::Down, until: Until::Cursor(1) }];
        let (pads, _) = run(plan, &mut menu, 5);
        assert!(pads.iter().all(|pad| *pad == Joypad::default()), "no press while busy");
        menu.busy = false;
        let (pads, outcome) = run(plan_down_to_1(), &mut menu, 40);
        assert_eq!(outcome, Some(Outcome::Reached));
        assert!(pads[0].down, "the press comes once the menu is ready");
    }

    fn plan_down_to_1() -> Vec<Step> {
        vec![Step::Press { button: Button::Down, until: Until::Cursor(1) }]
    }

    #[test]
    fn a_step_that_never_gets_its_state_aborts_and_closes_the_menu_with_b() {
        // Fly takes off by itself and closes the menu, so the plan waiting for Cards is aborted
        // with the field free: no B press is needed.
        let mut menu = open_menu();
        menu.cursor = 2;
        let plan = vec![Step::Press { button: Button::A, until: Until::Screen(9) }];
        let (_, outcome) = run(plan, &mut menu, 400);
        assert_eq!(
            outcome,
            Some(Outcome::Aborted { reason: "a press did not reach the state it should" })
        );
        assert_eq!(menu.field, 0, "the field is free");
    }

    #[test]
    fn a_menu_that_stays_open_is_closed_with_b_presses_after_an_abort() {
        // Look shows its message and waits for A; B does not close it, so the abort gets stuck.
        let mut menu = open_menu();
        menu.cursor = 1;
        let plan = vec![Step::Press { button: Button::A, until: Until::Screen(9) }];
        let (pads, outcome) = run(plan, &mut menu, 5_000);
        assert_eq!(
            outcome,
            Some(Outcome::Stuck { reason: "a press did not reach the state it should" })
        );
        assert!(pads.iter().any(|pad| pad.b), "B was pressed to close the menu");
        assert!(menu.message && menu.field == 1, "the message is still up");
    }

    #[test]
    fn a_menu_left_open_on_a_list_is_closed_by_b_presses_and_is_aborted_not_stuck() {
        // The Item list goes back to the command window with the first B and closes with the second:
        // the plan is aborted, and the field is free.
        let mut menu = open_menu();
        menu.cursor = 3;
        let plan = vec![Step::Press { button: Button::A, until: Until::Screen(9) }];
        let (pads, outcome) = run(plan, &mut menu, 2_000);
        assert_eq!(
            outcome,
            Some(Outcome::Aborted { reason: "a press did not reach the state it should" })
        );
        // A press is a rising edge of B: the plan stops as soon as the field is free.
        let presses =
            pads.windows(2).filter(|w| !w[0].b && w[1].b).count() + usize::from(pads[0].b);
        assert_eq!(presses, 2, "two B presses");
        assert_eq!(menu.field, 0);
    }

    #[test]
    fn a_use_is_taken_when_its_effect_runs_on_the_choice_step() {
        let mut menu = open_menu();
        menu.screen = 2;
        menu.step = 2;
        let plan = vec![Step::Wait { until: Until::UseTaken }];
        let (_, outcome) = run(plan.clone(), &mut menu, 400);
        assert!(matches!(outcome, Some(Outcome::Aborted { .. } | Outcome::Stuck { .. })));
        let mut menu = open_menu();
        menu.screen = 2;
        menu.step = 2;
        menu.effect = true;
        let (_, outcome) = run(plan, &mut menu, 10);
        assert_eq!(outcome, Some(Outcome::Reached));
    }

    #[test]
    fn a_plan_with_no_steps_reaches_at_once() {
        let mut menu = open_menu();
        let (pads, outcome) = run(Vec::new(), &mut menu, 3);
        assert_eq!(pads, Vec::<Joypad>::new());
        assert_eq!(outcome, Some(Outcome::Reached));
    }
}
