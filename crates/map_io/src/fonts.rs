//! The fonts installed on this computer: their families (with their
//! Chinese names where the font has one), which of them have Chinese
//! characters, and their files, read when a map asks for one.
//!
//! Listing reads only each file's table directory, its name table and its
//! character map (not the whole file), so the hundreds of fonts of a
//! system are listed in well under a second; the list is made once per run
//! (`warm` starts it in the background). A font chosen in a map is stored
//! by its family name; on a computer without it the text falls back to the
//! default font.

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// One face (a style of a family) in a font file.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFace {
    /// The family name in English (what a map stores).
    pub family: String,
    /// The family name to show: its Chinese name if it has one.
    pub display: String,
    /// The style ("Regular", "Bold", "Light Italic"...).
    pub style: String,
    pub path: PathBuf,
    /// The face's place in a collection (.ttc); 0 otherwise.
    pub index: u32,
    pub bold: bool,
    pub italic: bool,
    /// Has Chinese characters (at least 中, 国 and 山).
    pub cjk: bool,
}

/// A family, with its regular and bold faces.
#[derive(Clone, Debug, PartialEq)]
pub struct FontFamily {
    pub family: String,
    pub display: String,
    pub cjk: bool,
    pub regular: FontFace,
    pub bold: Option<FontFace>,
}

/// The font that comes with Atlas Forge: Noto Sans SC Regular (SIL Open
/// Font License 1.1, the licence beside it). The default font of the
/// exports and the interface, so a picture looks the same on any computer.
pub const BUNDLED: &str = "NotoSansSC-Regular.ttf";

/// Folders the bundled font may be in: `fonts` beside the program (as
/// packaged), else `assets/fonts` in any folder above the program (a build
/// in the source tree, and the tests: newer cargo puts test programs as
/// deep as `target/release/build/<crate>/<hash>/out`).
pub fn bundled_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
        v.push(dir.join("fonts"));
        v.extend(dir.ancestors().map(|a| a.join("assets").join("fonts")));
    }
    v
}

/// The bundled font's file and contents, when it is there.
pub fn bundled() -> Option<(PathBuf, Vec<u8>)> {
    bundled_dirs().into_iter().map(|d| d.join(BUNDLED)).find_map(|p| std::fs::read(&p).ok().map(|b| (p, b)))
}

/// Folders fonts are installed in on this system (the bundled font's
/// folder first, so it can be chosen by name like any other).
pub fn font_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = bundled_dirs().into_iter().filter(|d| d.join(BUNDLED).is_file()).take(1).collect();
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    if cfg!(windows) {
        v.push(env("WINDIR").unwrap_or_else(|| PathBuf::from(r"C:\Windows")).join("Fonts"));
        if let Some(l) = env("LOCALAPPDATA") {
            v.push(l.join("Microsoft").join("Windows").join("Fonts"));
        }
    } else if cfg!(target_os = "macos") {
        v.extend(["/System/Library/Fonts", "/Library/Fonts", "/System/Library/Fonts/Supplemental"].map(PathBuf::from));
        if let Some(h) = env("HOME") {
            v.push(h.join("Library").join("Fonts"));
        }
    } else {
        v.extend(["/usr/share/fonts", "/usr/local/share/fonts"].map(PathBuf::from));
        if let Some(h) = env("HOME") {
            v.push(h.join(".local").join("share").join("fonts"));
            v.push(h.join(".fonts"));
        }
    }
    v
}

fn read_at(f: &mut File, offset: u64, len: usize) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(offset)).ok()?;
    let mut buf = vec![0u8; len];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|x| u32::from_be_bytes([x[0], x[1], x[2], x[3]]))
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|x| u16::from_be_bytes([x[0], x[1]]))
}

/// The name and character-map tables of the face starting at `offset`.
fn face_tables(f: &mut File, offset: u64) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
    let head = read_at(f, offset, 12)?;
    let n = be16(&head, 4)? as usize;
    if n == 0 || n > 512 {
        return None;
    }
    let dir = read_at(f, offset + 12, n * 16)?;
    let (mut name, mut cmap) = (None, None);
    for i in 0..n {
        let r = &dir[i * 16..i * 16 + 16];
        let (off, len) = (be32(r, 8)? as u64, be32(r, 12)? as usize);
        if len > 64 << 20 {
            continue;
        }
        match &r[0..4] {
            b"name" => name = read_at(f, off, len),
            b"cmap" => cmap = read_at(f, off, len),
            _ => {}
        }
    }
    Some((name?, cmap))
}

