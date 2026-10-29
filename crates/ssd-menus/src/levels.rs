//! The party's growth past the game's own level table (the pack parameter `level_cap` of the feature `free_progression`,
//! `docs/mods/free-progression.md`, "The level cap"): pure integer arithmetic, shared by the game's level-up hooks
//! (`ssd_game::mods::levels`) and by the save editor, so that both give the same numbers.
//!
//! The game's own rows follow, to the accuracy the research found, shifted power laws in `u = level + 5` (BP about `u^3`, KI and
//! HP about `u`, the EXP a level needs about `u^2`). Past the last level a character reaches in the original (its cap, 47 or 48:
//! the [`Anchor`]) the law continues from the character's own last values, so that the curve is continuous where the game's
//! table ends:
//!
//! - the BP total at level `L` is `BP(anchor) * (u / u_anchor)^3`, and the BP the level gives is the difference of two totals;
//! - the KI total is `KI(anchor) * (u / u_anchor)`, the maximum HP is `HP(anchor) * (u / u_anchor)`, at most [`HP_KI_MAX`];
//! - the EXP at which the level begins is `EXP(anchor) * (u / u_anchor)^2`;
//! - byte `$0E` of the record (a tie-break of the battle's order, read by the special attack's roll in 8 bits) grows with `u` as
//!   well, at most [`BYTE_0E_MAX`], so that the roll cannot wrap.

/// The highest level a record's own level byte holds when a level above it is kept in the mod state: the original's screens show
/// two digits, so the byte saturates at 99 and they read "99".
pub const RECORD_LEVEL_MAX: u8 = 99;
/// The most a party member's maximum HP or KI is given: four digits, what the original's windows and the HD screens carry.
pub const HP_KI_MAX: u64 = 9_999;
/// The most byte `$0E` is given.
pub const BYTE_0E_MAX: u64 = 100;
/// The highest level the pack's `level_cap` can be.
pub const LEVEL_MAX: u16 = 999;
/// The shift of the levels in the law: `u = level + 5`.
const SHIFT: u128 = 5;

/// The values of a character at the last level the game's table gives it: the totals of BP and KI (the sums of the rows up to the
/// level), the maximum HP, the EXP the level begins at and byte `$0E`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Anchor {
    pub level: u16,
    pub bp: u64,
    pub ki: u64,
    pub max_hp: u64,
    pub exp: u64,
    pub byte_0e: u64,
}

/// What a level above the anchor is: the totals, and the values of its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub level: u16,
    /// The battle power a record has built up to this level, and the maximum KI.
    pub bp: u64,
    pub ki: u64,
    pub max_hp: u16,
    /// The EXP the level begins at (more than 24 bits at the top levels).
    pub exp: u64,
    pub byte_0e: u8,
}

/// `value * (u / u0)^power`, rounded to the nearest whole number (halves up).
fn scaled(value: u64, u: u128, u0: u128, power: u32) -> u64 {
    let (num, den) = (u.pow(power), u0.pow(power));
    u64::try_from((u128::from(value) * num + den / 2) / den).unwrap_or(u64::MAX)
}

impl Anchor {
    /// The point at `level`, which is at least the anchor's (at the anchor it is the anchor's own values).
    #[must_use]
    pub fn at(&self, level: u16) -> Point {
        let level = level.max(self.level);
        let (u, u0) = (u128::from(level) + SHIFT, u128::from(self.level) + SHIFT);
        Point {
            level,
            bp: scaled(self.bp, u, u0, 3),
            ki: scaled(self.ki, u, u0, 1).min(HP_KI_MAX),
            max_hp: scaled(self.max_hp, u, u0, 1).min(HP_KI_MAX) as u16,
            exp: scaled(self.exp, u, u0, 2),
            byte_0e: scaled(self.byte_0e, u, u0, 1).min(BYTE_0E_MAX) as u8,
        }
    }

    /// The BP the level gives (the difference of the totals of the level and the one below it), for a level above the anchor.
    #[must_use]
    pub fn bp_gain(&self, level: u16) -> u64 {
        self.at(level).bp - self.at(level.saturating_sub(1)).bp
    }

    /// The KI the level adds to the maximum.
    #[must_use]
    pub fn ki_gain(&self, level: u16) -> u64 {
        self.at(level).ki - self.at(level.saturating_sub(1)).ki
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A character like the game's Goku: 47 rows, BP total 260 090, KI 138, HP 745, EXP 170 000, byte 8.
    fn goku() -> Anchor {
        Anchor { level: 47, bp: 260_090, ki: 138, max_hp: 745, exp: 170_000, byte_0e: 8 }
    }

    #[test]
    fn at_the_anchor_the_curve_is_the_games_own_values() {
        let a = goku();
        let p = a.at(47);
        assert_eq!((p.bp, p.ki, p.max_hp, p.exp, p.byte_0e), (260_090, 138, 745, 170_000, 8));
        assert_eq!(
            a.at(3),
            a.at(47),
            "a level below the anchor is the anchor (the game's own rows are used there)"
        );
    }

    #[test]
    fn every_number_grows_with_the_level_and_stays_in_its_width() {
        let a = goku();
        let mut before = a.at(47);
        for level in 48..=LEVEL_MAX {
            let now = a.at(level);
            assert!(
                now.bp >= before.bp && now.exp >= before.exp && now.ki >= before.ki,
                "level {level}"
            );
            assert!(now.max_hp >= before.max_hp, "level {level}");
            assert!(
                u64::from(now.max_hp) <= HP_KI_MAX
                    && now.ki <= HP_KI_MAX
                    && u64::from(now.byte_0e) <= BYTE_0E_MAX
            );
            assert!(u32::try_from(now.exp).is_ok(), "the EXP fits 32 bits at level {level}");
            assert!(now.bp < 1 << 40, "the BP fits 40 bits at level {level}");
            before = now;
        }
    }

    #[test]
    fn the_law_is_cubic_in_bp_and_linear_in_hp_at_the_top() {
        let a = goku();
        let top = a.at(999);
        // (1004 / 52)^3 = 7197.2 times the BP of level 47.
        assert!((1_860_000_000..1_880_000_000).contains(&top.bp), "{}", top.bp);
        // The EXP is (1004 / 52)^2 = 372.8 times 170 000.
        assert!((63_000_000..63_700_000).contains(&top.exp), "{}", top.exp);
        // HP 745 * 1004 / 52 = 14 384 is cut at four digits.
        assert_eq!(top.max_hp, 9_999);
        assert_eq!(a.at(100).max_hp, 1_504, "745 * 105 / 52 = 1504.3: linear in u below the cut");
    }

    #[test]
    fn a_gain_is_the_difference_of_two_totals_and_the_gains_sum_to_the_total() {
        let a = goku();
        let sum: u64 = (48..=300).map(|level| a.bp_gain(level)).sum();
        assert_eq!(a.at(300).bp - a.at(47).bp, sum);
        assert!(
            a.bp_gain(48) > 0 && a.ki_gain(300) <= 3,
            "a KI gain is a few points a level at most"
        );
        assert_eq!(a.bp_gain(48), a.at(48).bp - 260_090);
    }
}
