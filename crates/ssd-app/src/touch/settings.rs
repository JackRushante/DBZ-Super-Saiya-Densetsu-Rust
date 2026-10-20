//! The touch controls' choices, kept per device in `ssd.settings` (M7's per-PC file) under `controls.touch.*`:
//! the style of the d-pad, the opacity, the haptic tick, the automatic switch in battle, hiding with a gamepad, and
//! the place and size of each group of controls in the landscape and in the portrait layout. Whole numbers only,
//! like the rest of the file; a missing key is the default and a wild value is clamped.

use ssd_menus::options::{Values, controls as keys};

/// The groups of controls the player can move and resize.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Dpad,
    Face,
    L,
    R,
    StartSelect,
    Speed,
    Menu,
}

impl Group {
    pub const ALL: [Group; 7] = [
        Group::Dpad,
        Group::Face,
        Group::L,
        Group::R,
        Group::StartSelect,
        Group::Speed,
        Group::Menu,
    ];

    /// The place in [`Group::ALL`].
    #[must_use]
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|&g| g == self).unwrap_or(0)
    }

    /// The suffix of the settings keys.
    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Dpad => "dpad",
            Self::Face => "face",
            Self::L => "l",
            Self::R => "r",
            Self::StartSelect => "ss",
            Self::Speed => "speed",
            Self::Menu => "menu",
        }
    }

    /// The name the edit panel shows.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Dpad => "D-pad",
            Self::Face => "A / B / X / Y",
            Self::L => "L button",
            Self::R => "R button",
            Self::StartSelect => "Start / Select",
            Self::Speed => "Speed button",
            Self::Menu => "Menu button",
        }
    }
}

/// What the player changed for one group: its centre (per mille of the screen's width and height) and its size
/// (percent). `None` is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Place {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub scale: Option<u32>,
}

/// The smallest and the largest size of a group, in percent.
pub const SCALE_MIN: u32 = 100;
pub const SCALE_MAX: u32 = 180;
/// The least and the most opaque the controls can be, in percent.
pub const OPACITY_MIN: u32 = 20;
pub const OPACITY_MAX: u32 = 100;
pub const OPACITY_DEFAULT: u32 = 75;

/// How much of the screen the picture takes beside the controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Picture {
    /// The design's: a 16:9 area above the controls (portrait), between them (landscape).
    #[default]
    Standard,
    /// As big as the controls, made smaller, leave room for: the picture's whole width in portrait where it fits.
    Large,
    /// The whole screen, the controls over the picture.
    Full,
}

impl Picture {
    const ALL: [Picture; 3] = [Picture::Standard, Picture::Large, Picture::Full];

    /// The next size (the options' button goes round them).
    #[must_use]
    pub fn next(self) -> Self {
        Self::ALL[(self as usize + 1) % Self::ALL.len()]
    }

    /// The name the options show.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Large => "Large",
            Self::Full => "Full screen",
        }
    }
}

/// Which layout a place belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orient {
    Landscape,
    Portrait,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// A floating stick under the thumb instead of the fixed d-pad.
    pub stick: bool,
    pub opacity: u32,
    pub haptics: bool,
    pub auto_battle: bool,
    pub hide_pad: bool,
    /// The frame rate and the sound buffer at the top of the screen.
    pub show_fps: bool,
    pub picture: Picture,
    pub land: [Place; 7],
    pub port: [Place; 7],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            stick: false,
            opacity: OPACITY_DEFAULT,
            haptics: true,
            auto_battle: false,
            hide_pad: true,
            show_fps: false,
            picture: Picture::Standard,
            land: [Place::default(); 7],
            port: [Place::default(); 7],
        }
    }
}

fn flag(values: &Values, key: &str, default: bool) -> bool {
    values.get(key).map_or(default, |v| v != 0)
}

fn key(orient: Orient, group: Group, field: &str) -> String {
    let p = if orient == Orient::Portrait { "p." } else { "" };
    format!("controls.touch.{p}{}.{field}", group.id())
}

impl Settings {
    /// The choices stored in `values`.
    #[must_use]
    pub fn from_values(values: &Values) -> Self {
        let mut s = Self {
            stick: flag(values, keys::TOUCH_STYLE, false),
            opacity: values.get(keys::TOUCH_OPACITY).map_or(OPACITY_DEFAULT, |v| {
                (OPACITY_MIN + 5 * v.clamp(0, 16) as u32).min(OPACITY_MAX)
            }),
            haptics: flag(values, keys::TOUCH_HAPTICS, true),
            auto_battle: flag(values, keys::TOUCH_AUTO_BATTLE, false),
            hide_pad: flag(values, keys::TOUCH_HIDE_PAD, true),
            show_fps: flag(values, keys::TOUCH_SHOW_FPS, false),
            picture: values.get(keys::TOUCH_PICTURE).map_or(Picture::Standard, |v| {
                Picture::ALL[usize::try_from(v).unwrap_or(0).min(Picture::ALL.len() - 1)]
            }),
            ..Self::default()
        };
        for (orient, places) in [(Orient::Landscape, &mut s.land), (Orient::Portrait, &mut s.port)]
        {
            for g in Group::ALL {
                let get = |f: &str| values.get(&key(orient, g, f));
                places[g.index()] = Place {
                    x: get("x").map(|v| v.clamp(0, 1000) as i32),
                    y: get("y").map(|v| v.clamp(0, 1000) as i32),
                    scale: get("s").map(|v| v.clamp(SCALE_MIN.into(), SCALE_MAX.into()) as u32),
                };
            }
        }
        s
    }

