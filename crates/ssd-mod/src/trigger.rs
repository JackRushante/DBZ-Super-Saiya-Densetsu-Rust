//! The triggers of a data pack (M10, Task 6): a `[[triggers]]` file names, for each trigger, the rectangle of a map where the
//! leader starts a scene, and the conditions that must hold there. The triggers of all the packs of a set are compiled into
//! one data item of the image (`Family::Triggers`, slot 0), which the game's dispatch reads (`ssd_game::mods::triggers`).
//!
//! The file, `triggers.toml`: one `[[trigger]]` table per trigger, the numbers in decimal.
//!
//! ```toml
//! [[trigger]]
//! map = 64                      # 0 to 127: an original map (0 to 63) or a pack map (64 to 127)
//! rect = [960, 704, 128, 128]   # x, y, width, height in pixels: tested on the leader's position ($01A2, $01A4)
//! when = ["flag 3", "not flag 29", "flagx 12", "member 2", "flying"]   # all must hold; any subset, or none
//! scene = 82                    # 0 to 255: the scene to start (an original scene or a pack scene)
//! ```
//!
//! A rectangle holds the position `(x, y)` when `x` is from its `x` up to, not including, `x + width`, and `y` likewise.
//! The conditions are `flag N` (story flag `N`, 0 to 127), `flagx N` (extended flag `N`, 0 to 2047), `member C` (character
//! `C`, 0 to 11, is in the party) and `flying` (the leader is airborne: `$01BC` bit 0); each may be preceded by `not`.
//! Every error names the trigger by its number in the file, counted from 1.
//!
//! The data item (all numbers little-endian): `count:u8`, then `count` records, sorted by map id (the triggers of one map
//! keep the order of their packs, by pack id, and of their files). A record is `map:u8`, `scene:u8`, `x:u16`, `y:u16`,
//! `width:u16`, `height:u16` and `n:u8` (the number of conditions), then the `n` conditions, three bytes each: `kind:u8`
//! and `arg:u16`. The kind is [`KIND_FLAG`], [`KIND_FLAGX`], [`KIND_MEMBER`] or [`KIND_FLYING`] (its argument is 0), with
//! [`KIND_NOT`] added for a `not`. A set holds at most [`MAX_TRIGGERS`] triggers: the game keeps the number of the trigger the
//! party stands in (its position in the item, from 1) in one byte.

use serde::Deserialize;
use std::fmt;

/// The highest map id a trigger may name: the map byte of the game (`$01A0`) is an original map or a pack map.
pub const MAX_MAP: u8 = 127;
/// The most triggers in a set.
pub const MAX_TRIGGERS: usize = 255;
/// The most conditions of one trigger (the count is a byte).
pub const MAX_CONDITIONS: usize = 255;
/// The length of a record's header (up to the count of its conditions, inclusive) and of a condition.
pub const HEADER_LEN: usize = 11;
pub const CONDITION_LEN: usize = 3;
/// The kinds of a condition: a story flag, an extended flag, a party member, the flight bit.
pub const KIND_FLAG: u8 = 0;
pub const KIND_FLAGX: u8 = 1;
pub const KIND_MEMBER: u8 = 2;
pub const KIND_FLYING: u8 = 3;
/// The bit of a condition's kind that is set for a `not`.
pub const KIND_NOT: u8 = 0x80;

/// One trigger: the map, the rectangle `[x, y, width, height]` of the leader's position, the conditions (all must
/// hold, in any order), and the scene it starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Trigger {
    pub map: u8,
    pub rect: [u16; 4],
    pub conditions: Vec<Condition>,
    pub scene: u8,
}

/// One condition of a trigger: what it tests, and whether it is negated (`not`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Condition {
    pub test: Test,
    pub negated: bool,
}

/// What a condition tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Test {
    /// A story flag, 0 to 127.
    Flag(u8),
    /// An extended flag, 0 to 2047.
    FlagX(u16),
    /// A character in the party, 0 to 11.
    Member(u8),
    /// The leader is airborne.
    Flying,
}

impl Test {
    /// The kind byte (without [`KIND_NOT`]) and the argument word of the layout.
    #[must_use]
    pub fn encode(self) -> (u8, u16) {
        match self {
            Self::Flag(flag) => (KIND_FLAG, u16::from(flag)),
            Self::FlagX(flag) => (KIND_FLAGX, flag),
            Self::Member(character) => (KIND_MEMBER, u16::from(character)),
            Self::Flying => (KIND_FLYING, 0),
        }
    }

