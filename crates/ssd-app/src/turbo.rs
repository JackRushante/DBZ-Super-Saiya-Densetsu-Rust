//! Turbo (Options > Controls > Turbo): a held A (or A and B) is pressed and let go again at a steady rate, so the
//! ki training, which asks for fast presses of A, needs only a finger held down.
//!
//! It works on the joypad the player's devices make (keyboard, gamepad and touch buttons together), one machine frame
//! at a time, right before the frame is run and recorded: a replay holds the presses the game saw, and plays back the
//! same with turbo off. The HD menus' own navigation reads the raw joypad (`hd::keys_pressed`), so a held A confirms
//! there once, not at the turbo rate. Classic never calls it (no HD layer, no Options page).

use ssd_menus::options::{Values, controls};
use ssd_oracle::Joypad;

/// The machine's frames in a second, to turn a rate into a period (the console runs a hair above 60).
const FRAMES_PER_SECOND: u32 = 60;

/// Which buttons turbo works on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Buttons {
    #[default]
    Off,
    A,
    AAndB,
}

/// The choice of the Turbo page: the buttons and the presses per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Settings {
    pub buttons: Buttons,
    pub rate: u8,
}

impl Settings {
    /// The turbo of the PC file's `values` (off at 20 a second when it holds nothing).
    pub fn from_values(values: &Values) -> Self {
        let buttons = match controls::turbo_mode(values) {
            1 => Buttons::A,
            2 => Buttons::AAndB,
            _ => Buttons::Off,
        };
        Self { buttons, rate: controls::turbo_rate(values) }
    }

    /// The frames of one press and release: 3 at 20 a second.
    fn period(self) -> u32 {
        (FRAMES_PER_SECOND / u32::from(self.rate.max(1))).max(2)
    }
}

/// How long each turbo button has been held, in the frames the machine ran.
#[derive(Debug, Default)]
pub(crate) struct Turbo {
    a: u32,
    b: u32,
}

/// Whether the button is pressed on the frame that is `held` frames into the hold (the first frame is pressed): it
/// is pressed for the first half of each period, rounded up, and let go for the rest, so a period always has a frame
/// of release for the game to see.
fn pulse(held: u32, period: u32) -> bool {
    held % period < period.div_ceil(2)
}

impl Turbo {
    /// The joypad the game gets for the next frame, given the one the devices hold: A (and B) held are pulsed.
    pub fn step(&mut self, settings: Settings, pad: Joypad) -> Joypad {
        let (use_a, use_b) = match settings.buttons {
            Buttons::Off => (false, false),
            Buttons::A => (true, false),
            Buttons::AAndB => (true, true),
        };
        let period = settings.period();
        let mut out = pad;
        if use_a && pad.a {
            out.a = pulse(self.a, period);
            self.a = self.a.wrapping_add(1);
        } else {
            self.a = 0;
        }
        if use_b && pad.b {
            out.b = pulse(self.b, period);
            self.b = self.b.wrapping_add(1);
        } else {
            self.b = 0;
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn held(a: bool, b: bool) -> Joypad {
        Joypad { a, b, ..Joypad::default() }
    }

    fn stream(
        turbo: &mut Turbo,
        settings: Settings,
        pad: Joypad,
        frames: u32,
    ) -> Vec<(bool, bool)> {
        (0..frames)
            .map(|_| {
                let out = turbo.step(settings, pad);
                (out.a, out.b)
            })
            .collect()
    }

    fn presses(frames: &[(bool, bool)], pick: impl Fn(&(bool, bool)) -> bool) -> u32 {
        let mut count = 0;
        let mut before = false;
        for frame in frames {
            let now = pick(frame);
            count += u32::from(now && !before);
            before = now;
        }
        count
    }

    fn settings(buttons: Buttons, rate: u8) -> Settings {
        Settings { buttons, rate }
    }

    #[test]
    fn each_rate_presses_that_many_times_a_second_with_a_release_between() {
        for rate in [15, 20, 30] {
            let mut turbo = Turbo::default();
            let frames = stream(&mut turbo, settings(Buttons::A, rate), held(true, false), 60);
            assert_eq!(presses(&frames, |f| f.0), u32::from(rate), "{rate} a second");
            assert!(frames[0].0, "the first frame of the hold is a press");
            assert!(frames.iter().any(|f| !f.0), "and it lets go");
            assert!(frames.iter().all(|f| !f.1), "B is not held");
            // Never two releases or two presses of more than half a period in a row.
            let longest_release =
                frames.split(|f| f.0).map(<[(bool, bool)]>::len).max().unwrap_or(0);
            assert!(longest_release <= 2, "{rate}: {longest_release} frames let go");
        }
    }

    #[test]
    fn a_alone_leaves_b_to_the_player_and_a_and_b_pulse_both() {
        let mut turbo = Turbo::default();
        let frames = stream(&mut turbo, settings(Buttons::A, 20), held(true, true), 30);
        assert!(frames.iter().all(|f| f.1), "B stays held under the A mode");
        let mut turbo = Turbo::default();
        let frames = stream(&mut turbo, settings(Buttons::AAndB, 20), held(true, true), 60);
        assert_eq!(presses(&frames, |f| f.0), 20);
        assert_eq!(presses(&frames, |f| f.1), 20);
    }

    #[test]
    fn letting_go_releases_at_once_and_a_new_hold_starts_with_a_press() {
        let mut turbo = Turbo::default();
        let on = settings(Buttons::A, 15);
        stream(&mut turbo, on, held(true, false), 5);
        assert_eq!(stream(&mut turbo, on, held(false, false), 3), vec![(false, false); 3]);
        assert!(turbo.step(on, held(true, false)).a, "the next hold presses on its first frame");
    }

    #[test]
    fn off_passes_the_joypad_through_and_other_buttons_are_never_touched() {
        let mut turbo = Turbo::default();
        let pad = Joypad { a: true, b: true, up: true, start: true, ..Joypad::default() };
        for _ in 0..10 {
            assert_eq!(turbo.step(settings(Buttons::Off, 20), pad), pad);
        }
        for _ in 0..10 {
            let out = turbo.step(settings(Buttons::AAndB, 30), pad);
            assert!(out.up && out.start && !out.x && !out.l);
        }
    }

    #[test]
    fn the_settings_come_from_the_pc_file() {
        let mut values = Values::new();
        assert_eq!(Settings::from_values(&values), settings(Buttons::Off, 20));
        values.set(controls::TURBO_MODE, 1);
        values.set(controls::TURBO_RATE, 0);
        assert_eq!(Settings::from_values(&values), settings(Buttons::A, 15));
        values.set(controls::TURBO_MODE, 2);
        values.set(controls::TURBO_RATE, 2);
        assert_eq!(Settings::from_values(&values), settings(Buttons::AAndB, 30));
    }
}
