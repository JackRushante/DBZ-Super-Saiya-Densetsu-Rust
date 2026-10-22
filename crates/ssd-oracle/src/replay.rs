//! Replays (`*.ssdreplay`): a session recorded from power-on, played back frame by frame.
//!
//! File layout, format 1 (integers little-endian):
//!
//! ```text
//! magic "SSDRPLAY" | format u32 | crc32 u32 | rom_crc32 u32 | wram_seed u64 |
//! core_version_len u32 | core_version | sram_len u32 | initial SRAM | frames u32 |
//! inputs: frames × u16
//! ```
//!
//! `crc32` covers every byte after it up to the end of the file; it is checked (after the magic
//! and the format) before anything else is read, so a flipped bit anywhere is caught. A
//! `sram_len` of 0 means no initial SRAM (a cartridge without SRAM). Input `f` is the joypad held
//! during frame `f` (from the tick after frame `f` is rendered, power-on for frame 0, to the tick
//! that renders frame `f + 1`: one [`Machine::run_frame`] call), as a word of
//! [`joypad_bits`](crate::joypad_bits): bit 0 B, 1 Y, 2 Select, 3 Start, 4 Up, 5 Down, 6 Left,
//! 7 Right, 8 A, 9 X, 10 L, 11 R; bits 12–15 clear.
//!
//! File layout, format 2 (the Enhanced mode's, `ssd_game::Mode::Enhanced`): the magic, the format
//! (2), the crc32 and the body of format 1 up to the frame count, then a mode byte (`0` Classic,
//! `1` Enhanced) and, per frame, the joypad word and its host events. A host event is the host's
//! change to the machine at the boundary before the frame: a work RAM write (kind `0`: address u32,
//! length u32, bytes) or an application event (kind `1`: tag u8, length u32, payload), which the
//! emulator keeps but does not interpret. A replay with no mode and no events is written as
//! format 1, byte for byte as before.
//!
//! File layout, format 3 (a run on a pack set, `ssd_mod`): the format 2 layout with, right after the
//! mode byte, the set's id (`u64`) and the CRC32 of its image (`u32`). A replay with no pack set is
//! never written as format 3: its bytes are those of format 1 or 2, as before.
//!
//! A replay names the ROM, the power-on WRAM fill and the emulator core it was recorded with
//! ([`CORE_VERSION`]); reading it for another ROM or core, or playing it on a cartridge whose
//! SRAM does not take its initial SRAM, is an error naming the mismatch. A replay with a pack set
//! plays only on that set's image ([`Replay::power_on_image`]).

use crate::input::{JOYPAD_USED_BITS, joypad_bits, joypad_from_bits};
use crate::{AudioSink, Joypad, Machine, MemorySaveStore, OracleError};
use std::path::Path;

/// The literal of [`ORACLE_REVISION`], for `concat!`: the revision is bumped here.
macro_rules! oracle_revision {
    () => {
        "1"
    };
}

/// The emulator version of this build, which replays carry:
/// `jgenesis <upstream commit>+<CRC32 of the vendored sources> oracle <ORACLE_REVISION>`. The
/// first part, set by `build.rs` (`build/core_version.rs`), is the upstream commit that
/// `third_party/jgenesis/PATCHES.md` records and the CRC32 of the vendored `.rs` files and crate
/// manifests (line endings normalised, so a CRLF checkout has the same version); the second is
/// [`ORACLE_REVISION`], for this crate's own code that decides how the core runs. It changes
/// whenever the code of the core can: any edit of the vendored sources (a local patch, a new
/// upstream version), but not of `PATCHES.md` or other documentation. A replay is never played on
/// an emulator that may run it differently.
pub const CORE_VERSION: &str = concat!(env!("SSD_CORE_VERSION"), " oracle ", oracle_revision!());

