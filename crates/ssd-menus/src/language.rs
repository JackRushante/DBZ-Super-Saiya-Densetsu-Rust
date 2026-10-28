//! The language of the game's text: English (the patch's own text), or a translation the player has, which the
//! Enhanced mode draws over the original's windows from files loaded at run time. Classic is always English.
//!
//! A language other than English is named by its code (`zh-hk`, `xx`...), the name of its files: `<code>.tsv` for
//! the game's text and `ui-<code>.tsv` for the HD menus. The app finds them in the data folder and offers them
//! ([`crate::options::offer_languages`]), each with the name its file gives ([`Locale`]).
//!
//! The choice is kept for this PC (`ssd.settings`) under [`Language::KEY`], as the Accessibility page's Language row
//! stores it: the language's code. The first versions stored the index of the choice instead (0 English, 1
//! Traditional Chinese for Hong Kong); such an index still reads as its language, and [`Language::migrate`] turns
//! it into the code once. Nothing stored means the player has not chosen yet (the first launch asks).

use crate::options::Values;
use std::sync::Mutex;

/// A language of the game's text: its code (see [`Language::from_code`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Language {
    code: &'static str,
}

impl Default for Language {
    fn default() -> Self {
        Self::ENGLISH
    }
}

/// The codes read so far, each kept once for the run (a language is `Copy`, so its code lives as long as the game).
static CODES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// The longest code taken.
const CODE_MAX: usize = 32;

impl Language {
    /// The key of the choice in the PC file (the Accessibility page's row).
    pub const KEY: &'static str = "access.language";

    /// The English of the patch: the game's own text, untouched. Built in: it has no files.
    pub const ENGLISH: Self = Self { code: "en" };

    /// Traditional Chinese for Hong Kong (`zh-hk.tsv`), the first translation: the language the stored index 1 of
    /// the first versions is, and the one whose name is known without a `#name` line ([`Language::default_name`]).
    pub const ZH_HK: Self = Self { code: "zh-hk" };

    /// The language stored in the PC file's `values`; `None` when none was chosen yet. A code that is not one reads
    /// as English; an index of the first versions as its language (a wild one as the last, as it did).
    #[must_use]
    pub fn from_values(values: &Values) -> Option<Self> {
        if let Some(code) = values.word(Self::KEY) {
            return Some(Self::from_code(code).unwrap_or_default());
        }
        values.get(Self::KEY).map(|index| if index >= 1 { Self::ZH_HK } else { Self::ENGLISH })
    }

    /// Stores the language in the PC file's `values`: its code.
    pub fn store(self, values: &mut Values) {
        values.set_word(Self::KEY, self.code);
    }

    /// Turns an index the first versions stored into the language's code; `true` when `values` changed (the PC
    /// file is to be written once).
    pub fn migrate(values: &mut Values) -> bool {
        if values.get(Self::KEY).is_none() {
            return false;
        }
        Self::from_values(values).unwrap_or_default().store(values);
        true
    }

    /// The language's code, as the command line's `--lang` takes it and as its files are named.
    #[must_use]
    pub const fn code(self) -> &'static str {
        self.code
    }

    /// Whether the language is English (built in: no translation is read).
    #[must_use]
    pub fn is_english(self) -> bool {
        self == Self::ENGLISH
    }

    /// The language of a code: `en` (or `english`) is English, any other code is a translation's. Any case, `_` for
    /// `-`. `None` for a code that cannot name a file: empty, longer than 32, with a character other than a letter,
    /// a digit or `-`, or starting with `ui-` (the HD menus' files).
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        let code = code.trim().to_ascii_lowercase().replace('_', "-");
        if code == "en" || code == "english" {
            return Some(Self::ENGLISH);
        }
        let fine = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
        if code.is_empty()
            || code.len() > CODE_MAX
            || !code.chars().all(fine)
            || code.starts_with("ui-")
        {
            return None;
        }
        if code == Self::ZH_HK.code {
            return Some(Self::ZH_HK);
        }
        let mut codes = CODES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let code = match codes.iter().find(|&&known| known == code) {
            Some(&known) => known,
            None => {
                let leaked: &'static str = Box::leak(code.into_boxed_str());
                codes.push(leaked);
                leaked
            }
        };
        Some(Self { code })
    }

    /// The name the language shows when its file gives none: `English`, `繁體中文（香港）` for Hong Kong (as the
    /// first versions named it), else the code.
    #[must_use]
    pub fn default_name(self) -> String {
        match self.code {
            "en" => "English".to_owned(),
            "zh-hk" => "繁體中文（香港）".to_owned(),
            code => code.to_owned(),
        }
    }
}

