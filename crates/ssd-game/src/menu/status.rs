//! The Status screen's data: what the original's two Status pages show (`docs/re/data/menus.md`), read from the party records
//! of work RAM and the cartridge's tables the labels carry. Nothing is written: the HD screen reads the game's state, and the
//! original's own Status screen stays under it, untouched.
//!
//! A record's bytes (`docs/re/data/characters.md`): `+01` the unit id (its command list is the techniques), `+0E` the figure
//! the original labels "PLA.", `+0F` to `+11` the EXP, `+1B` the class (the kanji under the portrait; the same number as a
//! card's symbol).

use super::presses::{Button, Step, Until};
use super::{MenuProbe, RECORD_CHARACTER, RECORD_FLAGS, RECORD_IN_USE, RECORD_LENGTH, RECORDS};
use crate::data as d;
use ssd_menus::screens::{StatusMember, StatusView, Technique};

/// The characters the level cap extends: those with a level table of the game's (Goku to Vegeta).
const EXTENDED_IDS: u8 = 8;
/// The record's unit id: the key of the unit tables.
const RECORD_UNIT: u32 = 0x01;
/// The record's "PLA." figure.
const RECORD_PLA: u32 = 0x0E;
/// The record's class.
const RECORD_CLASS: u32 = 0x1B;

impl MenuProbe<'_> {
    /// The Status screen of the party, with the cursor on member `cursor` (kept inside the party). The members are the records
    /// in use in the order of their slots, which is the order the original's Status page lists them in (the Order screen swaps
    /// the records themselves).
    #[must_use]
    pub fn status_view(&self, cursor: usize) -> StatusView {
        let members: Vec<StatusMember> =
            self.status_records().filter_map(|at| self.status_member(at)).collect();
        let cursor = cursor.min(members.len().saturating_sub(1));
        StatusView { members, cursor }
    }

    /// The EXP the level after `level` begins at, past the game's own rows: only for a character the pack's level cap extends
    /// (`StatusLabels::level_cap`) that has not reached it.
    fn extended_start(&self, character: u8, level: u16) -> Option<u64> {
        let cap = self.labels.status.level_cap?;
        if character >= EXTENDED_IDS || level >= cap {
            return None;
        }
        self.labels.growth.get(usize::from(character))?.stats_at(level + 1, true).map(|row| row.exp)
    }

    /// The members Status lists.
    #[must_use]
    pub fn status_count(&self) -> usize {
        self.status_records().count()
    }

    /// The bus addresses of the party records in use, in slot order.
    fn status_records(&self) -> impl Iterator<Item = u32> + '_ {
        (0..RECORDS).map(|slot| d::PARTY_RECORDS + slot * RECORD_LENGTH).filter(|&record| {
            self.byte(record + RECORD_FLAGS).is_some_and(|flags| flags & RECORD_IN_USE != 0)
        })
    }

    /// The Status figures of the member whose record is at `record`.
    fn status_member(&self, record: u32) -> Option<StatusMember> {
        let character = self.byte(record + RECORD_CHARACTER)?;
        let member = self.member(character)?;
        let exp = self.exp_of(character, record)?;
        let level = member.level;
        let tables = &self.labels.status;
        // The EXP at which the next level begins (the row of the level after this one), as the original's "EX LV UP." counts
        // it; a character with no level table, or at the top of it, has none to reach. Past the game's own rows, with the
        // pack's level cap, the extension's row of the next level.
        let next = tables
            .levels
            .get(usize::from(character))
            .and_then(|rows| rows.get(usize::from(level)))
            .map(|&start| u64::from(start))
            .or_else(|| self.extended_start(character, level));
        let unit = self.byte(record + RECORD_UNIT)?;
        let techniques = tables
            .techniques
            .get(usize::from(unit))
            .map(|ids| {
                ids.iter()
                    .filter_map(|&id| tables.commands.get(usize::from(id)))
                    .map(|command| Technique { name: command.name.clone(), cost: command.ki_cost })
                    .collect()
            })
            .unwrap_or_default();
        Some(StatusMember {
            name: member.name,
            level: member.level,
            hp: member.hp,
            max_hp: member.max_hp,
            ki: member.ki,
            max_ki: member.max_ki,
            bp: member.bp,
            pla: self.byte(record + RECORD_PLA)?,
            exp,
            to_next: next.map_or(0, |start| start.saturating_sub(exp)),
            class: self.byte(record + RECORD_CLASS)?,
            face: member.face,
            techniques,
        })
    }
}

