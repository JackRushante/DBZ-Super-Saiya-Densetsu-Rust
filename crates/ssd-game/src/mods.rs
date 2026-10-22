//! The redirectors of the data packs (M10): the wrappers that give the game's readers the pack's
//! data for a slot, read from the directory of the image the machine runs (`ssd_mod::find`).
//!
//! A wrapper follows the cheats' pattern ([`crate::cheats`]): it runs at an instruction the game reaches
//! and either declines, with nothing written and `resume_at` its own key, or replaces the pointer the game
//! just loaded and then does the same. Declining is always the case when there is no pack, so a run
//! without packs is the original's. The wrappers cost nothing of their own (`cycles: Some(0)`), as the
//! cheats' do. Their keys are the stops of the recompiler too ([`entry_points`]).
//!
//! The families: the scripts ([`families`], `03:ACCA`), the face portraits (`01:CA0B`), the message lists
//! (`01:D3D5`) and the strings of the dialogue and the narration (`01:CF63`), and the extension commands of the
//! scene language, which the dispatcher of `03:AC41` runs ([`scene_ext`]). The data
//! of a slot is `ssd_mod::find`'s address; the directory lives in the image, so nothing is installed
//! per pack and [`install`] takes no argument.
//!
//! Two more hooks serve the pack's triggers and objects: the per-frame trigger dispatch, which starts a pack's scene
//! ([`triggers`], `03:F163`), and the talk to an object of a pack map, which speaks the pack's message ([`talk`], `03:9AFC`).
//! The data tables (unit rows, names and command lists, attacks, groups, level rows and caps) are the redirectors of [`tables`].

mod bp_copies;
mod bp_damage;
mod bp_kaioken;
pub mod chapter;
mod families;
mod features;
mod maps;
mod progression;
mod redirect;
mod scaling;
mod scene_ext;
pub mod state;
mod tables;
mod talk;
mod triggers;

use ssd_oracle::{HookAction, HookEntry, HookMode, HookTable, ReturnKind};

/// The hook entries of the redirectors, in Rust mode (`mod_script`, `mod_face`, `mod_message`, `mod_string`,
/// `mod_scene_ext`).
#[must_use]
pub fn hook_entries() -> Vec<HookEntry> {
    let entry = |name: &str, key: u32, func: ssd_oracle::RoutineFn| HookEntry {
        name: name.to_owned(),
        key,
        // A redirector must act on every call: a patch never falls back to the original for timing.
        mode: HookMode::Patch,
        action: HookAction::Routine { func, ret: ReturnKind::Rtl },
        entry_m8: None,
        entry_x8: None,
        flags_mask: 0,
        verify_sample: 1,
        // A wrapper's own cost is zero, as the cheats' (see the module documentation).
        cycles: Some(0),
        bound: None,
        budget_frames: None,
    };
    let mut entries = vec![
        entry("mod_script", families::SCRIPT_KEY, families::script_wrapper),
        entry("mod_face", families::FACE_KEY, families::face_wrapper),
        entry("mod_message", families::MESSAGE_KEY, families::message_wrapper),
        entry("mod_string", families::STRING_KEY, families::string_wrapper),
        // The dispatcher is entered with the accumulator 8-bit (see `scene_ext`): the hook runs only there.
        HookEntry {
            entry_m8: Some(true),
            ..entry("mod_scene_ext", scene_ext::DISPATCH_KEY, scene_ext::scene_ext_wrapper)
        },
        // The pack maps (`maps`): the header, the block words, the objects, the zones and their group, the palettes,
        // the start, the trigger dispatch and the entrances. Each declines for an original map.
        entry("mod_map_header", maps::HEADER_KEY, maps::header_wrapper),
        entry("mod_map_block", maps::BLOCK_KEY, maps::block_wrapper),
        entry("mod_map_objects", maps::OBJECTS_KEY, maps::objects_wrapper),
        entry("mod_map_zone", maps::ZONE_KEY, maps::zone_wrapper),
        entry("mod_map_reset", maps::RESET_KEY, maps::reset_wrapper),
        entry("mod_map_group", maps::GROUP_KEY, maps::group_wrapper),
        entry("mod_map_palette_879a", maps::PALETTE_KEYS[0], maps::palette_879a),
        entry("mod_map_palette_87c6", maps::PALETTE_KEYS[1], maps::palette_87c6),
        entry("mod_map_palette_8e01", maps::PALETTE_KEYS[2], maps::palette_8e01),
        entry("mod_map_palette_86db", maps::PALETTE_KEYS[3], maps::palette_86db),
        entry("mod_map_start_x", maps::START_X_KEY, maps::start_x_wrapper),
        entry("mod_map_start_y", maps::START_Y_KEY, maps::start_y_wrapper),
        entry("mod_map_trigger", maps::TRIGGER_KEY, maps::trigger_wrapper),
        entry("mod_map_entrance", maps::ENTRANCE_KEY, maps::entrance_wrapper),
        entry("mod_map_list", maps::LIST_KEY, maps::list_wrapper),
        // The per-frame trigger dispatch (`triggers`), before the original's gates: it starts a pack's scene when one holds.
        entry("mod_trigger_dispatch", triggers::DISPATCH_KEY, triggers::dispatch_wrapper),
        // The talk to a map object (`talk`), after the object's own action: a pack map's object speaks its pack message.
        entry("mod_object_talk", talk::TALK_KEY, talk::talk_wrapper),
    ];
    // The data tables of the packs (`tables`): the unit rows, names and command lists, the attacks, the groups and the levels.
    entries.extend(tables::hook_entries());
    // The wide BP of the battle and the boost (`state`): the damage, the snapshot and copies, the Kaioken boost.
    entries.extend(bp_damage::hook_entries());
    entries.extend(bp_copies::hook_entries());
    entries.extend(bp_kaioken::hook_entries());
    // The chapter wrappers (`chapter`): the two endings, which mark the game completed.
    entries.extend(chapter::hook_entries());
    // The named features of the packs (`features`): each declines when its slot is not in the image.
    entries.extend(features::hook_entries());
    entries
}

