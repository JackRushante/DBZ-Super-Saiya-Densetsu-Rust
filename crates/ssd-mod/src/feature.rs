//! The named features of a pack (M10, Task 10): a behaviour the game has by name, which a pack selects in `pack.toml`.
//!
//! ```toml
//! [[feature]]                       # one per feature; the name selects a behaviour the game has
//! name = "ssj_kaioken"
//! max_factor = 10                   # its parameters: the keys of the same table, all required
//! hp_cost_percent = 10
//! ```
//!
//! A pack holds no code: the behaviour is in the game (`ssd-game::mods::features`), and the pack selects it and gives its
//! parameters. Each feature is one slot of [`Family::Feature`], numbered by its id here, whose bytes are its parameters,
//! one byte each, in the order of the table. The game finds the slot as it finds any other, so a feature whose slot is not
//! in the image (no pack, or a pack without it) is off, and its hooks decline.
//!
//! A parameter is a whole number in a range, or one of the names the feature gives to its values (`difficulty = "hard"`):
//! the names of a parameter are in [`NAMED`], and a parameter that has names takes only them.
//!
//! A feature has one **set** of parameters, or more than one when it grew a second way to be set while its first one stays
//! valid (`enemy_scaling`: `difficulty`, or `random_offset` with `boss_offset`). A manifest gives every parameter of exactly one
//! set; the keys of two sets together are refused. A set may begin its slot with fixed bytes (its `prefix`) that tell the game
//! which set the slot is: the first set has none, so a pack written for it compiles to the same bytes as before.

use crate::Family;
use crate::manifest::{FeatureEntry, ParamValue};
use crate::tables::Compiled;
use std::collections::BTreeSet;
use std::fmt;

/// The id of `ssj_kaioken`, the Kaioken of a Super Saiyan (`docs/re/data/kaioken-bp.md`). The features are numbered from 1.
pub const SSJ_KAIOKEN: u16 = 1;
/// The id of `enemy_scaling`, the enemies that follow the party's level (`docs/mods/free-progression.md`).
pub const ENEMY_SCALING: u16 = 2;

/// The id of `free_progression`, the story without level, EXP or BP gates (`docs/re/data/free-progression.md`). The ids are
/// not contiguous: the number of a feature is only its slot.
pub const FREE_PROGRESSION: u16 = 3;

/// The id of `party_boosts`, the Guru and Porunga's level wish objects that act on the whole party
/// (`docs/mods/features.md`).
pub const PARTY_BOOSTS: u16 = 4;

/// The first byte of the slot of `free_progression` when it has a `level_cap`; the values 0 to 3 are the first set's `ginyu_arrival`.
pub const LEVEL_CAP_SET: u8 = 0x80;

/// The first byte of the slot of `enemy_scaling` when it is set by offsets (`random_offset`, `boss_offset`); the values 0 to 2
/// are the difficulties of the first set, which has no prefix.
pub const SCALING_OFFSETS: u8 = 3;

/// One parameter of a feature: its key in the manifest and the values it may take (a byte each).
struct Parameter {
    name: &'static str,
    min: i64,
    max: i64,
    /// The bytes the value takes in the slot: 1, or 2 for a value that does not fit a byte (little endian).
    width: usize,
}

/// One way to set a feature: the bytes its slot begins with, then its parameters in the slot's byte order, all required.
struct ParameterSet {
    prefix: &'static [u8],
    parameters: &'static [Parameter],
}

/// A feature the game has: its name in a manifest, its slot's id, and the sets of parameters it accepts (at least one).
struct Known {
    name: &'static str,
    id: u16,
    sets: &'static [ParameterSet],
}

