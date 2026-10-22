//! `--record` and `--replay`: a session recorded as a replay from power-on, and its playback.
//!
//! A recording holds every emulated frame's joypad and the SRAM at power-on (see
//! [`ssd_oracle::Replay`]); it needs `--hooks original`, since a hooked call can run on past a
//! frame's render with that frame's joypad, which a replay played frame by frame would not
//! reproduce. It is written when the run ends, on errors and panics too (see [`Session`]). A
//! playback starts the machine from the replay's power-on with its initial SRAM in memory, so the
//! real save file is neither read nor written, and feeds every frame its recorded joypad, looked
//! up by frame (with hooks too); after the last frame all buttons are released. Playing while
//! recording records what is played, to another file. Loading a state is refused meanwhile: it
//! would leave the replay's timeline.
//!
//! WRAM freezes ([`ssd_oracle::Machine::set_freezes`]) are not part of a replay: a machine with
//! them cannot be recorded (the recorder refuses it), and `--replay` with them is allowed but plays
//! another session than the recorded one, not reproducible across hook modes.

use anyhow::Context;
use ssd_oracle::{
    AudioSink, CORE_VERSION, DEFAULT_WRAM_SEED, HostEvent, Joypad, MODE_CLASSIC, Machine,
    OracleError, PackTag, Recorder, Replay, SaveStore, Stopped, Stops, apply_events,
};
use std::path::{Path, PathBuf};

/// A replay the run cannot play or record as asked: unreadable, not a replay, for another ROM or
/// emulator version, with an initial SRAM the cartridge does not take, or to be overwritten by
/// the recording. The app exits with its own code.
#[derive(Debug)]
pub struct ReplayRejected(anyhow::Error);

impl std::fmt::Display for ReplayRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "replay rejected: {:#}", self.0)
    }
}

impl std::error::Error for ReplayRejected {}

/// Whether `error` is, or was caused by, a [`ReplayRejected`].
pub fn is_rejection(error: &anyhow::Error) -> bool {
    error.chain().any(<dyn std::error::Error>::is::<ReplayRejected>)
}

/// The replay a run plays and the recording it makes, if any.
///
/// The recording is written by [`Session::finish`], or, if the session is dropped without it (a
/// panic unwinding through the run), by `Drop`, best effort and logged. A process stopped
/// without unwinding (Ctrl+C in the console, a killed process, a panic that aborts) loses it.
#[derive(Default)]
pub struct Session {
    playback: Option<Replay>,
    /// Where the recording goes; the recorder starts with the machine.
    record_to: Option<PathBuf>,
    recorder: Option<Recorder>,
    /// [`Session::finish`] has run: nothing is left for `Drop` to write.
    finished: bool,
    /// The mode a recording is made in (`ssd_oracle::MODE_ENHANCED` for the HD menus).
    mode: u8,
    /// The data packs the run is on (`--mods`): the image a machine is built on, and whose tag a
    /// recording carries. `None` for a run on the ROM alone.
    packs: Option<ssd_mod::Image>,
}

impl Session {
    /// The session of `--replay` and `--record` for `rom`: the replay is read and checked now.
    ///
    /// # Errors
    ///
    /// [`ReplayRejected`] if the replay file cannot be read, is not a valid replay of `rom` on
    /// this build's emulator, or is the file the recording would be written to.
    pub fn new(replay: Option<&Path>, record: Option<&Path>, rom: &[u8]) -> anyhow::Result<Self> {
        let rom_crc32 = ssd_rom::crc32(rom);
        let playback = replay
            .map(|path| {
                if record.is_some_and(|record| same_file(path, record)) {
                    anyhow::bail!(
                        "--record names the replay file {} itself: recording would overwrite it",
                        path.display()
                    );
                }
                let bytes =
                    std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
                Replay::from_bytes(&bytes, rom_crc32, CORE_VERSION)
                    .with_context(|| format!("{}", path.display()))
            })
            .transpose()
            .map_err(|e| anyhow::Error::new(ReplayRejected(e)))?;
        Ok(Self {
            playback,
            record_to: record.map(Path::to_path_buf),
            recorder: None,
            finished: false,
            mode: MODE_CLASSIC,
            packs: None,
        })
    }

    /// Sets the mode a recording is made in: the replay then carries it (format 2 unless classic).
    pub fn set_mode(&mut self, mode: u8) {
        self.mode = mode;
    }

