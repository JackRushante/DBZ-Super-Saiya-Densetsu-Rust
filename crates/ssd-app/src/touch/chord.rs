//! Select + Start held together for two seconds (Enhanced): the gesture that opens the touch MENU from a gamepad, as
//! Select + Start opens the Mode page in Classic. While the two are held together the game sees neither, so the MENU
//! does not open over a stray Start or Select. A press of one alone reaches the game a moment late ([`GRACE`]): long
//! enough for the other to join it, short enough not to be felt; a tap shorter than that is given to the game whole
//! once it ends.

use std::time::{Duration, Instant};

/// How long the two are held together before the MENU opens.
pub const HOLD: Duration = Duration::from_secs(2);
/// How long a press of one alone waits for the other before the game gets it.
pub const GRACE: Duration = Duration::from_millis(100);

/// The state of the gesture, given the two buttons once a window frame ([`Chord::update`]).
#[derive(Debug, Default, Clone)]
pub struct Chord {
    /// One of the two (0 Select, 1 Start) went down alone at this time and waits for the other.
    alone: Option<(usize, Instant)>,
    /// The game has been given this one, held alone past the grace: it gets it while it is held.
    passed: [bool; 2],
    /// A tap that ended within the grace: the game gets it until this time.
    owed: [Option<Instant>; 2],
    /// Both are held since this time.
    both: Option<Instant>,
    /// The two were held together: the game sees neither until both are up.
    masking: bool,
    /// The MENU opened for this hold.
    fired: bool,
}

impl Chord {
    /// The buttons held at `now`: what the game sees of them (Select, Start), and whether the MENU opens now (once
    /// for a hold of two seconds; a hold that one button leaves starts again when it comes back).
    pub fn update(&mut self, now: Instant, select: bool, start: bool) -> (bool, bool, bool) {
        let held = [select, start];
        if select && start {
            self.alone = None;
            self.passed = [false; 2];
            self.owed = [None; 2];
            self.masking = true;
            let since = *self.both.get_or_insert(now);
            let fire = !self.fired && now.duration_since(since) >= HOLD;
            self.fired |= fire;
            return (false, false, fire);
        }
        self.both = None;
        self.fired = false;
        if self.masking {
            if select || start {
                return (false, false, false);
            }
            self.masking = false;
        }
        let mut out = [false; 2];
        for i in 0..2 {
            if held[i] {
                if self.passed[i] {
                    out[i] = true;
                    continue;
                }
                let since = match self.alone {
                    Some((j, at)) if j == i => at,
                    _ => {
                        self.alone = Some((i, now));
                        now
                    }
                };
                if now.duration_since(since) >= GRACE {
                    self.alone = None;
                    self.passed[i] = true;
                    out[i] = true;
                }
            } else {
                self.passed[i] = false;
                if matches!(self.alone, Some((j, _)) if j == i) {
                    self.alone = None;
                    self.owed[i] = Some(now + GRACE);
                }
                if let Some(until) = self.owed[i] {
                    if now < until {
                        out[i] = true;
                    } else {
                        self.owed[i] = None;
                    }
                }
            }
        }
        (out[0], out[1], false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays `steps` of (milliseconds held, select, start) at 60 updates a second from `t0`; gives the time reached,
    /// the frames the game saw Select and Start pressed, and the frames the MENU opened at.
    fn play(
        chord: &mut Chord,
        t0: Instant,
        steps: &[(u64, bool, bool)],
    ) -> (Instant, u32, u32, Vec<u32>) {
        let mut now = t0;
        let (mut select, mut start, mut fired, mut frame) = (0, 0, Vec::new(), 0);
        for &(ms, s, t) in steps {
            let end = now + Duration::from_millis(ms);
            while now < end {
                let (seen_select, seen_start, fire) = chord.update(now, s, t);
                select += u32::from(seen_select);
                start += u32::from(seen_start);
                if fire {
                    fired.push(frame);
                }
                frame += 1;
                now += Duration::from_micros(16_667);
            }
        }
        (now, select, start, fired)
    }

    #[test]
    fn held_two_seconds_opens_the_menu_once_and_the_game_never_sees_the_two() {
        let mut chord = Chord::default();
        let (_, select, start, fired) =
            play(&mut chord, Instant::now(), &[(3000, true, true), (500, false, false)]);
        assert_eq!(fired.len(), 1, "once for the hold: {fired:?}");
        assert!((119..=121).contains(&fired[0]), "after two seconds: frame {}", fired[0]);
        assert_eq!((select, start), (0, 0), "no stray press");
    }

    #[test]
    fn not_held_long_enough_or_released_early_opens_nothing() {
        let mut chord = Chord::default();
        let (_, select, start, fired) = play(
            &mut chord,
            Instant::now(),
            &[(1900, true, true), (200, false, false), (1000, true, true), (200, false, false)],
        );
        assert!(fired.is_empty(), "{fired:?}");
        assert_eq!((select, start), (0, 0));
    }

    #[test]
    fn one_released_and_pressed_again_starts_the_count_again() {
        let mut chord = Chord::default();
        let t0 = Instant::now();
        // 1.5 s, Start let go for a moment (Select still held), then 1.5 s more: no MENU.
        let (now, select, start, fired) =
            play(&mut chord, t0, &[(1500, true, true), (100, true, false), (1500, true, true)]);
        assert!(fired.is_empty(), "the count started again: {fired:?}");
        assert_eq!((select, start), (0, 0), "Select stays hidden while it waits for Start");
        // Another half second together reaches two seconds of the second hold.
        let (_, _, _, fired) = play(&mut chord, now, &[(600, true, true)]);
        assert_eq!(fired.len(), 1);
    }

    #[test]
    fn a_short_press_of_one_reaches_the_game_and_a_long_one_all_its_length() {
        let mut chord = Chord::default();
        // A tap of 50 ms (three updates): the game gets it, whole, once it ends.
        let (now, select, start, _) =
            play(&mut chord, Instant::now(), &[(50, false, true), (300, false, false)]);
        assert_eq!(select, 0);
        assert!((5..=7).contains(&start), "the tap is given for the grace: {start}");
        // Start held 1 s alone: the game sees it from the grace on, until it is let go.
        let (_, select, start, fired) =
            play(&mut chord, now, &[(1000, false, true), (300, false, false)]);
        assert_eq!(fired, Vec::<u32>::new());
        assert_eq!(select, 0);
        assert!((53..=55).contains(&start), "held all but the grace: {start}");
    }
}
