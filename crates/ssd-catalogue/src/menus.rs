//! The names and texts the HD menus show, read from the catalogue: the labels of `ssd-menus`, by
//! character id and by object id. The menus' faces and prices come with them.

use crate::Catalogue;
use crate::translation::{self, Translation};
use ssd_menus::{CharacterLabel, CommandLabel, Labels, ObjectLabel, StatusLabels};

/// The face index of each character, in the order of the catalogue's characters (the second byte of its party
/// record, which the field menu uses for the corner face). Measured on the field menu: Goku 00, Piccolo 01, Gohan
/// 09, Krillin 0A, Tien 0B, Yamcha 0C, Chiaotzu 0D, Vegeta 50 and Dende 61. Nail, Tsumuri and Maima have not been
/// seen in a party yet, so they have no face (`docs/re/data/menus.md`, "Corner faces").
const CHARACTER_FACES: [Option<u8>; 12] = [
    Some(0x00),
    Some(0x09),
    Some(0x01),
    Some(0x0A),
    Some(0x0B),
    Some(0x0C),
    Some(0x0D),
    Some(0x50),
    None,
    None,
    None,
    Some(0x61),
];

/// The labels of the menus, from the catalogue: a character's face is the face index of the field menu's corner
/// picture ([`CHARACTER_FACES`]).
#[must_use]
pub fn labels(catalogue: &Catalogue) -> Labels {
    let characters = catalogue
        .characters
        .iter()
        .enumerate()
        .map(|(index, character)| CharacterLabel {
            name: plain(character.name.trimmed()),
            face: CHARACTER_FACES.get(index).copied().flatten(),
            // The class the battle copies into the fighter's record: its unit's row of the unit table
            class: catalogue
                .enemies
                .iter()
                .find(|unit| unit.id == character.unit_id)
                .map(|unit| unit.unknown_00),
        })
        .collect();
    let objects = catalogue
        .items
        .iter()
        .map(|item| ObjectLabel {
            name: plain(item.name.trimmed()),
            price: item.price,
            help: item.help.iter().map(|line| plain(line.trimmed())).collect(),
            attribute: item.attribute,
            battle: item.usable_in_battle(),
            field: item.usable_on_map(),
            story: item.attribute & 0x20 != 0,
            fixed: item.attribute & 0x04 != 0,
            member: item.attribute & 0x08 != 0,
            card: item.attribute & 0x10 != 0,
            ally: catalogue.characters.iter().any(|c| c.name.trimmed() == item.name.trimmed()),
        })
        .collect();
    Labels { characters, objects, status: status(catalogue) }
}

/// The tables of the Status screen: the commands' names and KI costs, the techniques each unit's list has, and the EXP
/// each level of a character begins at (up to the character's level cap).
fn status(catalogue: &Catalogue) -> StatusLabels {
    let commands = catalogue
        .attacks
        .iter()
        .map(|attack| CommandLabel { name: plain(attack.name.trimmed()), ki_cost: attack.ki_cost })
        .collect();
    let techniques = catalogue.enemies.iter().map(|unit| techniques(&unit.commands)).collect();
    let levels = catalogue
        .characters
        .iter()
        .map(|character| {
            // The rows up to the character's cap: the row after the last level is no level to reach.
            let reachable = character.levels.iter().take(usize::from(character.level_cap));
            reachable.map(|row| row.exp_start).collect()
        })
        .collect();
    StatusLabels { commands, techniques, levels }
}

/// The command ids of a unit's list that the original's Status page draws: the list is its slot count, then one command id
/// per slot (`docs/re/data/kaioken-bp.md`, section 3); slot 0 is the plain attack and has no line, and a blank line
/// (`$FF`) has no technique.
fn techniques(list: &[u8]) -> Vec<u8> {
    let Some((&count, rest)) = list.split_first() else { return Vec::new() };
    rest.iter().take(usize::from(count)).skip(1).copied().filter(|&id| id != 0xFF).collect()
}