    /// Sets the data packs the run is on (`--mods`): the machines are built on `packs`' image, and a
    /// recording carries its tag. `None` for the ROM alone.
    pub fn set_packs(&mut self, packs: Option<ssd_mod::Image>) {
        self.packs = packs;
    }

    /// The mode of the replay played, if one is played (classic otherwise).
    #[must_use]
    pub fn playback_mode(&self) -> u8 {
        self.playback.as_ref().map_or(MODE_CLASSIC, |replay| replay.mode)
    }

    /// The joypad and the host events a replay gives `frame`, when one is played.
    #[must_use]
    pub fn playback_at(&self, frame: u64) -> Option<(Joypad, Vec<HostEvent>)> {
        self.playback.as_ref().map(|replay| (replay.input(frame), replay.events_at(frame).to_vec()))
    }

    /// Whether the run records or plays a replay.
    pub fn is_active(&self) -> bool {
        self.playback.is_some() || self.record_to.is_some()
    }

    /// Whether the run records a replay (which needs the original code, no hooks).
    pub fn is_recording(&self) -> bool {
        self.record_to.is_some()
    }

    /// Whether a replay is played (a recording alone is not).
    pub fn is_playing(&self) -> bool {
        self.playback.is_some()
    }

    /// Stops the playback: from the next frame the run takes its buttons live (a recording goes on
    /// with them too).
    pub fn stop_playback(&mut self) {
        self.playback = None;
    }

    /// The machine at power-on: the replay's (its WRAM fill, its initial SRAM in memory) when one
    /// is played, else `rom` with the store `saves` gives. A recording starts on it.
    ///
    /// # Errors
    ///
    /// [`ReplayRejected`] if the replay's initial SRAM does not fit the cartridge; the save store
    /// or the machine cannot be created.
    pub fn power_on(
        &mut self,
        rom: Vec<u8>,
        saves: impl FnOnce() -> anyhow::Result<Box<dyn SaveStore>>,
    ) -> anyhow::Result<Machine> {
        let rejected = |e: OracleError| match e {
            OracleError::Replay(_) => anyhow::Error::new(ReplayRejected(e.into())),
            other => other.into(),
        };
        let machine = match (&self.playback, &self.packs) {
            (Some(replay), Some(packs)) if replay.packs.is_some() => {
                replay.power_on_image(packs.bytes.clone()).map_err(rejected)?
            }
            (Some(replay), None) if replay.packs.is_some() => {
                return Err(anyhow::Error::new(ReplayRejected(anyhow::anyhow!(
                    "the replay was recorded with data packs: play it with the same --mods"
                ))));
            }
            (Some(replay), Some(_)) if replay.packs.is_none() => {
                return Err(anyhow::Error::new(ReplayRejected(anyhow::anyhow!(
                    "the replay was recorded without data packs: play it without --mods"
                ))));
            }
            (Some(replay), _) => replay.power_on(rom).map_err(rejected)?,
            (None, Some(packs)) => Machine::with_image(
                packs.bytes.clone(),
                ssd_rom::crc32(&rom),
                saves()?,
                DEFAULT_WRAM_SEED,
            )?,
            (None, None) => Machine::new(rom, saves()?)?,
        };
        if let Some(replay) = &self.playback {
            log::info!("playing a replay of {} frames", replay.frames());
        }
        if let Some(path) = &self.record_to {
            let recorder = Recorder::start(&machine)?.in_mode(self.mode).in_packs(self.pack_tag());
            self.recorder = Some(recorder);
            log::info!("recording to {}", path.display());
        }
        Ok(machine)
    }

    /// The tag of the data packs the run is on, for a recording ([`Replay::packs`]).
    fn pack_tag(&self) -> Option<PackTag> {
        self.packs.as_ref().map(|packs| PackTag { set_id: packs.set_id, image_crc32: packs.crc32 })
    }

    /// Whether a replay is played and has no input for the machine's next frame.
    pub fn playback_over(&self, machine: &Machine) -> bool {
        self.playback.as_ref().is_some_and(|replay| machine.frame_count() >= replay.frames())
    }