/// The revision of `ssd-oracle`'s emulation-affecting code, part of [`CORE_VERSION`].
///
/// Bump it (1, 2, …) in the commit that changes what a run without hooks does from power-on with
/// the same input, outside the vendored core: the `SnesEmulatorConfig` that
/// `Machine::with_wram_seed` creates the core with, the frame a tick's joypad is looked up by
/// (`FramePoller` in `input.rs`) or how a joypad reaches the core (`joypad_inputs`), the ticks
/// `Machine::run_frame` takes without hooks, and how `Replay::power_on` builds the machine.
/// Replays recorded before are then rejected (regenerate the reference replays with
/// `cargo run --release -p xtask -- make-replays`). The file layout has its own number,
/// [`FORMAT_VERSION`].
pub const ORACLE_REVISION: &str = oracle_revision!();

const MAGIC: &[u8; 8] = b"SSDRPLAY";
/// The file layout's number (see the module documentation); bump it when the layout changes.
/// What the emulator does is versioned by [`CORE_VERSION`] instead (see [`ORACLE_REVISION`]).
const FORMAT_VERSION: u32 = 1;
/// The layout with the mode and the host events (format 2).
const FORMAT_HOST: u32 = 2;
/// The layout with a pack set (format 3): format 2 with the set's id and image CRC32 after the mode.
const FORMAT_PACKS: u32 = 3;
/// The length of the ROM a replay is for: 1 MiB, the first part of any pack set's image.
const ROM_LEN: usize = 0x10_0000;
/// The mode of the original's rules (`ssd_game::Mode::Classic`): the replays of the reference
/// suite. Any other mode is written as format 2.
pub const MODE_CLASSIC: u8 = 0;
/// The mode of the enhanced rules (`ssd_game::Mode::Enhanced`).
pub const MODE_ENHANCED: u8 = 1;

/// A change the host makes to the machine at the boundary before a frame, not a joypad word.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostEvent {
    /// Writes `bytes` into work RAM at the bus address `address` (`$7E:0000` onwards): the host's
    /// write, made by [`Machine::write_wram`].
    Wram { address: u32, bytes: Vec<u8> },
    /// An event of the application: its tag and payload mean what the application says. The
    /// emulator keeps them in the replay and does not act on them.
    App { tag: u8, payload: Vec<u8> },
}

/// The pack set a replay was recorded with: the set's id and the CRC32 of its image. The image is not
/// in the replay: the loader finds the packs and builds it, and it must have this CRC32.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PackTag {
    /// The set's id (`ssd_mod::Image::set_id`).
    pub set_id: u64,
    /// CRC32 of the set's image (`ssd_mod::Image::crc32`).
    pub image_crc32: u32,
}

/// A session from power-on: the ROM, power-on state and joypad input of every frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Replay {
    /// CRC32 of the original ROM ([`Machine::rom_crc32`]), also for a replay on a pack set's image.
    pub rom_crc32: u32,
    /// Seed of the power-on WRAM fill ([`Machine::wram_seed`]).
    pub wram_seed: u64,
    /// Cartridge SRAM at power-on; `None` for a cartridge without SRAM.
    pub initial_sram: Option<Vec<u8>>,
    /// The joypad held during each frame, from frame 0.
    pub inputs: Vec<Joypad>,
    /// The mode the run was recorded in: [`MODE_CLASSIC`] or [`MODE_ENHANCED`] (format 2 for any
    /// other than classic).
    pub mode: u8,
    /// The host events of each frame, as long as `inputs`, or empty when there are none.
    pub events: Vec<Vec<HostEvent>>,
    /// The pack set the run was recorded with, if it was not on the ROM alone (format 3).
    pub packs: Option<PackTag>,
}

impl Replay {
    /// An empty replay of `machine` as it is now (meant for power-on): its ROM, WRAM seed and
    /// SRAM.
    #[must_use]
    pub fn of(machine: &Machine) -> Self {
        Self {
            rom_crc32: machine.rom_crc32(),
            wram_seed: machine.wram_seed(),
            initial_sram: machine.sram().map(<[u8]>::to_vec),
            inputs: Vec::new(),
            mode: MODE_CLASSIC,
            events: Vec::new(),
            packs: None,
        }
    }

