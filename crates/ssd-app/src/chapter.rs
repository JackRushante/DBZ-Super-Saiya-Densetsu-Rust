//! The chapter session of a run (`--chapter <id>`, M10 Task 8). A chapter starts the game from a save slot its pack
//! synthesizes (`ssd_mod::chapter::synthesize`): the slot is the battery save of the chapter's own folder,
//! `saves/chapters/<id>/`, beside the sidecars of that save. The main saves are never touched. A folder that has a
//! battery save already continues from it. The fresh files are written only by a run that persists (see
//! [`paths::persists`]); any other run keeps them in memory.

use crate::{hd, paths};
use ssd_catalogue::Game;
use ssd_menus::mods_state::{ModState, ModsSidecar};
use ssd_save::{CHECKSUM_OFFSET, SLOT_BASES};
use std::path::PathBuf;
use std::sync::OnceLock;

/// The folder, under the saves folder, that holds the chapters' folders.
const FOLDER: &str = "chapters";

/// A chapter session: its folder, and the files a fresh start writes (`None` when the folder has its own).
pub struct Session {
    pub dir: PathBuf,
    /// The battery save of a fresh start (`ssd.sav`): the chapter's slot as slot 1.
    pub sram: Option<Vec<u8>>,
    /// The sidecar of that slot (`ssd.mods`), when the chapter sets extended flags.
    pub mods: Option<Vec<u8>>,
}

/// The session of this run, set once by `main` before the machine starts.
static SESSION: OnceLock<Session> = OnceLock::new();

/// Sets the session of this run (the first call wins).
pub fn configure(session: Session) {
    let _ = SESSION.set(session);
}

/// The folder of the battery save and of the sidecars: the chapter's folder in a chapter session, else the saves folder.
pub fn battery_dir() -> PathBuf {
    SESSION.get().map_or_else(paths::saves_dir, |session| session.dir.clone())
}

/// The fresh file `name` of the session (`ssd.sav` or `ssd.mods`), when this run starts the chapter from it.
pub fn fresh(name: &str) -> Option<&'static [u8]> {
    let session = SESSION.get()?;
    match name {
        "ssd.sav" => session.sram.as_deref(),
        "ssd.mods" => session.mods.as_deref(),
        _ => None,
    }
}

/// Writes the fresh files of the session to its folder. Only a run that persists writes them (see [`hd::write_beside`]).
///
/// # Errors
///
/// The folder or a file cannot be written.
pub fn write_fresh() -> anyhow::Result<()> {
    let Some(session) = SESSION.get() else { return Ok(()) };
    for (name, bytes) in [("ssd.sav", &session.sram), ("ssd.mods", &session.mods)] {
        if let Some(bytes) = bytes {
            hd::write_beside(&session.dir.join(name), bytes)?;
        }
    }
    Ok(())
}

/// The session of chapter `id` in `packs`, on `rom`: its folder under the saves folder, and the files of a fresh start (none
/// when the folder has a battery save already).
///
/// # Errors
///
/// A chapter of that id in no pack, a table the game does not give, or the packs' files cannot be read.
pub fn start(id: &str, packs: &[ssd_mod::Pack], rom: &[u8]) -> Result<Session, String> {
    let chapter = find(id, packs)?;
    let dir = paths::saves_dir().join(FOLDER).join(id);
    if dir.join("ssd.sav").exists() {
        log::info!("chapter {id}: continues its own saves in {}", dir.display());
        return Ok(Session { dir, sram: None, mods: None });
    }
    let game = Game::from_rom(rom).map_err(|error| format!("{error:#}"))?;
    let synthesized =
        ssd_mod::chapter::synthesize(&game, &chapter).map_err(|error| error.to_string())?;
    let sram = synthesized.save.as_bytes().to_vec();
    let mods = synthesized.flagx.map(|flagx| {
        let at = SLOT_BASES[0] + CHECKSUM_OFFSET;
        let checksum = u16::from_le_bytes([sram[at], sram[at + 1]]);
        let mut sidecar = ModsSidecar::empty();
        sidecar.store(0, checksum, ModState { flags: Some(flagx), ..ModState::default() });
        sidecar.to_bytes()
    });
    log::info!("chapter {id}: a fresh start from its slot, in {}", dir.display());
    Ok(Session { dir, sram: Some(sram), mods })
}

