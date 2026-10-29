//! The mod state of the save slots (M10, Task 9): the extended flags and the high parts of the wide BP. Both live in
//! work RAM outside the save image, so a save keeps them in the sidecar file `ssd.mods` beside the saves, as the cheats
//! keep `ssd.cheats`. The layout is the game's (`ssd_game::mods::state`, the block and the flags bank).
//!
//! `flags` holds the 256 bytes of the extended flags (`$7F:FD00`) and `high` the 56 bytes of the block that hold the
//! wide BP's high parts (`$7F:FB00`). Each is `None` when its marker was not in work RAM at the save, so a slot saved
//! without them loads with them neutral.

use serde::{Deserialize, Serialize};

use crate::cheats::config;

/// The layout version of [`ModsSidecar`] (2: the wide levels; version 1 files are still read).
pub const SIDECAR_VERSION: u32 = 2;
/// The version before the wide levels.
const LEGACY_VERSION: u32 = 1;
/// The bytes of the extended flags (2048 bits, `$7F:FD00`).
pub const FLAG_BYTES: usize = 256;
/// The bytes of the block the sidecar keeps: the wide BP's high parts (`$7F:FB00` to `$7F:FB37`).
pub const HIGH_BYTES: usize = 0x38;

/// The bytes of the wide levels (`levels`): twelve levels of two bytes (little endian, character `id` at `2 * id`), then the
/// EXP's bits 24 to 31 of the twelve characters, one byte each.
pub const LEVEL_BYTES: usize = 36;

/// The mod state of one save: the flags, the high parts and the wide levels, each with its marker when there was one.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModState {
    /// The 256 bytes of the extended flags, when their marker was there.
    pub flags: Option<Vec<u8>>,
    /// The 56 bytes of the wide BP's high parts, when the block's marker was there.
    pub high: Option<Vec<u8>>,
    /// The 36 bytes of the wide levels and EXP's high bytes, when their marker was there (`free_progression`'s `level_cap`).
    pub levels: Option<Vec<u8>>,
}

/// The state as it was written before the wide levels (the replays and the sidecars of the version 1).
#[derive(Deserialize)]
struct LegacyState {
    flags: Option<Vec<u8>>,
    high: Option<Vec<u8>>,
}

impl From<LegacyState> for ModState {
    fn from(old: LegacyState) -> Self {
        Self { flags: old.flags, high: old.high, levels: None }
    }
}

impl ModState {
    /// Whether no marker was there: the save keeps nothing.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.flags.is_none() && self.high.is_none() && self.levels.is_none()
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
        if self.levels.as_ref().is_some_and(|levels| levels.len() != LEVEL_BYTES) {
            return Err(format!("the wide levels are {LEVEL_BYTES} bytes"));
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
        let state = match bincode::serde::decode_from_slice::<Self, _>(bytes, config()) {
            Ok((state, used)) if used == bytes.len() => state,
            // A replay made before the wide levels: two parts.
            _ => {
                let (old, _): (LegacyState, usize) =
                    bincode::serde::decode_from_slice(bytes, config())
                        .map_err(|error| format!("not a mod state: {error}"))?;
                old.into()
            }
        };
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

/// The entries of a version 1 file.
#[derive(Deserialize)]
struct LegacySaved {
    slot: u8,
    checksum: u16,
    state: LegacyState,
}

/// A version 1 file.
#[derive(Deserialize)]
struct LegacySidecar {
    #[allow(dead_code)]
    version: u32,
    saves: Vec<LegacySaved>,
}

impl From<LegacySidecar> for ModsSidecar {
    fn from(old: LegacySidecar) -> Self {
        let saves = old
            .saves
            .into_iter()
            .map(|saved| SavedMods {
                slot: saved.slot,
                checksum: saved.checksum,
                state: saved.state.into(),
            })
            .collect();
        Self { version: SIDECAR_VERSION, saves }
    }
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
        let sidecar = match version {
            SIDECAR_VERSION => {
                let (sidecar, _): (Self, usize) =
                    bincode::serde::decode_from_slice(bytes, config())
                        .map_err(|error| format!("not a mod state file: {error}"))?;
                sidecar
            }
            LEGACY_VERSION => {
                let (old, _): (LegacySidecar, usize) =
                    bincode::serde::decode_from_slice(bytes, config())
                        .map_err(|error| format!("not a mod state file: {error}"))?;
                old.into()
            }
            _ => {
                return Err(format!(
                    "mod state file version {version}, expected {SIDECAR_VERSION}"
                ));
            }
        };
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
            levels: Some((0..LEVEL_BYTES).map(|k| (k * 5) as u8).collect()),
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
        sidecar.store(
            2,
            10,
            ModState { flags: None, high: Some(vec![1; HIGH_BYTES]), levels: None },
        );
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
            state: ModState { flags: Some(vec![0; 3]), high: None, levels: None },
        });
        let back = ModsSidecar::from_bytes(&sidecar.to_bytes()).expect("a mod state file");
        assert!(back.saves.is_empty(), "a short flag bank is not kept");
        let state = full();
        assert_eq!(ModState::from_bytes(&state.to_bytes()), Ok(state));
        assert!(ModState::from_bytes(&[0xAA; 4]).is_err());
    }

    /// A state and a sidecar as the version 1 wrote them (before the wide levels).
    #[derive(serde::Serialize)]
    struct OldState {
        flags: Option<Vec<u8>>,
        high: Option<Vec<u8>>,
    }
    #[derive(serde::Serialize)]
    struct OldSaved {
        slot: u8,
        checksum: u16,
        state: OldState,
    }
    #[derive(serde::Serialize)]
    struct OldSidecar {
        version: u32,
        saves: Vec<OldSaved>,
    }

    #[test]
    fn a_state_and_a_sidecar_written_before_the_wide_levels_are_still_read() {
        let old = OldState { flags: Some(vec![9; FLAG_BYTES]), high: None };
        let bytes = bincode::serde::encode_to_vec(&old, config()).unwrap();
        let state = ModState::from_bytes(&bytes).expect("a version 1 state");
        assert_eq!(state.flags, old.flags);
        assert_eq!(state.levels, None);
        let file = OldSidecar {
            version: 1,
            saves: vec![OldSaved {
                slot: 2,
                checksum: 0x77,
                state: OldState { flags: None, high: Some(vec![4; HIGH_BYTES]) },
            }],
        };
        let bytes = bincode::serde::encode_to_vec(&file, config()).unwrap();
        let back = ModsSidecar::from_bytes(&bytes).expect("a version 1 file");
        assert_eq!(back.version, SIDECAR_VERSION);
        assert_eq!(back.entry_for(2, 0x77).and_then(|s| s.high.clone()), Some(vec![4; HIGH_BYTES]));
    }
}
