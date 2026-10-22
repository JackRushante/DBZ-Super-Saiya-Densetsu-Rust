//! The pose variants of a pack (M10, batch 6b): `[[pose_variant]]`, a picture drawn in place of the art of a battle pose while a
//! condition of the game holds.
//!
//! ```toml
//! [[pose_variant]]                  # one per variant: the object's art it replaces, and when
//! art = "battle-goku-ssj-000"
//! when = "kaioken"                  # a condition the game has, by name (see [`Condition`])
//! file = "art/hd/variants/battle-goku-ssj-000.png"
//! margin = 24                       # art pixels on each side of the object's picture (a whole number of game pixels)
//! ```
//!
//! A variant is HD only: the Enhanced picture draws it (`ssd-hd`), over the box of the object's picture grown by the margin.
//! The pack holds no code. Its file is not needed by the checks: a variant whose file is missing is not drawn.

use crate::manifest::PoseVariantEntry;

/// The conditions a variant can name. The Enhanced picture reads each one from the game's work RAM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Condition {
    /// The Kaioken of the Super Saiyan Goku is on: `$0C36` bit 7 is set, and a party record (`$0B00 + 0x20 * k`, for the
    /// nine records) has unit `$3B` at `+01` (`docs/re/data/kaioken-bp.md`, section 4).
    Kaioken,
}

/// The names of the conditions, each with its condition.
const NAMES: &[(&str, Condition)] = &[("kaioken", Condition::Kaioken)];

impl Condition {
    /// The condition a manifest names by `name`, or `None` for a name the game does not have.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        NAMES.iter().find(|(known, _)| *known == name).map(|&(_, condition)| condition)
    }
}

/// The largest margin, in art pixels on each side of the object's picture (24 game pixels).
pub const MAX_MARGIN: u32 = 96;

/// Why a variant entry is refused, if it is: an empty `art` or `file`, a condition the game does not have (the message lists
/// the known ones), or a margin that is not a multiple of 4 (a whole game pixel) up to [`MAX_MARGIN`].
#[must_use]
pub fn refusal(entry: &PoseVariantEntry) -> Option<String> {
    if entry.art.is_empty() || entry.file.is_empty() {
        return Some("`art` and `file` must name the object and its picture".to_owned());
    }
    if Condition::from_name(&entry.when).is_none() {
        let names: Vec<&str> = NAMES.iter().map(|&(name, _)| name).collect();
        return Some(format!(
            "the condition `{}` is not known; the known conditions are: {}",
            entry.when,
            names.join(", ")
        ));
    }
    if !entry.margin.is_multiple_of(4) || entry.margin > MAX_MARGIN {
        return Some(format!(
            "`margin` is {}; it must be a multiple of 4 up to {MAX_MARGIN} (whole game pixels)",
            entry.margin
        ));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(when: &str, margin: u32) -> PoseVariantEntry {
        PoseVariantEntry {
            art: "battle-goku-ssj-000".to_owned(),
            when: when.to_owned(),
            file: "art/hd/variants/battle-goku-ssj-000.png".to_owned(),
            margin,
        }
    }

    #[test]
    fn a_known_condition_is_read_by_its_name_and_another_is_refused_with_the_known_ones() {
        assert_eq!(Condition::from_name("kaioken"), Some(Condition::Kaioken));
        assert_eq!(Condition::from_name("Kaioken"), None, "the names are lower case");
        assert_eq!(refusal(&entry("kaioken", 24)), None);
        let refused = refusal(&entry("bloom", 24)).unwrap();
        assert!(refused.contains("`bloom`") && refused.contains("kaioken"), "{refused}");
    }

    #[test]
    fn a_margin_is_a_whole_number_of_game_pixels_up_to_the_limit() {
        assert_eq!(refusal(&entry("kaioken", 0)), None, "no margin is allowed");
        assert_eq!(refusal(&entry("kaioken", MAX_MARGIN)), None);
        assert!(refusal(&entry("kaioken", 23)).is_some(), "not a whole game pixel");
        assert!(refusal(&entry("kaioken", MAX_MARGIN + 4)).is_some(), "past the limit");
    }

    #[test]
    fn an_entry_without_its_art_or_its_file_is_refused() {
        let mut no_art = entry("kaioken", 24);
        no_art.art.clear();
        assert!(refusal(&no_art).is_some());
        let mut no_file = entry("kaioken", 24);
        no_file.file.clear();
        assert!(refusal(&no_file).is_some());
    }
}
