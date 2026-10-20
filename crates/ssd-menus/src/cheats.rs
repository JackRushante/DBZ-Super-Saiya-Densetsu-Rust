//! The cheats (M5): the choices of the Cheats screen, the cheat block they are written to in work RAM,
//! and their bytes in a file. Pure Rust: no emulator, no ROM.
//!
//! A choice is a step of a short list. A multiplier is a fraction (`num` / `den`), so x1/2 and x1/4 are
//! exact. The block is the bytes the wrappers of `ssd_game::cheats` read: a num/den pair for each
//! multiplier, then one byte for each switch (random battles, the Z cards and their fighter, max zeni,
//! endless items) and the state of the wrappers' random generator, seeded with the settings; then the
//! runtime bytes the wrappers keep (the scripted-loss gate and the random-encounter mark) and the marker.
//! A neutral choice is the identity, so a block of neutral choices changes nothing.

use serde::{Deserialize, Serialize};

/// The bytes of the cheat block in work RAM.
pub const BLOCK_LEN: usize = 32;
/// The number of bytes the multipliers take: a num/den pair for each [`Setting`].
const PAIRS_LEN: usize = 2 * Setting::ALL.len();
/// `1` when random battles are off (the cheat), `0` when they are on (the neutral choice).
pub const RANDOM_OFF: usize = PAIRS_LEN;
/// The Z cards: bit 0 the attack corner, bit 1 the defence corner ([`ZMode::bits`]).
pub const ZCARDS: usize = PAIRS_LEN + 1;
/// The fighter the Z cards are for: `0` the whole party, else the character id plus one ([`ZTarget`]).
pub const ZCARDS_FIGHTER: usize = PAIRS_LEN + 2;
/// `1` when the max zeni is on.
pub const ZENI: usize = PAIRS_LEN + 3;
/// `1` when the endless items are on.
pub const ENDLESS: usize = PAIRS_LEN + 4;
/// The wrappers' random generator: a 32-bit word, little-endian, seeded with [`GENERATOR_SEED`] by the settings.
pub const GENERATOR: usize = PAIRS_LEN + 5;
/// The bytes of the settings at the start of the block: the multipliers, the switches and the generator.
pub const SETTINGS_LEN: usize = GENERATOR + 4;
/// The runtime byte the scripted-loss gate sets: nonzero while the battle cheats are off for a fight.
pub const SUSPENDED: usize = SETTINGS_LEN;
/// The runtime byte the group pick sets: nonzero while the battle being set up is a random encounter.
pub const RANDOM_ONLY: usize = SETTINGS_LEN + 1;
/// Where the marker starts: the work RAM keeps its power-on fill until a run writes the block, so the wrappers trust
/// a block only with this marker (a block without it is neutral).
pub const MARKER: usize = SETTINGS_LEN + 2;
/// The marker a written block carries at [`MARKER`].
pub const MARKER_BYTES: [u8; 4] = *b"SSDc";
/// The money multiplier: a num/den pair after the marker (the zeni a story event gives). The Cheats screen has no row for it: the
/// Difficulty presets of the Options write it. A pair with a zero is read as x1, so a block of an older run is neutral.
pub const MONEY: usize = MARKER + 4;
/// The zeni-gained multiplier: a num/den pair after the money pair, for the zeni a sale or a story event adds (the Cheats
/// screen's "Zeni gained"). A pair with a zero is read as x1, so a block of an older run is neutral.
pub const GAIN: usize = MONEY + 2;
const _: () = assert!(GAIN + 2 <= BLOCK_LEN, "the gain pair is inside the block");
/// The version of the bytes [`Cheats::to_bytes`] writes (2: with the zeni gained, 3: with the item spawner's mark;
/// versions 1 and 2 are still read).
pub const VERSION: u32 = 3;
/// The seed of the generator, written with the settings (so a settings change starts the same sequence).
pub const GENERATOR_SEED: u32 = 0x2545_F491;
/// The value of a Z ball: a corner showing a Z is the value 8 (the battle code counts it as 10).
pub const Z_VALUE: u8 = 8;
/// The number of characters a Z card can be for (the party's twelve, character ids 0 to 11).
pub const CHARACTERS: u8 = 12;
/// The most zeni a player holds.
pub const MAX_ZENI: u16 = 999;

/// A multiplier as a fraction: x1/2 is `1 / 2`, x3 is `3 / 1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Multiplier {
    pub num: u8,
    pub den: u8,
}

impl Multiplier {
    #[must_use]
    pub const fn new(num: u8, den: u8) -> Self {
        Self { num, den }
    }

    /// Whether the multiplier is 1: the wrappers do nothing for it.
    #[must_use]
    pub const fn is_identity(self) -> bool {
        self.num == self.den
    }

    /// `value * num / den`, at least 1 for a positive value, saturating at `max`. A zero stays zero.
    #[must_use]
    pub fn scale(self, value: u32, max: u32) -> u32 {
        Self::apply(value, u64::from(self.num), u64::from(self.den), max)
    }

    /// `value * den / num`: the multiplier as a divisor (the defence), with the same bounds as
    /// [`Multiplier::scale`].
    #[must_use]
    pub fn divide(self, value: u32, max: u32) -> u32 {
        Self::apply(value, u64::from(self.den), u64::from(self.num), max)
    }

    fn apply(value: u32, num: u64, den: u64, max: u32) -> u32 {
        if value == 0 {
            return 0;
        }
        let scaled = (u64::from(value) * num / den).max(1);
        u32::try_from(scaled.min(u64::from(max))).unwrap_or(max)
    }
}

/// `x n`, the whole multiplier n.
const fn times(n: u8) -> Multiplier {
    Multiplier::new(n, 1)
}

/// The steps of the damage dealt and taken, as the Cheats screen offers them: x1/2, x1, x2, x3, x5, x10
/// (x1 is the neutral step).
const DAMAGE_STEPS: [Multiplier; 6] =
    [Multiplier::new(1, 2), times(1), times(2), times(3), times(5), times(10)];

/// The steps of the EXP per battle: x1, x2, x3, x5, x10 (x1 is the neutral step).
const EXP_STEPS: [Multiplier; 5] = [times(1), times(2), times(3), times(5), times(10)];

/// The steps of the enemy power: x1/2, x1, x2, x3, x5 (x1 is the neutral step).
const POWER_STEPS: [Multiplier; 5] =
    [Multiplier::new(1, 2), times(1), times(2), times(3), times(5)];

/// The steps of the enemies per battle: x1/2, x1, x2 and x3 (x1 is the neutral step).
const ENEMIES_STEPS: [Multiplier; 4] = [Multiplier::new(1, 2), times(1), times(2), times(3)];

/// The steps of the encounter rate: x1/4, x1/2, x1, x2 and x4 (x1 is the neutral step).
const ENCOUNTERS_STEPS: [Multiplier; 5] =
    [Multiplier::new(1, 4), Multiplier::new(1, 2), times(1), times(2), times(4)];

/// The steps of the money from story events: x1/2, x1, x2 (x1 is the neutral step). Not a [`Setting`]: the Cheats screen has no
/// row for it, and its pair lies after the marker ([`MONEY`]).
pub const MONEY_STEPS: [Multiplier; 3] = [Multiplier::new(1, 2), times(1), times(2)];
/// The index of x1 in [`MONEY_STEPS`].
pub const MONEY_NEUTRAL: usize = 1;

/// The steps of the zeni gained, by a sale or a story event: x1, x2, x3, x5, x10 (x1 is the neutral step). The game pays
/// no zeni for a battle.
pub const GAIN_STEPS: [Multiplier; 5] = [times(1), times(2), times(3), times(5), times(10)];

