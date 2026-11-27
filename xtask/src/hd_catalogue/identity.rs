//! Who an entry is (`tools/hd-identity.toml`) and what an art file says it shows.

use super::{Category, Entry};
use anyhow::Context;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

/// What the label of the art must say for an identity.
#[derive(Debug, Default, Deserialize)]
struct Def {
    family: Option<String>,
    #[serde(default)]
    words: Vec<String>,
    #[serde(default)]
    form_words: Vec<String>,
    #[serde(default)]
    not_words: Vec<String>,
    /// Plain words for what the character looks like, for whoever draws it again.
    describe: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct File {
    #[serde(default)]
    units: BTreeMap<String, String>,
    #[serde(default)]
    forms: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    identities: BTreeMap<String, Def>,
    #[serde(default)]
    art: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    portraits: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    objects: BTreeMap<String, String>,
}

/// How the identity an art file is made for compares with the identity of an entry it draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Compare {
    /// The art says nothing about who it is made for.
    Unknown,
    /// The same look.
    Same,
    /// Another unit of the same character whose look is not told apart yet.
    SameFamily,
    /// Two looks of one character that are told apart (a Super Saiyan and a normal one).
    OtherForm(String),
    /// Another character.
    Other(String),
}

/// The identity table.
#[derive(Debug, Default)]
pub struct Identity {
    file: File,
}

/// The unit number an art stem is made for: `gallery-enemy-vegeta-u75` is of 75.
fn unit_tag(stem: &str) -> Option<u8> {
    let (_, tag) = stem.rsplit_once("-u")?;
    if tag.is_empty() || !tag.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    tag.parse().ok()
}

/// The slug without its unit number: `vegeta-u74` is `vegeta`.
fn without_unit(slug: &str) -> &str {
    match slug.rsplit_once("-u") {
        Some((name, tag)) if !tag.is_empty() && tag.bytes().all(|b| b.is_ascii_digit()) => name,
        _ => slug,
    }
}

