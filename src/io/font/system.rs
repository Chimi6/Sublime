//! Fonts installed on this machine: where they live, and which one covers
//! a character. Well-known broad fonts are tried first (Noto Sans, DejaVu
//! Sans, Liberation Sans, Arial, Segoe UI, Droid Sans), then a sans serif
//! per script, symbol fonts, the large CJK fallbacks, and every other
//! font file, each read only as far as its character map. Only fonts
//! with TrueType outlines are chosen, since only those are subset.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::io::font::{Coverage, Font};

/// Broad text fonts (lowercase file names without extension), tried
/// first: Latin, Greek, and Cyrillic in a sans serif.
const BROAD: &[&str] = &[
    "notosans-regular",
    "dejavusans",
    "liberationsans-regular",
    "arial",
    "segoeui",
    "droidsans",
];

/// Symbol and math fonts: tried after the fonts for a script, since
/// they carry scattered letters of many scripts.
const SYMBOLS: &[&str] = &[
    "notosanssymbols-regular",
    "notosanssymbols2-regular",
    "seguisym",
    "notosansmath-regular",
    "symbola",
];

/// Fonts that cover whole large scripts (CJK), tried last of the known.
const FALLBACKS: &[&str] = &[
    "arialuni",
    "arial unicode",
    "droidsansfallbackfull",
    "droidsansfallback",
];

/// The order a file is tried in: broad fonts, then one sans serif per
/// script (Noto Sans Arabic, Hebrew, ...), then symbols, then the large
/// fallbacks, then every other font.
fn rank(path: &Path) -> (usize, usize) {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if let Some(index) = BROAD.iter().position(|name| *name == stem) {
        return (0, index);
    }
    if let Some(index) = SYMBOLS.iter().position(|name| *name == stem) {
        return (2, index);
    }
    if let Some(index) = FALLBACKS.iter().position(|name| *name == stem) {
        return (3, index);
    }
    let script_sans = (stem.starts_with("notosans")
        || stem.starts_with("notonaskh")
        || stem.starts_with("droidsans"))
        && (stem.ends_with("-regular") || !stem.contains('-'));
    if script_sans {
        return (1, 0);
    }
    (4, 0)
}

/// The directories fonts are installed in on this system.
pub fn font_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        directories.push(PathBuf::from("/System/Library/Fonts"));
        directories.push(PathBuf::from("/Library/Fonts"));
        if let Some(home) = &home {
            directories.push(home.join("Library/Fonts"));
        }
    } else if cfg!(target_os = "windows") {
        let windows =
            std::env::var_os("WINDIR").map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
        directories.push(windows.join("Fonts"));
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            directories.push(PathBuf::from(local).join("Microsoft\\Windows\\Fonts"));
        }
    } else {
        directories.push(PathBuf::from("/usr/share/fonts"));
        directories.push(PathBuf::from("/usr/local/share/fonts"));
        if let Some(home) = &home {
            directories.push(home.join(".local/share/fonts"));
            directories.push(home.join(".fonts"));
        }
    }
    directories
}

/// Font files under `directory`, a few levels deep.
fn font_files(directory: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .collect();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            if depth > 0 {
                font_files(&path, depth - 1, out);
            }
            continue;
        }
        let extension = path
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        if matches!(extension.as_deref(), Some("ttf" | "otf" | "ttc")) {
            out.push(path);
        }
    }
}

/// Reads the character map of a font file's first face.
fn probe(path: &Path) -> Option<Coverage> {
    let mut file = File::open(path).ok()?;
    let mut head = vec![0u8; 64 * 1024];
    let read = file.read(&mut head).ok()?;
    head.truncate(read);
    Coverage::probe(&head, 0, &mut |offset, length| {
        if length > 16 * 1024 * 1024 {
            return None;
        }
        let mut table = vec![0u8; length];
        file.seek(SeekFrom::Start(offset as u64)).ok()?;
        file.read_exact(&mut table).ok()?;
        Some(table)
    })
    .ok()
}

/// The machine's fonts, searched lazily for characters the page needs.
#[derive(Default)]
pub struct SystemFonts {
    /// Every font file, preferred ones first; filled on first use.
    files: Option<Vec<PathBuf>>,
    /// Probed so far: each file's coverage (`None` when it cannot be used).
    probed: Vec<Option<Coverage>>,
    /// Fonts loaded for embedding, by file.
    loaded: HashMap<PathBuf, Arc<Font>>,
}

impl SystemFonts {
    fn files(&mut self) -> &[PathBuf] {
        self.files.get_or_insert_with(|| {
            let mut all = Vec::new();
            for directory in font_directories() {
                font_files(&directory, 4, &mut all);
            }
            all.sort_by_key(|path| rank(path));
            all
        })
    }

    /// A loaded font that covers `char`, the first found in preference
    /// order; `None` when no installed font with TrueType outlines does.
    pub fn font_for(&mut self, char: char) -> Option<(PathBuf, Arc<Font>)> {
        let count = self.files().len();
        for index in 0..count {
            if index == self.probed.len() {
                let path = self.files()[index].clone();
                let coverage = probe(&path).filter(|coverage| coverage.glyf);
                self.probed.push(coverage);
            }
            let covers = self.probed[index]
                .as_ref()
                .is_some_and(|coverage| coverage.covers(char));
            if !covers {
                continue;
            }
            let path = self.files()[index].clone();
            if let Some(font) = self.loaded.get(&path) {
                return Some((path, font.clone()));
            }
            let data = std::fs::read(&path).ok()?;
            let font = Arc::new(Font::parse(Arc::new(data), 0).ok()?);
            self.loaded.insert(path.clone(), font.clone());
            return Some((path, font));
        }
        None
    }
}