/// The multiplier settings, in the order of their pairs in the block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Setting {
    /// The damage the party deals (x1/2 to x10).
    Damage,
    /// The damage the party takes, divided (x1/2 to x10).
    Defence,
    /// The EXP each defeated enemy gives (x1 to x10).
    Exp,
    /// The enemies' HP and BP (x1/2 to x5).
    Power,
    /// The enemies of a random battle (x1/2 to x3).
    Enemies,
    /// How often a random battle is rolled (x1/4 to x4).
    Encounters,
}

impl Setting {
    /// Every setting, in the order of their pairs in the block.
    pub const ALL: [Setting; 6] = [
        Setting::Damage,
        Setting::Defence,
        Setting::Exp,
        Setting::Power,
        Setting::Enemies,
        Setting::Encounters,
    ];

    /// The steps of the setting, the neutral one among them.
    #[must_use]
    pub const fn steps(self) -> &'static [Multiplier] {
        match self {
            Setting::Damage | Setting::Defence => &DAMAGE_STEPS,
            Setting::Exp => &EXP_STEPS,
            Setting::Power => &POWER_STEPS,
            Setting::Enemies => &ENEMIES_STEPS,
            Setting::Encounters => &ENCOUNTERS_STEPS,
        }
    }

    /// The index of the neutral step (x1) in [`Setting::steps`].
    #[must_use]
    pub const fn neutral(self) -> usize {
        match self {
            Setting::Exp => 0,
            Setting::Encounters => 2,
            _ => 1,
        }
    }

    /// The offset of the setting's num/den pair in the block.
    #[must_use]
    pub const fn offset(self) -> usize {
        2 * match self {
            Setting::Damage => 0,
            Setting::Defence => 1,
            Setting::Exp => 2,
            Setting::Power => 3,
            Setting::Enemies => 4,
            Setting::Encounters => 5,
        }
    }
}

/// Which corners of the dealt cards are Z: the choice of the Z cards row (Off, Atk, Def or Both).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ZMode {
    Off,
    /// The attack corner (the top-left ball, the attack number).
    Attack,
    /// The defence corner (the bottom-right ball, the defence number).
    Defence,
    Both,
}

impl ZMode {
    /// Every mode, in the order the screen offers them.
    pub const ALL: [ZMode; 4] = [ZMode::Off, ZMode::Attack, ZMode::Defence, ZMode::Both];

    /// Whether the attack corner is Z.
    #[must_use]
    pub const fn attack(self) -> bool {
        matches!(self, ZMode::Attack | ZMode::Both)
    }

    /// Whether the defence corner is Z.
    #[must_use]
    pub const fn defence(self) -> bool {
        matches!(self, ZMode::Defence | ZMode::Both)
    }

    /// The block's byte: bit 0 the attack corner, bit 1 the defence corner.
    #[must_use]
    pub const fn bits(self) -> u8 {
        (self.attack() as u8) | ((self.defence() as u8) << 1)
    }
}

/// Whom the Z cards are for: the whole party, or one character.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ZTarget {
    /// Every card dealt to the party (the hand itself is rewritten when the choice changes).
    Party,
    /// One character, by character id (`+1C` of its record, 0 to 11): only its attacks and defence change.
    Fighter(u8),
}

impl ZTarget {
    /// The choices of the "For" row: the whole party, then each character.
    pub const COUNT: usize = CHARACTERS as usize + 1;

    /// The choice's index: 0 the whole party, else the character id plus one.
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Party => 0,
            Self::Fighter(id) => id as usize + 1,
        }
    }

    /// The target of a choice's index; an index past the last character is the last character.
    #[must_use]
    pub fn from_index(index: usize) -> Self {
        if index == 0 {
            Self::Party
        } else {
            Self::Fighter((index - 1).min(usize::from(CHARACTERS) - 1) as u8)
        }
    }
}

/// The choices of the battle cheats: the index of the step each multiplier is on, and the switches. The
/// default is neutral: every setting at its x1 or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cheats {
    damage: u8,
    defence: u8,
    exp: u8,
    power: u8,
    enemies: u8,
    encounters: u8,
    random_battles_off: bool,
    z_mode: ZMode,
    z_target: ZTarget,
    zeni: bool,
    endless: bool,
    /// The step of [`MONEY_STEPS`]. Not part of the saved cheats (the sidecar keeps the layout of version 1): it is kept
    /// with the game's options (`difficulty.money`).
    #[serde(skip, default = "neutral_money")]
    money: u8,
    /// The step of [`GAIN_STEPS`] (the saved cheats of version 2 carry it).
    gain: u8,
    /// The item spawner was used in this game: its saves carry the CHEATS badge whatever the choices, and Reset all
    /// keeps it (version 3).
    spawned: bool,
}

/// [`Cheats`] as version 2 saved them: without the item spawner's mark.
#[derive(Debug, Clone, Copy, Deserialize)]
struct CheatsV2 {
    damage: u8,
    defence: u8,
    exp: u8,
    power: u8,
    enemies: u8,
    encounters: u8,
    random_battles_off: bool,
    z_mode: ZMode,
    z_target: ZTarget,
    zeni: bool,
    endless: bool,
    gain: u8,
}

impl From<CheatsV2> for Cheats {
    fn from(old: CheatsV2) -> Self {
        Self {
            damage: old.damage,
            defence: old.defence,
            exp: old.exp,
            power: old.power,
            enemies: old.enemies,
            encounters: old.encounters,
            random_battles_off: old.random_battles_off,
            z_mode: old.z_mode,
            z_target: old.z_target,
            zeni: old.zeni,
            endless: old.endless,
            money: MONEY_NEUTRAL as u8,
            gain: old.gain,
            spawned: false,
        }
    }
}

/// [`CheatSidecar`] as version 1 saved it: the version, then each save's slot, checksum, mark and cheats.
type SidecarV1 = (u32, Vec<(u8, u16, bool, CheatsV1)>);
/// [`CheatSidecar`] as version 2 saved it.
type SidecarV2 = (u32, Vec<(u8, u16, bool, CheatsV2)>);

/// [`Cheats`] as version 1 saved them: without the zeni gained, read as x1.
#[derive(Debug, Clone, Copy, Deserialize)]
struct CheatsV1 {
    damage: u8,
    defence: u8,
    exp: u8,
    power: u8,
    enemies: u8,
    encounters: u8,
    random_battles_off: bool,
    z_mode: ZMode,
    z_target: ZTarget,
    zeni: bool,
    endless: bool,
}

impl From<CheatsV1> for Cheats {
    fn from(old: CheatsV1) -> Self {
        Self {
            damage: old.damage,
            defence: old.defence,
            exp: old.exp,
            power: old.power,
            enemies: old.enemies,
            encounters: old.encounters,
            random_battles_off: old.random_battles_off,
            z_mode: old.z_mode,
            z_target: old.z_target,
            zeni: old.zeni,
            endless: old.endless,
            money: MONEY_NEUTRAL as u8,
            gain: 0,
            spawned: false,
        }
    }
}

fn neutral_money() -> u8 {
    MONEY_NEUTRAL as u8
}

/// The bytes a [`Cheats`] is saved as: its version, then the choices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct Versioned {
    version: u32,
    cheats: Cheats,
}

impl Default for Cheats {
    fn default() -> Self {
        Self::neutral()
    }
}

