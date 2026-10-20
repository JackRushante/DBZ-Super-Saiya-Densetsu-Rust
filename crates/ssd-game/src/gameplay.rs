//! The Gameplay page's two host-side additions (M7): the message auto-run and the battle animation speed. Neither
//! installs a hook or writes the machine: the first is a press of A that goes into the joypad of a frame (so a
//! recording carries it like any other input, and a replay plays it), the second is a number of machine frames to run
//! for one frame on the screen, decided by the host from the work RAM. The machine, the RNG and every outcome are
//! exactly the original's. Research: `docs/re/data/gameplay.md`.

use crate::data as d;
use ssd_menus::options::{Values, gameplay as page};
use ssd_oracle::Joypad;

/// The bus address of the first byte of work RAM.
const WRAM_BASE: u32 = 0x7E_0000;
/// The game mode (`$24`) of a battle's running states, without its flag bit (bit 7).
const BATTLE_MODE: u8 = 4;
/// The game mode (`$24`) that starts a battle and that the command selection runs in.
const BATTLE_START_MODE: u8 = 2;
/// `in_battle` (`$0364`) in a battle.
const IN_BATTLE: u8 = 0xFF;
/// The battle state of the attack animation (`$AB`): the damage is known, the figures strike, the HP drops.
const ATTACK_ANIMATION: u8 = 4;
/// The frames A is held when the auto-run presses it: a press lasts a few frames, as a hand's, so the game's
/// once-a-frame read cannot miss the new press. It is let go as soon as the message leaves its wait.
const PRESS_FRAMES: u8 = 3;

fn byte(wram: &[u8], address: u32) -> u8 {
    wram.get((address - WRAM_BASE) as usize).copied().unwrap_or(0)
}

/// Whether a finished message waits for the player's A: the message engine is in the state that writes a page
/// and waits (2, the end of a page) or the one that waits to close (3), and neither bit 4 (closes by itself) nor
/// bits 5 and 6 (the instant kinds) is set.
#[must_use]
pub fn message_waits_for_a(wram: &[u8]) -> bool {
    let state = byte(wram, d::MESSAGE_STATE);
    state & 0xF0 == 0x80 && matches!(state & 0x0F, 2 | 3)
}

/// The auto-run of messages: counts the frames a finished message has waited and presses A when the wait
/// of the Gameplay page is up.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AutoRun {
    /// Frames the message has waited so far.
    waited: u32,
    /// Frames left of the press being made.
    pressing: u8,
}

impl AutoRun {
    /// One frame: with the auto-run on and a message waiting for A, `pad` gets A held once the message has
    /// waited the page's wait (a press of the player's own starts the count again). Anything else leaves `pad`
    /// as it is. `wram` is the work RAM after the frame before this one.
    pub fn apply(&mut self, wram: &[u8], values: &Values, pad: &mut Joypad) {
        if !page::auto_run_on(values) || !message_waits_for_a(wram) {
            *self = Self::default();
            return;
        }
        if self.pressing > 0 {
            pad.a = true;
            self.pressing -= 1;
            if self.pressing == 0 {
                self.waited = 0;
            }
            return;
        }
        if pad.a {
            self.waited = 0;
            return;
        }
        self.waited += 1;
        if self.waited >= page::auto_wait_frames(values) {
            pad.a = true;
            self.pressing = PRESS_FRAMES - 1;
        }
    }
}

/// Whether a battle is on: `in_battle` is set and the game mode is one of a battle's, 2 (its start and the command
/// selection) or 4 (the rest). The touch controls' automatic switch to the minimal layout and the Cheats over a battle
/// read it.
#[must_use]
pub fn in_battle(wram: &[u8]) -> bool {
    byte(wram, d::IN_BATTLE) == IN_BATTLE
        && matches!(byte(wram, d::GAME_MODE) & 0x7F, BATTLE_START_MODE | BATTLE_MODE)
}

/// Whether the battle is in its attack animation: nothing is asked of the player, no message is up, and the state
/// machine only waits for the figures to finish. The player's buttons do nothing here (checked on the reference
/// battles, `docs/re/data/gameplay.md`), so the frames can run faster without a press being missed.
#[must_use]
pub fn attack_animation(wram: &[u8]) -> bool {
    byte(wram, d::IN_BATTLE) == IN_BATTLE
        && byte(wram, d::GAME_MODE) & 0x7F == BATTLE_MODE
        && byte(wram, d::BATTLE_PHASE) == ATTACK_ANIMATION
        && byte(wram, d::MESSAGE_STATE) == 0
}