    /// The test of a kind byte (without [`KIND_NOT`]) and an argument word, `None` for an unknown kind or an argument out
    /// of its range.
    #[must_use]
    pub fn decode(kind: u8, arg: u16) -> Option<Self> {
        match kind {
            KIND_FLAG => u8::try_from(arg).ok().filter(|&flag| flag <= 127).map(Self::Flag),
            KIND_FLAGX => Some(arg).filter(|&flag| flag <= 2047).map(Self::FlagX),
            KIND_MEMBER => u8::try_from(arg).ok().filter(|&c| c <= 11).map(Self::Member),
            KIND_FLYING => (arg == 0).then_some(Self::Flying),
            _ => None,
        }
    }
}

/// A triggers file that is refused. Each names the file's part: the trigger's number, counted from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerError {
    /// The file is not a triggers file: its TOML does not parse, or it has a key other than `trigger`.
    File(String),
    /// Trigger `index` is refused for `problem` (which gives the value or the text).
    Trigger { index: usize, problem: String },
    /// A set has `count` triggers, more than [`MAX_TRIGGERS`].
    Triggers { count: usize },
    /// A trigger has `count` conditions, more than [`MAX_CONDITIONS`].
    Conditions { count: usize },
}

impl fmt::Display for TriggerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::File(message) => write!(f, "the file is not a triggers file: {message}"),
            Self::Trigger { index, problem } => write!(f, "trigger {index}: {problem}"),
            Self::Triggers { count } => {
                write!(f, "the set has {count} triggers: at most {MAX_TRIGGERS} are allowed")
            }
            Self::Conditions { count } => {
                write!(f, "a trigger has {count} conditions: at most {MAX_CONDITIONS} are allowed")
            }
        }
    }
}

impl std::error::Error for TriggerError {}

/// A `[[trigger]]` table as it is written. Every key is known, and each value is checked by [`parse`] afterwards, so the
/// numbers are read as `i64` and their range is named in the error.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTrigger {
    map: i64,
    rect: Vec<i64>,
    #[serde(default)]
    when: Vec<String>,
    scene: i64,
}

/// The triggers of a triggers file, in the order the file gives them.
///
/// # Errors
///
/// [`TriggerError::File`] if the text is not TOML or holds a key other than `trigger`, and [`TriggerError::Trigger`] for
/// the first trigger that is refused (a number out of its range, a rectangle that is not four numbers, a condition that is
/// not one of the forms above, an unknown key).
pub fn parse(text: &str) -> Result<Vec<Trigger>, TriggerError> {
    let table: toml::Table =
        toml::from_str(text).map_err(|error| TriggerError::File(error.to_string()))?;
    if let Some(key) = table.keys().find(|key| key.as_str() != "trigger") {
        return Err(TriggerError::File(format!(
            "the key `{key}` is not `trigger`: a triggers file holds `[[trigger]]` tables"
        )));
    }
    let Some(value) = table.get("trigger") else { return Ok(Vec::new()) };
    let toml::Value::Array(items) = value else {
        return Err(TriggerError::File("`trigger` is not a list of `[[trigger]]` tables".into()));
    };
    items.iter().enumerate().map(|(at, item)| parse_trigger(at + 1, item)).collect()
}