    /// Writes the choices into `values`: the touch keys are replaced, a default is not written, and the
    /// other keys are left alone.
    pub fn store(&self, values: &mut Values) {
        values.remove_prefix(keys::TOUCH_PREFIX);
        let mut put = |k: &str, v: i64, default: i64| {
            if v != default {
                values.set(k, v);
            }
        };
        put(keys::TOUCH_STYLE, i64::from(self.stick), 0);
        put(
            keys::TOUCH_OPACITY,
            i64::from(self.opacity.saturating_sub(OPACITY_MIN) / 5),
            i64::from(keys::TOUCH_OPACITY_DEFAULT),
        );
        put(keys::TOUCH_HAPTICS, i64::from(self.haptics), 1);
        put(keys::TOUCH_AUTO_BATTLE, i64::from(self.auto_battle), 0);
        put(keys::TOUCH_HIDE_PAD, i64::from(self.hide_pad), 1);
        put(keys::TOUCH_SHOW_FPS, i64::from(self.show_fps), 0);
        put(keys::TOUCH_PICTURE, self.picture as i64, 0);
        for (orient, places) in [(Orient::Landscape, &self.land), (Orient::Portrait, &self.port)] {
            for g in Group::ALL {
                let p = places[g.index()];
                if let Some(x) = p.x {
                    values.set(&key(orient, g, "x"), i64::from(x));
                }
                if let Some(y) = p.y {
                    values.set(&key(orient, g, "y"), i64::from(y));
                }
                if let Some(s) = p.scale {
                    values.set(&key(orient, g, "s"), i64::from(s));
                }
            }
        }
    }

    #[must_use]
    pub fn place(&self, orient: Orient, group: Group) -> Place {
        match orient {
            Orient::Landscape => self.land[group.index()],
            Orient::Portrait => self.port[group.index()],
        }
    }

    pub fn place_mut(&mut self, orient: Orient, group: Group) -> &mut Place {
        match orient {
            Orient::Landscape => &mut self.land[group.index()],
            Orient::Portrait => &mut self.port[group.index()],
        }
    }

    /// Puts every group back where the design has it (the other choices stay).
    pub fn reset_layout(&mut self) {
        self.land = [Place::default(); 7];
        self.port = [Place::default(); 7];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_stored_is_the_design_and_a_default_is_not_written() {
        let s = Settings::from_values(&Values::new());
        assert_eq!(s, Settings::default());
        let mut v = Values::new();
        s.store(&mut v);
        assert!(v.is_empty());
    }

    #[test]
    fn choices_round_trip_and_wild_values_clamp() {
        let mut s = Settings {
            stick: true,
            opacity: 40,
            haptics: false,
            show_fps: true,
            picture: Picture::Full,
            ..Settings::default()
        };
        *s.place_mut(Orient::Landscape, Group::Face) =
            Place { x: Some(800), y: Some(500), scale: Some(150) };
        *s.place_mut(Orient::Portrait, Group::Dpad) = Place { x: None, y: None, scale: Some(120) };
        let mut v = Values::new();
        v.set("video.scale", 3);
        s.store(&mut v);
        assert_eq!(v.get("video.scale"), Some(3), "other keys stay");
        assert_eq!(Settings::from_values(&v), s);
        v.set(keys::TOUCH_OPACITY, -5);
        v.set("controls.touch.face.s", 900);
        let wild = Settings::from_values(&v);
        assert_eq!(wild.opacity, OPACITY_MIN);
        assert_eq!(wild.place(Orient::Landscape, Group::Face).scale, Some(SCALE_MAX));
    }

    #[test]
    fn reset_layout_forgets_places_and_keeps_the_other_choices() {
        let mut s = Settings { opacity: 50, ..Settings::default() };
        s.place_mut(Orient::Landscape, Group::L).x = Some(10);
        s.reset_layout();
        assert_eq!(s.land, [Place::default(); 7]);
        assert_eq!(s.opacity, 50);
        let mut v = Values::new();
        s.store(&mut v);
        assert_eq!(v.get(keys::TOUCH_OPACITY), Some(6));
        assert_eq!(v.get("controls.touch.l.x"), None);
    }
}
