//! The battery save of a machine (`ssd.sav` in the saves folder). A run that persists reads and writes the file as
//! before. A run that does not (see [`paths::persists`]) reads the file, if there is one, so that a snapshot shows the
//! player's saves, and keeps every write in memory: the file is never written, and the saves folder is never created.

use crate::paths;
use ssd_oracle::{FsSaveStore, MemorySaveStore, SaveStore};
use std::path::PathBuf;

/// The battery save store of a machine at power-on, for the run's rule (see the module).
///
/// # Errors
///
/// The saves folder cannot be created, in a run that persists.
pub fn battery_saves() -> anyhow::Result<Box<dyn SaveStore>> {
    // A chapter session keeps its battery save in the chapter's folder (see `chapter`).
    let dir = crate::chapter::battery_dir();
    if paths::persists() {
        return Ok(Box::new(FsSaveStore::new(dir, "ssd")?));
    }
    // A run that does not persist starts a chapter from its fresh slot in memory, and writes nothing.
    let mut written = MemorySaveStore::default();
    if let Some(sram) = crate::chapter::fresh("ssd.sav") {
        written.persist("sav", sram)?;
    }
    Ok(Box::new(RunSaves { dir, stem: "ssd", written }))
}

/// The saves of a run that does not persist: a file of the saves folder is read as [`FsSaveStore`] reads it, and a
/// write goes to `written`, which the machine's later reads see first. The store lives as long as its machine.
struct RunSaves {
    dir: PathBuf,
    stem: &'static str,
    written: MemorySaveStore,
}

impl SaveStore for RunSaves {
    fn load(&mut self, ext: &str) -> std::io::Result<Option<Vec<u8>>> {
        if let Some(bytes) = self.written.load(ext)? {
            return Ok(Some(bytes));
        }
        match std::fs::read(self.dir.join(format!("{}.{ext}", self.stem))) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    fn persist(&mut self, ext: &str, bytes: &[u8]) -> std::io::Result<()> {
        self.written.persist(ext, bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::sram_counter_rom;
    use ssd_oracle::{Joypad, Machine, NullAudio};

    #[test]
    fn a_run_that_does_not_persist_reads_the_save_file_and_writes_none() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let file = paths::saves_dir().join("ssd.sav");
        paths::set_persistent(false);
        // No file yet: nothing is read, and a write stays in memory, with no folder made for it.
        let mut store = battery_saves().unwrap();
        assert_eq!(store.load("sav").unwrap(), None);
        store.persist("sav", &[1; 8]).unwrap();
        assert_eq!(store.load("sav").unwrap(), Some(vec![1; 8]), "the run sees its own write");
        assert!(!paths::saves_dir().exists(), "the saves folder is not created");
        // The player's file is read by a later store, and never written.
        std::fs::create_dir_all(paths::saves_dir()).unwrap();
        std::fs::write(&file, [7_u8; 8192]).unwrap();
        let mut store = battery_saves().unwrap();
        assert_eq!(store.load("sav").unwrap(), Some(vec![7; 8192]), "the file is read");
        store.persist("sav", &[9; 8192]).unwrap();
        assert_eq!(store.load("sav").unwrap(), Some(vec![9; 8192]), "the run sees its write");
        assert_eq!(std::fs::read(&file).unwrap(), vec![7; 8192], "the file is byte-identical");
        assert_eq!(std::fs::read_dir(paths::saves_dir()).unwrap().count(), 1, "no other file");
        // A windowed run writes the file.
        paths::set_persistent(true);
        let mut store = battery_saves().unwrap();
        store.persist("sav", &[9; 8192]).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), vec![9; 8192], "a windowed run writes it");
        paths::set_persistent(false);
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn a_machine_that_saves_writes_no_save_file_in_a_run_that_does_not_persist() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", dir.path()) };
        let file = paths::saves_dir().join("ssd.sav");
        paths::set_persistent(false);
        let mut machine = Machine::new(sram_counter_rom(), battery_saves().unwrap()).unwrap();
        for _ in 0..60 {
            machine.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        }
        machine.flush_saves().unwrap();
        assert_ne!(machine.sram().unwrap()[0], 0, "the game changed its SRAM, in memory");
        assert!(!paths::saves_dir().exists(), "no save file, and no saves folder");
        // The same game in a windowed run writes its SRAM.
        paths::set_persistent(true);
        let mut machine = Machine::new(sram_counter_rom(), battery_saves().unwrap()).unwrap();
        for _ in 0..60 {
            machine.run_frame(Joypad::default(), &mut NullAudio).unwrap();
        }
        machine.flush_saves().unwrap();
        assert!(file.exists(), "a windowed run writes the save file");
        paths::set_persistent(false);
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }
}
