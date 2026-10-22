//! The state kept when the app goes to the background on Android. The battery save is flushed as on desktop;
//! on top of it the machine state is written to one dedicated file, so that if the system then kills the
//! process (low memory), the next launch continues exactly where the game was. The file is removed when the
//! app comes back to the foreground, and when it has been loaded: it exists only between a background and the
//! next start, so it can never replace a newer battery save.
#![cfg(target_os = "android")]

use crate::paths;
use ssd_oracle::Machine;
use std::path::PathBuf;

/// The state file kept in the background (the HD menus' part of it is beside it, `background.hd`).
pub fn path() -> PathBuf {
    paths::states_dir().join("background.ssdstate")
}

/// Writes the state of `machine` (through a temporary file, so a kill never leaves a truncated one).
pub fn save(machine: &Machine) -> anyhow::Result<()> {
    let path = path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    let written = std::fs::write(&tmp, machine.save_state()).and_then(|()| std::fs::rename(&tmp, &path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    Ok(written?)
}

/// Removes the file, and the HD menus' part beside it.
pub fn discard() {
    let _ = std::fs::remove_file(path());
    let _ = std::fs::remove_file(path().with_extension("hd"));
}

/// Loads the file into `machine` if there is one (a launch after the system killed the app in the background),
/// and removes it; whether a state was loaded. A state that does not load leaves the machine as it was.
pub fn take(machine: &mut Machine) -> bool {
    let path = path();
    let Ok(bytes) = std::fs::read(&path) else { return false };
    let loaded = machine.load_state(&bytes);
    discard();
    match loaded {
        Ok(()) => {
            log::info!("continuing from the state kept when the app went to the background");
            true
        }
        Err(e) => {
            log::warn!("the state kept in the background does not load: {e}");
            false
        }
    }
}
