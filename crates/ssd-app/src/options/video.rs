//! The Video page's choices as the window uses them (`video.*` of the PC file): what a windowed Enhanced run
//! starts with, and what a change of a row does to the running window.

use crate::filter::{Filters, Shape};
use ssd_hd::{HdOptions, Look};
use ssd_menus::options::{Values, video};

/// The video choices of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoSettings {
    pub fullscreen: bool,
    /// Pixels of the picture per pixel of the PPU's frame, 2 to 8.
    pub scale: u32,
    pub look: Look,
    /// The 4:3 picture (the original window) instead of the 16:9 view.
    pub narrow: bool,
    pub vsync: bool,
    /// Latency Smooth (the default): the picture is drawn in the background while the next frame runs and shown one frame
    /// later. Low draws and shows each frame in turn.
    pub smooth: bool,
    pub filters: Filters,
}

impl Default for VideoSettings {
    /// What the game did before the Video page: a window, 4x, crisp, the 16:9 view, no V-sync, no filters.
    fn default() -> Self {
        Self::from_values(&Values::new())
    }
}

impl VideoSettings {
    /// The choices of the PC file's values (a missing or wild value is the row's default or its last choice).
    #[must_use]
    pub fn from_values(values: &Values) -> Self {
        let level = |key: &str| u8::try_from(video::index(values, key)).unwrap_or(0);
        Self {
            fullscreen: video::index(values, "video.display") == 1,
            scale: 2 + u32::try_from(video::index(values, "video.scale")).unwrap_or(2),
            look: if video::index(values, "video.look") == 1 { Look::Smooth } else { Look::Crisp },
            narrow: video::index(values, "video.aspect") == 1,
            vsync: video::index(values, "video.vsync") == 1,
            smooth: video::index(values, "video.latency") == 1,
            filters: Filters { scanlines: level("video.scanlines"), crt: level("video.crt") },
        }
    }

    /// How the Enhanced picture is made.
    #[must_use]
    pub const fn hd(&self) -> HdOptions {
        HdOptions { scale: self.scale, look: self.look }
    }

    /// What the final pass does to the picture.
    #[must_use]
    pub const fn shape(&self) -> Shape {
        Shape { crop: self.narrow, filters: self.filters }
    }

    /// The choices the player changed from `old` to `new` take effect here; the others stay as they are (a
    /// command-line flag keeps winning for the choice it gave, until the player changes that very choice).
    pub fn follow(&mut self, old: &Self, new: &Self) {
        if old.fullscreen != new.fullscreen {
            self.fullscreen = new.fullscreen;
        }
        if old.scale != new.scale {
            self.scale = new.scale;
        }
        if old.look != new.look {
            self.look = new.look;
        }
        if old.narrow != new.narrow {
            self.narrow = new.narrow;
        }
        if old.vsync != new.vsync {
            self.vsync = new.vsync;
        }
        if old.smooth != new.smooth {
            self.smooth = new.smooth;
        }
        if old.filters.scanlines != new.filters.scanlines {
            self.filters.scanlines = new.filters.scanlines;
        }
        if old.filters.crt != new.filters.crt {
            self.filters.crt = new.filters.crt;
        }
    }
}

/// What a windowed run starts with: the choices it shows, and the ones the file holds (the reference the
/// player's later changes are measured against).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Start {
    pub shown: VideoSettings,
    pub saved: VideoSettings,
}

/// The start of a windowed run: the file's choices where the command line gave none (`scale` and `look` are the
/// flags, which win for the run).
#[must_use]
pub fn start(saved: &Values, scale: Option<u32>, look: Option<Look>) -> Start {
    let saved = VideoSettings::from_values(saved);
    let mut shown = saved;
    shown.scale = scale.unwrap_or(saved.scale);
    shown.look = look.unwrap_or(saved.look);
    Start { shown, saved }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_are_what_the_game_did_before_the_page() {
        let defaults = VideoSettings::default();
        assert_eq!(defaults.hd(), HdOptions { scale: 4, look: Look::Crisp });
        assert!(!defaults.fullscreen && !defaults.narrow && !defaults.vsync);
        assert!(defaults.smooth, "the picture is drawn in the background unless Latency is Low");
        assert!(defaults.shape().is_identity(), "no crop and no filter: the picture is untouched");
    }

    #[test]
    fn the_values_of_the_file_become_the_choices() {
        let mut values = Values::new();
        for (key, value) in [
            ("video.display", 1),
            ("video.scale", 6),
            ("video.look", 1),
            ("video.aspect", 1),
            ("video.vsync", 1),
            ("video.latency", 0),
            ("video.scanlines", 2),
            ("video.crt", 99),
        ] {
            values.set(key, value);
        }
        let settings = VideoSettings::from_values(&values);
        assert!(settings.fullscreen && settings.narrow && settings.vsync && !settings.smooth);
        assert_eq!(settings.hd(), HdOptions { scale: 8, look: Look::Smooth });
        assert_eq!(settings.filters, Filters { scanlines: 2, crt: 2 }, "a wild value clamps");
        assert_eq!(settings.shape(), Shape { crop: true, filters: settings.filters });
    }

    #[test]
    fn a_flag_wins_until_the_player_changes_that_very_choice() {
        let mut saved = Values::new();
        saved.set("video.scale", 5);
        let begun = start(&saved, Some(3), None);
        assert_eq!(begun.shown.scale, 3, "the flag wins for the run");
        assert_eq!(begun.saved.scale, 7);
        let mut shown = begun.shown;
        // The player turns scanlines on: the scale of the flag stays.
        let mut next = saved.clone();
        next.set("video.scanlines", 1);
        let changed = VideoSettings::from_values(&next);
        shown.follow(&begun.saved, &changed);
        assert_eq!((shown.scale, shown.filters.scanlines), (3, 1));
        // The player changes the scale itself: now the page's choice shows.
        next.set("video.scale", 4);
        let changed_again = VideoSettings::from_values(&next);
        shown.follow(&changed, &changed_again);
        assert_eq!(shown.scale, 6, "6x is the page's choice now");
    }
}