    /// Runs one frame: the replay's joypad when one is played (looked up by frame), else `live`;
    /// recorded when recording.
    ///
    /// # Errors
    ///
    /// The errors of the machine's run, and of the recorder (a machine with hooks, or not at
    /// the recording's frame).
    pub fn run_frame(
        &mut self,
        machine: &mut Machine,
        live: Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<(), OracleError> {
        match (&mut self.recorder, &self.playback) {
            // Recording runs without hooks: the frame's joypad held for the frame is exact.
            (Some(recorder), playback) => {
                let input = playback.as_ref().map_or(live, |r| r.input(machine.frame_count()));
                recorder.run_frame(machine, input, audio)
            }
            (None, Some(replay)) => machine.run_frame_with(&mut |frame| replay.input(frame), audio),
            (None, None) => machine.run_frame(live, audio),
        }
    }

    /// The joypad of `frame` as a run feeds it: the replay's when one is played, else `live`.
    #[must_use]
    pub fn input_at(&self, frame: u64, live: Joypad) -> Joypad {
        self.playback.as_ref().map_or(live, |replay| replay.input(frame))
    }

    /// Runs one frame, as `stops` says ([`Machine::run`]): the stop that ended the run is returned,
    /// so a breakpoint can stop it before the frame ends. `stops` must have `frame_end` set. The
    /// host `events` of the frame are made on the machine first ([`apply_events`]), and a recording
    /// keeps them with the frame. A recording runs without hooks, so it ends its frame and never
    /// stops early.
    ///
    /// # Errors
    ///
    /// The errors of the machine's run and of the recorder, and of the machine's writes.
    pub fn run(
        &mut self,
        machine: &mut Machine,
        live: Joypad,
        events: &[HostEvent],
        audio: &mut dyn AudioSink,
        stops: &Stops,
    ) -> Result<Stopped, OracleError> {
        apply_events(machine, events)?;
        match (&mut self.recorder, &self.playback) {
            (Some(recorder), playback) => {
                let input = playback.as_ref().map_or(live, |r| r.input(machine.frame_count()));
                recorder.run_frame_with_events(machine, input, events, audio)?;
                Ok(Stopped::FrameEnd)
            }
            (None, Some(replay)) => {
                machine.run(stops, &mut |frame| replay.input(frame), audio, u64::MAX)
            }
            (None, None) => machine.run(stops, &mut |_| live, audio, u64::MAX),
        }
    }

    /// Writes the recording, if any ([`Replay::write_to`]); returns where.
    ///
    /// # Errors
    ///
    /// The file cannot be written.
    pub fn save_recording(&self) -> anyhow::Result<Option<&Path>> {
        let (Some(path), Some(recorder)) = (&self.record_to, &self.recorder) else {
            return Ok(None);
        };
        recorder
            .replay()
            .write_to(path, CORE_VERSION)
            .with_context(|| format!("writing {}", path.display()))?;
        log::info!("{} frames recorded to {}", recorder.replay().frames(), path.display());
        Ok(Some(path))
    }

    /// Ends a run: the recording is written whether the run succeeded or not. The run's own
    /// error wins over a write error.
    pub fn finish<T>(mut self, result: anyhow::Result<T>) -> anyhow::Result<T> {
        self.finished = true;
        match (result, self.save_recording()) {
            (Ok(value), Ok(_)) => Ok(value),
            (Ok(_), Err(save)) => Err(save).context("saving the recording"),
            (Err(error), Ok(_)) => Err(error),
            (Err(error), Err(save)) => {
                log::warn!("saving the recording after the error failed too: {save:#}");
                Err(error)
            }
        }
    }
}

impl Drop for Session {
    /// A session dropped without [`Session::finish`] (a panic unwinding through the run) still
    /// writes its recording, best effort.
    fn drop(&mut self) {
        if self.finished || self.recorder.is_none() {
            return;
        }
        match self.save_recording() {
            Ok(Some(path)) => {
                log::warn!("the run did not end normally; recording saved to {}", path.display());
            }
            Ok(None) => {}
            Err(e) => {
                log::warn!("the run did not end normally and saving the recording failed: {e:#}");
            }
        }
    }
}

/// Whether `a` and `b` name the same file: the same canonical path, the canonical directory plus
/// the file name for a path that does not exist yet.
fn same_file(a: &Path, b: &Path) -> bool {
    let canonical = |path: &Path| {
        path.canonicalize().ok().or_else(|| {
            let dir = path.parent().filter(|dir| !dir.as_os_str().is_empty());
            let dir = dir.unwrap_or(Path::new(".")).canonicalize().ok()?;
            Some(dir.join(path.file_name()?))
        })
    };
    matches!((canonical(a), canonical(b)), (Some(a), Some(b)) if a == b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{SharedStore, sram_counter_rom};
    use ssd_oracle::{MemorySaveStore, NullAudio, joypad_from_bits};

    fn shared(store: &SharedStore) -> impl FnOnce() -> anyhow::Result<Box<dyn SaveStore>> {
        let store = store.clone();
        move || Ok(Box::new(store))
    }

    fn input(frame: u64) -> Joypad {
        joypad_from_bits((frame as u16).wrapping_mul(0x2F1) & 0x0FFF)
    }

    /// Records `frames` frames of [`input`] on [`sram_counter_rom`] with the save `store`.
    fn record(path: &Path, frames: u64, store: &SharedStore) -> Machine {
        let rom = sram_counter_rom();
        let mut session = Session::new(None, Some(path), &rom).unwrap();
        let mut machine = session.power_on(rom, shared(store)).unwrap();
        while machine.frame_count() < frames {
            let live = input(machine.frame_count());
            session.run_frame(&mut machine, live, &mut NullAudio).unwrap();
        }
        session.finish(Ok(())).unwrap();
        machine
    }

    fn read(path: &Path) -> Replay {
        let crc = ssd_rom::crc32(&sram_counter_rom());
        Replay::from_bytes(&std::fs::read(path).unwrap(), crc, CORE_VERSION).unwrap()
    }

    /// The replay file at `path`, for `rom` (the tests of the pack sets use the 1 MiB ROM).
    fn read_of(path: &Path, rom: &[u8]) -> Replay {
        Replay::from_bytes(&std::fs::read(path).unwrap(), ssd_rom::crc32(rom), CORE_VERSION)
            .unwrap()
    }

    #[test]
    fn a_recording_holds_every_frame_and_the_initial_sram() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join("run.ssdreplay");
        let store = SharedStore::default();
        let mut sav = vec![0_u8; 2048];
        sav[0] = 40;
        store.clone().persist("sav", &sav).unwrap();
        record(&path, 30, &store);
        let replay = read(&path);
        assert_eq!(replay.inputs, (0..30).map(input).collect::<Vec<_>>());
        assert_eq!(replay.initial_sram.as_deref(), Some(&sav[..]), "the SRAM at power-on");
        assert!(!path.with_extension("ssdreplay.tmp").exists());
    }

    #[test]
    fn a_playback_keeps_the_save_file_and_ends_with_the_replay() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.ssdreplay");
        let recorded = record(&path, 30, &SharedStore::default());

        let rom = sram_counter_rom();
        let real = SharedStore::default();
        real.clone().persist("sav", &[0x55; 2048]).unwrap();
        let mut session = Session::new(Some(&path), None, &rom).unwrap();
        assert!(session.is_active());
        let mut machine = session.power_on(rom, shared(&real)).unwrap();
        assert_eq!(machine.sram().unwrap()[0], 0xFF, "the replay's SRAM, not the save file's");
        while !session.playback_over(&machine) {
            session
                .run_frame(
                    &mut machine,
                    Joypad { start: true, ..Joypad::default() },
                    &mut NullAudio,
                )
                .unwrap();
        }
        assert_eq!(machine.frame_count(), 30);
        assert!(machine.save_state() == recorded.save_state(), "the live joypad is ignored");
        machine.flush_saves().unwrap();
        assert_eq!(real.get("sav"), Some(vec![0x55; 2048]), "the save file is untouched");
        // After the end, all buttons are released.
        session
            .run_frame(&mut machine, Joypad { a: true, ..Joypad::default() }, &mut NullAudio)
            .unwrap();
        assert_eq!(machine.frame_count(), 31);
    }