/// `labels` in the words of `table` (a translation, see [`crate::translation`]): a character's name is its
/// `string:N` text, an object's name and help are its `object:N` and `help:N` texts (the help's lines at its line
/// breaks). What the table does not translate keeps its English words; the ids, faces, prices and flags never change.
/// A slot the game fills in while it runs (a card's value) becomes an ellipsis, as in [`plain`].
#[must_use]
pub fn translated(labels: &Labels, table: &Translation) -> Labels {
    let words = |key: String| {
        table.text(&key).map(|text| translation::render(text, |_| "\u{2026}".to_owned()))
    };
    let mut out = labels.clone();
    for (index, character) in out.characters.iter_mut().enumerate() {
        if let Some(name) = words(format!("string:{index}")) {
            character.name = name.replace('\n', "");
        }
    }
    for (index, object) in out.objects.iter_mut().enumerate() {
        if let Some(name) = words(format!("object:{index}")) {
            object.name = name.replace('\n', "");
        }
        if let Some(help) = words(format!("help:{index}")) {
            object.help = help.lines().map(str::to_owned).collect();
        }
    }
    out
}

/// Text as the HD shows it: the codes the catalogue cannot spell (the game fills them in while it
/// runs, as a card's value) become an ellipsis.
#[must_use]
pub fn plain(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        match tail.find('>') {
            Some(end) => {
                out.push('\u{2026}');
                rest = &tail[end + 1..];
            }
            None => {
                out.push_str(tail);
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_units_techniques_are_its_slots_after_the_plain_attack_without_the_blank_lines() {
        // Six slots: the plain attack, two moves, a blank line and two more, then their attributes and factors.
        let list = [6, 0x00, 0x01, 0x38, 0xFF, 0x04, 0x05, 9, 9, 9, 9, 9, 9, 4, 0, 6, 0];
        assert_eq!(techniques(&list), [0x01, 0x38, 0x04, 0x05]);
        assert_eq!(techniques(&[]), Vec::<u8>::new());
        assert_eq!(techniques(&[1, 0x00]), Vec::<u8>::new(), "the plain attack alone has no line");
    }

    #[test]
    fn text_codes_the_catalogue_cannot_spell_become_an_ellipsis() {
        assert_eq!(plain("Change a card's attack to <FC>"), "Change a card's attack to \u{2026}");
        assert_eq!(plain("<FA>!<FD>"), "\u{2026}!\u{2026}");
        assert_eq!(plain("Restores 25"), "Restores 25");
        assert_eq!(plain("open <FC"), "open <FC", "an unclosed code is kept as it is");
    }

    #[test]
    fn a_translation_changes_the_words_and_nothing_else() {
        let labels = Labels {
            status: StatusLabels::default(),
            characters: vec![
                CharacterLabel { name: "Ann".into(), face: Some(3), class: None },
                CharacterLabel { name: "Bob".into(), face: None, class: None },
            ],
            objects: vec![ObjectLabel {
                name: "Stone".into(),
                price: Some(9),
                help: vec!["Line one".into(), "two".into()],
                battle: true,
                ..ObjectLabel::default()
            }],
        };
        let table = Translation::parse(
            "key\tzh\nstring:0\t甲\nobject:0\t石頭\nhelp:0\t第一行\\n第二行{FC}\n",
        )
        .unwrap();
        let shown = translated(&labels, &table);
        assert_eq!(shown.characters[0].name, "甲");
        assert_eq!(shown.characters[0].face, Some(3));
        assert_eq!(shown.characters[1].name, "Bob", "not translated: English stays");
        assert_eq!(shown.objects[0].name, "石頭");
        assert_eq!(shown.objects[0].help, ["第一行", "第二行"]);
        assert_eq!((shown.objects[0].price, shown.objects[0].battle), (Some(9), true));
    }

    #[test]
    fn a_character_without_an_object_of_its_name_has_no_face() {
        let labels = Labels {
            status: StatusLabels::default(),
            characters: vec![CharacterLabel { name: "Krillin".into(), face: None, class: None }],
            objects: vec![ObjectLabel::default()],
        };
        assert_eq!(labels.characters[0].face, None);
    }
}