/// The machine frames to run for one frame of the screen now: the page's animation speed in the attack animation,
/// else 1. The frames run are the game's own, every one whole (only the sound of the extra ones is not played), so
/// the turn order, the numbers and the random draws are the original's.
#[must_use]
pub fn animation_factor(wram: &[u8], values: &Values) -> u32 {
    let factor = page::anim_factor(values);
    if factor > 1 && attack_animation(wram) { factor } else { 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wram_with(message: u8, mode: u8, phase: u8, in_battle: u8) -> Vec<u8> {
        let mut wram = vec![0_u8; 0x2_0000];
        wram[(d::MESSAGE_STATE - WRAM_BASE) as usize] = message;
        wram[(d::GAME_MODE - WRAM_BASE) as usize] = mode;
        wram[(d::BATTLE_PHASE - WRAM_BASE) as usize] = phase;
        wram[(d::IN_BATTLE - WRAM_BASE) as usize] = in_battle;
        wram
    }

    fn on(wait: i64) -> Values {
        let mut values = Values::new();
        values.set(page::AUTO_RUN, 1);
        values.set(page::AUTO_WAIT, wait);
        values
    }

    #[test]
    fn only_the_two_waiting_states_of_a_plain_message_wait_for_a() {
        for (state, waits) in [
            (0x00, false),
            (0x80, false),
            (0x81, false),
            (0x82, true),
            (0x83, true),
            (0x93, false), // closes by itself
            (0x85, false),
            (0x96, false), // a timed wait
            (0xE3, false), // the instant kinds
            (0x02, false), // the message flag is not set
        ] {
            assert_eq!(message_waits_for_a(&wram_with(state, 0, 0, 0)), waits, "{state:#04x}");
        }
    }

    #[test]
    fn the_auto_run_presses_a_after_the_wait_and_lets_go_when_the_message_moves_on() {
        let waiting = wram_with(0x83, 0, 0, 0);
        let mut run = AutoRun::default();
        let values = on(0); // Short: 45 frames
        let mut pressed = Vec::new();
        for frame in 0..60 {
            let mut pad = Joypad::default();
            run.apply(&waiting, &values, &mut pad);
            if pad.a {
                pressed.push(frame);
            }
        }
        assert_eq!(
            pressed,
            [44, 45, 46],
            "A is held three frames once the 45th frame of the wait comes"
        );
        // The message moved on after the first frame of the press: A is let go at once.
        let mut run = AutoRun::default();
        let mut pad = Joypad::default();
        for _ in 0..45 {
            pad = Joypad::default();
            run.apply(&waiting, &values, &mut pad);
        }
        assert!(pad.a);
        let mut pad = Joypad::default();
        run.apply(&wram_with(0x81, 0, 0, 0), &values, &mut pad);
        assert!(!pad.a, "the next message is printing: no press");
        assert_eq!(run, AutoRun::default());
    }

    #[test]
    fn the_wait_is_the_pages_and_a_press_of_the_players_starts_it_again() {
        let waiting = wram_with(0x82, 0, 0, 0);
        for (choice, frames) in [(0, 45), (1, 90), (2, 180)] {
            let mut run = AutoRun::default();
            let values = on(choice);
            let mut count = 0;
            loop {
                count += 1;
                let mut pad = Joypad::default();
                run.apply(&waiting, &values, &mut pad);
                if pad.a {
                    break;
                }
            }
            assert_eq!(count, frames, "choice {choice}");
        }
        let mut run = AutoRun::default();
        let values = on(0);
        for _ in 0..40 {
            run.apply(&waiting, &values, &mut Joypad::default());
        }
        let mut live = Joypad { a: true, ..Joypad::default() };
        run.apply(&waiting, &values, &mut live);
        assert_eq!(run.waited, 0, "the player pressed: the wait starts again");
    }

    #[test]
    fn off_it_never_presses_and_leaves_the_pad_alone() {
        let waiting = wram_with(0x83, 0, 0, 0);
        let mut run = AutoRun::default();
        let values = Values::new();
        for _ in 0..500 {
            let mut pad = Joypad { left: true, ..Joypad::default() };
            run.apply(&waiting, &values, &mut pad);
            assert_eq!(pad, Joypad { left: true, ..Joypad::default() });
        }
    }

    #[test]
    fn a_battle_is_on_in_its_command_selection_too() {
        // The command selection of the Nappa fight, from a player's state: in_battle $FF, game mode 2.
        assert!(in_battle(&wram_with(0, 2, 0, 0xFF)));
        assert!(in_battle(&wram_with(0, 4, 0, 0xFF)));
        assert!(in_battle(&wram_with(0, 0x84, 0, 0xFF)), "the just-entered bit does not count");
        assert!(!in_battle(&wram_with(0, 0, 0, 0xFF)), "the map");
        assert!(!in_battle(&wram_with(0, 2, 0, 0)), "in_battle clear");
    }

    #[test]
    fn the_attack_animation_is_the_battle_state_four_with_no_message() {
        assert!(attack_animation(&wram_with(0, 4, 4, 0xFF)));
        assert!(
            attack_animation(&wram_with(0, 0x84, 4, 0xFF)),
            "the flag bit of the mode is not the mode"
        );
        for (message, mode, phase, in_battle) in [
            (0x96, 4, 4, 0xFF), // a message is up
            (0, 2, 4, 0xFF),    // the other battle mode
            (0, 4, 3, 0xFF),    // the damage message
            (0, 4, 5, 0xFF),    // the result message
            (0, 4, 4, 0x00),    // not in a battle: the byte is a map's
            (0, 0, 4, 0x00),
        ] {
            assert!(!attack_animation(&wram_with(message, mode, phase, in_battle)));
        }
    }

    #[test]
    fn the_animation_factor_is_one_unless_the_page_asks_and_the_animation_plays() {
        let animation = wram_with(0, 4, 4, 0xFF);
        let other = wram_with(0x96, 4, 5, 0xFF);
        let mut values = Values::new();
        assert_eq!(animation_factor(&animation, &values), 1, "Normal");
        for (choice, factor) in [(1, 2), (2, 3), (3, 4)] {
            values.set(page::ANIM_SPEED, choice);
            assert_eq!(animation_factor(&animation, &values), factor);
            assert_eq!(animation_factor(&other, &values), 1, "a message is not sped up");
        }
    }
}
