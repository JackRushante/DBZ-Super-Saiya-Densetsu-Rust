//! The catalogue read from the real ROM: internal consistency, and agreement with what the live
//! game holds in WRAM during the reference battle.
//!
//! Needs the game's ROM:
//! `SSD_ROM_PATH=... cargo test -p ssd-catalogue --release --test real_rom -- --ignored`.

use ssd_catalogue::{Card, Catalogue, Slot};
use ssd_game::data as d;
use ssd_game::scripts::battle_input;
use ssd_oracle::{Machine, MemorySaveStore, NullAudio};

fn rom() -> Vec<u8> {
    let path = std::env::var("SSD_ROM_PATH").expect("SSD_ROM_PATH");
    let bytes = std::fs::read(path).unwrap();
    let rom = ssd_rom::normalize(&bytes, None).unwrap();
    assert_eq!(ssd_rom::crc32(&rom.bytes), ssd_rom::CANONICAL_CRC32, "not the canonical ROM");
    rom.bytes
}

fn catalogue() -> Catalogue {
    Catalogue::from_rom(&rom()).unwrap()
}

#[test]
#[ignore = "needs the ROM"]
fn the_characters_have_their_names_and_consistent_level_tables() {
    let catalogue = catalogue();
    let names: Vec<&str> = catalogue.characters.iter().map(|c| c.name.text.as_str()).collect();
    assert_eq!(
        names,
        [
            "Goku", "Gohan", "Piccolo", "Krillin", "Tien", "Yamcha", "Chiaotzu", "Vegeta", "Nail",
            "Tsumuri", "Maima", "Dende"
        ]
    );
    for character in &catalogue.characters {
        assert!(character.name.is_text(), "{character:?}");
        // The id the party table gives is a unit with the same name, but for id 8 (Nail), whose
        // unit row is named Chiaotzu in the unit name table.
        let unit = &catalogue.enemies[usize::from(character.unit_id)];
        if character.id != 8 {
            assert_eq!(unit.name.text, character.name.text, "character {}", character.id);
        }
        match character.fixed_stats_unit {
            Some(row) => {
                assert_eq!(character.levels.len(), 0);
                assert!(!catalogue.enemies[usize::from(row)].is_placeholder(), "row {row}");
                assert_eq!(row, character.unit_id, "the party table names the row it uses");
            }
            None => {
                let expected = if character.id == 11 { 3 } else { 48 };
                assert_eq!(character.levels.len(), expected, "character {}", character.id);
                let levels = &character.levels;
                assert!(levels.iter().all(|r| r.max_hp < 10_000), "{}", character.name.text);
                assert_eq!(levels[0].exp_start, 0, "everyone starts at no EXP");
                if character.id != 7 {
                    // Vegeta's first levels are empty rows, and at level 8 the tables of four
                    // characters drop in HP (and one in EXP) for a reason not known yet; the
                    // first seven levels of the others grow steadily.
                    let first = &levels[..levels.len().min(7)];
                    assert!(first.windows(2).all(|w| w[0].max_hp <= w[1].max_hp), "HP grows");
                    assert!(
                        first.windows(2).all(|w| w[0].exp_start < w[1].exp_start),
                        "the EXP to reach a level grows: {}",
                        character.name.text
                    );
                    assert!(levels[0].max_hp > 0 && levels[0].bp_gain > 0);
                }
            }
        }
    }
}

