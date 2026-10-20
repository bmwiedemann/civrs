//! Sound: the original AdLib driver (`ASOUND.CVL`) re-implemented and played through an OPL2
//! emulator. `Audio` is the game-facing handle; the platform backend pulls PCM from the
//! shared [`Engine`] on its audio thread.
//!
//! Sound IDs are those of the driver's `PlayTune` (see `driver.rs`); the mapping of game
//! events to IDs follows CIV.EXE's call sites.

pub mod cvl;
pub mod driver;
pub mod engine;

use crate::assets::AssetDir;
pub use engine::Engine;
use std::sync::{Arc, Mutex};

/// The driver's sound IDs the game uses directly. National themes are
/// [`Audio::national_theme`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Sound {
    /// Stop everything and reset the chip.
    Stop = 0,
    /// Fade everything out over about a second.
    FadeOut = 1,
    /// Title intro and spaceship arrival.
    MainTheme = 3,
    /// World creation.
    Evolution = 4,
    /// Threats, cancelled treaties, war declared.
    WarDrums = 33,
    /// Game won; also a city entering "We Love the King Day".
    Win = 34,
    /// Game lost; also "We Love the King Day" ending.
    Lose = 35,
    /// Civil disorder, famine, Manhattan Project, meltdown, the Barbarians' theme.
    Disorder = 36,
    /// Zone-of-control violation and other refused orders.
    IllegalMove = 37,
    BattleWon = 38,
    BattleLost = 39,
    ModernBattleWon = 40,
    ModernBattleLost = 41,
    Nuke = 42,
    Bomber = 43,
    /// A city completed what it was building.
    BuildComplete = 44,
}

/// Long (5-18) and short (19-32) national theme IDs per civ index (rules order: 0 Barbarians,
/// 1 Romans ... 15 Mongols; 8 unused). The driver's order is Americans, Aztecs, Egyptians,
/// Zulus, French, Romans, Russians, Greeks, English, Babylonians, Chinese, Mongols, Indians,
/// Germans.
const THEME_OF_CIV: [u8; 16] = [36, 10, 14, 18, 7, 5, 12, 17, 36, 11, 8, 9, 6, 15, 13, 16];

/// ID of a civ's national theme; `long` = the full piece (dialogs), else the jingle.
pub fn theme_id(civ: u8, long: bool) -> u8 {
    let id = THEME_OF_CIV[(civ & 15) as usize];
    if id == 36 || long {
        id
    } else {
        id + 14
    }
}

/// Battle sounds use the "modern" explosion variants when either unit is strong.
pub fn battle_sound(won: bool, modern: bool) -> Sound {
    match (won, modern) {
        (true, false) => Sound::BattleWon,
        (false, false) => Sound::BattleLost,
        (true, true) => Sound::ModernBattleWon,
        (false, true) => Sound::ModernBattleLost,
    }
}

/// Game-facing sound handle. Without a driver image (or when the backend has no audio
/// device) every call is a cheap no-op.
pub struct Audio {
    engine: Option<Arc<Mutex<Engine>>>,
}

impl Default for Audio {
    fn default() -> Self {
        Audio::silent()
    }
}

/// Sample rate the engine renders at; the backend converts if its device differs.
pub const SAMPLE_RATE: u32 = 48_000;

impl Audio {
    pub fn silent() -> Audio {
        Audio { engine: None }
    }

    /// Load `ASOUND.CVL` from the asset directory; silent (with a message) when missing or
    /// of an unsupported version.
    pub fn load(assets: &AssetDir) -> Audio {
        let bytes = match assets.read("ASOUND.CVL") {
            Ok(b) => b,
            Err(e) => {
                eprintln!("sound: {e}; running silent");
                return Audio::silent();
            }
        };
        match cvl::Image::parse(&bytes) {
            Ok(im) => Audio::from_image(im),
            Err(e) => {
                eprintln!("sound: ASOUND.CVL: {e}; running silent");
                Audio::silent()
            }
        }
    }

    pub fn from_image(im: cvl::Image) -> Audio {
        let engine = Engine::new(driver::Driver::new(im), SAMPLE_RATE);
        Audio { engine: Some(Arc::new(Mutex::new(engine))) }
    }

    /// The shared engine for the platform backend's audio callback.
    pub fn engine(&self) -> Option<Arc<Mutex<Engine>>> {
        self.engine.clone()
    }

    pub fn available(&self) -> bool {
        self.engine.is_some()
    }

    fn with<R>(&self, f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
        let e = self.engine.as_ref()?;
        let mut g = e.lock().unwrap_or_else(|p| p.into_inner());
        Some(f(&mut g))
    }

    /// `PlayTune(id, level)`.
    pub fn play_id(&self, id: u8, level: u8) {
        self.with(|e| {
            e.play(id, level);
        });
    }

    pub fn play(&self, s: Sound) {
        self.play_id(s as u8, 0);
    }

    /// A civ's national theme: the full arrangement (`long`) or the short jingle.
    pub fn national_theme(&self, civ: u8, long: bool) {
        self.play_id(theme_id(civ, long), 3);
    }

    pub fn stop(&self) {
        self.play(Sound::Stop);
    }

    pub fn fade_out(&self) {
        self.play(Sound::FadeOut);
    }

    /// `PlayTune(2)`: is anything still playing?
    pub fn is_playing(&self) -> bool {
        self.with(|e| e.is_active()).unwrap_or(false)
    }

    /// `SoundTimer`: 60 Hz ticks since the last `play`.
    pub fn timer(&self) -> u16 {
        self.with(|e| e.driver.timer()).unwrap_or(0)
    }

    /// The in-game Sound option. Turning it off stops the current sound.
    pub fn set_enabled(&self, on: bool) {
        self.with(|e| {
            if e.enabled && !on {
                e.play(0, 0);
            }
            e.enabled = on;
        });
    }

    pub fn enabled(&self) -> bool {
        self.with(|e| e.enabled).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes() {
        assert_eq!(theme_id(5, true), 5); // Americans
        assert_eq!(theme_id(5, false), 19);
        assert_eq!(theme_id(1, true), 10); // Romans
        assert_eq!(theme_id(15, false), 30); // Mongols
        assert_eq!(theme_id(0, false), 36); // Barbarians
        assert_eq!(theme_id(3, true), 18); // Germans
    }
}