/// The steps of Back from the HD Status screen: the original sits on its Status page 1 (the HD never opens a member's page 2),
/// and B takes it back to its Menu list.
pub(super) fn back_steps() -> Vec<Step> {
    vec![Step::Press { button: Button::B, until: Until::Screen(MENU_SCREEN) }]
}

/// The steps of Start on the HD Status screen: the menu closes from where it is, with the original's own B presses (Status
/// page 1 to the Menu list, the Menu list to the command window, the command window to the field).
pub(super) fn close_steps() -> Vec<Step> {
    vec![
        Step::Press { button: Button::B, until: Until::Screen(MENU_SCREEN) },
        Step::Press { button: Button::B, until: Until::Command },
        Step::Press { button: Button::B, until: Until::Closed },
    ]
}

/// The original's Menu list screen.
const MENU_SCREEN: u8 = 4;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data as d;
    use ssd_menus::{CharacterLabel, CommandLabel, Labels, StatusLabels};

    /// A party record of `character` (unit `unit`) with the figures the original's Status shows for Gohan in the reference
    /// capture: LV 40, HP 560, KI 142, BP 42673, PLA. 06, EX 65400, class 2.
    fn record(character: u8, unit: u8, level: u8, class: u8) -> [u8; 32] {
        let mut bytes = [0_u8; 32];
        bytes[0] = 0xC0;
        bytes[1] = unit;
        bytes[2] = level;
        bytes[3..5].copy_from_slice(&560_u16.to_le_bytes());
        bytes[5..7].copy_from_slice(&142_u16.to_le_bytes());
        bytes[7..10].copy_from_slice(&42673_u32.to_le_bytes()[..3]);
        bytes[10..12].copy_from_slice(&560_u16.to_le_bytes());
        bytes[12..14].copy_from_slice(&142_u16.to_le_bytes());
        bytes[0x0E] = 6;
        bytes[0x0F..0x12].copy_from_slice(&65400_u32.to_le_bytes()[..3]);
        bytes[0x1B] = class;
        bytes[0x1C] = character;
        bytes
    }

    /// Work RAM with `records` in the party records (the first record is the first slot), `order` as the party order.
    fn ram(records: &[[u8; 32]], order: &[u8]) -> Vec<u8> {
        let mut ram = vec![0_u8; 0x2_0000];
        let at = |address: u32| (address - 0x7E_0000) as usize;
        for (slot, bytes) in records.iter().enumerate() {
            let start = at(d::PARTY_RECORDS) + 32 * slot;
            ram[start..start + 32].copy_from_slice(bytes);
        }
        let start = at(d::PARTY_ORDER);
        ram[start..start + order.len()].copy_from_slice(order);
        ram[start + order.len()] = 0xFF;
        ram
    }

    fn labels() -> Labels {
        let command = |name: &str, ki_cost| CommandLabel { name: name.to_owned(), ki_cost };
        let mut commands = vec![CommandLabel::default(); 0x3A];
        commands[0x01] = command("Ki Blast", 2);
        commands[0x38] = command("Energy Wave", 4);
        commands[0x39] = command("Masenko", 15);
        let mut techniques = vec![Vec::new(); 0x0A];
        techniques[0x09] = vec![0x38, 0x39];
        // Level 41 begins at 73150 EXP: 7750 past Gohan's 65400. The row after level 40.
        let mut gohan = vec![0_u32; 41];
        gohan[40] = 73150;
        let levels = vec![Vec::new(), gohan];
        let characters = vec![
            CharacterLabel { name: "Goku".into(), face: Some(0), class: None },
            CharacterLabel { name: "Gohan".into(), face: Some(9), class: None },
        ];
        Labels {
            characters,
            status: StatusLabels { commands, techniques, levels, level_cap: None },
            ..Labels::default()
        }
    }

    #[test]
    fn the_status_figures_are_the_records_and_the_cartridge_tables() {
        let labels = labels();
        let ram = ram(&[record(1, 0x09, 40, 2)], &[1]);
        let view = MenuProbe::new(&ram, &labels).status_view(0);
        assert_eq!(view.members.len(), 1);
        let gohan = &view.members[0];
        assert_eq!(gohan.name, "Gohan");
        assert_eq!(
            (gohan.level, gohan.hp, gohan.max_hp, gohan.ki, gohan.max_ki),
            (40, 560, 560, 142, 142)
        );
        assert_eq!((gohan.bp, gohan.pla, gohan.exp), (42673, 6, 65400));
        assert_eq!(gohan.to_next, 7750, "the EXP the next level begins at, less the EXP");
        assert_eq!((gohan.class, gohan.face), (2, Some(9)));
        let techniques: Vec<(&str, u8)> =
            gohan.techniques.iter().map(|t| (t.name.as_str(), t.cost)).collect();
        assert_eq!(techniques, [("Energy Wave", 4), ("Masenko", 15)]);
    }

    #[test]
    fn the_party_is_the_records_in_use_in_slot_order_and_the_cursor_stays_inside_it() {
        let labels = labels();
        let mut empty = [0_u8; 32];
        empty[0x1C] = 5;
        // The Order screen swaps the records themselves: slot order is the party's order, whatever `party_order` holds.
        let ram = ram(&[record(1, 0x09, 40, 2), empty, record(0, 0x00, 47, 1)], &[]);
        let probe = MenuProbe::new(&ram, &labels);
        let names: Vec<String> = probe.status_view(0).members.into_iter().map(|m| m.name).collect();
        assert_eq!(names, ["Gohan", "Goku"], "an unused slot is not a member");
        assert_eq!(probe.status_count(), 2);
        assert_eq!(probe.status_view(7).cursor, 1, "a cursor past the party is the last member");
        assert_eq!(probe.status_view(0).cursor, 0);
    }

    #[test]
    fn a_member_past_the_last_level_row_has_nothing_to_reach_and_no_party_gives_an_empty_screen() {
        let labels = labels();
        let mut top = record(1, 0x09, 40, 2);
        top[0x0F..0x12].copy_from_slice(&80_000_u32.to_le_bytes()[..3]);
        let ram_top = ram(&[top], &[1]);
        assert_eq!(MenuProbe::new(&ram_top, &labels).status_view(0).members[0].to_next, 0);
        let mut last = record(1, 0x09, 41, 2);
        last[0x0F..0x12].copy_from_slice(&1_u32.to_le_bytes()[..3]);
        let ram_last = ram(&[last], &[1]);
        let member = &MenuProbe::new(&ram_last, &labels).status_view(0).members[0];
        assert_eq!(member.to_next, 0, "level 41 has no row after it in the table");
        let empty = ram(&[], &[]);
        let view = MenuProbe::new(&empty, &labels).status_view(3);
        assert_eq!((view.members.len(), view.cursor), (0, 0));
    }

    #[test]
    fn a_unit_with_no_list_has_no_techniques_and_an_unknown_command_is_left_out() {
        let labels = labels();
        let ram = ram(&[record(0, 0x7F, 40, 1), record(1, 0x09, 40, 2)], &[0, 1]);
        let view = MenuProbe::new(&ram, &labels).status_view(0);
        assert_eq!(view.members[0].techniques.len(), 0);
        let mut gap = labels;
        gap.status.techniques[0x09] = vec![0x38, 0xF0];
        let ram = self::ram(&[record(1, 0x09, 40, 2)], &[1]);
        let view = MenuProbe::new(&ram, &gap).status_view(0);
        assert_eq!(view.members[0].techniques.len(), 1, "command $F0 is not in the table");
    }

    /// Goku with a 47-row table (BP `100 n`, HP `10 n`, KI `2 n`, EXP `10 n^2`), the extension's anchor, and the pack's cap.
    fn wide_labels(cap: Option<u16>) -> Labels {
        let mut labels = labels();
        labels.status.level_cap = cap;
        let rows = 1..=47_u32;
        labels.growth = vec![ssd_menus::Growth {
            bp: rows.clone().map(|n| n * 100).collect(),
            max_hp: rows.clone().map(|n| (n * 10) as u16).collect(),
            max_ki: rows.clone().map(|n| (n * 2) as u16).collect(),
            exp: rows.clone().map(|n| n * n * 10).collect(),
            byte_0e: rows.map(|_| 1).collect(),
            cap: 47,
            min: 1,
        }];
        labels
    }

    /// The mod state of a run with Goku (the first record) at wide level `level`, EXP high byte `exp_high` and BP high part
    /// `bp_high`.
    fn with_wide_state(ram: &mut [u8], level: u16, exp_high: u8, bp_high: u16) {
        use crate::mods::state::{
            BLOCK, LEVELS_MARK, LEVELS_MARKER_AT, MARKER, MARKER_AT, exp_high_at, wide_level_at,
        };
        let at = |address: u32| (address - 0x7E_0000) as usize;
        ram[at(LEVELS_MARKER_AT)..at(LEVELS_MARKER_AT) + 4].copy_from_slice(&LEVELS_MARK);
        ram[at(wide_level_at(0))..at(wide_level_at(0)) + 2].copy_from_slice(&level.to_le_bytes());
        ram[at(exp_high_at(0))] = exp_high;
        ram[at(MARKER_AT)..at(MARKER_AT) + 4].copy_from_slice(&MARKER);
        ram[at(BLOCK)..at(BLOCK) + 2].copy_from_slice(&bp_high.to_le_bytes());
    }

    #[test]
    fn a_wide_level_the_exp_high_byte_and_the_wide_bp_are_read_from_the_mod_state() {
        let labels = wide_labels(Some(999));
        let mut ram = ram(&[record(0, 0x00, 99, 1)], &[0]);
        with_wide_state(&mut ram, 150, 0, 2);
        let goku = &MenuProbe::new(&ram, &labels).status_view(0).members[0];
        assert_eq!(goku.level, 150, "the wide level, the record's byte being 99");
        assert_eq!(goku.bp, (2 << 24) + 42673, "the wide BP's high part");
        assert_eq!(goku.exp, 65400);
        // The level after 150 begins at 22090 * (156 / 52)^2 = 198 810 EXP: the extension's row.
        assert_eq!(goku.to_next, 198_810 - 65_400);
        with_wide_state(&mut ram, 999, 1, 0x1D);
        let top = &MenuProbe::new(&ram, &labels).status_view(0).members[0];
        assert_eq!((top.level, top.exp, top.bp >> 24), (999, (1 << 24) + 65400, 0x1D));
        assert_eq!(top.to_next, 0, "the cap is the last level");
        // A record whose byte is not the wide level's saturation is read as the byte says, and without the pack the Status
        // screen has no level to reach past the game's table.
        let mut low = ram.clone();
        low[0x0B02] = 60;
        assert_eq!(MenuProbe::new(&low, &labels).status_view(0).members[0].level, 60);
        let plain = wide_labels(None);
        with_wide_state(&mut ram, 150, 0, 0);
        assert_eq!(MenuProbe::new(&ram, &plain).status_view(0).members[0].to_next, 0);
        // No mod state: the record's byte and its 24 bits.
        let bare = self::ram(&[record(0, 0x00, 99, 1)], &[0]);
        let goku = &MenuProbe::new(&bare, &labels).status_view(0).members[0];
        assert_eq!((goku.level, goku.bp, goku.exp), (99, 42673, 65400));
    }
}