impl Cheats {
    /// Every setting at its neutral step: no cheat is on.
    #[must_use]
    pub fn neutral() -> Self {
        Self {
            damage: Setting::Damage.neutral() as u8,
            defence: Setting::Defence.neutral() as u8,
            exp: Setting::Exp.neutral() as u8,
            power: Setting::Power.neutral() as u8,
            enemies: Setting::Enemies.neutral() as u8,
            encounters: Setting::Encounters.neutral() as u8,
            random_battles_off: false,
            z_mode: ZMode::Off,
            z_target: ZTarget::Party,
            zeni: false,
            endless: false,
            money: MONEY_NEUTRAL as u8,
            gain: 0,
            spawned: false,
        }
    }

    /// Whether the item spawner was used in this game (the CHEATS badge of its saves).
    #[must_use]
    pub fn spawned(&self) -> bool {
        self.spawned
    }

    /// Marks the game as one the item spawner was used in.
    pub fn set_spawned(&mut self, spawned: bool) {
        self.spawned = spawned;
    }

    /// The index of the step `setting` is on.
    #[must_use]
    pub fn index(&self, setting: Setting) -> usize {
        usize::from(self.slot(setting))
    }

    /// Puts `setting` on the step `index`, clamped to the last step of its list.
    pub fn set_index(&mut self, setting: Setting, index: usize) {
        let last = setting.steps().len() - 1;
        *self.slot_mut(setting) = index.min(last) as u8;
    }

    /// The multiplier `setting` is on.
    #[must_use]
    pub fn multiplier(&self, setting: Setting) -> Multiplier {
        setting.steps()[self.index(setting)]
    }

    /// Whether `setting` is on (not at its neutral step).
    #[must_use]
    pub fn is_on(&self, setting: Setting) -> bool {
        self.index(setting) != setting.neutral()
    }

    /// Whether random battles are off.
    #[must_use]
    pub fn random_battles_off(&self) -> bool {
        self.random_battles_off
    }

    pub fn set_random_battles_off(&mut self, off: bool) {
        self.random_battles_off = off;
    }

    /// The Z cards' corners.
    #[must_use]
    pub fn z_mode(&self) -> ZMode {
        self.z_mode
    }

    pub fn set_z_mode(&mut self, mode: ZMode) {
        self.z_mode = mode;
    }

    /// Whom the Z cards are for.
    #[must_use]
    pub fn z_target(&self) -> ZTarget {
        self.z_target
    }

    /// Chooses whom the Z cards are for; a character id past the last one is the last character.
    pub fn set_z_target(&mut self, target: ZTarget) {
        self.z_target = match target {
            ZTarget::Fighter(id) => ZTarget::Fighter(id.min(CHARACTERS - 1)),
            ZTarget::Party => ZTarget::Party,
        };
    }

    /// Whether the max zeni is on.
    #[must_use]
    pub fn zeni(&self) -> bool {
        self.zeni
    }

    pub fn set_zeni(&mut self, on: bool) {
        self.zeni = on;
    }

    /// Whether the endless items are on.
    #[must_use]
    pub fn endless(&self) -> bool {
        self.endless
    }

    pub fn set_endless(&mut self, on: bool) {
        self.endless = on;
    }

    /// The index of the step of [`MONEY_STEPS`] the money is on.
    #[must_use]
    pub fn money_index(&self) -> usize {
        usize::from(self.money).min(MONEY_STEPS.len() - 1)
    }

    /// Puts the money on the step `index`, clamped to the last step.
    pub fn set_money_index(&mut self, index: usize) {
        self.money = index.min(MONEY_STEPS.len() - 1) as u8;
    }

    /// The multiplier of the zeni a story event gives.
    #[must_use]
    pub fn money(&self) -> Multiplier {
        MONEY_STEPS[self.money_index()]
    }

    /// Whether the money is not at x1.
    #[must_use]
    pub fn money_on(&self) -> bool {
        self.money_index() != MONEY_NEUTRAL
    }

    /// The index of the step of [`GAIN_STEPS`] the zeni gained is on.
    #[must_use]
    pub fn gain_index(&self) -> usize {
        usize::from(self.gain).min(GAIN_STEPS.len() - 1)
    }

    /// Puts the zeni gained on the step `index`, clamped to the last step.
    pub fn set_gain_index(&mut self, index: usize) {
        self.gain = index.min(GAIN_STEPS.len() - 1) as u8;
    }

    /// The multiplier of the zeni a sale or a story event adds.
    #[must_use]
    pub fn gain(&self) -> Multiplier {
        GAIN_STEPS[self.gain_index()]
    }

    /// Whether the zeni gained is not at x1.
    #[must_use]
    pub fn gain_on(&self) -> bool {
        self.gain_index() != 0
    }

    /// The zeni gained's pair, as the block holds it at [`GAIN`].
    #[must_use]
    pub fn gain_bytes(&self) -> [u8; 2] {
        let gain = self.gain();
        [gain.num, gain.den]
    }

    /// Whether the encounter rate is on. It only changes the random battles, so it counts while they are on.
    #[must_use]
    pub fn encounters_on(&self) -> bool {
        !self.random_battles_off && self.is_on(Setting::Encounters)
    }

    /// The number of settings that are on: the "On: N" of the screen. The Z cards count once, whatever
    /// their target, and the encounter rate only while random battles are on.
    #[must_use]
    pub fn on_count(&self) -> usize {
        let multipliers =
            [Setting::Damage, Setting::Defence, Setting::Exp, Setting::Power, Setting::Enemies]
                .iter()
                .filter(|&&setting| self.is_on(setting))
                .count();
        let switches =
            [self.random_battles_off, self.z_mode != ZMode::Off, self.zeni, self.endless];
        multipliers
            + usize::from(self.encounters_on())
            + usize::from(self.money_on())
            + usize::from(self.gain_on())
            + switches.iter().filter(|&&on| on).count()
    }

    /// The settings that are on, as the screen's "On" list shows them: a label and its value. `names` are the
    /// characters' names, in character id order (the "For" row's names).
    #[must_use]
    pub fn lines(&self, names: &[String]) -> Vec<(&'static str, String)> {
        let mut lines = Vec::new();
        for (setting, label) in
            [(Setting::Damage, "Damage"), (Setting::Defence, "Defence"), (Setting::Exp, "EXP")]
        {
            if self.is_on(setting) {
                lines.push((label, multiplier_text(self.multiplier(setting))));
            }
        }
        if self.z_mode != ZMode::Off {
            let word = match (self.z_mode.attack(), self.z_mode.defence()) {
                (true, true) => "Both",
                (true, false) => "Atk",
                _ => "Def",
            };
            let value = match self.z_target {
                ZTarget::Party => word.to_owned(),
                ZTarget::Fighter(id) => {
                    format!("{word} {}", names.get(usize::from(id)).map_or("?", String::as_str))
                }
            };
            lines.push(("Z cards", value));
        }
        if self.zeni {
            lines.push(("Zeni", "max".to_owned()));
        }
        if self.gain_on() {
            lines.push(("Zeni gained", multiplier_text(self.gain())));
        }
        if self.money_on() {
            lines.push(("Money", multiplier_text(self.money())));
        }
        if self.endless {
            lines.push(("Items", "endless".to_owned()));
        }
        if self.random_battles_off {
            lines.push(("Random battles", "off".to_owned()));
        }
        if self.encounters_on() {
            lines.push(("How often", multiplier_text(self.multiplier(Setting::Encounters))));
        }
        for (setting, label) in [(Setting::Enemies, "Enemies"), (Setting::Power, "Enemy power")] {
            if self.is_on(setting) {
                lines.push((label, multiplier_text(self.multiplier(setting))));
            }
        }
        lines
    }

