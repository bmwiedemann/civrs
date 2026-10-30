//! Loading and decoding of the original game data files (PIC pictures, PAL palettes,
//! FONTS.CV bitmap fonts, the sectioned text files and the MAP.PIC Earth map).

pub mod earth;
pub mod fonts;
pub mod pal;
pub mod pic;
pub mod text;

pub use earth::EarthMap;
pub use fonts::{Font, FontSet, Glyph};
pub use pal::Palette;
pub use pic::{Depth, Pic};
pub use text::TextFile;

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub enum AssetError {
    NotFound(String),
    Io(String, std::io::Error),
    Format { file: String, msg: String },
}

impl fmt::Display for AssetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AssetError::NotFound(n) => write!(f, "asset not found: {n}"),
            AssetError::Io(n, e) => write!(f, "I/O error reading {n}: {e}"),
            AssetError::Format { file, msg } => write!(f, "bad format in {file}: {msg}"),
        }
    }
}

impl std::error::Error for AssetError {}

pub type Result<T> = std::result::Result<T, AssetError>;

/// A directory holding the original game files, indexed case-insensitively.
pub struct AssetDir {
    root: PathBuf,
    index: HashMap<String, PathBuf>,
}

impl AssetDir {
    pub fn open(root: &Path) -> Result<Self> {
        let rd = std::fs::read_dir(root)
            .map_err(|e| AssetError::Io(root.display().to_string(), e))?;
        let mut index = HashMap::new();
        for entry in rd.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                    index.insert(name.to_ascii_uppercase(), path.clone());
                }
            }
        }
        if !index.contains_key("CIV.EXE") && !index.contains_key("SP257.PIC") {
            return Err(AssetError::NotFound(format!(
                "{} does not look like a Civilization directory (no CIV.EXE / SP257.PIC)",
                root.display()
            )));
        }
        Ok(AssetDir { root: root.to_path_buf(), index })
    }

    /// Find the asset directory: an explicit path, `$CIV_DIR`, or the usual relative spots.
    pub fn locate(explicit: Option<&Path>) -> Result<Self> {
        if let Some(p) = explicit {
            return Self::open(p);
        }
        if let Ok(env) = std::env::var("CIV_DIR") {
            return Self::open(Path::new(&env));
        }
        let mut candidates: Vec<PathBuf> = vec![
            PathBuf::from("civ"),
            PathBuf::from("../civ"),
            PathBuf::from("."),
        ];
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("civ"));
                candidates.push(dir.join("../civ"));
                candidates.push(dir.join("../../civ"));
                candidates.push(dir.join("../../../civ"));
            }
        }
        candidates.push(PathBuf::from("/home/tester/git/reverse/civ"));
        for c in &candidates {
            if c.join("CIV.EXE").exists() || c.join("civ.exe").exists() {
                return Self::open(c);
            }
        }
        Err(AssetError::NotFound(
            "Civilization data directory (use --assets DIR or set CIV_DIR)".to_string(),
        ))
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn exists(&self, name: &str) -> bool {
        self.index.contains_key(&name.to_ascii_uppercase())
    }

    pub fn path_of(&self, name: &str) -> Option<&Path> {
        self.index.get(&name.to_ascii_uppercase()).map(|p| p.as_path())
    }

    pub fn names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.index.keys().cloned().collect();
        v.sort();
        v
    }

    pub fn read(&self, name: &str) -> Result<Vec<u8>> {
        let path = self
            .path_of(name)
            .ok_or_else(|| AssetError::NotFound(name.to_string()))?;
        std::fs::read(path).map_err(|e| AssetError::Io(name.to_string(), e))
    }

    fn with_ext(name: &str, ext: &str) -> String {
        if name.contains('.') {
            name.to_ascii_uppercase()
        } else {
            format!("{}.{}", name.to_ascii_uppercase(), ext)
        }
    }

    /// Load a picture; `name` may omit the `.PIC` extension.
    pub fn load_pic(&self, name: &str) -> Result<Pic> {
        let file = Self::with_ext(name, "PIC");
        let bytes = self.read(&file)?;
        pic::decode_pic(&bytes).map_err(|e| AssetError::Format { file, msg: e.to_string() })
    }

    /// Load a standalone palette; `name` may omit the `.PAL` extension.
    pub fn load_pal(&self, name: &str) -> Result<Palette> {
        let file = Self::with_ext(name, "PAL");
        let bytes = self.read(&file)?;
        let p = pic::decode_pic(&bytes).map_err(|e| AssetError::Format { file: file.clone(), msg: e.to_string() })?;
        p.palette.ok_or(AssetError::Format { file, msg: "no M0 palette block".into() })
    }

    pub fn load_fonts(&self) -> Result<FontSet> {
        let bytes = self.read("FONTS.CV")?;
        fonts::decode_fonts(&bytes).map_err(|msg| AssetError::Format { file: "FONTS.CV".into(), msg })
    }

    /// Load a sectioned text file (KING.TXT, BLURB0.TXT, ...); `name` may omit `.TXT`.
    pub fn load_text(&self, name: &str) -> Result<TextFile> {
        let file = Self::with_ext(name, "TXT");
        let bytes = self.read(&file)?;
        Ok(TextFile::parse(&bytes))
    }

    pub fn load_earth(&self) -> Result<EarthMap> {
        let pic = self.load_pic("MAP.PIC")?;
        earth::decode_earth(&pic).map_err(|msg| AssetError::Format { file: "MAP.PIC".into(), msg })
    }
}