    /// Frames of input recorded.
    #[must_use]
    pub fn frames(&self) -> u64 {
        self.inputs.len() as u64
    }

    /// The joypad of `frame`; all buttons released after the last recorded frame.
    #[must_use]
    pub fn input(&self, frame: u64) -> Joypad {
        usize::try_from(frame)
            .ok()
            .and_then(|frame| self.inputs.get(frame))
            .copied()
            .unwrap_or_default()
    }

    /// The host events made before `frame` (none for a format 1 replay, or a frame without any).
    #[must_use]
    pub fn events_at(&self, frame: u64) -> &[HostEvent] {
        usize::try_from(frame)
            .ok()
            .and_then(|frame| self.events.get(frame))
            .map_or(&[], Vec::as_slice)
    }

    /// The machine at the replay's power-on: `rom` with the replay's WRAM fill, and its initial
    /// SRAM in an in-memory save store (no save file is read or written). Play it with
    /// [`Replay::input`] as the lookup of [`Machine::run_frame_with`] or
    /// [`Machine::run_until_sync`].
    ///
    /// # Errors
    ///
    /// [`OracleError::Replay`] if `rom` is not the replay's ROM or the cartridge does not take the
    /// initial SRAM (another SRAM size, SRAM where the replay has none or the other way round);
    /// [`OracleError::Load`] if the core rejects the ROM.
    pub fn power_on(&self, rom: Vec<u8>) -> Result<Machine, OracleError> {
        if let Some(tag) = self.packs {
            return Err(replay_error(&format!(
                "the replay was recorded with pack set {:016X} (image {:08X}): it plays on that \
                 image, not on the ROM alone",
                tag.set_id, tag.image_crc32
            )));
        }
        let rom_crc32 = ssd_rom::crc32(&rom);
        if rom_crc32 != self.rom_crc32 {
            return Err(rom_mismatch(self.rom_crc32, rom_crc32));
        }
        let machine = Machine::with_wram_seed(rom, Box::new(self.initial_saves()), self.wram_seed)?;
        self.started(machine)
    }

    /// The machine at the replay's power-on on a pack set's `image` (`ssd_mod::build`), as
    /// [`Replay::power_on`] makes it on the ROM: the image's first 1 MiB is the ROM the replay is for,
    /// and the image must be the one the replay was recorded with ([`Replay::packs`]).
    ///
    /// # Errors
    ///
    /// [`OracleError::Replay`] if the replay has no pack set, if `image` is shorter than 1 MiB or its
    /// first 1 MiB is not the replay's ROM, if its CRC32 is not the set's image CRC32, or for the SRAM
    /// as [`Replay::power_on`]; [`OracleError::Load`] if the core rejects the image.
    pub fn power_on_image(&self, image: Vec<u8>) -> Result<Machine, OracleError> {
        let Some(tag) = self.packs else {
            return Err(replay_error(
                "the replay was recorded without packs: it plays on the ROM alone, power_on",
            ));
        };
        let Some(rom) = image.get(..ROM_LEN) else {
            return Err(replay_error("the image is shorter than the 1 MiB ROM"));
        };
        let rom_crc32 = ssd_rom::crc32(rom);
        if rom_crc32 != self.rom_crc32 {
            return Err(rom_mismatch(self.rom_crc32, rom_crc32));
        }
        let image_crc32 = ssd_rom::crc32(&image);
        if image_crc32 != tag.image_crc32 {
            return Err(replay_error(&format!(
                "the replay's pack set is image {:08X}, this image is {image_crc32:08X}",
                tag.image_crc32
            )));
        }
        let machine = Machine::with_image(
            image,
            self.rom_crc32,
            Box::new(self.initial_saves()),
            self.wram_seed,
        )?;
        self.started(machine)
    }

    /// The save store a machine at the replay's power-on starts with: its initial SRAM, in memory.
    fn initial_saves(&self) -> MemorySaveStore {
        let mut saves = MemorySaveStore::default();
        if let Some(sram) = &self.initial_sram {
            saves.0.insert("sav".to_owned(), sram.clone());
        }
        saves
    }

