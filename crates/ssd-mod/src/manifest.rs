//! `pack.toml`, the manifest of a pack: its identity, the packs it needs and the data it carries.
//!
//! Every key is known: an unknown key is an error, so a misspelt key is never a silent default.
//! Versions and the game's CRC32 are kept as text here and checked by [`crate::check_set`], which
//! names the pack and the value in its refusal.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// The whole of a `pack.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    /// The `[pack]` table: the identity of the pack and the ROM it is for.
    pub pack: PackInfo,
    /// The `[[depends]]` entries: packs that must be in the set, at least at their version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends: Vec<Dependency>,
    /// The `[[data]]` entries: one file of data for one slot of one family.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data: Vec<DataEntry>,
    /// The `[[scene]]` entries: one scene script for one slot of the `script` family.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scene: Vec<SceneEntry>,
    /// The `[[messages]]` entries: one file of messages, written in the message language (see [`crate::text`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<MessagesEntry>,
    /// The `[[map]]` entries: one Tiled map file for one pack map id (see [`crate::tiled`] and [`crate::map`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub map: Vec<MapEntry>,
    /// The `[[triggers]]` entries: one file of triggers (see [`crate::trigger`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub triggers: Vec<TriggersEntry>,
    /// The `[[tables]]` entries: one file of data tables, the units, attacks, groups and characters' levels (see [`crate::tables`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tables: Vec<TablesEntry>,
    /// The `[[chapters]]` entries: one file of chapters, each a start the title can offer (see [`crate::chapter`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chapters: Vec<ChaptersEntry>,
    /// The `[[map_art]]` entries: the HD picture tiles of a pack map (see [`crate::derive`]; drawn by Enhanced only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub map_art: Vec<MapArtEntry>,
    /// The `[[face]]` entries: the HD picture of one face index, from which its SNES art is derived (see [`crate::derive`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub face: Vec<FaceEntry>,
    /// The `[[feature]]` entries: a behaviour the game has by name, with its parameters (see [`crate::feature`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub feature: Vec<FeatureEntry>,
    /// The `[[pose_variant]]` entries: a picture drawn in place of a battle pose's art while a condition holds (see
    /// [`crate::pose`]; drawn by Enhanced only).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pose_variant: Vec<PoseVariantEntry>,
}

/// A `[[pose_variant]]` entry: the picture `file` (a PNG, relative to the pack) is drawn in place of the art of the battle pose
/// `art` while the condition `when` holds (see [`crate::pose`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoseVariantEntry {
    /// The name of the object's art the variant replaces: its file's stem in the HD art (`battle-goku-ssj-000`).
    pub art: String,
    /// The condition the game has, by name (`kaioken`, or `always`).
    pub when: String,
    /// The picture, relative to the pack: the object's picture grown by `margin` on each side.
    pub file: String,
    /// The margin, in art pixels on each side of the object's picture: a multiple of 4 (a whole game pixel), at most 96.
    pub margin: u32,
}

/// A `[[feature]]` entry: the behaviour the game has under `name`, with its parameters as the other keys of the table. The
/// parameters are checked against the feature by [`crate::feature::compile`], which refuses a key the feature does not have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureEntry {
    /// The feature's name (`ssj_kaioken`).
    pub name: String,
    /// The parameters: every key but `name`, each a whole number or a name the feature gives to a number.
    #[serde(flatten)]
    pub params: BTreeMap<String, ParamValue>,
}

/// The value of a feature's parameter as the manifest writes it: a whole number (`max_factor = 10`), or a name the feature gives
/// to a number (`difficulty = "hard"`). Which one a parameter takes is the feature's rule, checked by
/// [`crate::feature::compile`].
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    /// A whole number.
    Number(i64),
    /// A name.
    Name(String),
}

impl From<i64> for ParamValue {
    fn from(value: i64) -> Self {
        Self::Number(value)
    }
}

impl From<&str> for ParamValue {
    fn from(name: &str) -> Self {
        Self::Name(name.to_owned())
    }
}

/// The value as the manifest writes it (a bare number, or the name in quotes): the form a set id hashes, which for a number is
/// the form it had when the parameters were only numbers.
impl fmt::Debug for ParamValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Number(value) => write!(f, "{value}"),
            Self::Name(name) => write!(f, "{name:?}"),
        }
    }
}

/// A `[[map_art]]` entry: the HD picture tiles of pack map `map`. `tiles` is a file name relative to the pack, with `{x}`
/// and `{y}` for the tile's column and row: one tile per 256 x 256 pixels of the map (1024 x 1024 pixels each, at 4x).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapArtEntry {
    /// The pack map's id (its `[[map]]` entry).
    pub map: u8,
    /// The tiles' file name, with `{x}` and `{y}`.
    pub tiles: String,
}

