//! The language of the game's text: English (the patch's own text) or Traditional Chinese for Hong Kong, which the
//! Enhanced mode draws over the original's windows from a translation loaded at run time. Classic is always English.
//!
//! The choice is kept for this PC (`ssd.settings`) under [`Language::KEY`], as the Accessibility page's Language row
//! stores it: the index of the choice. Nothing stored means the player has not chosen yet (the first launch asks).

use crate::options::Values;

/// A language of the game's text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Language {
    /// The English of the patch: the game's own text, untouched.
    #[default]
    English,
    /// Traditional Chinese (Hong Kong): the translation's text over the original's windows (Enhanced only).
    ZhHk,
}

impl Language {
    /// The key of the choice in the PC file (the Accessibility page's row).
    pub const KEY: &'static str = "access.language";

    /// Every language, in the order of the row's choices (the stored index).
    pub const ALL: [Language; 2] = [Language::English, Language::ZhHk];

    /// The language stored in the PC file's `values`; `None` when none was chosen yet. A wild index is the last
    /// choice, as for any row.
    #[must_use]
    pub fn from_values(values: &Values) -> Option<Self> {
        let index = values.get(Self::KEY)?;
        let index = usize::try_from(index).unwrap_or(0).min(Self::ALL.len() - 1);
        Some(Self::ALL[index])
    }

    /// Stores the language in the PC file's `values`.
    pub fn store(self, values: &mut Values) {
        values.set(Self::KEY, self.index() as i64);
    }

    /// The index of the language among the row's choices.
    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&language| language == self).unwrap_or(0)
    }

    /// The language's code, as the command line's `--lang` takes it.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::ZhHk => "zh-hk",
        }
    }

    /// The language of a code (`en`, `zh-hk`; any case, `_` for `-`).
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        match code.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "en" | "english" => Some(Self::English),
            "zh-hk" => Some(Self::ZhHk),
            _ => None,
        }
    }

    /// The language's name in itself: what the first launch's choice says.
    #[must_use]
    pub const fn native_name(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::ZhHk => "繁體中文（香港）",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_stored_is_no_choice_yet_and_a_stored_choice_reads_back() {
        let mut values = Values::new();
        assert_eq!(Language::from_values(&values), None);
        Language::ZhHk.store(&mut values);
        assert_eq!(values.get(Language::KEY), Some(1));
        assert_eq!(Language::from_values(&values), Some(Language::ZhHk));
        Language::English.store(&mut values);
        assert_eq!(Language::from_values(&values), Some(Language::English));
        values.set(Language::KEY, 9);
        assert_eq!(
            Language::from_values(&values),
            Some(Language::ZhHk),
            "clamped to the last choice"
        );
        values.set(Language::KEY, -3);
        assert_eq!(Language::from_values(&values), Some(Language::English));
    }

    #[test]
    fn the_codes_round_trip() {
        for language in Language::ALL {
            assert_eq!(Language::from_code(language.code()), Some(language));
        }
        assert_eq!(Language::from_code("ZH_HK"), Some(Language::ZhHk));
        assert_eq!(Language::from_code("fr"), None);
    }

    #[test]
    fn the_row_of_the_accessibility_page_has_the_choices_in_this_order() {
        let row = crate::options::accessibility::ROWS_WITH_LANGUAGE
            .iter()
            .find(|row| row.key == Language::KEY)
            .expect("the Language row");
        assert_eq!(row.count(), Language::ALL.len());
        assert_eq!(row.text(Language::ZhHk.index()), "繁體中文");
    }
}