    /// `machine`, powered on for this replay, once its cartridge has been checked against the initial
    /// SRAM (see [`Replay::power_on`]).
    fn started(&self, machine: Machine) -> Result<Machine, OracleError> {
        let cartridge = machine.sram().map(<[u8]>::len);
        let replay = self.initial_sram.as_ref().map(Vec::len);
        match (cartridge, replay) {
            (Some(_), Some(_)) if machine.sram() == self.initial_sram.as_deref() => Ok(machine),
            (None, None) => Ok(machine),
            (Some(cartridge), Some(replay)) => Err(OracleError::Replay(format!(
                "the replay's initial SRAM ({replay} bytes) does not fit the cartridge's \
                 {cartridge} bytes of SRAM"
            ))),
            (Some(cartridge), None) => Err(OracleError::Replay(format!(
                "the replay has no initial SRAM, the cartridge has {cartridge} bytes of SRAM"
            ))),
            (None, Some(replay)) => Err(OracleError::Replay(format!(
                "the replay has {replay} bytes of initial SRAM, the cartridge has no SRAM"
            ))),
        }
    }

    /// Plays the rest of the replay on `machine` (powered on by [`Replay::power_on`], hooks
    /// installed or not): frame after frame, [`Machine::run_frame_with`] with the joypad
    /// recorded for each, until the machine has rendered the replay's last frame. A machine
    /// already there plays nothing. The hooks' dispatches run as in `run_frame_with`, so a
    /// `Verify` or `Rust` playback stays on the recorded input of every frame. The machine's
    /// WRAM freezes ([`Machine::set_freezes`]) apply as in any run; a replay played with them is
    /// not the recorded session, and not reproducible across hook modes.
    ///
    /// # Errors
    ///
    /// The errors of [`Machine::run_frame_with`].
    pub fn play(
        &self,
        machine: &mut Machine,
        audio: &mut dyn AudioSink,
    ) -> Result<(), OracleError> {
        while machine.frame_count() < self.frames() {
            machine.run_frame_with(&mut |frame| self.input(frame), audio)?;
        }
        Ok(())
    }