impl MapArtEntry {
    /// The file name of the tile in column `x` and row `y`.
    #[must_use]
    pub fn tile_file(&self, x: usize, y: usize) -> String {
        self.tiles.replace("{x}", &x.to_string()).replace("{y}", &y.to_string())
    }
}

/// A `[[face]]` entry: the HD picture `hd` (a PNG, 128 x 128 pixels) is the picture of face `id`, the index the party
/// record and the corner portraits use. Its SNES art is derived from it at the palette row the game draws the face in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceEntry {
    /// The face index.
    pub id: u8,
    /// The HD picture, relative to the pack.
    pub hd: String,
}

/// A `[[triggers]]` entry: the file `file` (relative to the pack) holds the triggers of the pack, in the `[[trigger]]` form of
/// [`crate::trigger`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggersEntry {
    /// The file of the triggers, relative to the pack's folder or zip root.
    pub file: String,
}

/// A `[[tables]]` entry: the file `file` (relative to the pack) holds data tables in the form of [`crate::tables`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TablesEntry {
    /// The file of the tables, relative to the pack's folder or zip root.
    pub file: String,
}

/// A `[[chapters]]` entry: the file `file` (relative to the pack) holds the chapters of the pack, in the `[[chapter]]` form
/// of [`crate::chapter`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChaptersEntry {
    /// The file of the chapters, relative to the pack's folder or zip root.
    pub file: String,
}

/// A `[[map]]` entry: the Tiled map `file` (relative to the pack) is the pack map `id` (`64` to `127`, see
/// [`crate::map`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MapEntry {
    /// The pack map's id.
    pub id: u8,
    /// The Tiled map file (`.tmj`, JSON), relative to the pack's folder or zip root.
    pub file: String,
}

/// A `[[messages]]` entry: the file `file` (relative to the pack) holds messages in the `[[message]]` form of
/// [`crate::text`]. Each message fills its message slot, and its strings are placed by the image builder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessagesEntry {
    /// The file of the messages, relative to the pack's folder or zip root.
    pub file: String,
}

/// The `[pack]` table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PackInfo {
    /// The pack's id: 3 to 64 characters of `[a-z0-9._-]` (see [`valid_id`]).
    pub id: String,
    /// A name for people (the loader screen shows it).
    pub name: String,
    /// `major.minor.patch`, each a `u16` (see [`Version::parse`]).
    pub version: String,
    /// The CRC32 (hexadecimal) of the 1 MiB ROM the pack is for.
    pub game: String,
    /// Who made the pack, as the loader screen shows it (optional).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub author: String,
    /// What the pack does, in a few lines, as the loader screen shows it (optional).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// A `[[depends]]` entry: the pack `id`, at `version` or later (a minimum, `>=`, nothing else).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dependency {
    pub id: String,
    pub version: String,
}

/// A `[[data]]` entry: the file `file` (relative to the pack) is the data of slot `id` of `family`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DataEntry {
    /// A family name (`script` or `face`; the text families are filled by `[[messages]]`, see [`crate::Family::from_name`]).
    pub family: String,
    /// The slot's id within its family.
    pub id: u16,
    /// The file of the data, relative to the pack's folder or zip root.
    pub file: String,
}

/// A `[[scene]]` entry: the file `file` (relative to the pack) is the scene script of the scene `id`, written in
/// the scene language (see [`crate::scene`]). The scene is the `script` slot `id`, as a `[[data]]` entry of that
/// family would be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneEntry {
    /// The scene's id: the slot of the `script` family it fills.
    pub id: u16,
    /// The file of the script, relative to the pack's folder or zip root.
    pub file: String,
}