    /// Whether every setting is neutral (the block then changes nothing).
    #[must_use]
    pub fn is_neutral(&self) -> bool {
        self.on_count() == 0
    }

    /// The settings bytes: the num/den pairs, the switches, and the generator seeded with
    /// [`GENERATOR_SEED`] (the first [`SETTINGS_LEN`] bytes of the block).
    #[must_use]
    pub fn settings_bytes(&self) -> [u8; SETTINGS_LEN] {
        let mut bytes = [0_u8; SETTINGS_LEN];
        for setting in Setting::ALL {
            let multiplier = self.multiplier(setting);
            let at = setting.offset();
            bytes[at] = multiplier.num;
            bytes[at + 1] = multiplier.den;
        }
        bytes[RANDOM_OFF] = u8::from(self.random_battles_off);
        bytes[ZCARDS] = self.z_mode.bits();
        bytes[ZCARDS_FIGHTER] = match self.z_target {
            ZTarget::Party => 0,
            ZTarget::Fighter(id) => id + 1,
        };
        bytes[ZENI] = u8::from(self.zeni);
        bytes[ENDLESS] = u8::from(self.endless);
        bytes[GENERATOR..GENERATOR + 4].copy_from_slice(&GENERATOR_SEED.to_le_bytes());
        bytes
    }

    /// The money multiplier's pair, as the block holds it at [`MONEY`].
    #[must_use]
    pub fn money_bytes(&self) -> [u8; 2] {
        let money = self.money();
        [money.num, money.den]
    }

    /// The whole cheat block: the settings, and the runtime bytes at zero (the scripted-loss gate
    /// open, no random-encounter mark). The game's work RAM holds the power-on fill until it is
    /// written, so the block is written whole once, when a run starts.
    #[must_use]
    pub fn to_block(&self) -> [u8; BLOCK_LEN] {
        let mut block = [0_u8; BLOCK_LEN];
        block[..SETTINGS_LEN].copy_from_slice(&self.settings_bytes());
        block[MARKER..MARKER + MARKER_BYTES.len()].copy_from_slice(&MARKER_BYTES);
        let money = self.money();
        block[MONEY] = money.num;
        block[MONEY + 1] = money.den;
        block[GAIN..GAIN + 2].copy_from_slice(&self.gain_bytes());
        block
    }

    /// The cheats as bytes: the version, then the choices (bincode, as the bag's sidecar).
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        let versioned = Versioned { version: VERSION, cheats: *self };
        bincode::serde::encode_to_vec(versioned, config()).unwrap_or_default()
    }

    /// Reads [`Cheats::to_bytes`].
    ///
    /// # Errors
    ///
    /// A message if the bytes are not a cheats file, or of another version, or name an index that no
    /// step has.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let not = |error| format!("not a cheats file: {error}");
        let cheats = match version_of(bytes)? {
            1 => {
                let (old, _): ((u32, CheatsV1), usize) =
                    bincode::serde::decode_from_slice(bytes, config()).map_err(not)?;
                Self::from(old.1)
            }
            2 => {
                let (old, _): ((u32, CheatsV2), usize) =
                    bincode::serde::decode_from_slice(bytes, config()).map_err(not)?;
                Self::from(old.1)
            }
            VERSION => {
                let (versioned, _): (Versioned, usize) =
                    bincode::serde::decode_from_slice(bytes, config()).map_err(not)?;
                versioned.cheats
            }
            other => return Err(format!("a cheats file of version {other}, not {VERSION}")),
        };
        cheats.check()?;
        Ok(cheats)
    }

    /// Whether every choice is a step of its list, and the Z cards' character exists.
    ///
    /// # Errors
    ///
    /// A message naming the choice that is out of range.
    pub fn check(&self) -> Result<(), String> {
        for setting in Setting::ALL {
            if self.index(setting) >= setting.steps().len() {
                return Err(format!("the step of {setting:?} is out of range"));
            }
        }
        if usize::from(self.gain) >= GAIN_STEPS.len() {
            return Err("the step of the zeni gained is out of range".to_owned());
        }
        if matches!(self.z_target, ZTarget::Fighter(id) if id >= CHARACTERS) {
            return Err("the Z cards are for a character that does not exist".to_owned());
        }
        Ok(())
    }

    fn slot(self, setting: Setting) -> u8 {
        match setting {
            Setting::Damage => self.damage,
            Setting::Defence => self.defence,
            Setting::Exp => self.exp,
            Setting::Power => self.power,
            Setting::Enemies => self.enemies,
            Setting::Encounters => self.encounters,
        }
    }

    fn slot_mut(&mut self, setting: Setting) -> &mut u8 {
        match setting {
            Setting::Damage => &mut self.damage,
            Setting::Defence => &mut self.defence,
            Setting::Exp => &mut self.exp,
            Setting::Power => &mut self.power,
            Setting::Enemies => &mut self.enemies,
            Setting::Encounters => &mut self.encounters,
        }
    }
}

/// The text of a multiplier as the screen shows it: `×2`, `×½`, `×¼`.
#[must_use]
pub fn multiplier_text(multiplier: Multiplier) -> String {
    match (multiplier.num, multiplier.den) {
        (1, 2) => "×½".to_owned(),
        (1, 4) => "×¼".to_owned(),
        (num, 1) => format!("×{num}"),
        (num, den) => format!("×{num}/{den}"),
    }
}

/// The rows of the Cheats screen in the order it reads them: the Battle and Cards panels, then the
/// Resources and Encounters panels, then Reset all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Row {
    Damage,
    Defence,
    Exp,
    ZCards,
    ZFor,
    MaxZeni,
    ZeniGained,
    EndlessItems,
    ItemSpawner,
    RandomBattles,
    HowOften,
    Enemies,
    EnemyPower,
    ResetAll,
}

impl Row {
    /// Every row, in the order the cursor moves through them.
    pub const ALL: [Row; 14] = [
        Row::Damage,
        Row::Defence,
        Row::Exp,
        Row::ZCards,
        Row::ZFor,
        Row::MaxZeni,
        Row::ZeniGained,
        Row::EndlessItems,
        Row::ItemSpawner,
        Row::RandomBattles,
        Row::HowOften,
        Row::Enemies,
        Row::EnemyPower,
        Row::ResetAll,
    ];