#[test]
#[ignore = "needs the ROM"]
fn the_units_encounters_and_cards_are_in_range() {
    let catalogue = catalogue();
    assert_eq!(catalogue.enemies.len(), 105);
    let fighters: Vec<_> = catalogue.enemies.iter().filter(|e| !e.is_placeholder()).collect();
    assert!(fighters.len() > 60, "{} units with stats", fighters.len());
    for unit in &fighters {
        assert!((1..=48).contains(&unit.level), "{unit:?}");
        assert!(unit.bp > 0 && unit.bp <= 10_000_000, "{unit:?}");
        assert!(unit.hp <= 2_000 && unit.ki <= 999, "{unit:?}");
        assert!(!unit.commands.is_empty(), "{unit:?}");
    }
    // Every named unit is English but the soldiers of ids 85 to 95, whose name pointer leads
    // into other data.
    for unit in &catalogue.enemies {
        if !(82..=95).contains(&unit.id) {
            assert!(unit.name.is_text(), "{unit:?}");
        }
    }
    // The bosses of the story are where the story puts them.
    let named = |name: &str| fighters.iter().filter(|u| u.name.text == name).count();
    assert!(named("Raditz") >= 2 && named("Nappa") == 1 && named("Frieza") >= 4);
    let top = fighters.iter().map(|u| u.bp).max().unwrap();
    assert_eq!(top, 10_000_000);

    assert_eq!(catalogue.encounters.len(), 112);
    for group in &catalogue.encounters {
        assert!(group.units().count() >= 1, "group {} is empty", group.index);
        for unit in group.units() {
            assert!(usize::from(unit) < catalogue.enemies.len(), "group {}", group.index);
        }
    }

    let rules = &catalogue.cards;
    assert_eq!(rules.values.iter().map(|v| u32::from(v.chances)).sum::<u32>(), 32);
    assert_eq!(rules.values.len(), 8);
    assert_eq!(rules.symbols, 7);

    assert_eq!(catalogue.items.len(), 40);
    let item = |name: &str| catalogue.items.iter().any(|i| i.name.text == name);
    assert!(item("Scouter") && item("D.Radar") && item("\"DB'7\""));
    let attack = |name: &str| catalogue.attacks.iter().any(|a| a.name.text == name);
    assert_eq!(catalogue.attacks.len(), 69);
    assert!(attack("KameHameHa") && attack("Spirit Bomb") && attack("Heal"));
}

fn wram(machine: &Machine, offset: u32) -> u8 {
    machine.peek(0x7E_0000 + offset).unwrap()
}

fn wram16(machine: &Machine, offset: u32) -> u32 {
    u32::from(wram(machine, offset)) | u32::from(wram(machine, offset + 1)) << 8
}

fn wram24(machine: &Machine, offset: u32) -> u32 {
    wram16(machine, offset) | u32::from(wram(machine, offset + 2)) << 16
}

fn run_to(machine: &mut Machine, frame: u64) {
    while machine.frame_count() < frame {
        machine.run_frame(battle_input(machine.frame_count()), &mut NullAudio).unwrap();
    }
}

/// The tile words of a name as the text engine writes them (code, then attribute `$24`).
fn tile_words(tiles: &[u8]) -> Vec<u8> {
    tiles.iter().flat_map(|&t| [t, 0x24]).collect()
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}