/// A version, `major.minor.patch`; ordered by major, then minor, then patch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl Version {
    /// The version of `text`, which is three numbers separated by dots, each a `u16`; `None` for
    /// anything else.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let mut next = || parts.next()?.parse::<u16>().ok();
        let version = Self { major: next()?, minor: next()?, patch: next()? };
        parts.next().is_none().then_some(version)
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Whether `id` is a valid pack id: 3 to 64 characters, each a lowercase letter, a digit, `.`, `_` or `-`.
#[must_use]
pub fn valid_id(id: &str) -> bool {
    (3..=64).contains(&id.len())
        && id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
[pack]
id = "example.room"
name = "Example room"
version = "1.0.0"
game = "A8CF4F2A"
[[depends]]
id = "other.pack"
version = "1.2.0"
[[data]]
family = "script"
id = 82
file = "data/script-082.bin"
"#;

    #[test]
    fn a_manifest_parses_with_its_tables() {
        let manifest: Manifest = toml::from_str(EXAMPLE).unwrap();
        assert_eq!(manifest.pack.id, "example.room");
        assert_eq!(manifest.pack.game, "A8CF4F2A");
        assert_eq!(
            manifest.depends,
            [Dependency { id: "other.pack".into(), version: "1.2.0".into() }]
        );
        assert_eq!(
            manifest.data,
            [DataEntry { family: "script".into(), id: 82, file: "data/script-082.bin".into() }]
        );
    }

    #[test]
    fn a_scene_entry_parses_and_a_misspelt_key_in_it_is_refused() {
        let text = format!("{EXAMPLE}\n[[scene]]\nid = 82\nfile = \"scenes/82.scene\"\n");
        let manifest: Manifest = toml::from_str(&text).unwrap();
        assert_eq!(manifest.scene, [SceneEntry { id: 82, file: "scenes/82.scene".into() }]);
        let misspelt = text.replace("file =", "fiel =");
        assert!(toml::from_str::<Manifest>(&misspelt).is_err(), "an unknown key in [[scene]]");
        let plain: Manifest = toml::from_str(EXAMPLE).unwrap();
        assert!(plain.scene.is_empty(), "no scene entries by default");
    }

    #[test]
    fn a_messages_entry_parses_and_a_misspelt_key_in_it_is_refused() {
        let text = format!("{EXAMPLE}\n[[messages]]\nfile = \"text/messages.toml\"\n");
        let manifest: Manifest = toml::from_str(&text).unwrap();
        assert_eq!(manifest.messages, [MessagesEntry { file: "text/messages.toml".into() }]);
        let misspelt = text.replace("file =", "fiel =");
        assert!(toml::from_str::<Manifest>(&misspelt).is_err(), "an unknown key in [[messages]]");
        let plain: Manifest = toml::from_str(EXAMPLE).unwrap();
        assert!(plain.messages.is_empty(), "no messages entries by default");
    }

    #[test]
    fn a_triggers_entry_parses_and_a_misspelt_key_in_it_is_refused() {
        let text = format!("{EXAMPLE}\n[[triggers]]\nfile = \"triggers.toml\"\n");
        let manifest: Manifest = toml::from_str(&text).unwrap();
        assert_eq!(manifest.triggers, [TriggersEntry { file: "triggers.toml".into() }]);
        let misspelt = text.replace("file =", "fiel =");
        assert!(toml::from_str::<Manifest>(&misspelt).is_err(), "an unknown key in [[triggers]]");
        let plain: Manifest = toml::from_str(EXAMPLE).unwrap();
        assert!(plain.triggers.is_empty(), "no triggers entries by default");
    }

    #[test]
    fn a_tables_entry_parses_and_a_misspelt_key_in_it_is_refused() {
        let text = format!("{EXAMPLE}\n[[tables]]\nfile = \"tables.toml\"\n");
        let manifest: Manifest = toml::from_str(&text).unwrap();
        assert_eq!(manifest.tables, [TablesEntry { file: "tables.toml".into() }]);
        let misspelt = text.replace("file =", "fiel =");
        assert!(toml::from_str::<Manifest>(&misspelt).is_err(), "an unknown key in [[tables]]");
        let plain: Manifest = toml::from_str(EXAMPLE).unwrap();
        assert!(plain.tables.is_empty(), "no tables entries by default");
    }

    #[test]
    fn a_chapters_entry_parses_and_a_misspelt_key_in_it_is_refused() {
        let text = format!("{EXAMPLE}\n[[chapters]]\nfile = \"chapters.toml\"\n");
        let manifest: Manifest = toml::from_str(&text).unwrap();
        assert_eq!(manifest.chapters, [ChaptersEntry { file: "chapters.toml".into() }]);
        let misspelt = text.replace("file =", "fiel =");
        assert!(toml::from_str::<Manifest>(&misspelt).is_err(), "an unknown key in [[chapters]]");
        let plain: Manifest = toml::from_str(EXAMPLE).unwrap();
        assert!(plain.chapters.is_empty(), "no chapters entries by default");
    }

    #[test]
    fn a_pose_variant_entry_parses_and_a_misspelt_key_in_it_is_refused() {
        let text = format!(
            "{EXAMPLE}\n[[pose_variant]]\nart = \"battle-goku-ssj-000\"\nwhen = \"kaioken\"\nfile = \"art/hd/variants/battle-goku-ssj-000.png\"\nmargin = 24\n"
        );
        let manifest: Manifest = toml::from_str(&text).unwrap();
        assert_eq!(
            manifest.pose_variant,
            [PoseVariantEntry {
                art: "battle-goku-ssj-000".into(),
                when: "kaioken".into(),
                file: "art/hd/variants/battle-goku-ssj-000.png".into(),
                margin: 24,
            }]
        );
        let always: Manifest =
            toml::from_str(&text.replace("when = \"kaioken\"", "when = \"always\"")).unwrap();
        assert_eq!(always.pose_variant[0].when, "always");
        let misspelt = text.replace("margin =", "margn =");
        assert!(
            toml::from_str::<Manifest>(&misspelt).is_err(),
            "an unknown key in [[pose_variant]]"
        );
        let plain: Manifest = toml::from_str(EXAMPLE).unwrap();
        assert!(plain.pose_variant.is_empty(), "no pose variants by default");
    }

    #[test]
    fn a_feature_entry_parses_with_its_parameters_as_keys_each_a_number_or_a_name() {
        let text = format!(
            "{EXAMPLE}\n[[feature]]\nname = \"ssj_kaioken\"\nmax_factor = 10\nhp_cost_percent = 10\n"
        );
        let manifest: Manifest = toml::from_str(&text).unwrap();
        let params = BTreeMap::from([
            ("max_factor".to_owned(), ParamValue::Number(10)),
            ("hp_cost_percent".to_owned(), ParamValue::Number(10)),
        ]);
        assert_eq!(manifest.feature, [FeatureEntry { name: "ssj_kaioken".into(), params }]);
        let plain: Manifest = toml::from_str(EXAMPLE).unwrap();
        assert!(plain.feature.is_empty(), "no feature entries by default");
        // A name parses as a name: whether the feature takes it is the feature's rule (`feature::compile`).
        let named =
            format!("{EXAMPLE}\n[[feature]]\nname = \"x\"\nlevel = \"hard\"\nmargin = -2\n");
        let manifest: Manifest = toml::from_str(&named).unwrap();
        let params = &manifest.feature[0].params;
        assert_eq!(params["level"], ParamValue::Name("hard".into()));
        assert_eq!(params["margin"], ParamValue::Number(-2));
        let fraction = format!("{EXAMPLE}\n[[feature]]\nname = \"x\"\nmax_factor = 1.5\n");
        assert!(toml::from_str::<Manifest>(&fraction).is_err(), "a fraction is neither");
    }

    #[test]
    fn a_parameter_is_written_as_the_manifest_writes_it_when_a_set_id_hashes_it() {
        // The set id hashes the parameters' `Debug` text: a number keeps the form it had when parameters were only numbers.
        let params = BTreeMap::from([
            ("a".to_owned(), ParamValue::Number(10)),
            ("b".to_owned(), ParamValue::Name("hard".into())),
        ]);
        assert_eq!(format!("{params:?}"), "{\"a\": 10, \"b\": \"hard\"}");
        assert_eq!(ParamValue::from(7), ParamValue::Number(7));
        assert_eq!(ParamValue::from("easy"), ParamValue::Name("easy".into()));
    }

    #[test]
    fn an_unknown_key_is_refused_wherever_it_is() {
        let top = format!("{EXAMPLE}\nextra = 1\n");
        assert!(toml::from_str::<Manifest>(&top).is_err(), "an unknown top-level key");
        let in_pack = EXAMPLE.replace("game =", "gaem =");
        assert!(toml::from_str::<Manifest>(&in_pack).is_err(), "a misspelt key in [pack]");
        let in_data = EXAMPLE.replace("id = 82", "id = 82\nlevel = 3");
        assert!(toml::from_str::<Manifest>(&in_data).is_err(), "an unknown key in [[data]]");
    }

    #[test]
    fn versions_parse_as_three_numbers_and_order_by_major_minor_patch() {
        assert_eq!(Version::parse("1.2.3"), Some(Version { major: 1, minor: 2, patch: 3 }));
        for bad in ["1.2", "1.2.3.4", "1.2.x", "1.2.-3", "1.2.70000", "", "1..3"] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert!(Version::parse("1.10.0") > Version::parse("1.9.9"));
        assert!(Version::parse("2.0.0") > Version::parse("1.99.99"));
        assert_eq!(Version::parse("1.2.3").unwrap().to_string(), "1.2.3");
    }

    #[test]
    fn ids_are_three_to_sixty_four_of_the_allowed_characters() {
        assert!(valid_id("abc"));
        assert!(valid_id("example.room-2_b"));
        assert!(valid_id(&"a".repeat(64)));
        assert!(!valid_id("ab"));
        assert!(!valid_id(&"a".repeat(65)));
        assert!(!valid_id("Example"), "uppercase");
        assert!(!valid_id("a b c"), "a space");
        assert!(!valid_id("a/b/c"), "a path separator");
    }
}