    /// The row's label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Damage => "Damage",
            Self::Defence => "Defence",
            Self::Exp => "EXP",
            Self::ZCards => "Z cards",
            Self::ZFor => "For",
            Self::MaxZeni => "Max zeni",
            Self::ZeniGained => "Zeni gained",
            Self::EndlessItems => "Endless items",
            Self::ItemSpawner => "Item spawner",
            Self::RandomBattles => "Random battles",
            Self::HowOften => "How often",
            Self::Enemies => "Enemies",
            Self::EnemyPower => "Enemy power",
            Self::ResetAll => "Reset all",
        }
    }

    /// The group the row sits under on its panel (Reset all has none).
    #[must_use]
    pub const fn group(self) -> &'static str {
        match self {
            Self::Damage | Self::Defence | Self::Exp => "Battle",
            Self::ZCards | Self::ZFor => "Cards",
            Self::MaxZeni | Self::ZeniGained | Self::EndlessItems | Self::ItemSpawner => {
                "Resources"
            }
            Self::RandomBattles | Self::HowOften | Self::Enemies | Self::EnemyPower => "Encounters",
            Self::ResetAll => "",
        }
    }

    /// The help line shown while the row is under the cursor.
    #[must_use]
    pub const fn help(self) -> &'static str {
        match self {
            Self::Damage => "How hard your fighters hit.",
            Self::Defence => "Damage you take is divided by this.",
            Self::Exp => "EXP gained after every battle.",
            Self::ZCards => "Every card you draw carries a Z.",
            Self::ZFor => "Who gets the Z cards.",
            Self::MaxZeni => "Keep zeni at the most you can carry.",
            Self::ZeniGained => "Zeni from selling and from the story, multiplied.",
            Self::EndlessItems => "Using an item does not use it up.",
            Self::ItemSpawner => "Put any object in the bag: A opens the list.",
            Self::RandomBattles => "Battles on the map. Story battles always happen.",
            Self::HowOften => "How often a random battle starts.",
            Self::Enemies => "How many enemies come at once.",
            Self::EnemyPower => "HP and BP of every enemy.",
            Self::ResetAll => "Turn every cheat off.",
        }
    }

    /// The number of choices the row offers in a row (none for Reset all).
    #[must_use]
    pub const fn choices(self) -> usize {
        match self {
            Self::Damage | Self::Defence => DAMAGE_STEPS.len(),
            Self::Exp => EXP_STEPS.len(),
            Self::Enemies => ENEMIES_STEPS.len(),
            Self::EnemyPower => POWER_STEPS.len(),
            Self::HowOften => ENCOUNTERS_STEPS.len(),
            Self::ZeniGained => GAIN_STEPS.len(),
            Self::ItemSpawner => 1,
            Self::ZCards => ZMode::ALL.len(),
            Self::ZFor => ZTarget::COUNT,
            Self::MaxZeni | Self::EndlessItems | Self::RandomBattles => 2,
            Self::ResetAll => 0,
        }
    }

    /// The multiplier the row sets, if it sets one.
    fn setting(self) -> Option<Setting> {
        match self {
            Self::Damage => Some(Setting::Damage),
            Self::Defence => Some(Setting::Defence),
            Self::Exp => Some(Setting::Exp),
            Self::Enemies => Some(Setting::Enemies),
            Self::EnemyPower => Some(Setting::Power),
            Self::HowOften => Some(Setting::Encounters),
            _ => None,
        }
    }

    /// The text of choice `index` in the row: `names` are the characters' names by character id (the
    /// "For" row's choices after the whole party).
    #[must_use]
    pub fn choice_text(self, index: usize, names: &[String]) -> String {
        match self {
            Self::ZCards => ["Off", "Atk", "Def", "Both"][index.min(3)].to_owned(),
            Self::ZFor if index == 0 => "Whole party".to_owned(),
            Self::ZFor => names.get(index - 1).map_or("?", String::as_str).to_owned(),
            Self::MaxZeni | Self::EndlessItems => ["Off", "On"][index.min(1)].to_owned(),
            Self::RandomBattles => ["On", "Off"][index.min(1)].to_owned(),
            Self::ZeniGained => multiplier_text(GAIN_STEPS[index.min(GAIN_STEPS.len() - 1)]),
            Self::ItemSpawner => "Open".to_owned(),
            _ => self.setting().map_or_else(String::new, |setting| {
                multiplier_text(setting.steps()[index.min(setting.steps().len() - 1)])
            }),
        }
    }

    /// The index of the choice the row is on (0 for Reset all).
    #[must_use]
    pub fn index(self, cheats: &Cheats) -> usize {
        match self {
            Self::ZCards => {
                ZMode::ALL.iter().position(|&mode| mode == cheats.z_mode()).unwrap_or(0)
            }
            Self::ZFor => cheats.z_target().index(),
            Self::MaxZeni => usize::from(cheats.zeni()),
            Self::EndlessItems => usize::from(cheats.endless()),
            Self::RandomBattles => usize::from(cheats.random_battles_off()),
            Self::ZeniGained => cheats.gain_index(),
            Self::ItemSpawner | Self::ResetAll => 0,
            _ => self.setting().map_or(0, |setting| cheats.index(setting)),
        }
    }

    /// Puts the row on choice `index`, clamped to its choices. Reset all sets nothing.
    pub fn set(self, cheats: &mut Cheats, index: usize) {
        let index = index.min(self.choices().saturating_sub(1));
        match self {
            Self::ZCards => cheats.set_z_mode(ZMode::ALL[index]),
            Self::ZFor => cheats.set_z_target(ZTarget::from_index(index)),
            Self::MaxZeni => cheats.set_zeni(index == 1),
            Self::EndlessItems => cheats.set_endless(index == 1),
            Self::RandomBattles => cheats.set_random_battles_off(index == 1),
            Self::ZeniGained => cheats.set_gain_index(index),
            Self::ItemSpawner | Self::ResetAll => {}
            _ => {
                if let Some(setting) = self.setting() {
                    cheats.set_index(setting, index);
                }
            }
        }
    }

    /// Moves the row's choice by `by` steps (-1 for Left, 1 for Right), wrapping at both ends. Reset
    /// all does nothing.
    pub fn step(self, cheats: &mut Cheats, by: i8) {
        let count = self.choices();
        if count == 0 {
            return;
        }
        let next = (self.index(cheats) as i64 + i64::from(by)).rem_euclid(count as i64) as usize;
        self.set(cheats, next);
    }
}

/// The layout version of [`CheatSidecar`].
pub const SIDECAR_VERSION: u32 = 3;

/// The cheats of one save slot, as the sidecar file keeps them. The slot's checksum word says which save
/// they belong to; `marked` says whether that save was made with a cheat on (the CHEATS badge).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedCheats {
    /// The save slot, 0 to 2.
    pub slot: u8,
    /// The checksum word of the slot when the cheats were saved.
    pub checksum: u16,
    /// Whether a cheat was on when the save was made.
    pub marked: bool,
    pub cheats: Cheats,
}

/// The sidecar file (`saves/ssd.cheats`): the cheats of the save slots, versioned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheatSidecar {
    /// The file layout's version.
    pub version: u32,
    pub saves: Vec<SavedCheats>,
}

impl CheatSidecar {
    /// The empty sidecar of this version.
    #[must_use]
    pub fn empty() -> Self {
        Self { version: SIDECAR_VERSION, saves: Vec::new() }
    }

    /// The file's bytes.
    #[must_use]
    pub fn to_bytes(&self) -> Vec<u8> {
        bincode::serde::encode_to_vec(self, config()).unwrap_or_default()
    }