#[test]
#[ignore = "needs the ROM; plays the reference battle to frame 7000 (about 30 s)"]
fn the_catalogue_agrees_with_the_game_state_during_the_reference_battle() {
    let rom = rom();
    let catalogue = Catalogue::from_rom(&rom).unwrap();
    let mut machine = Machine::new(rom, Box::new(MemorySaveStore::default())).unwrap();

    // Frame 4720: the first hand, dealt from the table (five cards of six bytes).
    run_to(&mut machine, 4_720);
    for k in 0..5 {
        let at = (d::CARD_HAND & 0xFFFF) + 6 * k + 2;
        let card =
            Card::from_bytes([wram(&machine, at), wram(&machine, at + 1), wram(&machine, at + 2)]);
        assert!(catalogue.cards.can_deal(&card), "card {k}: {card:?}");
    }

    // Frame 5200: Goku and Piccolo against one Saibai, all at level 1 or the unit's level.
    run_to(&mut machine, 5_200);
    let records = d::PARTY_RECORDS & 0xFFFF;
    for (slot, character_id) in [(0, 0_u8), (1, 2)] {
        let character = &catalogue.characters[usize::from(character_id)];
        let row = &character.levels[0];
        let at = records + 0x20 * slot;
        assert_eq!(wram(&machine, at + 0x1C), character_id);
        assert_eq!(wram(&machine, at + 1), character.unit_id, "{}", character.name.text);
        assert_eq!(wram(&machine, at + 2), 1, "level");
        assert_eq!(wram24(&machine, at + 0x07), row.bp_gain, "BP of {}", character.name.text);
        assert_eq!(
            wram16(&machine, at + 0x0A),
            u32::from(row.max_hp),
            "HP of {}",
            character.name.text
        );
        assert_eq!(
            wram16(&machine, at + 0x0C),
            u32::from(row.ki_gain),
            "KI of {}",
            character.name.text
        );
        assert_eq!(wram(&machine, at + 0x0E), row.unknown_0a);
        assert_eq!(wram24(&machine, at + 0x0F), row.exp_start, "EXP");
    }

    let group =
        &catalogue.encounters[usize::from(wram(&machine, d::ENCOUNTER_GROUP_INDEX & 0xFFFF))];
    let enemy_records = (d::ENTITY_IDS & 0xFFFF) - 1;
    for (slot, expected) in group.slots.iter().enumerate() {
        let at = enemy_records + 0x20 * u32::try_from(slot).unwrap();
        match expected {
            Slot::Empty => assert_eq!(wram(&machine, at), 0, "slot {slot} is empty"),
            Slot::Unit(id) => {
                let unit = &catalogue.enemies[usize::from(*id)];
                assert_ne!(wram(&machine, at), 0, "slot {slot} is in use");
                assert_eq!(wram(&machine, at + 1), *id);
                assert_eq!(wram(&machine, at + 0x1C), *id);
                assert_eq!(wram(&machine, at + 0x1B), unit.unknown_00);
                assert_eq!(wram(&machine, at + 2), unit.level);
                assert_eq!(wram16(&machine, at + 0x0A), u32::from(unit.hp));
                assert_eq!(wram16(&machine, at + 0x0C), u32::from(unit.ki));
                assert_eq!(wram(&machine, at + 0x0E), unit.unknown_0a);
                assert_eq!(wram(&machine, at + 0x17), unit.unknown_09);
                // The battle starts the unit's BP at a random value near the table's.
                let bp = wram24(&machine, at + 0x07);
                assert!(
                    bp >= unit.bp / 2 && bp <= unit.bp + unit.bp / 2,
                    "BP {bp} of {}",
                    unit.name.text
                );
            }
        }
    }
    assert_eq!(group.units().collect::<Vec<_>>(), [4], "the reference battle is one Saibai");

    // Frame 5600: "Piccolo uses a special attack on Saibai!" is on the screen, in tile words.
    run_to(&mut machine, 5_600);
    let ram = machine.wram().to_vec();
    assert!(contains(&ram, &tile_words(&catalogue.characters[2].name.tiles)), "Piccolo's name");
    assert!(contains(&ram, &tile_words(&catalogue.enemies[4].name.tiles)), "the Saibai's name");

    // Frame 7000: the battle is won, and Goku's EXP is the reward of the Saibai.
    run_to(&mut machine, 7_000);
    assert_eq!(wram24(&machine, records + 0x0F), u32::from(catalogue.enemies[4].exp_reward));
}

#[test]
#[ignore = "needs the ROM"]
fn the_status_tables_give_the_figures_of_the_original_status_screen() {
    let catalogue = catalogue();
    let labels = ssd_catalogue::menu_labels(&catalogue);
    let status = &labels.status;
    let techniques = |unit: usize| -> Vec<(String, u8)> {
        status.techniques[unit]
            .iter()
            .map(|&id| {
                let command = &status.commands[usize::from(id)];
                (command.name.clone(), command.ki_cost)
            })
            .collect()
    };
    // The reference capture of the original's Status page (Goku level 47, Gohan level 40, Piccolo, Krillin, Vegeta).
    let goku = [("Ki Blast", 2), ("Energy Wave", 4), ("Solar Flare", 6), ("KameHameHa", 8)];
    assert_eq!(techniques(0x00), goku.map(|(name, cost)| (name.to_owned(), cost)));
    let gohan = [("Energy Wave", 4), ("Masenko", 15)];
    assert_eq!(techniques(0x09), gohan.map(|(name, cost)| (name.to_owned(), cost)));
    assert_eq!(techniques(0x3A).last().unwrap(), &("Spirit Bomb".to_owned(), 20));
    // Gohan at level 40 with 65400 EXP needs 7750 more; Goku at his cap of 47 has no level to reach.
    assert_eq!(status.levels[1][40] - 65_400, 7_750);
    assert_eq!(status.levels[0].len(), 47, "Goku's rows end at his level cap");
    assert_eq!(status.levels[0].get(47), None);
    assert_eq!(status.levels[11].len(), 3);
    assert!(status.levels[8].is_empty(), "Nail has no level table");
}