impl Identity {
    /// Reads the table.
    ///
    /// # Errors
    ///
    /// The file cannot be read or parsed.
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let file: File =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self { file })
    }

    /// The identity slugs of an entry: its unit's, or the portrait's.
    #[must_use]
    pub fn of_entry(&self, entry: &Entry) -> Vec<String> {
        match entry.category {
            Category::BattlePose => entry
                .units
                .iter()
                .filter_map(|unit| self.file.units.get(&unit.to_string()).cloned())
                .collect(),
            Category::Face | Category::ObjectPortrait => {
                let mut found = Vec::new();
                if let (Some(stream), Some(half)) = (&entry.stream, entry.half) {
                    found.extend(
                        self.file
                            .portraits
                            .get(&format!("{stream}/{half}"))
                            .cloned()
                            .unwrap_or_default(),
                    );
                }
                if let Some(slug) = entry.object.and_then(|o| self.file.objects.get(&o.to_string()))
                    && !found.contains(slug)
                {
                    found.push(slug.clone());
                }
                found
            }
        }
    }

    /// The identity of unit `unit`.
    #[must_use]
    pub fn of_unit(&self, unit: u8) -> Option<&str> {
        self.file.units.get(&unit.to_string()).map(String::as_str)
    }

    /// The character a slug is a look of.
    #[must_use]
    pub fn family<'a>(&'a self, slug: &'a str) -> &'a str {
        if let Some(family) = self.file.identities.get(slug).and_then(|def| def.family.as_deref()) {
            return family;
        }
        let name = without_unit(slug);
        name.strip_suffix("-ssj").unwrap_or(name)
    }

    /// The slugs an art id is made for: by the unit number in its name, or the table's `[art]`.
    #[must_use]
    pub fn art_slugs(&self, art_id: &str) -> Vec<String> {
        let stem = super::verify::stem_of(art_id);
        if let Some(unit) = unit_tag(&stem) {
            return self.of_unit(unit).map(str::to_owned).into_iter().collect();
        }
        self.file.art.get(&stem).cloned().unwrap_or_default()
    }

    /// What the character looks like in plain words, when the table says (the slug's own, else its character's).
    #[must_use]
    pub fn describe(&self, slug: &str) -> Option<&str> {
        let own = self.file.identities.get(slug).and_then(|d| d.describe.as_deref());
        own.or_else(|| {
            self.file.identities.get(self.family(slug)).and_then(|d| d.describe.as_deref())
        })
    }

    /// The unit an art id is made for when its name numbers one (`gallery-enemy-vegeta-u75-005`).
    #[must_use]
    pub fn art_unit(art_id: &str) -> Option<u8> {
        unit_tag(&super::verify::stem_of(art_id))
    }

    /// Whether two identities are the same character (the family of a slug is its name).
    #[must_use]
    pub fn same_family(&self, a: &str, b: &str) -> bool {
        self.family(a) == self.family(b)
    }

    /// How the art made for `art` compares with an entry of identity `slug`.
    #[must_use]
    pub fn compare(&self, slug: &str, art: &[String]) -> Compare {
        if art.is_empty() {
            return Compare::Unknown;
        }
        if art.iter().any(|a| a == slug) {
            return Compare::Same;
        }
        let family = self.family(slug);
        let mut same_family = false;
        for other in art {
            if self.family(other) == family {
                let told_apart = self.file.forms.get(family).is_some_and(|forms| {
                    forms.iter().any(|f| f == slug) && forms.iter().any(|f| f == other)
                });
                if told_apart {
                    return Compare::OtherForm(other.clone());
                }
                same_family = true;
            }
        }
        if same_family { Compare::SameFamily } else { Compare::Other(art[0].clone()) }
    }

    /// Whether two slugs are told apart as looks of one character (`vegeta`, `vegeta-ssj`).
    #[must_use]
    pub fn told_apart(&self, a: &str, b: &str) -> bool {
        let family = self.family(a);
        a != b
            && family == self.family(b)
            && self
                .file
                .forms
                .get(family)
                .is_some_and(|forms| forms.iter().any(|f| f == a) && forms.iter().any(|f| f == b))
    }

    /// What is wrong with `label` (the text the art's entry says about it) for an entry of identity
    /// `slug`, if anything: it names nobody it should, or a look it should not.
    #[must_use]
    pub fn label_problem(&self, slug: &str, label: &str) -> Option<String> {
        let label = label.to_lowercase();
        let def = self.file.identities.get(slug);
        let default = [slug.to_owned(), self.family(slug).to_owned()];
        let words: &[String] = match def {
            Some(def) if !def.words.is_empty() => &def.words,
            _ => &default,
        };
        if !words.iter().any(|word| label.contains(&word.to_lowercase())) {
            return Some(format!(
                "the label does not name {slug} (looked for: {})",
                words.join(", ")
            ));
        }
        if let Some(def) = def {
            if !def.form_words.is_empty()
                && !def.form_words.iter().any(|word| label.contains(&word.to_lowercase()))
            {
                return Some(format!(
                    "the label does not describe the {slug} look (looked for: {})",
                    def.form_words.join(", ")
                ));
            }
            if let Some(word) =
                def.not_words.iter().find(|word| label.contains(&word.to_lowercase()))
            {
                return Some(format!("the label says \"{word}\", which {slug} is not"));
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Identity {
        let file: File = toml::from_str(
            r#"
[units]
15 = "vegeta"
75 = "vegeta-ssj"
74 = "vegeta-u74"
[forms]
vegeta = ["vegeta", "vegeta-ssj"]
[identities.vegeta-ssj]
words = ["vegeta"]
form_words = ["super saiyan"]
[identities.vegeta]
words = ["vegeta"]
not_words = ["super saiyan"]
[art]
battle-vegeta = ["vegeta"]
"#,
        )
        .unwrap();
        Identity { file }
    }

    #[test]
    fn art_is_made_for_the_unit_its_name_numbers_or_the_table_says() {
        let table = table();
        assert_eq!(table.art_slugs("gallery-enemy-vegeta-u75-005"), ["vegeta-ssj"]);
        assert_eq!(table.art_slugs("gallery-enemy-vegeta-u15-005"), ["vegeta"]);
        assert_eq!(table.art_slugs("battle-vegeta-012"), ["vegeta"]);
        assert_eq!(table.art_slugs("battle-unknown-001"), Vec::<String>::new());
    }

    #[test]
    fn a_super_saiyan_and_a_normal_look_are_told_apart_but_unnumbered_units_are_not() {
        let table = table();
        let normal = table.art_slugs("gallery-enemy-vegeta-u15-001");
        assert_eq!(table.compare("vegeta-ssj", &normal), Compare::OtherForm("vegeta".to_owned()));
        assert_eq!(table.compare("vegeta", &normal), Compare::Same);
        assert_eq!(table.compare("vegeta-u74", &normal), Compare::SameFamily);
        assert_eq!(table.compare("vegeta", &[]), Compare::Unknown);
    }

    #[test]
    fn a_label_must_name_the_identity_and_its_look() {
        let table = table();
        assert!(table.label_problem("vegeta", "Vegeta (black flame-shaped hair)").is_none());
        assert!(table.label_problem("vegeta", "Super Saiyan Vegeta").is_some());
        assert!(table.label_problem("vegeta-ssj", "Vegeta (black flame-shaped hair)").is_some());
        assert!(table.label_problem("vegeta-ssj", "Super Saiyan Vegeta (golden hair)").is_none());
        assert!(table.label_problem("dende", "a green Namekian warrior with antennae").is_some());
    }
}