    /// Reads the file's bytes. An entry whose choices are out of range is left out, so its save starts
    /// with the cheats off.
    ///
    /// # Errors
    ///
    /// The bytes are not a cheats file, or of another version.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let not = |error| format!("not a cheats file: {error}");
        let sidecar = match version_of(bytes)? {
            // Version 1: the cheats without the zeni gained, read as x1; the file is written as version 2.
            1 => {
                let (old, _): (SidecarV1, usize) =
                    bincode::serde::decode_from_slice(bytes, config()).map_err(not)?;
                let saves = old
                    .1
                    .into_iter()
                    .map(|(slot, checksum, marked, cheats)| SavedCheats {
                        slot,
                        checksum,
                        marked,
                        cheats: cheats.into(),
                    })
                    .collect();
                Self { version: SIDECAR_VERSION, saves }
            }
            2 => {
                let (old, _): (SidecarV2, usize) =
                    bincode::serde::decode_from_slice(bytes, config()).map_err(not)?;
                let saves = old
                    .1
                    .into_iter()
                    .map(|(slot, checksum, marked, cheats)| SavedCheats {
                        slot,
                        checksum,
                        marked,
                        cheats: cheats.into(),
                    })
                    .collect();
                Self { version: SIDECAR_VERSION, saves }
            }
            SIDECAR_VERSION => {
                let (sidecar, _): (Self, usize) =
                    bincode::serde::decode_from_slice(bytes, config()).map_err(not)?;
                sidecar
            }
            other => {
                return Err(format!("cheats file version {other}, expected {SIDECAR_VERSION}"));
            }
        };
        let saves =
            sidecar.saves.into_iter().filter(|saved| saved.cheats.check().is_ok()).collect();
        Ok(Self { saves, ..sidecar })
    }

    /// The cheats saved for `slot` with `checksum`, if the sidecar has an entry for exactly that save.
    #[must_use]
    pub fn entry_for(&self, slot: u8, checksum: u16) -> Option<&SavedCheats> {
        self.saves.iter().find(|saved| saved.slot == slot && saved.checksum == checksum)
    }

    /// Whether the save of `slot` with `checksum` was made with a cheat on (its badge).
    #[must_use]
    pub fn marked(&self, slot: u8, checksum: u16) -> bool {
        self.entry_for(slot, checksum).is_some_and(|saved| saved.marked)
    }

    /// Stores `cheats` for `slot` with `checksum`, replacing the slot's older entry. The entry is marked when
    /// a cheat is on.
    pub fn store(&mut self, slot: u8, checksum: u16, cheats: Cheats) {
        self.saves.retain(|saved| saved.slot != slot);
        let marked = !cheats.is_neutral() || cheats.spawned;
        self.saves.push(SavedCheats { slot, checksum, marked, cheats });
    }
}

/// The version word a cheats file starts with (both [`Cheats::to_bytes`] and [`CheatSidecar::to_bytes`]).
fn version_of(bytes: &[u8]) -> Result<u32, String> {
    bincode::serde::decode_from_slice::<u32, _>(bytes, config())
        .map(|(version, _)| version)
        .map_err(|error| format!("not a cheats file: {error}"))
}

/// The encoding of [`Cheats::to_bytes`]: bincode, little-endian, varint integers, as the bag's.
fn config() -> bincode::config::Configuration<
    bincode::config::LittleEndian,
    bincode::config::Varint,
    bincode::config::Limit<{ 1024 * 1024 }>,