/// The features the game has. A manifest that names another one is refused, with this list.
const KNOWN: &[Known] = &[
    Known {
        name: "ssj_kaioken",
        id: SSJ_KAIOKEN,
        sets: &[ParameterSet {
            prefix: &[],
            parameters: &[
                // The largest factor of the Kaioken is `1 + m / 2` with `m` at most 20 (the table `1D:8E7B`), so 11 is the top:
                // the cap on `m` is `2 * (max_factor - 1)`, and a larger `max_factor` would cap nothing.
                Parameter { name: "max_factor", min: 1, max: 11, width: 1 },
                Parameter { name: "hp_cost_percent", min: 0, max: 100, width: 1 },
            ],
        }],
    },
    Known {
        name: "free_progression",
        id: FREE_PROGRESSION,
        sets: &[
            ParameterSet {
                prefix: &[],
                parameters: &[
                    // Which of the three arrivals of Goku in the Ginyu squad's chapter the story gives: 0 is the original's rule
                    // (his level decides), 1 to 3 are the arrival during the first fight, the second, or at the last one.
                    Parameter { name: "ginyu_arrival", min: 0, max: 3, width: 1 },
                ],
            },
            // The same with the level cap: the slot is [`LEVEL_CAP_SET`], the arrival, and the cap as two bytes. A pack that does
            // not give `level_cap` keeps the first set (and the game's own cap, 47 or 48 per character).
            ParameterSet {
                prefix: &[LEVEL_CAP_SET],
                parameters: &[
                    Parameter { name: "ginyu_arrival", min: 0, max: 3, width: 1 },
                    // The highest level a character reaches (the party's growth past the game's own table is extrapolated,
                    // `docs/mods/free-progression.md`).
                    Parameter { name: "level_cap", min: 48, max: 999, width: 2 },
                ],
            },
        ],
    },
    Known {
        name: "party_boosts",
        id: PARTY_BOOSTS,
        // Each is a switch: 1 makes the object act on every member of the party, 0 leaves it on the one the player chose.
        sets: &[ParameterSet {
            prefix: &[],
            parameters: &[
                Parameter { name: "guru", min: 0, max: 1, width: 1 },
                Parameter { name: "porunga_level", min: 0, max: 1, width: 1 },
                Parameter { name: "lunch", min: 0, max: 1, width: 1 },
            ],
        }],
    },
    Known {
        name: "enemy_scaling",
        id: ENEMY_SCALING,
        sets: &[
            // The first set, the only one up to the pack 0.2.0 of Free progression: the difficulty is one of the names of
            // [`NAMED`], easy, normal or hard (the byte is 0, 1 or 2), a factor on the party's level.
            ParameterSet {
                prefix: &[],
                parameters: &[Parameter { name: "difficulty", min: 0, max: 2, width: 1 }],
            },
            // The second: two offsets in levels from the party's, one for the random battles and one for every other battle.
            // The slot is [`SCALING_OFFSETS`] and the two offsets as signed bytes (two's complement).
            ParameterSet {
                prefix: &[SCALING_OFFSETS],
                parameters: &[
                    Parameter { name: "random_offset", min: -10, max: 10, width: 1 },
                    Parameter { name: "boss_offset", min: -10, max: 10, width: 1 },
                ],
            },
        ],
    },
];

/// The names one parameter of one feature takes in place of its numbers, and the number each stands for.
struct Names {
    feature: &'static str,
    parameter: &'static str,
    names: &'static [(&'static str, i64)],
}

/// The parameters that have names. A parameter with names takes only them (the manifest says `difficulty = "hard"`, not `2`),
/// and a parameter without takes only whole numbers.
const NAMED: &[Names] = &[Names {
    feature: "enemy_scaling",
    parameter: "difficulty",
    names: &[("easy", 0), ("normal", 1), ("hard", 2)],
}];

/// The names of a feature's parameter and the values they stand for, or `None` for a parameter that takes numbers.
fn names_of(feature: &str, parameter: &str) -> Option<&'static [(&'static str, i64)]> {
    NAMED
        .iter()
        .find(|known| known.feature == feature && known.parameter == parameter)
        .map(|known| known.names)
}

/// The names the game knows, in the order of [`KNOWN`].
#[must_use]
pub fn known_names() -> Vec<&'static str> {
    KNOWN.iter().map(|known| known.name).collect()
}

/// Why a feature of a manifest is refused. Each names the feature, and the parameter or the rule that is wrong.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FeatureError {
    /// The name is not one the game has (the message lists the known names).
    Unknown { name: String },
    /// The same feature is given twice.
    Twice { name: String },
    /// A parameter is missing, unknown, not a whole number or out of its range.
    Parameter { feature: String, message: String },
}