/// The addresses of the redirectors. The recompiler must stop at each of them, as it does at the
/// cheats' (`xtask recompile`): generated code calls its callees directly, so a redirector it ran
/// inline would never see the game's reader.
#[must_use]
pub fn entry_points() -> Vec<u32> {
    hook_entries().into_iter().map(|entry| entry.key).collect()
}

/// Installs the redirectors into `table`, replacing the entries at their addresses. Only the
/// Enhanced mode gets them: Classic never installs them.
pub fn install(table: &mut HookTable) {
    for entry in hook_entries() {
        table.insert(entry);
    }
}

/// Whether the redirectors are in `table` (the script's entry is the redirector's). A table that was
/// replaced by another one does not have them.
#[must_use]
pub fn installed_in(table: Option<&HookTable>) -> bool {
    table
        .and_then(|table| table.get(families::SCRIPT_KEY))
        .is_some_and(|entry| entry.name == "mod_script")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entries_exist_run_in_rust_and_cost_nothing() {
        let entries = hook_entries();
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "mod_script",
                "mod_face",
                "mod_message",
                "mod_string",
                "mod_scene_ext",
                "mod_map_header",
                "mod_map_block",
                "mod_map_objects",
                "mod_map_zone",
                "mod_map_reset",
                "mod_map_group",
                "mod_map_palette_879a",
                "mod_map_palette_87c6",
                "mod_map_palette_8e01",
                "mod_map_palette_86db",
                "mod_map_start_x",
                "mod_map_start_y",
                "mod_map_trigger",
                "mod_map_entrance",
                "mod_map_list",
                "mod_trigger_dispatch",
                "mod_object_talk",
                "mod_command_list",
                "mod_unit_name",
                "mod_enemy_stats",
                "mod_character_stats",
                "mod_party_stats",
                "mod_unit_sprite",
                "mod_unit_anchor",
                "mod_unit_source",
                "mod_unit_pieces",
                "mod_unit_staging",
                "mod_attack_name",
                "mod_attack_ki",
                "mod_attack_animation",
                "mod_group",
                "mod_level_row",
                "mod_level_threshold",
                "mod_cap",
                "mod_cap_check",
                "mod_damage_shift",
                "mod_damage_restore",
                "mod_snapshot_save",
                "mod_snapshot_restore",
                "mod_side_a_copy",
                "mod_side_b_copy",
                "mod_side_a_copy_back",
                "mod_side_b_copy_back",
                "mod_kaioken_boost_start",
                "mod_kaioken_boost_end",
                "mod_kaioken_ki_restore",
                "mod_kaioken_stop_restore",
                "mod_completed_11",
                "mod_completed_12",
                "mod_ssj_kaioken_multiplier",
                "mod_ssj_kaioken_hp_cost",
                "mod_ssj_unit_store_1",
                "mod_ssj_unit_store_2",
                "mod_ssj_unit_store_3",
                "mod_ssj_unit_store_4",
                "mod_ssj_unit_store_5",
                "mod_ssj_unit_store_6",
                "mod_ssj_unit_store_7",
                "mod_ssj_unit_store_8",
                "mod_fp_raditz_goku",
                "mod_fp_raditz_piccolo",
                "mod_fp_encounters",
                "mod_fp_deadline_piccolo",
                "mod_fp_deadline_gohan",
                "mod_fp_ssj_trigger",
                "mod_fp_ssj_build",
                "mod_fp_ginyu_arrival",
                "mod_fp_piccolo_bp_low",
                "mod_fp_piccolo_bp_high",
                "mod_enemy_scaling_stats",
                "mod_enemy_scaling_exp",
            ]
        );
        for entry in &entries {
            assert_eq!(entry.mode, HookMode::Patch, "{}", entry.name);
            assert_eq!(entry.cycles, Some(0), "{}", entry.name);
            assert!(matches!(entry.action, HookAction::Routine { .. }), "{}", entry.name);
        }
        assert_eq!(
            entry_points(),
            [
                families::SCRIPT_KEY,
                families::FACE_KEY,
                families::MESSAGE_KEY,
                families::STRING_KEY,
                scene_ext::DISPATCH_KEY,
                maps::HEADER_KEY,
                maps::BLOCK_KEY,
                maps::OBJECTS_KEY,
                maps::ZONE_KEY,
                maps::RESET_KEY,
                maps::GROUP_KEY,
                maps::PALETTE_KEYS[0],
                maps::PALETTE_KEYS[1],
                maps::PALETTE_KEYS[2],
                maps::PALETTE_KEYS[3],
                maps::START_X_KEY,
                maps::START_Y_KEY,
                maps::TRIGGER_KEY,
                maps::ENTRANCE_KEY,
                maps::LIST_KEY,
                triggers::DISPATCH_KEY,
                talk::TALK_KEY,
                tables::COMMAND_LIST_KEY,
                tables::UNIT_NAME_KEY,
                tables::ENEMY_STATS_KEY,
                tables::CHARACTER_STATS_KEY,
                tables::PARTY_STATS_KEY,
                tables::SPRITE_KEY,
                tables::UNIT_ANCHOR_KEY,
                tables::UNIT_SOURCE_KEY,
                tables::UNIT_PIECES_KEY,
                tables::UNIT_STAGING_KEY,
                tables::ATTACK_NAME_KEY,
                tables::ATTACK_KI_KEY,
                tables::ATTACK_ANIMATION_KEY,
                tables::GROUP_KEY,
                tables::LEVEL_ROW_KEY,
                tables::LEVEL_THRESHOLD_KEY,
                tables::CAP_KEY,
                tables::CAP_CHECK_KEY,
                bp_damage::ENTRY_KEY,
                bp_damage::EXIT_KEY,
                bp_copies::SNAPSHOT_SAVE_KEY,
                bp_copies::SNAPSHOT_RESTORE_KEY,
                bp_copies::SIDE_A_COPY_KEY,
                bp_copies::SIDE_B_COPY_KEY,
                bp_copies::SIDE_A_COPY_BACK_KEY,
                bp_copies::SIDE_B_COPY_BACK_KEY,
                bp_kaioken::BOOST_START_KEY,
                bp_kaioken::BOOST_END_KEY,
                bp_kaioken::KI_RESTORE_KEY,
                bp_kaioken::STOP_RESTORE_KEY,
                chapter::ENDING_11,
                chapter::ENDING_12,
                features::SSJ_MULTIPLIER_KEY,
                features::SSJ_HP_COST_KEY,
                features::SSJ_UNIT_STORE_KEYS[0],
                features::SSJ_UNIT_STORE_KEYS[1],
                features::SSJ_UNIT_STORE_KEYS[2],
                features::SSJ_UNIT_STORE_KEYS[3],
                features::SSJ_UNIT_STORE_KEYS[4],
                features::SSJ_UNIT_STORE_KEYS[5],
                features::SSJ_UNIT_STORE_KEYS[6],
                features::SSJ_UNIT_STORE_KEYS[7],
                progression::RADITZ_GOKU_KEY,
                progression::RADITZ_PICCOLO_KEY,
                progression::ENCOUNTER_KEY,
                progression::DEADLINE_PICCOLO_KEY,
                progression::DEADLINE_GOHAN_KEY,
                progression::SSJ_TRIGGER_KEY,
                progression::SSJ_BUILD_KEY,
                progression::GINYU_KEY,
                progression::PICCOLO_BP_LOW_KEY,
                progression::PICCOLO_BP_HIGH_KEY,
                scaling::STATS_KEY,
                scaling::EXP_KEY,
            ]
        );
    }

    #[test]
    fn the_scene_extension_hook_is_the_dispatch_with_an_8_bit_accumulator() {
        let entries = hook_entries();
        let scene = entries.iter().find(|entry| entry.name == "mod_scene_ext").unwrap();
        assert_eq!(scene.key, 0x03_AC41);
        assert_eq!(scene.entry_m8, Some(true));
        assert_eq!(scene.entry_x8, None, "the wrapper does not use the index widths");
        assert_eq!(scene.cycles, Some(0));
        assert_eq!(scene.mode, HookMode::Patch);
    }

    #[test]
    fn no_redirector_key_is_a_cheat_key() {
        let cheats = crate::cheats::entry_points();
        for key in entry_points() {
            assert!(!cheats.contains(&key), "${key:06X} is a cheat's key");
        }
    }

    #[test]
    fn install_puts_the_redirectors_in_the_table_and_installed_in_finds_them() {
        let mut table = HookTable::new();
        assert!(!installed_in(Some(&table)), "an empty table has none");
        install(&mut table);
        assert!(installed_in(Some(&table)));
        assert!(!installed_in(None));
        assert_eq!(
            table.get(families::FACE_KEY).map(|entry| entry.name.as_str()),
            Some("mod_face")
        );
    }
}