> {
    bincode::config::standard().with_limit::<{ 1024 * 1024 }>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_version_1_file_reads_with_the_zeni_gained_at_x1_and_is_written_as_version_2() {
        // A sidecar a player's game wrote with version 1: slot 0, its checksum, marked, damage x1/2, EXP x5, power x2,
        // Z cards on both corners.
        let v1 = [
            0x01, 0x01, 0x00, 0xFB, 0x04, 0xA2, 0x01, 0x00, 0x01, 0x03, 0x02, 0x01, 0x02, 0x00,
            0x03, 0x00, 0x00, 0x00,
        ];
        let sidecar = CheatSidecar::from_bytes(&v1).unwrap();
        assert_eq!(sidecar.version, SIDECAR_VERSION);
        let saved = sidecar.entry_for(0, 0xA204).unwrap();
        assert!(saved.marked);
        assert_eq!(saved.cheats.multiplier(Setting::Exp), times(5));
        assert_eq!(saved.cheats.z_mode(), ZMode::Both);
        assert_eq!(saved.cheats.gain(), times(1));
        let again = CheatSidecar::from_bytes(&sidecar.to_bytes()).unwrap();
        assert_eq!(again, sidecar, "version 2 reads back the same");
        let mut cheats = saved.cheats;
        cheats.set_gain_index(4);
        assert_eq!(Cheats::from_bytes(&cheats.to_bytes()).unwrap().gain(), times(10));
    }

    #[test]
    fn the_spawners_mark_puts_the_badge_on_the_save_and_reads_back() {
        let mut cheats = Cheats::neutral();
        cheats.set_spawned(true);
        assert!(cheats.is_neutral(), "the mark is not a choice: On stays 0");
        let mut sidecar = CheatSidecar::empty();
        sidecar.store(1, 0x1234, cheats);
        assert!(sidecar.marked(1, 0x1234), "a game the spawner was used in carries the badge");
        let again = CheatSidecar::from_bytes(&sidecar.to_bytes()).unwrap();
        assert!(again.entry_for(1, 0x1234).unwrap().cheats.spawned());
        assert!(Cheats::from_bytes(&cheats.to_bytes()).unwrap().spawned());
    }

    #[test]
    fn the_zeni_gained_is_a_row_a_line_and_a_pair_of_the_block() {
        let mut cheats = Cheats::neutral();
        assert_eq!(cheats.to_block()[GAIN..GAIN + 2], [1, 1], "x1 by default");
        Row::ZeniGained.step(&mut cheats, 1);
        assert_eq!(cheats.gain(), times(2));
        assert_eq!(cheats.on_count(), 1);
        assert_eq!(cheats.lines(&[]), vec![("Zeni gained", "×2".to_owned())]);
        Row::ZeniGained.step(&mut cheats, -2);
        assert_eq!(cheats.gain(), times(10), "Left from x2 wraps past x1 to x10");
        assert_eq!(cheats.to_block()[GAIN..GAIN + 2], [10, 1]);
    }

    fn at(setting: Setting, index: usize) -> Cheats {
        let mut cheats = Cheats::neutral();
        cheats.set_index(setting, index);
        cheats
    }

    #[test]
    fn the_neutral_choices_are_the_identity_and_none_is_on() {
        let neutral = Cheats::default();
        assert!(neutral.is_neutral());
        assert_eq!(neutral.on_count(), 0);
        for setting in Setting::ALL {
            assert_eq!(neutral.multiplier(setting), setting.steps()[setting.neutral()]);
            assert!(neutral.multiplier(setting).is_identity(), "{setting:?}");
            assert!(!neutral.is_on(setting));
        }
        assert_eq!(neutral.multiplier(Setting::Exp), times(1));
        assert_eq!(neutral.multiplier(Setting::Damage), times(1));
        assert_eq!(neutral.multiplier(Setting::Encounters), times(1));
        assert!(!neutral.random_battles_off());
        assert_eq!(neutral.z_mode(), ZMode::Off);
        assert_eq!(neutral.z_target(), ZTarget::Party);
        assert!(!neutral.zeni() && !neutral.endless());
    }

    #[test]
    fn every_step_list_is_as_the_screen_offers_it() {
        let shown = |setting: Setting| {
            setting
                .steps()
                .iter()
                .map(|m| {
                    if m.den == 1 { format!("x{}", m.num) } else { format!("x{}/{}", m.num, m.den) }
                })
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(shown(Setting::Damage), "x1/2 x1 x2 x3 x5 x10");
        assert_eq!(shown(Setting::Defence), "x1/2 x1 x2 x3 x5 x10");
        assert_eq!(shown(Setting::Exp), "x1 x2 x3 x5 x10");
        assert_eq!(shown(Setting::Power), "x1/2 x1 x2 x3 x5");
        assert_eq!(shown(Setting::Enemies), "x1/2 x1 x2 x3");
        assert_eq!(shown(Setting::Encounters), "x1/4 x1/2 x1 x2 x4");
        for setting in Setting::ALL {
            assert!(setting.neutral() < setting.steps().len());
            assert!(setting.steps()[setting.neutral()].is_identity(), "{setting:?}");
        }
    }

    #[test]
    fn a_choice_clamps_to_its_list_and_counts_as_on() {
        let mut cheats = Cheats::neutral();
        cheats.set_index(Setting::Enemies, 99);
        assert_eq!(cheats.index(Setting::Enemies), 3);
        assert_eq!(cheats.multiplier(Setting::Enemies), times(3));
        cheats.set_index(Setting::Enemies, 0);
        assert_eq!(cheats.multiplier(Setting::Enemies), Multiplier::new(1, 2));
        assert!(cheats.is_on(Setting::Enemies));
        assert_eq!(cheats.on_count(), 1);
        cheats.set_index(Setting::Enemies, Setting::Enemies.neutral());
        assert!(cheats.is_neutral());
    }

    #[test]
    fn the_block_holds_each_setting_as_a_num_den_pair_and_zero_runtime_bytes() {
        let mut cheats = Cheats::neutral();
        cheats.set_index(Setting::Damage, 2); // x2
        cheats.set_index(Setting::Defence, 0); // x1/2
        cheats.set_index(Setting::Exp, 2); // x3
        cheats.set_index(Setting::Power, 0); // x1/2
        cheats.set_index(Setting::Encounters, 0); // x1/4
        let block = cheats.to_block();
        assert_eq!(block.len(), BLOCK_LEN);
        assert_eq!(&block[0..2], &[2, 1]);
        assert_eq!(&block[2..4], &[1, 2]);
        assert_eq!(&block[4..6], &[3, 1]);
        assert_eq!(&block[6..8], &[1, 2]);
        assert_eq!(&block[8..10], &[1, 1]);
        assert_eq!(&block[10..12], &[1, 4]);
        assert_eq!(&block[SUSPENDED..MARKER], &[0; MARKER - SUSPENDED]);
        assert_eq!(&block[..SETTINGS_LEN], &cheats.settings_bytes()[..]);
        assert_eq!(&block[MARKER..MARKER + MARKER_BYTES.len()], &MARKER_BYTES);
        assert!(MARKER + MARKER_BYTES.len() <= BLOCK_LEN);
    }

    #[test]
    fn the_money_multiplier_is_a_pair_after_the_marker_counts_as_on_and_is_not_saved_in_the_sidecar()
     {
        let mut cheats = Cheats::neutral();
        assert_eq!(&cheats.to_block()[MONEY..MONEY + 2], &[1, 1], "neutral is x1");
        assert!(cheats.is_neutral() && !cheats.money_on());
        cheats.set_money_index(2);
        let block = cheats.to_block();
        assert_eq!(&block[MONEY..MONEY + 2], &[2, 1]);
        assert_eq!(
            &block[..SETTINGS_LEN],
            &Cheats::neutral().settings_bytes()[..],
            "the settings do not move"
        );
        assert_eq!(
            (cheats.on_count(), cheats.is_neutral()),
            (1, false),
            "money alone is a cheat: the save is marked"
        );
        assert_eq!(cheats.lines(&[]), vec![("Money", "×2".to_owned())]);
        cheats.set_money_index(0);
        assert_eq!(cheats.money(), Multiplier::new(1, 2));
        cheats.set_money_index(9);
        assert_eq!(cheats.money_index(), 2, "clamped");
        const { assert!(MONEY + 2 <= BLOCK_LEN) };
        // The sidecar's bytes are those of version 1: the money is not among them.
        assert_eq!(cheats.to_bytes(), Cheats::neutral().to_bytes());
        assert!(!Cheats::from_bytes(&cheats.to_bytes()).unwrap().money_on());
    }

    #[test]
    fn the_switches_and_the_z_cards_are_bytes_of_the_block() {
        let mut cheats = Cheats::neutral();
        cheats.set_random_battles_off(true);
        cheats.set_zeni(true);
        cheats.set_endless(true);
        cheats.set_z_mode(ZMode::Attack);
        cheats.set_z_target(ZTarget::Fighter(3));
        let block = cheats.to_block();
        assert_eq!(block[RANDOM_OFF], 1);
        assert_eq!(block[ZCARDS], 1, "attack is bit 0");
        assert_eq!(block[ZCARDS_FIGHTER], 4, "character 3 is byte 4");
        assert_eq!(block[ZENI], 1);
        assert_eq!(block[ENDLESS], 1);
        assert_eq!(&block[GENERATOR..GENERATOR + 4], &GENERATOR_SEED.to_le_bytes());

        let mut both = Cheats::neutral();
        both.set_z_mode(ZMode::Both);
        assert_eq!(both.settings_bytes()[ZCARDS], 3);
        assert_eq!(both.settings_bytes()[ZCARDS_FIGHTER], 0, "the whole party");
        assert_eq!(ZMode::Defence.bits(), 2);
        assert_eq!(ZMode::Off.bits(), 0);
    }

    #[test]
    fn the_z_cards_count_once_whatever_their_target() {
        let mut cheats = Cheats::neutral();
        cheats.set_z_mode(ZMode::Both);
        cheats.set_z_target(ZTarget::Fighter(5));
        assert_eq!(cheats.on_count(), 1, "one fighter counts 1");
        cheats.set_z_target(ZTarget::Party);
        assert_eq!(cheats.on_count(), 1);
        cheats.set_random_battles_off(true);
        cheats.set_zeni(true);
        cheats.set_endless(true);
        cheats.set_index(Setting::Encounters, 3);
        assert_eq!(cheats.on_count(), 4, "the rate is not on while random battles are off");
        cheats.set_random_battles_off(false);
        assert_eq!(cheats.on_count(), 4, "the Z cards, zeni, items and the rate");
        cheats.set_z_mode(ZMode::Off);
        cheats.set_z_target(ZTarget::Fighter(5));
        assert_eq!(cheats.on_count(), 3, "a target without a mode is not on");
        assert!(!cheats.is_neutral());
    }

    #[test]
    fn the_on_list_names_each_setting_on_and_counts_as_the_screen_does() {
        let names: Vec<String> = ["Goku", "Gohan"].iter().map(|name| (*name).to_owned()).collect();
        let mut cheats = Cheats::neutral();
        assert_eq!(cheats.lines(&names), Vec::new());
        cheats.set_index(Setting::Damage, 2);
        cheats.set_index(Setting::Exp, 4);
        cheats.set_index(Setting::Encounters, 0);
        cheats.set_z_mode(ZMode::Both);
        cheats.set_z_target(ZTarget::Fighter(1));
        cheats.set_zeni(true);
        assert_eq!(
            cheats.lines(&names),
            [
                ("Damage", "×2".to_owned()),
                ("EXP", "×10".to_owned()),
                ("Z cards", "Both Gohan".to_owned()),
                ("Zeni", "max".to_owned()),
                ("How often", "×¼".to_owned()),
            ]
        );
        assert_eq!(cheats.on_count(), 5);
        cheats.set_random_battles_off(true);
        assert_eq!(cheats.lines(&names).len(), 5, "the random battles line replaces the rate's");
        assert_eq!(cheats.on_count(), cheats.lines(&names).len());
        cheats.set_index(Setting::Power, 0);
        cheats.set_index(Setting::Enemies, 3);
        cheats.set_endless(true);
        assert_eq!(cheats.on_count(), cheats.lines(&names).len(), "the count is the list's length");
    }

    #[test]
    fn every_row_reads_back_the_choice_it_sets_and_wraps_at_both_ends() {
        let names: Vec<String> = (0..12).map(|id| format!("Fighter {id}")).collect();
        for row in Row::ALL {
            let mut cheats = Cheats::neutral();
            let count = row.choices();
            if row == Row::ResetAll {
                assert_eq!(count, 0, "Reset all has no choices");
                row.step(&mut cheats, 1);
                assert_eq!(cheats, Cheats::neutral(), "Reset all does nothing to a row");
                continue;
            }
            for index in 0..count {
                row.set(&mut cheats, index);
                assert_eq!(row.index(&cheats), index, "{row:?} on {index}");
                assert!(!row.choice_text(index, &names).is_empty(), "{row:?} {index} has a text");
            }
            row.set(&mut cheats, count - 1);
            row.step(&mut cheats, 1);
            assert_eq!(row.index(&cheats), 0, "{row:?}: Right from the last choice wraps");
            row.step(&mut cheats, -1);
            assert_eq!(row.index(&cheats), count - 1, "{row:?}: Left from the first wraps");
        }
        assert_eq!(Row::ZFor.choice_text(0, &names), "Whole party");
        assert_eq!(Row::ZFor.choice_text(1, &names), "Fighter 0");
        assert_eq!(Row::ZFor.choice_text(12, &names), "Fighter 11");
        assert_eq!(Row::RandomBattles.choice_text(0, &names), "On");
        assert_eq!(Row::RandomBattles.choice_text(1, &names), "Off");
        assert_eq!(Row::Damage.choice_text(0, &names), "×½");
        assert_eq!(Row::Damage.choice_text(5, &names), "×10");
        assert_eq!(Row::HowOften.choice_text(0, &names), "×¼");
        assert_eq!(Row::ZCards.choice_text(3, &names), "Both");
    }

    #[test]
    fn the_for_row_moves_between_the_whole_party_and_each_character() {
        assert_eq!(ZTarget::from_index(0), ZTarget::Party);
        assert_eq!(ZTarget::from_index(1), ZTarget::Fighter(0));
        assert_eq!(ZTarget::from_index(12), ZTarget::Fighter(11));
        assert_eq!(ZTarget::from_index(99), ZTarget::Fighter(11), "past the end is the last one");
        assert_eq!(ZTarget::Fighter(11).index(), 12);
        assert_eq!(ZTarget::COUNT, 13);
    }

    #[test]
    fn the_sidecar_keeps_each_slots_cheats_and_badges_the_ones_with_a_cheat_on() {
        let mut on = Cheats::neutral();
        on.set_index(Setting::Exp, 2);
        let mut sidecar = CheatSidecar::empty();
        sidecar.store(0, 0xBEEF, on);
        sidecar.store(1, 0x1234, Cheats::neutral());
        let back = CheatSidecar::from_bytes(&sidecar.to_bytes()).unwrap();
        assert_eq!(back, sidecar);
        assert_eq!(back.entry_for(0, 0xBEEF).map(|saved| saved.cheats), Some(on));
        assert!(back.marked(0, 0xBEEF), "a save with a cheat on has the badge");
        assert!(!back.marked(1, 0x1234), "a save with all off has none");
        assert!(back.entry_for(0, 0xBEEE).is_none(), "another save's checksum has no entry");
        assert!(back.entry_for(2, 0xBEEF).is_none(), "another slot has no entry");
    }

    #[test]
    fn a_slot_changed_by_another_way_has_no_entry_and_a_damaged_entry_is_left_out() {
        let mut sidecar = CheatSidecar::empty();
        sidecar.store(0, 0xBEEF, Cheats::neutral());
        sidecar.store(1, 0x2222, Cheats::neutral());
        sidecar.saves[1].cheats.enemies = 9;
        let back = CheatSidecar::from_bytes(&sidecar.to_bytes()).unwrap();
        assert_eq!(back.saves.len(), 1, "the damaged entry is left out");
        assert!(back.entry_for(1, 0x2222).is_none());
        assert!(!back.marked(0, 0x9999), "a changed checksum has no badge");
        let mut other = CheatSidecar::empty();
        other.version = 9;
        assert!(CheatSidecar::from_bytes(&other.to_bytes()).unwrap_err().contains("version 9"));
        assert!(CheatSidecar::from_bytes(b"\x01\x02").is_err());
    }

    #[test]
    fn storing_a_slot_replaces_its_entry_and_marks_it_by_its_settings() {
        let mut sidecar = CheatSidecar::empty();
        let mut on = Cheats::neutral();
        on.set_zeni(true);
        sidecar.store(2, 7, on);
        assert!(sidecar.marked(2, 7));
        sidecar.store(2, 8, Cheats::neutral());
        assert_eq!(sidecar.saves.len(), 1, "one entry per slot");
        assert!(!sidecar.marked(2, 8) && sidecar.entry_for(2, 7).is_none());
    }

    #[test]
    fn the_fighter_is_clamped_to_the_last_character() {
        let mut cheats = Cheats::neutral();
        cheats.set_z_target(ZTarget::Fighter(200));
        assert_eq!(cheats.z_target(), ZTarget::Fighter(CHARACTERS - 1));
    }

    #[test]
    fn scaling_is_exact_floors_at_one_and_saturates() {
        assert_eq!(times(2).scale(19, u32::MAX), 38);
        assert_eq!(times(3).scale(9, u32::MAX), 27);
        assert_eq!(Multiplier::new(1, 2).scale(19, u32::MAX), 9);
        assert_eq!(
            Multiplier::new(1, 2).scale(1, u32::MAX),
            1,
            "a positive value stays at least 1"
        );
        assert_eq!(times(10).scale(0, u32::MAX), 0, "zero stays zero");
        assert_eq!(times(5).scale(0xFFFF, 0xFFFF), 0xFFFF, "saturates at the maximum");
        assert_eq!(times(10).scale(0x00FF_FFFF, 0x00FF_FFFF), 0x00FF_FFFF);
        assert_eq!(times(2).divide(16, u32::MAX), 8, "defence x2 halves the damage taken");
        assert_eq!(Multiplier::new(1, 2).divide(16, u32::MAX), 32, "defence x1/2 doubles it");
        assert_eq!(times(10).divide(3, u32::MAX), 1, "a positive damage stays at least 1");
    }

    #[test]
    fn the_bytes_round_trip_and_another_version_or_index_is_refused() {
        for setting in Setting::ALL {
            for index in 0..setting.steps().len() {
                let cheats = at(setting, index);
                assert_eq!(
                    Cheats::from_bytes(&cheats.to_bytes()),
                    Ok(cheats),
                    "{setting:?} {index}"
                );
            }
        }
        let mut switches = Cheats::neutral();
        switches.set_random_battles_off(true);
        switches.set_zeni(true);
        switches.set_endless(true);
        for mode in ZMode::ALL {
            for target in [ZTarget::Party, ZTarget::Fighter(CHARACTERS - 1)] {
                switches.set_z_mode(mode);
                switches.set_z_target(target);
                assert_eq!(Cheats::from_bytes(&switches.to_bytes()), Ok(switches));
            }
        }
        let mut other = Versioned { version: VERSION + 1, cheats: Cheats::neutral() };
        let bytes = bincode::serde::encode_to_vec(other, config()).unwrap();
        assert!(Cheats::from_bytes(&bytes).unwrap_err().contains("version"));
        other.version = VERSION;
        other.cheats.enemies = 9;
        let bytes = bincode::serde::encode_to_vec(other, config()).unwrap();
        assert!(Cheats::from_bytes(&bytes).unwrap_err().contains("Enemies"));
        other.cheats.enemies = 1;
        other.cheats.z_target = ZTarget::Fighter(CHARACTERS);
        let bytes = bincode::serde::encode_to_vec(other, config()).unwrap();
        assert!(Cheats::from_bytes(&bytes).unwrap_err().contains("character"));
        assert!(Cheats::from_bytes(&[0xFF]).is_err());
    }
}