    /// The replay file, recorded with emulator core `core_version` (normally [`CORE_VERSION`]).
    ///
    /// # Panics
    ///
    /// If the core version, the initial SRAM, the inputs or an event are longer than a `u32` can count.
    #[must_use]
    pub fn to_bytes(&self, core_version: &str) -> Vec<u8> {
        let len = |n: usize| u32::try_from(n).expect("replay section longer than u32::MAX");
        let host = self.packs.is_some()
            || self.mode != MODE_CLASSIC
            || self.events.iter().any(|events| !events.is_empty());
        let sram = self.initial_sram.as_deref().unwrap_or_default();
        let mut body =
            Vec::with_capacity(32 + core_version.len() + sram.len() + 2 * self.inputs.len());
        body.extend_from_slice(&self.rom_crc32.to_le_bytes());
        body.extend_from_slice(&self.wram_seed.to_le_bytes());
        body.extend_from_slice(&len(core_version.len()).to_le_bytes());
        body.extend_from_slice(core_version.as_bytes());
        body.extend_from_slice(&len(sram.len()).to_le_bytes());
        body.extend_from_slice(sram);
        body.extend_from_slice(&len(self.inputs.len()).to_le_bytes());
        if host {
            body.push(self.mode);
        }
        if let Some(tag) = self.packs {
            body.extend_from_slice(&tag.set_id.to_le_bytes());
            body.extend_from_slice(&tag.image_crc32.to_le_bytes());
        }
        for (frame, &input) in self.inputs.iter().enumerate() {
            body.extend_from_slice(&joypad_bits(input).to_le_bytes());
            if host {
                let events = self.events.get(frame).map_or(&[][..], Vec::as_slice);
                body.extend_from_slice(&len(events.len()).to_le_bytes());
                for event in events {
                    match event {
                        HostEvent::Wram { address, bytes } => {
                            body.push(0);
                            body.extend_from_slice(&address.to_le_bytes());
                            body.extend_from_slice(&len(bytes.len()).to_le_bytes());
                            body.extend_from_slice(bytes);
                        }
                        HostEvent::App { tag, payload } => {
                            body.push(1);
                            body.push(*tag);
                            body.extend_from_slice(&len(payload.len()).to_le_bytes());
                            body.extend_from_slice(payload);
                        }
                    }
                }
            }
        }

        let format = if self.packs.is_some() {
            FORMAT_PACKS
        } else if host {
            FORMAT_HOST
        } else {
            FORMAT_VERSION
        };
        let mut out = Vec::with_capacity(body.len() + 16);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&format.to_le_bytes());
        out.extend_from_slice(&ssd_rom::crc32(&body).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Writes the replay file ([`Replay::to_bytes`]) to `path`, creating its directory, through
    /// a temporary file next to it and a rename, so a failed write never leaves a truncated
    /// replay where a good one was.
    ///
    /// # Errors
    ///
    /// The directory, the temporary file or the rename fails (the temporary file is removed).
    pub fn write_to(&self, path: &Path, core_version: &str) -> std::io::Result<()> {
        if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("ssdreplay.tmp");
        let written = std::fs::write(&tmp, self.to_bytes(core_version))
            .and_then(|()| std::fs::rename(&tmp, path));
        if written.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        written
    }

    /// The replay file `bytes` as recorded with emulator core `core_version` instead, and the
    /// core version it carried: the same ROM, WRAM seed, initial SRAM and inputs. Only valid when
    /// the new core runs the replay as the old one did (a change that does not touch the
    /// emulation, such as an output-only patch); `verify-replays` and the game's own checks are
    /// what prove that, not this.
    ///
    /// # Errors
    ///
    /// [`OracleError::Replay`] if `bytes` is not a replay of this format, is corrupted, or would
    /// not come out of [`Replay::to_bytes`] unchanged (anything the replay holds that the
    /// rewrite would drop or alter).
    pub fn rebase(bytes: &[u8], core_version: &str) -> Result<(String, Vec<u8>), OracleError> {
        let mut r = Reader(bytes);
        if r.take(MAGIC.len()).ok() != Some(MAGIC.as_slice()) {
            return Err(replay_error("not a replay (no SSDRPLAY magic)"));
        }
        r.take(8)?; // format and checksum, checked by `from_bytes`
        let rom_crc32 = r.u32()?;
        r.u64()?;
        let version_len = r.len()?;
        let old_version = String::from_utf8_lossy(r.take(version_len)?).into_owned();
        let replay = Self::from_bytes(bytes, rom_crc32, &old_version)?;
        if replay.to_bytes(&old_version) != bytes {
            return Err(replay_error("the replay does not rewrite to itself"));
        }
        Ok((old_version, replay.to_bytes(core_version)))
    }

    /// Reads a replay file for the ROM with CRC32 `expected_rom_crc32`, played on emulator core
    /// `core_version` (normally [`CORE_VERSION`]).
    ///
    /// # Errors
    ///
    /// [`OracleError::Replay`], naming what is wrong: not a replay, another format, corrupted
    /// (checksum), truncated or with trailing bytes, recorded with another ROM or core version,
    /// or an input with unused bits set.
    pub fn from_bytes(
        bytes: &[u8],
        expected_rom_crc32: u32,
        core_version: &str,
    ) -> Result<Self, OracleError> {
        let mut r = Reader(bytes);
        if r.take(MAGIC.len()).ok() != Some(MAGIC.as_slice()) {
            return Err(replay_error("not a replay (no SSDRPLAY magic)"));
        }
        let format = r.u32()?;
        if format != FORMAT_VERSION && format != FORMAT_HOST && format != FORMAT_PACKS {
            return Err(OracleError::Replay(format!(
                "replay format {format}, expected {FORMAT_VERSION}, {FORMAT_HOST} or {FORMAT_PACKS}"
            )));
        }
        let crc = r.u32()?;
        if ssd_rom::crc32(r.0) != crc {
            return Err(replay_error("the replay is corrupted (checksum mismatch)"));
        }
        let rom_crc32 = r.u32()?;
        if rom_crc32 != expected_rom_crc32 {
            return Err(rom_mismatch(rom_crc32, expected_rom_crc32));
        }
        let wram_seed = r.u64()?;
        let version_len = r.len()?;
        let version = r.take(version_len)?;
        if version != core_version.as_bytes() {
            return Err(OracleError::Replay(format!(
                "the replay was recorded with emulator core `{}`, this build is `{core_version}`: \
                 record it again",
                String::from_utf8_lossy(version)
            )));
        }
        let sram_len = r.len()?;
        let sram = r.take(sram_len)?;
        let initial_sram = (sram_len > 0).then(|| sram.to_vec());
        let frames = r.len()?;
        let (mode, packs, mut events) = if format != FORMAT_VERSION {
            let mode = r.u8()?;
            if mode > MODE_ENHANCED {
                return Err(replay_error(&format!("the replay's mode {mode} is not known")));
            }
            let packs = if format == FORMAT_PACKS {
                Some(PackTag { set_id: r.u64()?, image_crc32: r.u32()? })
            } else {
                None
            };
            (mode, packs, Vec::with_capacity(frames.min(r.0.len())))
        } else {
            (MODE_CLASSIC, None, Vec::new())
        };
        let mut inputs = Vec::with_capacity(frames.min(r.0.len()));
        for frame in 0..frames {
            let word = r.take(2)?;
            let bits = u16::from_le_bytes([word[0], word[1]]);
            if bits & !JOYPAD_USED_BITS != 0 {
                return Err(OracleError::Replay(format!(
                    "the replay's input of frame {frame} has unused bits set ({bits:04X})"
                )));
            }
            inputs.push(joypad_from_bits(bits));
            if format != FORMAT_VERSION {
                let count = r.len()?;
                let mut frame_events = Vec::with_capacity(count.min(64));
                for _ in 0..count {
                    frame_events.push(match r.u8()? {
                        0 => {
                            let address = r.u32()?;
                            let n = r.len()?;
                            HostEvent::Wram { address, bytes: r.take(n)?.to_vec() }
                        }
                        1 => {
                            let tag = r.u8()?;
                            let n = r.len()?;
                            HostEvent::App { tag, payload: r.take(n)?.to_vec() }
                        }
                        kind => {
                            return Err(replay_error(&format!(
                                "frame {frame} has a host event of unknown kind {kind}"
                            )));
                        }
                    });
                }
                events.push(frame_events);
            }
        }
        if !r.0.is_empty() {
            return Err(replay_error("trailing bytes after the replay's inputs"));
        }
        Ok(Self { rom_crc32, wram_seed, initial_sram, inputs, mode, events, packs })
    }
}

/// Records a replay of a machine run from power-on without hooks: every frame's joypad, through
/// [`Recorder::run_frame`]. Recording needs a machine without hooks (every routine original): a
/// hooked call that renders a frame runs on with the joypad its frame started with, which a
/// replay played frame by frame would not reproduce.
#[derive(Debug)]
pub struct Recorder {
    replay: Replay,
}

impl Recorder {
    /// Starts recording `machine`, which must be at power-on, without hooks and without WRAM
    /// freezes ([`Machine::set_freezes`]): its ROM, WRAM seed and SRAM are recorded now.
    ///
    /// # Errors
    ///
    /// [`OracleError::Replay`] if the machine has run, has hooks or has freezes.
    pub fn start(machine: &Machine) -> Result<Self, OracleError> {
        if machine.frame_count() != 0 || machine.total_master_cycles() != 0 {
            return Err(OracleError::Replay(format!(
                "a recording starts at power-on; the machine is at frame {}",
                machine.frame_count()
            )));
        }
        check_unhooked(machine)?;
        Ok(Self { replay: Replay::of(machine) })
    }

