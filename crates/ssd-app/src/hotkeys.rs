//! Emulator hotkeys, speed multiplier and save-state slot paths.

use sdl3::keyboard::Scancode;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hotkey {
    SaveState,
    LoadState,
    Slot(u8),
    CycleSpeed,
    /// Saves the picture of the window as a PNG.
    Screenshot,
    Pause,
    /// Shows or hides the debug panel.
    TogglePanel,
    /// Opens the Cheats screen over a battle (Enhanced).
    BattleCheats,
    Quit,
}

/// The keys that belong to the window and cannot be remapped: the panel, the Cheats over a battle, the four
/// state slots and Esc. The
/// other shortcuts (speed, pause, save and load state) are the Controls page's (`Bindings::key_hotkey`).
pub(crate) fn fixed_hotkey(sc: Scancode) -> Option<Hotkey> {
    Some(match sc {
        Scancode::F1 => Hotkey::TogglePanel,
        Scancode::F3 => Hotkey::BattleCheats,
        Scancode::_1 => Hotkey::Slot(1),
        Scancode::_2 => Hotkey::Slot(2),
        Scancode::_3 => Hotkey::Slot(3),
        Scancode::_4 => Hotkey::Slot(4),
        Scancode::Escape => Hotkey::Quit,
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Speed(u8);

impl Default for Speed {
    fn default() -> Self {
        Self(1)
    }
}

impl Speed {
    #[must_use]
    pub fn next(self) -> Self {
        Self(if self.0 >= 8 { 1 } else { self.0 * 2 })
    }

    #[must_use]
    pub fn multiplier(self) -> u32 {
        u32::from(self.0)
    }

    /// The speeds the panel offers.
    pub const ALL: [Speed; 4] = [Speed(1), Speed(2), Speed(4), Speed(8)];
}

pub(crate) fn state_path(slot: u8) -> PathBuf {
    crate::paths::states_dir().join(format!("slot{slot}.ssdstate"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_cycles_through_1_2_4_8() {
        let s = Speed::default();
        assert_eq!(s.multiplier(), 1);
        let s = s.next();
        assert_eq!(s.multiplier(), 2);
        assert_eq!(s.next().next().multiplier(), 8);
        assert_eq!(s.next().next().next().multiplier(), 1);
        let offered: Vec<u32> = Speed::ALL.iter().map(|s| s.multiplier()).collect();
        assert_eq!(offered, [1, 2, 4, 8]);
    }

    #[test]
    fn state_path_is_one_file_per_slot_under_states_dir() {
        let _guard =
            crate::paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        assert_eq!(state_path(2), dir.path().join("states").join("slot2.ssdstate"));
        assert_ne!(state_path(1), state_path(2));
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn the_fixed_hotkeys_are_the_panel_the_slots_and_quit() {
        use sdl3::keyboard::Scancode;
        assert_eq!(fixed_hotkey(Scancode::_2), Some(Hotkey::Slot(2)));
        assert_eq!(fixed_hotkey(Scancode::F1), Some(Hotkey::TogglePanel));
        assert_eq!(fixed_hotkey(Scancode::F3), Some(Hotkey::BattleCheats));
        assert_eq!(fixed_hotkey(Scancode::Escape), Some(Hotkey::Quit));
        assert_eq!(fixed_hotkey(Scancode::F2), None, "the slots moved to the digit keys");
        for remappable in [Scancode::F5, Scancode::F9, Scancode::Tab, Scancode::P, Scancode::X] {
            assert_eq!(fixed_hotkey(remappable), None, "{remappable:?} is the Controls page's");
        }
    }
}