/// The chapter `id` of the packs.
fn find(id: &str, packs: &[ssd_mod::Pack]) -> Result<ssd_mod::chapter::Chapter, String> {
    for pack in packs {
        for entry in &pack.manifest.chapters {
            let chapters = pack.chapters(entry).map_err(|error| error.to_string())?;
            if let Some(chapter) = chapters.unwrap_or_default().into_iter().find(|c| c.id == id) {
                return Ok(chapter);
            }
        }
    }
    Err(format!("no chapter `{id}` in the data packs of this run"))
}

/// Writes the `completed` file of the saves folder when the game is completed: one line, the UTC date of the first completion
/// (`YYYY-MM-DD`). A file that exists keeps its date. Only a run that persists writes it (see [`hd::write_beside`]).
///
/// # Errors
///
/// The file cannot be written.
pub fn write_completed() -> anyhow::Result<()> {
    let path = paths::saves_dir().join("completed");
    if path.exists() {
        return Ok(());
    }
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    hd::write_beside(&path, format!("{}\n", date_of(seconds)).as_bytes())
}

/// The day the `completed` file names: the UTC date of `seconds` since the Unix epoch, as `YYYY-MM-DD`.
#[must_use]
pub fn date_of(seconds: u64) -> String {
    // Howard Hinnant's civil-from-days, for the days since 1970-01-01.
    let z = (seconds / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 { month_index + 3 } else { month_index - 9 };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::{date_of, start, write_completed};
    use crate::paths;

    /// A pack folder with one chapter `id` (the chapters are all `start` reads: the manifest's game is not checked here).
    fn pack_with_chapter(dir: &std::path::Path, id: &str) -> ssd_mod::Pack {
        let manifest = "[pack]\nid = \"chapter.test\"\nname = \"Chapter test\"\nversion = \"1.0.0\"\ngame = \"A8CF4F2A\"\n\
                        [[chapters]]\nfile = \"chapters.toml\"\n";
        let chapter = format!(
            "[[chapter]]\nid = \"{id}\"\nname = \"Test\"\nmap = 64\nleader = [1, 2]\nfollower = [1, 2]\n\
             party = [{{ character = 0, level = 1 }}]\n"
        );
        std::fs::write(dir.join("pack.toml"), manifest).unwrap();
        std::fs::write(dir.join("chapters.toml"), chapter).unwrap();
        ssd_mod::Pack::open(dir).unwrap()
    }

    #[test]
    fn a_folder_with_a_save_continues_it_and_an_unknown_id_is_named() {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let data = tempfile::tempdir().unwrap();
        let pack_dir = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", data.path()) };
        let packs = [pack_with_chapter(pack_dir.path(), "androids")];
        let folder = paths::saves_dir().join("chapters").join("androids");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join("ssd.sav"), [0xFF_u8; 8192]).unwrap();
        // The folder has its battery save: the chapter continues it, and no fresh file is made (no ROM is read).
        let session = start("androids", &packs, &[]).unwrap();
        assert_eq!(session.dir, folder);
        assert!(session.sram.is_none() && session.mods.is_none());
        // An id no pack has is named, before any file is read.
        let unknown = start("nope", &packs, &[]).err().unwrap();
        assert_eq!(unknown, "no chapter `nope` in the data packs of this run");
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }

    #[test]
    fn the_completed_file_holds_the_date_of_the_first_completion_and_only_a_persisting_run_writes_it()
     {
        let _guard = paths::ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let data = tempfile::tempdir().unwrap();
        // SAFETY: serialised by ENV_LOCK.
        unsafe { std::env::set_var("SSD_DATA_DIR", data.path()) };
        let file = paths::saves_dir().join("completed");
        paths::set_persistent(false);
        write_completed().unwrap();
        assert!(!file.exists(), "a run that does not persist writes no file");
        paths::set_persistent(true);
        write_completed().unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.len() == 11 && text.ends_with('\n'), "one line, the date: {text:?}");
        std::fs::write(&file, "2020-01-02\n").unwrap();
        write_completed().unwrap();
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "2020-01-02\n",
            "the first date is kept"
        );
        paths::set_persistent(false);
        unsafe { std::env::remove_var("SSD_DATA_DIR") };
    }
    #[test]
    fn the_date_of_a_second_is_its_utc_day() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(951_782_400), "2000-02-29", "a leap day");
        assert_eq!(date_of(1_792_540_800), "2026-10-21", "today's date");
        assert_eq!(date_of(1_792_540_800 + 86_399), "2026-10-21", "the last second of the day");
        assert_eq!(date_of(1_792_540_800 + 86_400), "2026-10-22");
    }
}