    /// Sets the mode the recording is made in ([`Replay::mode`]); it is set before the first frame.
    #[must_use]
    pub fn in_mode(mut self, mode: u8) -> Self {
        self.replay.mode = mode;
        self
    }

    /// Sets the pack set the recording is made with ([`Replay::packs`]); it is set before the first frame.
    #[must_use]
    pub fn in_packs(mut self, packs: Option<PackTag>) -> Self {
        self.replay.packs = packs;
        self
    }

    /// Runs `machine` one frame with `input` held ([`Machine::run_frame`]) and records `input`
    /// as that frame's.
    ///
    /// # Errors
    ///
    /// [`OracleError::Replay`] if the machine is not where the recording is (another frame: a
    /// state was loaded, or frames were run outside the recorder), has hooks or has freezes; the
    /// errors of [`Machine::run_frame`], after which nothing is recorded for the frame.
    pub fn run_frame(
        &mut self,
        machine: &mut Machine,
        input: Joypad,
        audio: &mut dyn AudioSink,
    ) -> Result<(), OracleError> {
        self.run_frame_with_events(machine, input, &[], audio)
    }

    /// [`Recorder::run_frame`] with the host `events` made before the frame, which are recorded with
    /// it. The caller makes them first ([`apply_events`]): the recorder only keeps them.
    ///
    /// # Errors
    ///
    /// As for [`Recorder::run_frame`].
    pub fn run_frame_with_events(
        &mut self,
        machine: &mut Machine,
        input: Joypad,
        events: &[HostEvent],
        audio: &mut dyn AudioSink,
    ) -> Result<(), OracleError> {
        if machine.frame_count() != self.replay.frames() {
            return Err(OracleError::Replay(format!(
                "the machine is at frame {}, the recording at frame {}",
                machine.frame_count(),
                self.replay.frames()
            )));
        }
        check_unhooked(machine)?;
        machine.run_frame(input, audio)?;
        self.replay.inputs.push(input);
        self.replay.events.push(events.to_vec());
        Ok(())
    }