/// The best name of one kind: Windows English, then any English, then the
/// first that reads.
fn pick_name(t: &ttf_parser::name::Table, ids: &[u16], lang: &[u16]) -> Option<String> {
    for id in ids {
        for want in lang {
            if let Some(s) = t.names.into_iter().find(|n| n.name_id == *id && n.language_id == *want && n.is_unicode()).and_then(|n| n.to_string()) {
                return Some(s);
            }
        }
    }
    None
}

fn parse_face(path: &Path, index: u32, name: &[u8], cmap: Option<&[u8]>) -> Option<FontFace> {
    use ttf_parser::name_id::{FAMILY, SUBFAMILY, TYPOGRAPHIC_FAMILY, TYPOGRAPHIC_SUBFAMILY};
    let t = ttf_parser::name::Table::parse(name)?;
    let english = [0x0409, 0x0809, 0];
    let family = pick_name(&t, &[TYPOGRAPHIC_FAMILY, FAMILY], &english).or_else(|| t.names.into_iter().find(|n| n.name_id == FAMILY).and_then(|n| n.to_string()))?;
    let style = pick_name(&t, &[TYPOGRAPHIC_SUBFAMILY, SUBFAMILY], &english).unwrap_or_else(|| "Regular".into());
    // Simplified, then Traditional Chinese (mainland, Taiwan, Hong Kong)
    let display = pick_name(&t, &[TYPOGRAPHIC_FAMILY, FAMILY], &[0x0804, 0x1004, 0x0404, 0x0C04]).unwrap_or_else(|| family.clone());
    let cjk = cmap.and_then(ttf_parser::cmap::Table::parse).is_some_and(|c| ['中', '国', '山'].iter().all(|ch| c.subtables.into_iter().any(|s| s.glyph_index(*ch as u32).is_some())));
    let low = style.to_lowercase();
    let bold = low.contains("bold") && !low.contains("semi") && !low.contains("demi");
    let italic = low.contains("italic") || low.contains("oblique");
    Some(FontFace { family, display, style, path: path.to_path_buf(), index, bold, italic, cjk })
}

/// The faces in one font file.
pub fn faces_in(path: &Path) -> Vec<FontFace> {
    let Ok(mut f) = File::open(path) else { return vec![] };
    let Some(head) = read_at(&mut f, 0, 12) else { return vec![] };
    let offsets: Vec<(u32, u64)> = if &head[0..4] == b"ttcf" {
        let n = be32(&head, 8).unwrap_or(0).min(64) as usize;
        let Some(table) = read_at(&mut f, 12, n * 4) else { return vec![] };
        (0..n).filter_map(|i| be32(&table, i * 4).map(|o| (i as u32, o as u64))).collect()
    } else {
        vec![(0, 0)]
    };
    offsets
        .into_iter()
        .filter_map(|(i, off)| {
            let (name, cmap) = face_tables(&mut f, off)?;
            parse_face(path, i, &name, cmap.as_deref())
        })
        .collect()
}

fn scan() -> Vec<FontFamily> {
    let mut files = Vec::new();
    let mut stack = font_dirs();
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "ttf" | "otf" | "ttc" | "otc")) {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut by_family: HashMap<String, Vec<FontFace>> = HashMap::new();
    for p in files {
        for face in faces_in(&p) {
            by_family.entry(face.family.clone()).or_default().push(face);
        }
    }
    let mut out: Vec<FontFamily> = by_family
        .into_values()
        .filter_map(|faces| {
            let plain = |f: &&FontFace| !f.bold && !f.italic;
            let normal = |f: &&FontFace| matches!(f.style.as_str(), "Regular" | "Normal" | "Book" | "Roman" | "Medium");
            let regular = faces.iter().filter(plain).find(normal).or_else(|| faces.iter().find(plain)).or_else(|| faces.first())?.clone();
            let bold = faces.iter().find(|f| f.bold && !f.italic).cloned();
            Some(FontFamily { family: regular.family.clone(), display: regular.display.clone(), cjk: faces.iter().any(|f| f.cjk), regular, bold })
        })
        .collect();
    // Chinese fonts first, then by name
    out.sort_by(|a, b| b.cjk.cmp(&a.cjk).then_with(|| a.display.to_lowercase().cmp(&b.display.to_lowercase())));
    out
}

static FAMILIES: OnceLock<Vec<FontFamily>> = OnceLock::new();

/// Every font family installed (listed on first call, then kept).
pub fn families() -> &'static [FontFamily] {
    FAMILIES.get_or_init(scan)
}

/// The families, if they have been listed already (`warm` lists them in
/// the background, so an interface does not wait for it).
pub fn families_if_ready() -> Option<&'static [FontFamily]> {
    FAMILIES.get().map(|v| v.as_slice())
}