    #[test]
    fn a_stopped_playback_hands_the_buttons_back_to_the_live_joypad() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.ssdreplay");
        record(&path, 30, &SharedStore::default());
        let rom = sram_counter_rom();
        let mut session = Session::new(Some(&path), None, &rom).unwrap();
        let live = Joypad { a: true, ..Joypad::default() };
        assert!(session.is_playing());
        assert_eq!(session.input_at(3, live), input(3), "the replay's joypad while it plays");
        session.stop_playback();
        assert!(!session.is_playing() && !session.is_active());
        assert_eq!(session.input_at(3, live), live, "the live joypad after the stop");
    }

    #[test]
    fn replaying_while_recording_records_the_same_replay() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.ssdreplay");
        let second = dir.path().join("second.ssdreplay");
        record(&first, 30, &SharedStore::default());

        let rom = sram_counter_rom();
        let mut session = Session::new(Some(&first), Some(&second), &rom).unwrap();
        let mut machine =
            session.power_on(rom, || Ok(Box::new(MemorySaveStore::default()))).unwrap();
        while !session.playback_over(&machine) {
            session.run_frame(&mut machine, Joypad::default(), &mut NullAudio).unwrap();
        }
        session.finish(Ok(())).unwrap();
        assert_eq!(std::fs::read(&first).unwrap(), std::fs::read(&second).unwrap());
    }

    #[test]
    fn a_failed_run_still_writes_its_recording_and_its_own_error_wins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.ssdreplay");
        let rom = sram_counter_rom();
        let mut session = Session::new(None, Some(&path), &rom).unwrap();
        let mut machine = session.power_on(rom, shared(&SharedStore::default())).unwrap();
        for _ in 0..5 {
            session.run_frame(&mut machine, Joypad::default(), &mut NullAudio).unwrap();
        }
        let error = session.finish(Err::<(), _>(anyhow::anyhow!("window lost"))).unwrap_err();
        assert_eq!(error.to_string(), "window lost");
        assert_eq!(read(&path).frames(), 5);
    }

    #[test]
    fn a_replay_of_another_rom_is_rejected_naming_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.ssdreplay");
        record(&path, 3, &SharedStore::default());
        let error = Session::new(Some(&path), None, &vec![0x42; 0x8000]).err().unwrap();
        assert!(is_rejection(&error));
        let message = format!("{error:#}");
        assert!(message.starts_with("replay rejected: "), "{message}");
        assert!(message.contains("run.ssdreplay") && message.contains("ROM"), "{message}");

        let missing = dir.path().join("missing.ssdreplay");
        let error = Session::new(Some(&missing), None, &sram_counter_rom()).err().unwrap();
        assert!(is_rejection(&error), "{error:#}");
        assert!(!is_rejection(&anyhow::anyhow!("window lost")));
    }

    #[test]
    fn an_initial_sram_the_cartridge_does_not_take_is_a_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.ssdreplay");
        record(&path, 3, &SharedStore::default());
        let rom = sram_counter_rom();
        let wrong = Replay { initial_sram: Some(vec![0; 1024]), ..read(&path) };
        wrong.write_to(&path, CORE_VERSION).unwrap();
        let mut session = Session::new(Some(&path), None, &rom).unwrap();
        let error = session.power_on(rom, shared(&SharedStore::default())).err().unwrap();
        assert!(is_rejection(&error), "{error:#}");
        assert!(format!("{error:#}").contains("SRAM"), "{error:#}");
    }

    #[test]
    fn recording_over_the_replay_played_is_a_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.ssdreplay");
        record(&path, 3, &SharedStore::default());
        let before = std::fs::read(&path).unwrap();
        let rom = sram_counter_rom();
        let same = dir.path().join(".").join("run.ssdreplay");
        let error = Session::new(Some(&path), Some(&same), &rom).err().unwrap();
        assert!(is_rejection(&error), "{error:#}");
        assert!(format!("{error:#}").contains("overwrite"), "{error:#}");
        assert_eq!(std::fs::read(&path).unwrap(), before);
        // Another file next to it, not created yet, is fine.
        Session::new(Some(&path), Some(&dir.path().join("other.ssdreplay")), &rom).unwrap();
    }

    #[test]
    fn a_run_stops_at_a_breakpoint_and_goes_on_once_it_is_cleared() {
        let rom = sram_counter_rom();
        let mut session = Session::default();
        let mut machine =
            session.power_on(rom, || Ok(Box::new(MemorySaveStore::default()))).unwrap();
        // The wait loop of the test ROM: `LDA $4210` at $800A, reached in the first frame.
        machine.set_breakpoint(0x00_800A);
        let stops = Stops { frame_end: true, breakpoint: true, ..Stops::default() };
        let live = Joypad::default();
        let stopped = session.run(&mut machine, live, &[], &mut NullAudio, &stops).unwrap();
        assert_eq!(stopped, Stopped::Breakpoint { key: 0x00_800A });
        assert_eq!(machine.frame_count(), 0, "stopped inside the first frame");
        machine.clear_breakpoint(0x00_800A);
        assert_eq!(
            session.run(&mut machine, live, &[], &mut NullAudio, &stops).unwrap(),
            Stopped::FrameEnd
        );
        assert_eq!(machine.frame_count(), 1);
    }

    #[test]
    fn a_session_dropped_without_finish_still_saves_its_recording() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.ssdreplay");
        let rom = sram_counter_rom();
        let unwound = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut session = Session::new(None, Some(&path), &rom).unwrap();
            let mut machine =
                session.power_on(rom.clone(), shared(&SharedStore::default())).unwrap();
            for _ in 0..4 {
                session.run_frame(&mut machine, Joypad::default(), &mut NullAudio).unwrap();
            }
            panic!("the run panics");
        }));
        assert!(unwound.is_err());
        assert_eq!(read(&path).frames(), 4, "saved while unwinding");
    }

    /// A pack set of one slot on `rom`, built as the run builds it (`--mods`).
    fn pack_image(rom: &[u8], byte: u8) -> ssd_mod::Image {
        let file = "data/script-1.bin".to_owned();
        let manifest = ssd_mod::Manifest {
            pack: ssd_mod::PackInfo {
                id: "test.pack".to_owned(),
                name: "Test".to_owned(),
                version: "1.0.0".to_owned(),
                game: format!("{:08X}", ssd_rom::crc32(rom)),
                author: String::new(),
                description: String::new(),
            },
            depends: Vec::new(),
            data: vec![ssd_mod::DataEntry {
                family: "script".to_owned(),
                id: 1,
                file: file.clone(),
            }],
            scene: Vec::new(),
            messages: Vec::new(),
            map: Vec::new(),
            triggers: Vec::new(),
            map_art: Vec::new(),
            face: Vec::new(),
            tables: Vec::new(),
            chapters: Vec::new(),
            feature: Vec::new(),
            pose_variant: Vec::new(),
        };
        let pack =
            ssd_mod::Pack { manifest, files: [(file, vec![byte; 64])].into_iter().collect() };
        ssd_mod::build(rom, &[pack]).unwrap()
    }

    /// The ROM of the pack tests: the synthetic cartridge, padded to a cartridge's 1 MiB.
    fn one_mebibyte() -> Vec<u8> {
        let mut rom = sram_counter_rom();
        rom.resize(0x10_0000, 0xFF);
        rom
    }

    #[test]
    fn a_recording_on_packs_plays_on_the_same_packs_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("packs.ssdreplay");
        let rom = one_mebibyte();
        let image = pack_image(&rom, 0x42);
        let mut recording = Session::new(None, Some(&path), &rom).unwrap();
        recording.set_packs(Some(image.clone()));
        let mut machine = recording.power_on(rom.clone(), shared(&SharedStore::default())).unwrap();
        assert_eq!(machine.image_crc32(), image.crc32, "the machine runs on the image");
        assert_eq!(machine.rom_crc32(), ssd_rom::crc32(&rom), "its ROM is the cartridge's");
        for _ in 0..3 {
            recording.run_frame(&mut machine, Joypad::default(), &mut NullAudio).unwrap();
        }
        recording.finish(Ok(())).unwrap();
        assert_eq!(
            read_of(&path, &one_mebibyte()).packs.map(|tag| tag.set_id),
            Some(image.set_id),
            "the tag is recorded"
        );

        let mut same = Session::new(Some(&path), None, &rom).unwrap();
        same.set_packs(Some(image.clone()));
        let played = same.power_on(rom.clone(), shared(&SharedStore::default())).unwrap();
        assert_eq!(played.image_crc32(), image.crc32);

        let mut without = Session::new(Some(&path), None, &rom).unwrap();
        let message = format!(
            "{:#}",
            without.power_on(rom.clone(), shared(&SharedStore::default())).err().unwrap()
        );
        assert!(message.contains("--mods"), "{message}");

        let mut other = Session::new(Some(&path), None, &rom).unwrap();
        other.set_packs(Some(pack_image(&rom, 0x43)));
        let message = format!(
            "{:#}",
            other.power_on(rom.clone(), shared(&SharedStore::default())).err().unwrap()
        );
        assert!(message.contains("pack set is image"), "{message}");
    }

    #[test]
    fn a_recording_without_packs_is_refused_with_packs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.ssdreplay");
        let rom = one_mebibyte();
        let mut recording = Session::new(None, Some(&path), &rom).unwrap();
        let mut machine = recording.power_on(rom.clone(), shared(&SharedStore::default())).unwrap();
        recording.run_frame(&mut machine, Joypad::default(), &mut NullAudio).unwrap();
        recording.finish(Ok(())).unwrap();
        assert_eq!(
            read_of(&path, &one_mebibyte()).packs,
            None,
            "a run on the ROM alone writes no tag"
        );
        let mut packed = Session::new(Some(&path), None, &rom).unwrap();
        packed.set_packs(Some(pack_image(&rom, 0x42)));
        let message = format!(
            "{:#}",
            packed.power_on(rom.clone(), shared(&SharedStore::default())).err().unwrap()
        );
        assert!(message.contains("without data packs"), "{message}");
    }
}
