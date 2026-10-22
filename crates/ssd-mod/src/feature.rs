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

/// One parameter of a feature: its key in the manifest and the values it may take (a byte each).
struct Parameter {
    name: &'static str,
    min: i64,
    max: i64,
}

/// A feature the game has: its name in a manifest, its slot's id, and its parameters in the slot's byte order.
struct Known {
    name: &'static str,
    id: u16,
    parameters: &'static [Parameter],
}

/// The features the game has. A manifest that names another one is refused, with this list.
const KNOWN: &[Known] = &[
    Known {
        name: "ssj_kaioken",
        id: SSJ_KAIOKEN,
        parameters: &[
            // The largest factor of the Kaioken is `1 + m / 2` with `m` at most 20 (the table `1D:8E7B`), so 11 is the top:
            // the cap on `m` is `2 * (max_factor - 1)`, and a larger `max_factor` would cap nothing.
            Parameter { name: "max_factor", min: 1, max: 11 },
            Parameter { name: "hp_cost_percent", min: 0, max: 100 },
        ],
    },
    Known {
        name: "free_progression",
        id: FREE_PROGRESSION,
        parameters: &[
            // Which of the three arrivals of Goku in the Ginyu squad's chapter the story gives: 0 is the original's rule (his
            // level decides), 1 to 3 are the arrival during the first fight, the second, or at the last one.
            Parameter { name: "ginyu_arrival", min: 0, max: 3 },
        ],
    },
    Known {
        name: "enemy_scaling",
        id: ENEMY_SCALING,
        // The difficulty is one of the names of [`NAMED`]: easy, normal or hard (the byte is 0, 1 or 2).
        parameters: &[Parameter { name: "difficulty", min: 0, max: 2 }],
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

/// The slots of the `[[feature]]` entries of a pack: one per entry, its bytes the parameters in the table's order.
///
/// # Errors
///
/// [`FeatureError::Unknown`] for a name the game does not have, [`FeatureError::Twice`] for a feature given twice, and
/// [`FeatureError::Parameter`] for a parameter that is missing, unknown, not a whole number or out of its range.
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
        if let Some(key) =
            entry.params.keys().find(|key| !known.parameters.iter().any(|p| p.name == key.as_str()))
        {
            let names: Vec<&str> = known.parameters.iter().map(|p| p.name).collect();
            return Err(refused(format!(
                "`{key}` is not a parameter of it; its parameters are {}",
                names.join(", ")
            )));
        }
        let mut bytes = Vec::with_capacity(known.parameters.len());
        for parameter in known.parameters {
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
            bytes.push(value as u8);
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
                .parameters
                .iter()
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
}