impl fmt::Display for FeatureError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown { name } => write!(
                f,
                "the feature `{name}` is not known; the known features are: {}",
                known_names().join(", ")
            ),
            Self::Twice { name } => write!(f, "the feature `{name}` is given twice"),
            Self::Parameter { feature, message } => {
                write!(f, "the feature `{feature}`: {message}")
            }
        }
    }
}

impl std::error::Error for FeatureError {}

/// How the sets of a feature are named in a message: the parameters of each set, the sets joined with `or`.
fn describe_sets(known: &Known) -> String {
    let each = |set: &ParameterSet| {
        let names: Vec<&str> = set.parameters.iter().map(|p| p.name).collect();
        names.join(", ")
    };
    known.sets.iter().map(each).collect::<Vec<_>>().join(" or ")
}

/// The set of parameters an entry gives. A key that is in no set is refused, and so are keys that no one set holds together;
/// the first set that holds every key given is the one (an entry with no key is taken for the first set, and the message then
/// names that set's first parameter as missing). A key two sets share (`ginyu_arrival`) does not choose between them.
fn choose_set<'a>(known: &'a Known, entry: &FeatureEntry) -> Result<&'a ParameterSet, String> {
    let in_a_set = |key: &String| {
        known.sets.iter().flat_map(|set| set.parameters).any(|p| p.name == key.as_str())
    };
    if let Some(key) = entry.params.keys().find(|key| !in_a_set(key)) {
        return Err(format!(
            "`{key}` is not a parameter of it; its parameters are {}",
            describe_sets(known)
        ));
    }
    known
        .sets
        .iter()
        .find(|set| {
            entry.params.keys().all(|key| set.parameters.iter().any(|p| p.name == key.as_str()))
        })
        .ok_or_else(|| {
            format!(
                "its parameters are mixed: give all of one set and none of the other, {}",
                describe_sets(known)
            )
        })
}