/// Trigger `index` (counted from 1), from its table.
fn parse_trigger(index: usize, value: &toml::Value) -> Result<Trigger, TriggerError> {
    let refused = |problem: String| TriggerError::Trigger { index, problem };
    let raw: RawTrigger =
        value.clone().try_into().map_err(|error: toml::de::Error| refused(error.to_string()))?;
    if !within(raw.map, 0, i64::from(MAX_MAP)) {
        return Err(refused(format!("map {} is not 0 to {MAX_MAP}", raw.map)));
    }
    let [x, y, width, height] = match raw.rect.as_slice() {
        &[x, y, width, height] => [x, y, width, height],
        other => {
            return Err(refused(format!(
                "rect has {} numbers: it takes x, y, width and height",
                other.len()
            )));
        }
    };
    for (name, value) in [("x", x), ("y", y)] {
        if !within(value, 0, 65535) {
            return Err(refused(format!("rect {name} {value} is not 0 to 65535")));
        }
    }
    for (name, value) in [("width", width), ("height", height)] {
        if !within(value, 1, 65535) {
            return Err(refused(format!("rect {name} {value} is not 1 to 65535")));
        }
    }
    if !within(raw.scene, 0, 255) {
        return Err(refused(format!(
            "scene {} is not 0 to 255: the scene register holds one byte",
            raw.scene
        )));
    }
    if raw.when.len() > MAX_CONDITIONS {
        return Err(TriggerError::Conditions { count: raw.when.len() });
    }
    let conditions = raw
        .when
        .iter()
        .map(|text| condition(text))
        .collect::<Result<Vec<_>, _>>()
        .map_err(refused)?;
    Ok(Trigger {
        map: raw.map as u8,
        rect: [x as u16, y as u16, width as u16, height as u16],
        conditions,
        scene: raw.scene as u8,
    })
}

fn within(value: i64, low: i64, high: i64) -> bool {
    (low..=high).contains(&value)
}

/// One condition written as text: `flag N`, `flagx N`, `member C` or `flying`, each with an optional `not` before it.
fn condition(text: &str) -> Result<Condition, String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let (negated, rest) = match words.as_slice() {
        ["not", rest @ ..] => (true, rest),
        rest => (false, rest),
    };
    let test = match rest {
        ["flying"] => Test::Flying,
        ["flag", number] => Test::Flag(bounded(text, number, 127)? as u8),
        ["flagx", number] => Test::FlagX(bounded(text, number, 2047)? as u16),
        ["member", number] => Test::Member(bounded(text, number, 11)? as u8),
        _ => {
            return Err(format!(
                "the condition \"{text}\" is not `flag N`, `flagx N`, `member C` or `flying`, with an optional `not`"
            ));
        }
    };
    Ok(Condition { test, negated })
}

/// A decimal number of a condition, at most `high`.
fn bounded(text: &str, number: &str, high: u32) -> Result<u32, String> {
    let value: u32 = number
        .parse()
        .map_err(|_| format!("the condition \"{text}\": `{number}` is not a decimal number"))?;
    if value > high {
        return Err(format!("the condition \"{text}\": {value} is above {high}"));
    }
    Ok(value)
}

