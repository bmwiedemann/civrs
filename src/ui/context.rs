//! Shared state handed to every screen: assets, fonts, sprite sheets, texts, the active
//! palette, the mouse position and the picture cache.

use crate::assets::text::{parse_credits, parse_story};
use crate::assets::{AssetDir, FontSet, Palette, TextFile};
use crate::audio::Audio;
use crate::gfx::Surface;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A decoded picture ready to blit, with its own palette.
pub struct Picture {
    pub surf: Surface,
    pub pal: Palette,
}

pub struct Texts {
    pub blurb: [TextFile; 5],
    pub king: TextFile,
    pub produce: TextFile,
    pub help: TextFile,
    pub error: TextFile,
    pub story: Vec<Vec<String>>,
    pub credits: Vec<(String, String)>,
}

impl Texts {
    pub fn load(a: &AssetDir) -> Result<Texts, crate::assets::AssetError> {
        let load = |n: &str| a.load_text(n).unwrap_or_default();
        Ok(Texts {
            blurb: [load("BLURB0"), load("BLURB1"), load("BLURB2"), load("BLURB3"), load("BLURB4")],
            king: load("KING"),
            produce: load("PRODUCE"),
            help: load("HELP"),
            error: load("ERROR"),
            story: a.read("STORY.TXT").map(|b| parse_story(&b)).unwrap_or_default(),
            credits: a.read("CREDITS.TXT").map(|b| parse_credits(&b)).unwrap_or_default(),
        })
    }
}

pub struct UiContext {
    pub assets: AssetDir,
    pub fonts: FontSet,
    /// SP257.PIC: units, overlays, icons.
    pub sp: Rc<Picture>,
    /// TER257.PIC: terrain tiles.
    pub ter: Rc<Picture>,
    pub texts: Texts,
    pub audio: Audio,
    /// Palette currently presented (screens may fade it).
    pub palette: Palette,
    pub frame: u64,
    pub mouse: (i32, i32),
    pub mouse_down: bool,
    pic_cache: RefCell<HashMap<String, Rc<Picture>>>,
    /// Game state, once a game is running.
    pub game: Option<crate::game::Game>,
    pub ai: crate::ai::Ai,
    pub hof: crate::save::HallOfFame,
    pub save_dir: std::path::PathBuf,
    pub skip_intro: bool,
    /// Skip timed transitions such as the pixel dissolve (headless previews).
    pub instant: bool,
    pub quit_requested: bool,
    /// Map tile the map view should centre on (set by FindCity).
    pub center_request: Option<(i32, i32)>,
}

impl UiContext {
    pub fn new(assets: AssetDir, save_dir: std::path::PathBuf) -> Result<UiContext, crate::assets::AssetError> {
        let fonts = assets.load_fonts()?;
        let texts = Texts::load(&assets)?;
        let mut cache: HashMap<String, Rc<Picture>> = HashMap::new();
        let sp = Self::load_into(&assets, &mut cache, "SP257")?;
        let ter = Self::load_into(&assets, &mut cache, "TER257")?;
        let palette = sp.pal;
        let hof = crate::save::HallOfFame::load(&save_dir);
        Ok(UiContext {
            assets,
            fonts,
            sp,
            ter,
            texts,
            audio: Audio::new(),
            palette,
            frame: 0,
            mouse: (160, 100),
            mouse_down: false,
            pic_cache: RefCell::new(cache),
            game: None,
            ai: crate::ai::Ai::new(1),
            hof,
            save_dir,
            skip_intro: false,
            instant: false,
            quit_requested: false,
            center_request: None,
        })
    }

    fn load_into(assets: &AssetDir, cache: &mut HashMap<String, Rc<Picture>>, name: &str) -> Result<Rc<Picture>, crate::assets::AssetError> {
        let key = name.to_ascii_uppercase();
        if let Some(p) = cache.get(&key) {
            return Ok(p.clone());
        }
        let pic = assets.load_pic(name)?;
        let pal = match pic.palette {
            Some(p) => p,
            None => Palette::ega(),
        };
        let p = Rc::new(Picture { surf: Surface::from_pic(&pic), pal });
        cache.insert(key, p.clone());
        Ok(p)
    }

    /// Load (and cache) a picture by base name, e.g. "KING03".
    pub fn pic(&self, name: &str) -> Rc<Picture> {
        let mut cache = self.pic_cache.borrow_mut();
        match Self::load_into(&self.assets, &mut cache, name) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("warning: {e}");
                Rc::new(Picture { surf: Surface::new(320, 200), pal: Palette::black() })
            }
        }
    }

    /// Load a standalone .PAL by base name (falls back to the picture's own palette).
    pub fn pal(&self, name: &str) -> Palette {
        match self.assets.load_pal(name) {
            Ok(p) => p,
            Err(_) => self.pic(name).pal,
        }
    }

    /// Palette of a picture (from its .PAL file if present, else its embedded palette).
    pub fn pic_pal(&self, name: &str) -> Palette {
        let key = format!("PAL:{}", name.to_ascii_uppercase());
        if let Some(p) = self.pic_cache.borrow().get(&key) {
            return p.pal;
        }
        let pal = self.pal(name);
        self.pic_cache.borrow_mut().insert(key, Rc::new(Picture { surf: Surface::new(0, 0), pal }));
        pal
    }

    pub fn game(&self) -> &crate::game::Game {
        self.game.as_ref().expect("no game running")
    }

    pub fn game_mut(&mut self) -> &mut crate::game::Game {
        self.game.as_mut().expect("no game running")
    }
}
