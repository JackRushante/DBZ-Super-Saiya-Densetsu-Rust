//! The mod state of the save slots (M10, Task 9): the extended flags and the high parts of the wide BP. Both live in
//! work RAM outside the save image, so a save keeps them in the sidecar file `ssd.mods` beside the saves, as the cheats
//! keep `ssd.cheats`. The layout is the game's (`ssd_game::mods::state`, the block and the flags bank).
//!
//! `flags` holds the 256 bytes of the extended flags (`$7F:FD00`) and `high` the 56 bytes of the block that hold the
//! wide BP's high parts (`$7F:FB00`). Each is `None` when its marker was not in work RAM at the save, so a slot saved
//! without them loads with them neutral.

use serde::{Deserialize, Serialize};

use crate::cheats::config;

/// The layout version of [`ModsSidecar`].
pub const SIDECAR_VERSION: u32 = 1;
/// The bytes of the extended flags (2048 bits, `$7F:FD00`).
pub const FLAG_BYTES: usize = 256;
/// The bytes of the block the sidecar keeps: the wide BP's high parts (`$7F:FB00` to `$7F:FB37`).
pub const HIGH_BYTES: usize = 0x38;

/// The mod state of one save: the flags and the high parts, each with its marker when there was one.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModState {
    /// The 256 bytes of the extended flags, when their marker was there.
    pub flags: Option<Vec<u8>>,
    /// The 56 bytes of the wide BP's high parts, when the block's marker was there.
    pub high: Option<Vec<u8>>,
}

impl ModState {
    /// Whether neither marker was there: the save keeps nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.flags.is_none() && self.high.is_none()
    }

    /// Checks the lengths of the two parts.
    ///
    /// # Errors
    ///
    /// A part whose length is not the layout's.
    pub fn check(&self) -> Result<(), String> {
        if self.flags.as_ref().is_some_and(|flags| flags.len() != FLAG_BYTES) {
            return Err(format!("the extended flags are {FLAG_BYTES} bytes"));
        }
        if self.high.as_ref().is_some_and(|high| high.len() != HIGH_BYTES) {
            return Err(format!("the high parts are {HIGH_BYTES} bytes"));
        }
        Ok(())
    }

    /// The bytes of the application event that carries the state in a replay ([`ssd_game`]'s `MODS_TAG`).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(self, config()).unwrap_or_default()
    }

    /// Reads the bytes of [`ModState::to_bytes`].
    ///
    /// # Errors
    ///
    /// The bytes are not a mod state, or a part has the wrong length.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let (state, _): (Self, usize) = bincode::serde::decode_from_slice(bytes, config())
            .map_err(|error| format!("not a mod state: {error}"))?;
        state.check()?;
        Ok(state)
    }
}

/// The state of one save slot, with the checksum word that says which save it belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedMods {
    /// The save slot, 0 to 2.
    pub slot: u8,
    /// The checksum word of the slot when the state was saved.
    pub checksum: u16,
    pub state: ModState,
}

/// The sidecar file (`saves/ssd.mods`): the mod state of the save slots, versioned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModsSidecar {
    /// The file layout's version.
    pub version: u32,
    pub saves: Vec<SavedMods>,
}

impl ModsSidecar {
    /// The empty sidecar of this version: what a save made before the file had no entry for is.
    #[must_use]
    pub fn empty() -> Self {
        Self { version: SIDECAR_VERSION, saves: Vec::new() }
    }