/// The slots of the `[[feature]]` entries of a pack: one per entry, its bytes the parameters in the table's order.
///
/// # Errors
///
/// [`FeatureError::Unknown`] for a name the game does not have, [`FeatureError::Twice`] for a feature given twice, and
/// [`FeatureError::Parameter`] for a parameter that is missing, unknown, not a whole number, out of its range, or from two
/// sets of parameters at once.
pub fn compile(entries: &[FeatureEntry]) -> Result<Vec<Compiled>, FeatureError> {
    let mut seen = BTreeSet::new();
    let mut slots = Vec::with_capacity(entries.len());
    for entry in entries {
        let known = KNOWN
            .iter()
            .find(|known| known.name == entry.name)
            .ok_or_else(|| FeatureError::Unknown { name: entry.name.clone() })?;
        if !seen.insert(known.id) {
            return Err(FeatureError::Twice { name: entry.name.clone() });
        }
        let refused =
            |message: String| FeatureError::Parameter { feature: entry.name.clone(), message };
        let set = choose_set(known, entry).map_err(refused)?;
        let mut bytes = Vec::with_capacity(set.prefix.len() + set.parameters.len());
        bytes.extend_from_slice(set.prefix);
        for parameter in set.parameters {
            let Some(given) = entry.params.get(parameter.name) else {
                return Err(refused(format!("its parameter `{}` is missing", parameter.name)));
            };
            let value = match (names_of(known.name, parameter.name), given) {
                (Some(names), ParamValue::Name(name)) => names
                    .iter()
                    .find(|(known_name, _)| *known_name == name)
                    .map(|&(_, value)| value)
                    .ok_or_else(|| {
                        refused(format!(
                            "`{}` is `{name}`, it must be one of {}",
                            parameter.name,
                            names.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                        ))
                    })?,
                (Some(names), ParamValue::Number(number)) => {
                    return Err(refused(format!(
                        "`{}` is {number}, it must be a name: one of {}",
                        parameter.name,
                        names.iter().map(|(n, _)| *n).collect::<Vec<_>>().join(", ")
                    )));
                }
                (None, ParamValue::Name(name)) => {
                    return Err(refused(format!(
                        "`{}` is `{name}`, it must be a whole number, {} to {}",
                        parameter.name, parameter.min, parameter.max
                    )));
                }
                (None, &ParamValue::Number(value)) => value,
            };
            if !(parameter.min..=parameter.max).contains(&value) {
                return Err(refused(format!(
                    "`{}` is {value}, it must be {} to {}",
                    parameter.name, parameter.min, parameter.max
                )));
            }
            // A negative value is kept in its byte as two's complement (the offsets of `enemy_scaling`); a value of two
            // bytes is little endian.
            bytes.extend_from_slice(&value.to_le_bytes()[..parameter.width]);
        }
        slots.push(Compiled { family: Family::Feature, id: known.id, bytes });
    }
    Ok(slots)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A `[[feature]]` entry with the given parameters.
    fn entry(name: &str, params: &[(&str, i64)]) -> FeatureEntry {
        FeatureEntry {
            name: name.to_owned(),
            params: params
                .iter()
                .map(|&(key, value)| (key.to_owned(), ParamValue::Number(value)))
                .collect::<BTreeMap<_, _>>(),
        }
    }

    /// A `[[feature]]` entry whose one parameter is a name.
    fn named(name: &str, parameter: &str, value: &str) -> FeatureEntry {
        FeatureEntry {
            name: name.to_owned(),
            params: BTreeMap::from([(parameter.to_owned(), ParamValue::from(value))]),
        }
    }

    fn kaioken(max_factor: i64, hp_cost_percent: i64) -> FeatureEntry {
        entry("ssj_kaioken", &[("max_factor", max_factor), ("hp_cost_percent", hp_cost_percent)])
    }

    fn free_progression(ginyu_arrival: i64) -> FeatureEntry {
        entry("free_progression", &[("ginyu_arrival", ginyu_arrival)])
    }

    #[test]
    fn the_item_of_ssj_kaioken_is_its_two_parameters_as_bytes() {
        let slots = compile(&[kaioken(10, 10)]).unwrap();
        assert_eq!(
            slots,
            [Compiled { family: Family::Feature, id: SSJ_KAIOKEN, bytes: vec![10, 10] }]
        );
        let slots = compile(&[kaioken(11, 0)]).unwrap();
        assert_eq!(slots[0].bytes, [11, 0], "the ends of the ranges are accepted");
        assert!(compile(&[]).unwrap().is_empty(), "no features, no slots");
    }

    #[test]
    fn free_progression_is_its_one_parameter_as_a_byte_in_a_slot_of_its_own() {
        for arrival in 0..=3_i64 {
            let slots = compile(&[free_progression(arrival)]).unwrap();
            assert_eq!(
                slots,
                [Compiled {
                    family: Family::Feature,
                    id: FREE_PROGRESSION,
                    bytes: vec![arrival as u8]
                }]
            );
        }
        assert_ne!(FREE_PROGRESSION, SSJ_KAIOKEN, "one slot each");
        let both = compile(&[kaioken(10, 10), free_progression(3)]).unwrap();
        assert_eq!(both.len(), 2, "two features in one pack are two slots");
        assert_eq!((both[0].id, both[1].id), (SSJ_KAIOKEN, FREE_PROGRESSION));
        let text = compile(&[free_progression(4)]).unwrap_err().to_string();
        assert!(
            text.starts_with("the feature `free_progression`: ")
                && text.contains("`ginyu_arrival` is 4, it must be 0 to 3"),
            "{text}"
        );
        let missing = compile(&[entry("free_progression", &[])]).unwrap_err().to_string();
        assert!(missing.contains("its parameter `ginyu_arrival` is missing"), "{missing}");
        assert_eq!(
            compile(&[free_progression(1), free_progression(2)]).unwrap_err(),
            FeatureError::Twice { name: "free_progression".into() }
        );
    }

    #[test]
    fn the_level_cap_is_a_second_set_with_a_prefix_and_two_bytes_and_its_absence_is_the_old_slot() {
        let with = |arrival: i64, cap: i64| {
            entry("free_progression", &[("ginyu_arrival", arrival), ("level_cap", cap)])
        };
        let slots = compile(&[with(3, 999)]).unwrap();
        assert_eq!(
            slots[0].bytes,
            [LEVEL_CAP_SET, 3, 0xE7, 0x03],
            "the prefix, the arrival, 999 little endian"
        );
        assert_eq!(compile(&[with(0, 48)]).unwrap()[0].bytes, [LEVEL_CAP_SET, 0, 48, 0]);
        assert_eq!(
            compile(&[free_progression(3)]).unwrap()[0].bytes,
            [3],
            "no cap: the slot of the old packs"
        );
        for cap in [47, 1000] {
            let text = compile(&[with(3, cap)]).unwrap_err().to_string();
            assert!(
                text.contains(&format!("`level_cap` is {cap}, it must be 48 to 999")),
                "{text}"
            );
        }
        let alone = compile(&[entry("free_progression", &[("level_cap", 99)])]).unwrap_err();
        assert!(alone.to_string().contains("its parameter `ginyu_arrival` is missing"), "{alone}");
    }

    fn party_boosts(guru: i64, porunga_level: i64, lunch: i64) -> FeatureEntry {
        entry("party_boosts", &[("guru", guru), ("porunga_level", porunga_level), ("lunch", lunch)])
    }

    #[test]
    fn party_boosts_is_its_three_switches_as_bytes_in_a_slot_of_its_own() {
        let slots = compile(&[party_boosts(1, 1, 1)]).unwrap();
        assert_eq!(
            slots,
            [Compiled { family: Family::Feature, id: PARTY_BOOSTS, bytes: vec![1, 1, 1] }]
        );
        assert_eq!(
            compile(&[party_boosts(1, 0, 0)]).unwrap()[0].bytes,
            [1, 0, 0],
            "guru, porunga_level, lunch"
        );
        assert_eq!(compile(&[party_boosts(0, 1, 0)]).unwrap()[0].bytes, [0, 1, 0]);
        assert_eq!(compile(&[party_boosts(0, 0, 1)]).unwrap()[0].bytes, [0, 0, 1]);
        assert_eq!(compile(&[party_boosts(0, 0, 0)]).unwrap()[0].bytes, [0, 0, 0]);
        for other in [SSJ_KAIOKEN, ENEMY_SCALING, FREE_PROGRESSION] {
            assert_ne!(PARTY_BOOSTS, other, "one slot each");
        }
        let both = compile(&[free_progression(3), party_boosts(1, 1, 1)]).unwrap();
        assert_eq!((both[0].id, both[1].id), (FREE_PROGRESSION, PARTY_BOOSTS));
        assert!(known_names().contains(&"party_boosts"));
    }

    #[test]
    fn a_party_boosts_parameter_that_is_missing_out_of_range_or_not_a_number_is_refused() {
        for missing in ["guru", "porunga_level", "lunch"] {
            let params: Vec<(&str, i64)> = [("guru", 1), ("porunga_level", 1), ("lunch", 1)]
                .into_iter()
                .filter(|&(key, _)| key != missing)
                .collect();
            let text = compile(&[entry("party_boosts", &params)]).unwrap_err().to_string();
            assert_eq!(
                text,
                format!("the feature `party_boosts`: its parameter `{missing}` is missing")
            );
        }
        let text = compile(&[party_boosts(2, 1, 1)]).unwrap_err().to_string();
        assert!(
            text.starts_with("the feature `party_boosts`: ")
                && text.ends_with("`guru` is 2, it must be 0 to 1"),
            "{text}"
        );
        let text = compile(&[party_boosts(1, -1, 1)]).unwrap_err().to_string();
        assert!(text.contains("`porunga_level` is -1"), "{text}");
        let text = compile(&[party_boosts(1, 7, 1)]).unwrap_err().to_string();
        assert!(text.contains("`porunga_level` is 7, it must be 0 to 1"), "{text}");
        let text = compile(&[party_boosts(1, 1, 3)]).unwrap_err().to_string();
        assert!(text.contains("`lunch` is 3, it must be 0 to 1"), "{text}");
        let mut by_name = party_boosts(1, 1, 1);
        by_name.params.insert("guru".to_owned(), ParamValue::from("yes"));
        let text = compile(&[by_name]).unwrap_err().to_string();
        assert!(text.contains("`guru` is `yes`, it must be a whole number, 0 to 1"), "{text}");
        let unknown = compile(&[entry(
            "party_boosts",
            &[("scouter", 1), ("guru", 1), ("porunga_level", 1), ("lunch", 1)],
        )])
        .unwrap_err()
        .to_string();
        assert!(unknown.contains("`scouter` is not a parameter of it"), "{unknown}");
        assert_eq!(
            compile(&[party_boosts(1, 1, 1), party_boosts(0, 0, 0)]).unwrap_err(),
            FeatureError::Twice { name: "party_boosts".into() }
        );
    }

    #[test]
    fn a_name_the_game_does_not_have_is_refused_with_the_known_ones() {
        let error = compile(&[entry("ssj_kamehameha", &[])]).unwrap_err();
        assert_eq!(error, FeatureError::Unknown { name: "ssj_kamehameha".into() });
        let text = error.to_string();
        assert!(
            text.contains("ssj_kamehameha")
                && text.contains("ssj_kaioken")
                && text.contains("free_progression"),
            "{text}"
        );
    }

    #[test]
    fn a_feature_given_twice_is_refused() {
        let error = compile(&[kaioken(10, 10), kaioken(9, 5)]).unwrap_err();
        assert_eq!(error, FeatureError::Twice { name: "ssj_kaioken".into() });
    }

    #[test]
    fn every_parameter_problem_names_the_feature_and_the_parameter() {
        let cases = [
            (
                entry("ssj_kaioken", &[("hp_cost_percent", 10)]),
                "its parameter `max_factor` is missing",
            ),
            (
                entry("ssj_kaioken", &[("max_factor", 10), ("hp_cost_percent", 10), ("factor", 2)]),
                "`factor` is not a parameter of it; its parameters are max_factor, hp_cost_percent",
            ),
            (kaioken(12, 10), "`max_factor` is 12, it must be 1 to 11"),
            (kaioken(0, 10), "`max_factor` is 0, it must be 1 to 11"),
            (kaioken(10, 101), "`hp_cost_percent` is 101, it must be 0 to 100"),
        ];
        for (entry, expected) in cases {
            let text = compile(&[entry]).unwrap_err().to_string();
            assert!(text.starts_with("the feature `ssj_kaioken`: "), "{text}");
            assert!(text.contains(expected), "{text} (want {expected})");
        }
    }

    #[test]
    fn a_number_parameter_refuses_a_name() {
        let mut bad = kaioken(10, 10);
        bad.params.insert("max_factor".into(), ParamValue::from("ten"));
        let text = compile(&[bad]).unwrap_err().to_string();
        assert_eq!(
            text,
            "the feature `ssj_kaioken`: `max_factor` is `ten`, it must be a whole number, 1 to 11"
        );
    }

    #[test]
    fn the_difficulty_of_enemy_scaling_is_a_name_and_its_byte_is_the_names_value() {
        for (name, byte) in [("easy", 0), ("normal", 1), ("hard", 2)] {
            let slots = compile(&[named("enemy_scaling", "difficulty", name)]).unwrap();
            assert_eq!(
                slots,
                [Compiled { family: Family::Feature, id: ENEMY_SCALING, bytes: vec![byte] }],
                "{name}"
            );
        }
    }

    #[test]
    fn the_difficulty_refuses_a_number_a_name_it_does_not_have_and_a_missing_value() {
        let number = compile(&[entry("enemy_scaling", &[("difficulty", 1)])]).unwrap_err();
        assert_eq!(
            number.to_string(),
            "the feature `enemy_scaling`: `difficulty` is 1, it must be a name: one of easy, normal, hard"
        );
        let unknown = compile(&[named("enemy_scaling", "difficulty", "brutal")]).unwrap_err();
        assert_eq!(
            unknown.to_string(),
            "the feature `enemy_scaling`: `difficulty` is `brutal`, it must be one of easy, normal, hard"
        );
        let missing = compile(&[entry("enemy_scaling", &[])]).unwrap_err();
        assert_eq!(
            missing.to_string(),
            "the feature `enemy_scaling`: its parameter `difficulty` is missing"
        );
        let extra = compile(&[entry("enemy_scaling", &[("difficulty", 1), ("margin", 3)])]);
        assert!(extra.unwrap_err().to_string().contains("`margin` is not a parameter of it"));
    }

    #[test]
    fn the_features_have_distinct_ids_and_the_names_belong_to_known_parameters() {
        let mut ids: Vec<u16> = KNOWN.iter().map(|known| known.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), KNOWN.len(), "two features share an id");
        assert!(KNOWN.iter().all(|known| known.id != 0), "the ids start at 1");
        for named in NAMED {
            let (feature, names) = (named.feature, named.names);
            let known = KNOWN.iter().find(|known| known.name == feature).expect("a known feature");
            let parameter = known
                .sets
                .iter()
                .flat_map(|set| set.parameters)
                .find(|p| p.name == named.parameter)
                .expect("a known parameter");
            let values: Vec<i64> = names.iter().map(|&(_, value)| value).collect();
            assert!(
                values.iter().all(|value| (parameter.min..=parameter.max).contains(value)),
                "{feature}: the names stay inside the parameter's range"
            );
            let mut distinct = values.clone();
            distinct.sort_unstable();
            distinct.dedup();
            assert_eq!(distinct.len(), values.len(), "{feature}: two names share a value");
        }
        assert!(known_names().contains(&"enemy_scaling"));
    }

    #[test]
    fn two_features_given_in_one_pack_compile_to_their_own_slots() {
        let slots =
            compile(&[kaioken(10, 10), named("enemy_scaling", "difficulty", "hard")]).unwrap();
        assert_eq!(slots.len(), 2);
        assert_eq!((slots[0].id, slots[1].id), (SSJ_KAIOKEN, ENEMY_SCALING));
    }

    /// An `enemy_scaling` entry given by offsets.
    fn offsets(random_offset: i64, boss_offset: i64) -> FeatureEntry {
        entry("enemy_scaling", &[("random_offset", random_offset), ("boss_offset", boss_offset)])
    }

    #[test]
    fn the_offsets_of_enemy_scaling_are_a_mode_byte_and_two_signed_bytes() {
        let slots = compile(&[offsets(-3, 3)]).unwrap();
        assert_eq!(
            slots,
            [Compiled {
                family: Family::Feature,
                id: ENEMY_SCALING,
                bytes: vec![SCALING_OFFSETS, 0xFD, 3]
            }]
        );
        assert_eq!(compile(&[offsets(-10, 10)]).unwrap()[0].bytes, [SCALING_OFFSETS, 0xF6, 10]);
        assert_eq!(compile(&[offsets(10, -10)]).unwrap()[0].bytes, [SCALING_OFFSETS, 10, 0xF6]);
        assert_eq!(compile(&[offsets(0, 0)]).unwrap()[0].bytes, [SCALING_OFFSETS, 0, 0]);
        const { assert!(SCALING_OFFSETS > 2, "the mode byte is outside the difficulties 0 to 2") };
    }

    #[test]
    fn a_pack_written_with_the_difficulty_alone_compiles_to_the_same_byte_as_before_the_offsets() {
        // The released pack 0.2.0 of Free progression: `difficulty` only. One byte, the difficulty's value, no prefix.
        for (name, byte) in [("easy", 0), ("normal", 1), ("hard", 2)] {
            let slots = compile(&[named("enemy_scaling", "difficulty", name)]).unwrap();
            assert_eq!(slots[0].bytes, [byte], "{name}");
        }
    }

    #[test]
    fn an_offset_that_is_missing_out_of_range_or_a_name_is_refused() {
        let missing = compile(&[entry("enemy_scaling", &[("random_offset", -3)])]).unwrap_err();
        assert_eq!(
            missing.to_string(),
            "the feature `enemy_scaling`: its parameter `boss_offset` is missing"
        );
        let missing = compile(&[entry("enemy_scaling", &[("boss_offset", 3)])]).unwrap_err();
        assert_eq!(
            missing.to_string(),
            "the feature `enemy_scaling`: its parameter `random_offset` is missing"
        );
        for (random, boss, wrong) in [(11, 0, "random_offset"), (0, -11, "boss_offset")] {
            let text = compile(&[offsets(random, boss)]).unwrap_err().to_string();
            assert!(
                text.starts_with("the feature `enemy_scaling`: `")
                    && text.contains(wrong)
                    && text.ends_with("it must be -10 to 10"),
                "{text}"
            );
        }
        let mut by_name = offsets(0, 0);
        by_name.params.insert("boss_offset".into(), ParamValue::from("hard"));
        let text = compile(&[by_name]).unwrap_err().to_string();
        assert!(text.contains("`boss_offset` is `hard`, it must be a whole number, -10 to 10"));
    }

    #[test]
    fn the_difficulty_and_the_offsets_together_are_refused_and_a_stray_key_names_both_sets() {
        let mut mixed = offsets(-3, 3);
        mixed.params.insert("difficulty".into(), ParamValue::from("normal"));
        let text = compile(&[mixed]).unwrap_err().to_string();
        assert!(
            text.starts_with("the feature `enemy_scaling`: its parameters are mixed")
                && text.ends_with("difficulty or random_offset, boss_offset"),
            "{text}"
        );
        let mut half = entry("enemy_scaling", &[("random_offset", 1)]);
        half.params.insert("difficulty".into(), ParamValue::from("easy"));
        assert!(compile(&[half]).unwrap_err().to_string().contains("mixed"));
        let stray = compile(&[entry("enemy_scaling", &[("random_offset", 1), ("margin", 3)])]);
        let text = stray.unwrap_err().to_string();
        assert!(
            text.contains("`margin` is not a parameter of it")
                && text.contains("difficulty or random_offset, boss_offset"),
            "{text}"
        );
    }

    #[test]
    fn enemy_scaling_in_either_form_is_one_slot_and_given_twice_is_refused() {
        assert_eq!(
            compile(&[offsets(-3, 3), named("enemy_scaling", "difficulty", "easy")]).unwrap_err(),
            FeatureError::Twice { name: "enemy_scaling".into() }
        );
        let both = compile(&[free_progression(3), offsets(-3, 3)]).unwrap();
        assert_eq!((both[0].id, both[1].id), (FREE_PROGRESSION, ENEMY_SCALING));
    }

    /// The manifest of the released pack 0.2.0 of Free progression, as it was published (its two features).
    const RELEASED_0_2_0: &str = r#"
[pack]
id = "free.progression"
name = "Free Progression"
version = "0.2.0"
game = "A8CF4F2A"
description = "The released pack."

[[feature]]
name = "free_progression"
ginyu_arrival = 3

[[feature]]
name = "enemy_scaling"
difficulty = "normal"
"#;

    #[test]
    fn the_released_free_progression_manifest_still_compiles_to_the_slots_it_always_did() {
        let manifest: crate::manifest::Manifest = toml::from_str(RELEASED_0_2_0).unwrap();
        let slots = compile(&manifest.feature).unwrap();
        assert_eq!(
            slots,
            [
                Compiled { family: Family::Feature, id: FREE_PROGRESSION, bytes: vec![3] },
                Compiled { family: Family::Feature, id: ENEMY_SCALING, bytes: vec![1] },
            ]
        );
        // The new pack: the same first feature, and the second with the offsets.
        let new = RELEASED_0_2_0.replace(
            "difficulty = \"normal\"",
            "random_offset = -3
boss_offset = 3",
        );
        let manifest: crate::manifest::Manifest = toml::from_str(&new).unwrap();
        let slots = compile(&manifest.feature).unwrap();
        assert_eq!(slots[1].bytes, [SCALING_OFFSETS, 0xFD, 3]);
    }
}