    /// The replay recorded so far.
    #[must_use]
    pub fn replay(&self) -> &Replay {
        &self.replay
    }

    #[must_use]
    pub fn into_replay(self) -> Replay {
        self.replay
    }
}

/// Makes the work RAM writes of `events` on `machine`; the other events are the application's.
///
/// # Errors
///
/// [`Machine::write_wram`]'s errors.
pub fn apply_events(machine: &mut Machine, events: &[HostEvent]) -> Result<(), OracleError> {
    for event in events {
        if let HostEvent::Wram { address, bytes } = event {
            machine.write_wram(*address, bytes)?;
        }
    }
    Ok(())
}

fn check_unhooked(machine: &Machine) -> Result<(), OracleError> {
    if machine.is_hooked() {
        return Err(replay_error(
            "recording needs a machine without hooks (every routine original)",
        ));
    }
    if !machine.freezes().is_empty() {
        return Err(replay_error(
            "recording needs a machine without WRAM freezes (they are not part of the replay)",
        ));
    }
    Ok(())
}

fn replay_error(message: &str) -> OracleError {
    OracleError::Replay(message.to_owned())
}

fn truncated() -> OracleError {
    replay_error("the replay is truncated")
}

fn rom_mismatch(replay: u32, current: u32) -> OracleError {
    OracleError::Replay(format!(
        "the replay is for the ROM with CRC32 {replay:08X}, this ROM is {current:08X}"
    ))
}

struct Reader<'a>(&'a [u8]);

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], OracleError> {
        if self.0.len() < n {
            return Err(truncated());
        }
        let (head, tail) = self.0.split_at(n);
        self.0 = tail;
        Ok(head)
    }

    fn u32(&mut self) -> Result<u32, OracleError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().expect("4 bytes")))
    }

    fn u64(&mut self) -> Result<u64, OracleError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().expect("8 bytes")))
    }

    fn u8(&mut self) -> Result<u8, OracleError> {
        Ok(self.take(1)?[0])
    }

    /// A `u32` length.
    fn len(&mut self) -> Result<usize, OracleError> {
        usize::try_from(self.u32()?).map_err(|_| truncated())
    }
}