    /// The file's bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(self, config()).unwrap_or_default()
    }

    /// Reads the file's bytes. An entry whose parts have the wrong length is left out, so its save loads neutral.
    ///
    /// # Errors
    ///
    /// The bytes are not a mod state file, or of another version.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let (version, _): (u32, usize) = bincode::serde::decode_from_slice(bytes, config())
            .map_err(|error| format!("not a mod state file: {error}"))?;
        if version != SIDECAR_VERSION {
            return Err(format!("mod state file version {version}, expected {SIDECAR_VERSION}"));
        }
        let (sidecar, _): (Self, usize) = bincode::serde::decode_from_slice(bytes, config())
            .map_err(|error| format!("not a mod state file: {error}"))?;
        let saves = sidecar.saves.into_iter().filter(|saved| saved.state.check().is_ok()).collect();
        Ok(Self { saves, ..sidecar })
    }

    /// The state saved for `slot` with `checksum`, if the sidecar has an entry for exactly that save.
    #[must_use]
    pub fn entry_for(&self, slot: u8, checksum: u16) -> Option<&ModState> {
        self.saves
            .iter()
            .find(|saved| saved.slot == slot && saved.checksum == checksum)
            .map(|saved| &saved.state)
    }

    /// Stores `state` for `slot` with `checksum`, replacing the slot's older entry. A state that keeps nothing leaves
    /// no entry, as a save without markers has none.
    pub fn store(&mut self, slot: u8, checksum: u16, state: ModState) {
        self.saves.retain(|saved| saved.slot != slot);
        if !state.is_empty() {
            self.saves.push(SavedMods { slot, checksum, state });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> ModState {
        ModState {
            flags: Some((0..FLAG_BYTES).map(|k| k as u8).collect()),
            high: Some((0..HIGH_BYTES).map(|k| (k * 3) as u8).collect()),
        }
    }

    #[test]
    fn a_sidecar_is_written_and_read_back_with_its_entries() {
        let mut sidecar = ModsSidecar::empty();
        sidecar.store(1, 0xBEEF, full());
        let bytes = sidecar.to_bytes();
        let back = ModsSidecar::from_bytes(&bytes).expect("a mod state file");
        assert_eq!(back, sidecar);
        assert_eq!(back.entry_for(1, 0xBEEF), Some(&full()));
        assert_eq!(back.entry_for(1, 0xBEE0), None, "another checksum is another save");
        assert_eq!(back.entry_for(0, 0xBEEF), None, "another slot is another save");
    }

    #[test]
    fn a_state_that_keeps_nothing_leaves_no_entry_and_replaces_the_old_one() {
        let mut sidecar = ModsSidecar::empty();
        sidecar.store(0, 7, full());
        sidecar.store(0, 7, ModState::default());
        assert!(sidecar.saves.is_empty(), "the slot's entry is removed");
        sidecar.store(2, 9, full());
        sidecar.store(2, 10, ModState { flags: None, high: Some(vec![1; HIGH_BYTES]) });
        assert_eq!(sidecar.saves.len(), 1, "the slot has one entry");
        assert_eq!(sidecar.entry_for(2, 10).and_then(|state| state.flags.as_ref()), None);
    }

    #[test]
    fn a_missing_or_old_file_is_the_empty_sidecar_and_a_damaged_one_is_refused() {
        assert_eq!(ModsSidecar::empty().saves, Vec::new());
        assert!(ModsSidecar::from_bytes(&[]).is_err(), "no bytes is not a file");
        assert!(ModsSidecar::from_bytes(&[0xFF; 9]).is_err(), "damaged bytes are refused");
        let mut other = ModsSidecar::empty();
        other.version = 7;
        assert!(ModsSidecar::from_bytes(&other.to_bytes()).is_err(), "another version is refused");
    }

    #[test]
    fn an_entry_with_a_wrong_length_is_left_out_and_the_state_bytes_round_trip() {
        let mut sidecar = ModsSidecar::empty();
        sidecar.saves.push(SavedMods {
            slot: 0,
            checksum: 1,
            state: ModState { flags: Some(vec![0; 3]), high: None },
        });
        let back = ModsSidecar::from_bytes(&sidecar.to_bytes()).expect("a mod state file");
        assert!(back.saves.is_empty(), "a short flag bank is not kept");
        let state = full();
        assert_eq!(ModState::from_bytes(&state.to_bytes()), Ok(state));
        assert!(ModState::from_bytes(&[0xAA; 4]).is_err());
    }
}