/// A language offered to the player: the language, and its name in itself (its file's `#name`, else
/// [`Language::default_name`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locale {
    pub language: Language,
    pub name: String,
}

impl Locale {
    /// English, named so.
    #[must_use]
    pub fn english() -> Self {
        Self::unnamed(Language::ENGLISH)
    }

    /// `language` with the name it shows when its file gives none.
    #[must_use]
    pub fn unnamed(language: Language) -> Self {
        Self { language, name: language.default_name() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_stored_is_no_choice_yet_and_a_stored_choice_reads_back_as_its_code() {
        let mut values = Values::new();
        assert_eq!(Language::from_values(&values), None);
        let made_up = Language::from_code("xx").unwrap();
        made_up.store(&mut values);
        assert_eq!(values.word(Language::KEY), Some("xx"), "the code, not an index");
        assert_eq!(Language::from_values(&values), Some(made_up));
        Language::ENGLISH.store(&mut values);
        assert_eq!(values.word(Language::KEY), Some("en"));
        assert_eq!(Language::from_values(&values), Some(Language::ENGLISH));
        values.set_word(Language::KEY, "not a code");
        assert_eq!(
            Language::from_values(&values),
            Some(Language::ENGLISH),
            "a bad code is English"
        );
    }

    #[test]
    fn an_index_of_the_first_versions_reads_as_its_language_and_is_migrated_once() {
        let mut values = Values::new();
        assert!(!Language::migrate(&mut values), "nothing stored: nothing to do");
        for (index, language) in [
            (0, Language::ENGLISH),
            (1, Language::ZH_HK),
            (9, Language::ZH_HK),
            (-3, Language::ENGLISH),
        ] {
            values.set(Language::KEY, index);
            assert_eq!(Language::from_values(&values), Some(language), "index {index}");
            assert!(Language::migrate(&mut values), "index {index} is migrated");
            assert_eq!(values.get(Language::KEY), None);
            assert_eq!(values.word(Language::KEY), Some(language.code()));
            assert!(!Language::migrate(&mut values), "once");
            assert_eq!(Language::from_values(&values), Some(language));
        }
        let mut values = Values::from_text("ssd-settings 1\naccess.language=1\n").unwrap();
        assert!(Language::migrate(&mut values));
        assert_eq!(values.to_text(), "ssd-settings 1\naccess.language=zh-hk\n");
    }

    #[test]
    fn a_code_names_a_file_and_reads_back_the_same_language() {
        assert_eq!(Language::from_code("EN"), Some(Language::ENGLISH));
        assert_eq!(Language::from_code("english"), Some(Language::ENGLISH));
        assert_eq!(Language::from_code("ZH_HK"), Some(Language::ZH_HK));
        let made_up = Language::from_code(" Xx-Yy ").unwrap();
        assert_eq!(made_up.code(), "xx-yy");
        assert_eq!(Language::from_code("xx-yy"), Some(made_up));
        assert!(!made_up.is_english() && Language::ENGLISH.is_english());
        for bad in ["", "x/y", "x.y", "ui-xx", "x y", "é", &"x".repeat(33)] {
            assert_eq!(Language::from_code(bad), None, "{bad:?}");
        }
        assert_eq!(Language::default(), Language::ENGLISH);
    }

    #[test]
    fn a_language_without_a_name_is_named_by_its_code_and_hong_kong_as_before() {
        assert_eq!(Locale::english().name, "English");
        assert_eq!(Locale::unnamed(Language::ZH_HK).name, "繁體中文（香港）");
        assert_eq!(Locale::unnamed(Language::from_code("xx").unwrap()).name, "xx");
    }
}