/// The data item of `triggers`: the layout of the module documentation, the records sorted by map id (a stable sort, so
/// the order of the triggers of one map is kept).
///
/// # Errors
///
/// [`TriggerError::Triggers`] for more than [`MAX_TRIGGERS`] triggers, [`TriggerError::Conditions`] for a trigger with more
/// than [`MAX_CONDITIONS`] conditions.
pub fn encode(triggers: &[Trigger]) -> Result<Vec<u8>, TriggerError> {
    if triggers.len() > MAX_TRIGGERS {
        return Err(TriggerError::Triggers { count: triggers.len() });
    }
    let mut sorted: Vec<&Trigger> = triggers.iter().collect();
    sorted.sort_by_key(|trigger| trigger.map);
    let mut bytes = vec![sorted.len() as u8];
    for trigger in sorted {
        if trigger.conditions.len() > MAX_CONDITIONS {
            return Err(TriggerError::Conditions { count: trigger.conditions.len() });
        }
        bytes.push(trigger.map);
        bytes.push(trigger.scene);
        for value in trigger.rect {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.push(trigger.conditions.len() as u8);
        for condition in &trigger.conditions {
            let (kind, arg) = condition.test.encode();
            bytes.push(if condition.negated { kind | KIND_NOT } else { kind });
            bytes.extend_from_slice(&arg.to_le_bytes());
        }
    }
    Ok(bytes)
}

/// The triggers of a data item written by [`encode`], in the item's order; `None` when the bytes are not a whole item.
#[must_use]
pub fn decode(bytes: &[u8]) -> Option<Vec<Trigger>> {
    let (&count, mut rest) = bytes.split_first()?;
    let mut triggers = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let header = rest.get(..HEADER_LEN)?;
        let word = |at: usize| u16::from_le_bytes([header[at], header[at + 1]]);
        let rect = [word(2), word(4), word(6), word(8)];
        let conditions_len = usize::from(header[10]);
        rest = &rest[HEADER_LEN..];
        let body = rest.get(..CONDITION_LEN * conditions_len)?;
        let mut conditions = Vec::with_capacity(conditions_len);
        for chunk in body.as_chunks::<CONDITION_LEN>().0 {
            let kind = chunk[0];
            let test = Test::decode(kind & !KIND_NOT, u16::from_le_bytes([chunk[1], chunk[2]]))?;
            conditions.push(Condition { test, negated: kind & KIND_NOT != 0 });
        }
        rest = &rest[CONDITION_LEN * conditions_len..];
        triggers.push(Trigger { map: header[0], rect, conditions, scene: header[1] });
    }
    rest.is_empty().then_some(triggers)
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: &str = r#"
[[trigger]]
map = 64
rect = [960, 704, 128, 128]
when = ["flag 3", "not flag 29", "flagx 12", "not flagx 13", "member 2", "flying", "not flying"]
scene = 82

[[trigger]]
map = 0
rect = [10, 20, 30, 40]
scene = 5
"#;

    fn refused(text: &str) -> String {
        match parse(text) {
            Err(error) => error.to_string(),
            Ok(triggers) => panic!("accepted: {triggers:?}"),
        }
    }

    #[test]
    fn a_file_parses_each_trigger_with_its_conditions_in_order() {
        let triggers = parse(EXAMPLE).unwrap();
        assert_eq!(triggers.len(), 2);
        assert_eq!(triggers[0].map, 64);
        assert_eq!(triggers[0].rect, [960, 704, 128, 128]);
        assert_eq!(triggers[0].scene, 82);
        assert_eq!(
            triggers[0].conditions,
            [
                Condition { test: Test::Flag(3), negated: false },
                Condition { test: Test::Flag(29), negated: true },
                Condition { test: Test::FlagX(12), negated: false },
                Condition { test: Test::FlagX(13), negated: true },
                Condition { test: Test::Member(2), negated: false },
                Condition { test: Test::Flying, negated: false },
                Condition { test: Test::Flying, negated: true },
            ]
        );
        assert!(triggers[1].conditions.is_empty(), "no `when` is no condition");
        assert_eq!(parse("").unwrap(), Vec::new(), "an empty file has no triggers");
    }

    #[test]
    fn the_limits_of_each_number_are_inclusive() {
        let at = |map: u32, scene: u32, rect: &str, when: &str| {
            format!("[[trigger]]\nmap = {map}\nrect = {rect}\nwhen = [{when}]\nscene = {scene}\n")
        };
        assert!(
            parse(&at(
                127,
                255,
                "[65535, 65535, 1, 1]",
                "\"flag 127\", \"flagx 2047\", \"member 11\""
            ))
            .is_ok()
        );
        assert!(parse(&at(0, 0, "[0, 0, 65535, 65535]", "")).is_ok());
    }

    #[test]
    fn every_refusal_names_the_trigger_and_the_value_or_the_text() {
        let one =
            |body: &str| format!("[[trigger]]\nmap = 64\nrect = [0, 0, 8, 8]\nscene = 1\n{body}");
        let refusals = [
            (one("").replace("map = 64", "map = 128"), "trigger 1: map 128 is not 0 to 127"),
            (one("").replace("map = 64", "map = -1"), "trigger 1: map -1 is not 0 to 127"),
            (one("").replace("[0, 0, 8, 8]", "[0, 0, 8]"), "trigger 1: rect has 3 numbers"),
            (
                one("").replace("[0, 0, 8, 8]", "[0, 0, 0, 8]"),
                "trigger 1: rect width 0 is not 1 to 65535",
            ),
            (one("").replace("[0, 0, 8, 8]", "[0, 0, 8, 65536]"), "trigger 1: rect height 65536"),
            (
                one("").replace("[0, 0, 8, 8]", "[-1, 0, 8, 8]"),
                "trigger 1: rect x -1 is not 0 to 65535",
            ),
            (one("").replace("scene = 1", "scene = 256"), "trigger 1: scene 256 is not 0 to 255"),
            (
                one("when = [\"flag 128\"]\n"),
                "trigger 1: the condition \"flag 128\": 128 is above 127",
            ),
            (
                one("when = [\"flagx 2048\"]\n"),
                "trigger 1: the condition \"flagx 2048\": 2048 is above 2047",
            ),
            (
                one("when = [\"member 12\"]\n"),
                "trigger 1: the condition \"member 12\": 12 is above 11",
            ),
            (
                one("when = [\"flag x\"]\n"),
                "trigger 1: the condition \"flag x\": `x` is not a decimal number",
            ),
            (one("when = [\"flying 1\"]\n"), "trigger 1: the condition \"flying 1\" is not"),
            (one("when = [\"not\"]\n"), "trigger 1: the condition \"not\" is not"),
            (
                one("when = [\"flag 3\"]\n").replace("scene = 1", "scene = 1\nlevel = 3"),
                "trigger 1: unknown field `level`",
            ),
            (
                format!("{}\n[[trigger]]\nmap = 1\nrect = [0, 0, 8, 8]\nscene = 300\n", one("")),
                "trigger 2: scene 300",
            ),
            ("[other]\nx = 1\n".to_owned(), "the key `other` is not `trigger`"),
            ("trigger = 3\n".to_owned(), "`trigger` is not a list of `[[trigger]]` tables"),
            ("[[trigger]\n".to_owned(), "the file is not a triggers file"),
        ];
        for (text, expected) in &refusals {
            let message = refused(text);
            assert!(message.contains(expected), "`{expected}` not in `{message}`");
        }
    }

    #[test]
    fn the_number_of_a_refused_trigger_counts_from_one_in_the_file() {
        let text = format!("{EXAMPLE}\n[[trigger]]\nmap = 64\nrect = [0, 0, 8, 8]\nscene = 300\n");
        assert!(refused(&text).starts_with("trigger 3: scene 300"), "{}", refused(&text));
    }

    #[test]
    fn the_binary_layout_is_the_documented_one_and_round_trips() {
        let triggers = parse(
            "[[trigger]]\nmap = 64\nrect = [960, 704, 128, 128]\nwhen = [\"flag 3\", \"not flagx 13\"]\nscene = 82\n",
        )
        .unwrap();
        let bytes = encode(&triggers).unwrap();
        assert_eq!(
            bytes,
            [
                0x01, // one record
                0x40, 0x52, // map 64, scene 82
                0xC0, 0x03, 0xC0, 0x02, // x 960, y 704
                0x80, 0x00, 0x80, 0x00, // width 128, height 128
                0x02, // two conditions
                0x00, 0x03, 0x00, // flag 3
                0x81, 0x0D, 0x00, // not flagx 13 (kind 1 with the not bit)
            ]
        );
        assert_eq!(decode(&bytes), Some(triggers), "the round trip");
        assert_eq!(decode(&bytes[..bytes.len() - 1]), None, "a cut item is refused");
        let mut extra = bytes.clone();
        extra.push(0);
        assert_eq!(decode(&extra), None, "a longer item is refused");
    }

    #[test]
    fn the_records_are_sorted_by_map_and_keep_their_order_within_a_map() {
        let make =
            |map: u8, scene: u8| Trigger { map, rect: [0, 0, 1, 1], conditions: Vec::new(), scene };
        let triggers = [make(70, 1), make(64, 2), make(70, 3), make(0, 4)];
        let bytes = encode(&triggers).unwrap();
        let decoded = decode(&bytes).unwrap();
        let order: Vec<(u8, u8)> = decoded.iter().map(|t| (t.map, t.scene)).collect();
        assert_eq!(order, [(0, 4), (64, 2), (70, 1), (70, 3)]);
    }

    #[test]
    fn a_set_of_too_many_triggers_or_conditions_is_refused() {
        let trigger = Trigger { map: 0, rect: [0, 0, 1, 1], conditions: Vec::new(), scene: 0 };
        assert!(encode(&vec![trigger.clone(); MAX_TRIGGERS]).is_ok());
        assert_eq!(
            encode(&vec![trigger.clone(); MAX_TRIGGERS + 1]),
            Err(TriggerError::Triggers { count: MAX_TRIGGERS + 1 })
        );
        let many = Trigger {
            conditions: vec![Condition { test: Test::Flying, negated: false }; MAX_CONDITIONS + 1],
            ..trigger
        };
        assert_eq!(encode(&[many]), Err(TriggerError::Conditions { count: MAX_CONDITIONS + 1 }));
    }
}