/// Start listing the fonts in the background.
pub fn warm() {
    if FAMILIES.get().is_none() {
        std::thread::spawn(|| {
            families();
        });
    }
}

/// A family by its stored name.
pub fn family(name: &str) -> Option<&'static FontFamily> {
    families().iter().find(|f| f.family == name)
}

type Loaded = Option<(Arc<Vec<u8>>, u32)>;

/// The bytes of a family's face (bold if asked and it has one) and its
/// index in the file; read once, then kept.
pub fn load(name: &str, bold: bool) -> Loaded {
    static CACHE: OnceLock<Mutex<HashMap<(String, bool), Loaded>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(v) = cache.lock().unwrap().get(&(name.to_string(), bold)) {
        return v.clone();
    }
    let got = family(name).and_then(|f| {
        let face = if bold { f.bold.as_ref().unwrap_or(&f.regular) } else { &f.regular };
        std::fs::read(&face.path).ok().map(|b| (Arc::new(b), face.index))
    });
    cache.lock().unwrap().insert((name.to_string(), bold), got.clone());
    got
}

/// The name a font is shown under (its Chinese name, if it has one).
pub fn display_name(name: &str) -> String {
    families_if_ready().and_then(|v| v.iter().find(|f| f.family == name)).map_or_else(|| name.to_string(), |f| f.display.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal font file: a table directory with a name table holding
    /// an English and a Chinese family name and a style.
    fn tiny_font() -> Vec<u8> {
        // name table, format 0, three records (Windows, Unicode BMP)
        let names: [(u16, u16, &str); 3] = [(0x0409, 1, "Test Serif"), (0x0804, 1, "测试宋体"), (0x0409, 2, "Bold")];
        let mut strings: Vec<u8> = Vec::new();
        let mut records: Vec<u8> = Vec::new();
        for (lang, id, s) in names {
            let utf16: Vec<u8> = s.encode_utf16().flat_map(|c| c.to_be_bytes()).collect();
            for v in [3u16, 1, lang, id, utf16.len() as u16, strings.len() as u16] {
                records.extend(v.to_be_bytes());
            }
            strings.extend(utf16);
        }
        let mut name = Vec::new();
        for v in [0u16, names.len() as u16, (6 + records.len()) as u16] {
            name.extend(v.to_be_bytes());
        }
        name.extend(records);
        name.extend(strings);
        // the file: offset table with one table record
        let mut f = Vec::new();
        f.extend(0x0001_0000u32.to_be_bytes());
        for v in [1u16, 16, 0, 0] {
            f.extend(v.to_be_bytes());
        }
        f.extend(b"name");
        f.extend(0u32.to_be_bytes());
        f.extend(28u32.to_be_bytes());
        f.extend((name.len() as u32).to_be_bytes());
        f.extend(name);
        f
    }

    #[test]
    fn names_are_read_from_the_tables_alone() {
        let dir = std::env::temp_dir().join(format!("atlas_fonts_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("tiny.ttf");
        std::fs::write(&p, tiny_font()).unwrap();
        let faces = faces_in(&p);
        assert_eq!(faces.len(), 1);
        let f = &faces[0];
        assert_eq!((f.family.as_str(), f.display.as_str(), f.style.as_str(), f.bold, f.italic, f.cjk), ("Test Serif", "测试宋体", "Bold", true, false, false));
        // not a font: nothing
        std::fs::write(dir.join("not.ttf"), b"hello").unwrap();
        assert!(faces_in(&dir.join("not.ttf")).is_empty());
        assert!(faces_in(&dir.join("missing.ttf")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// On a system with fonts, the list has families with Chinese
    /// characters and every face can be read (Windows has 微软雅黑).
    #[test]
    fn the_system_fonts_are_listed() {
        let all = families();
        if cfg!(windows) {
            assert!(all.iter().any(|f| f.cjk), "a Chinese font on Windows");
            let yahei = all.iter().find(|f| f.family == "Microsoft YaHei").expect("Microsoft YaHei");
            assert!(yahei.cjk && yahei.display == "微软雅黑", "{yahei:?}");
            let (bytes, index) = load("Microsoft YaHei", false).expect("its file");
            assert!(ab_glyph::FontRef::try_from_slice_and_index(&bytes, index).is_ok());
            assert!(load("Microsoft YaHei", true).is_some(), "bold, or the regular face");
        }
        // Chinese fonts are listed first
        let first_latin = all.iter().position(|f| !f.cjk).unwrap_or(all.len());
        assert!(all[first_latin..].iter().all(|f| !f.cjk));
        assert!(load("No Such Font Anywhere", false).is_none());
        assert_eq!(display_name("No Such Font Anywhere"), "No Such Font Anywhere");
    }
}
