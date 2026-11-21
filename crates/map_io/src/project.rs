//! Native project format (`.atlas`):
//!
//! ```text
//! "ATLASMAP" (8 bytes) | format version (u32 LE) | LZ4(size-prefixed) MessagePack(Document)
//! ```
//! Field names are stored (`to_vec_named`) so new optional fields with serde
//! defaults stay backward compatible. Saves are atomic (temp file + rename).

use map_core::doc::{Document, FORMAT_VERSION};
use map_core::topo::ensure_rev_above;
use map_core::world_book::{WorldBook, ROOT};
use std::path::Path;
use std::sync::Arc;

const MAGIC: &[u8; 8] = b"ATLASMAP";
pub const EXTENSION: &str = "atlas";

pub fn to_bytes(doc: &Document) -> Result<Vec<u8>, String> {
    // Older readers must reject features absent from their format, rather
    // than silently stripping the star chart or climate on their next save.
    let mut stored = doc.clone();
    if let Some(book) = &stored.book {
        book.validate()?;
        WorldBook::switch(&mut stored, ROOT, None)?;
    }
    validate_generated(&stored)?;
    if let Some(book) = &stored.book {
        for board in book.boards.values().filter_map(|b| b.doc.as_deref()) { validate_generated(board)?; }
    }
    let requires_six = |d: &Document| d.paper.is_some() || d.diplomacy.is_some();
    let requires_seven = |d: &Document| d.star_chart.as_ref().is_some_and(|s| s.requires_extended_format());
    let needs_eight = |d: &Document| d.environment.is_some() || d.visualization.is_some();
    let needs_nine = |d: &Document| {
        use map_core::{area::Pattern, styles::Look};
        d.areas.values().any(|area| area.pattern == Pattern::Solid)
            || d.styles.iter().any(|style| matches!(&style.look, Look::Area(area) if area.pattern == Pattern::Solid))
    };
    let version = if needs_nine(&stored) || stored.book.as_ref().is_some_and(|book| {
        book.boards.values().filter_map(|b| b.doc.as_deref()).any(needs_nine)
    }) { 9 } else if needs_eight(&stored) || stored.book.as_ref().is_some_and(|book| {
        book.worldview.is_some() || book.boards.values().filter_map(|b| b.doc.as_deref()).any(needs_eight)
    }) { 8 } else if requires_seven(&stored) || stored.book.as_ref().is_some_and(|b| {
        b.boards.values().filter_map(|x| x.doc.as_deref()).any(requires_seven)
    }) { 7 } else if requires_six(&stored) || stored.book.as_ref().is_some_and(|b| {
        b.boards.values().filter_map(|x| x.doc.as_deref()).any(requires_six)
    }) { 6 } else if stored.book.is_some() { 5 } else { match &stored.star_chart {
        Some(chart) if !chart.moon.is_default() => 4,
        Some(_) => 3,
        None if doc.climate.is_some() => 2,
        None => 1,
    }};
    stored.format_version = version;
    let body = rmp_serde::to_vec_named(&stored).map_err(|e| format!("序列化失败: {e}"))?;
    let packed = lz4_flex::compress_prepend_size(&body);
    let mut out = Vec::with_capacity(packed.len() + 12);
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&packed);
    Ok(out)
}

pub fn from_bytes(bytes: &[u8]) -> Result<Document, String> {
    if bytes.len() < 12 || &bytes[..8] != MAGIC {
        return Err("不是 Atlas 工程文件".into());
    }
    let version = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
    if version > FORMAT_VERSION {
        return Err(format!("文件版本 {version} 高于本程序支持的版本 {FORMAT_VERSION}，请升级软件"));
    }
    let body = lz4_flex::decompress_size_prepended(&bytes[12..]).map_err(|e| format!("解压失败: {e}"))?;
    let mut doc: Document = rmp_serde::from_slice(&body).map_err(|e| format!("文件损坏: {e}"))?;
    if let Some(book) = doc.book.as_mut() {
        book.validate()?;
        let boards = &mut Arc::make_mut(book).boards;
        let ids: Vec<_> = boards.keys().copied().collect();
        for id in ids {
            if let Some(board) = boards.get_mut(&id).and_then(|board|board.doc.as_mut()) { restore(Arc::make_mut(board), version)?; }
        }
        WorldBook::switch(&mut doc, ROOT, None)?;
    }
    restore(&mut doc, version)?;
    Ok(doc)
}

fn validate_generated(doc: &Document) -> Result<(), String> {
    if let Some(p) = &doc.environment { p.validate()?; }
    if let Some(p) = &doc.visualization { p.validate()?; }
    Ok(())
}

fn restore(doc: &mut Document, version: u32) -> Result<(), String> {
    if doc.home_view.is_some_and(|view| !view.valid()) {
        return Err("工程默认视角的经纬度无效".into());
    }
    if let Some(paper) = doc.paper { paper.validate()?; }
    if let Some(diplomacy) = &doc.diplomacy { diplomacy.validate()?; }
    if let Some(p) = &doc.climate { p.validate_storage()?; }
    validate_generated(doc)?;
    if let Some(chart) = &doc.star_chart { chart.validate()?; }
    migrate(doc, version);
    // merges before the fix could leave a unit its own parent
    doc.drop_self_parents();
    doc.topo.rebuild_index();
    // ownership in the selected year is derived, not stored
    doc.refresh_time_view();
    // geometry revisions must stay unique within this process (two files
    // may carry the same numbers for different shapes): give each face a
    // fresh one, so revision-keyed caches never mix files up
    let max_rev = doc.topo.faces.values().map(|f| f.rev).max().unwrap_or(0);
    ensure_rev_above(max_rev);
    let ids: Vec<_> = doc.topo.faces.keys().copied().collect();
    for f in ids {
        let mut face = doc.topo.faces[&f].clone();
        face.rev = map_core::topo::next_rev();
        doc.topo.faces.insert(f, face);
    }
    Ok(())
}

/// The document with every face revision set to 0: revisions are cache
/// keys renewed on load, not content (for comparing documents in tests).
#[cfg(test)]
pub(crate) fn without_revs(d: &Document) -> Document {
    let mut d = d.clone();
    let ids: Vec<_> = d.topo.faces.keys().copied().collect();
    for f in ids {
        let mut face = d.topo.faces[&f].clone();
        face.rev = 0;
        d.topo.faces.insert(f, face);
    }
    d
}

/// Hook for future format migrations.
fn migrate(doc: &mut Document, _from: u32) {
    doc.format_version = FORMAT_VERSION;
}

/// Save atomically: write a temp file, flush it to disk, then rename over
/// the target. An interruption at any point leaves the old file intact.
pub fn save(path: &Path, doc: &Document) -> Result<(), String> {
    let bytes = to_bytes(doc)?;
    crate::recovery::write_atomic(path, &bytes).map_err(|e| format!("保存失败: {e}"))
}

/// The `k`-th backup of a project file: `name.atlas.bak{k}` beside it.
pub fn backup_path(path: &Path, k: usize) -> std::path::PathBuf {
    let mut s = path.as_os_str().to_os_string();
    s.push(format!(".bak{k}"));
    std::path::PathBuf::from(s)
}

/// [`save`], first keeping the file being replaced as `.bak1` (the one
/// before that `.bak2`, … up to `keep`; the oldest dropped). A backup that
/// cannot be made does not stop the save.
pub fn save_with_backups(path: &Path, doc: &Document, keep: usize) -> Result<(), String> {
    let bytes = to_bytes(doc)?;
    if keep > 0 && path.exists() {
        let _ = std::fs::remove_file(backup_path(path, keep));
        for k in (1..keep).rev() {
            let from = backup_path(path, k);
            if from.exists() {
                let _ = std::fs::rename(&from, backup_path(path, k + 1));
            }
        }
        let _ = std::fs::copy(path, backup_path(path, 1));
    }
    crate::recovery::write_atomic(path, &bytes).map_err(|e| format!("保存失败: {e}"))
}

pub fn load(path: &Path) -> Result<Document, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取失败: {e}"))?;
    from_bytes(&bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Saving keeps the versions it replaces, newest as .bak1, up to `keep`.
    #[test]
    fn saving_keeps_backups() {
        let dir = std::env::temp_dir().join(format!("atlas_backups_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("世界.atlas");
        let mut d = map_core::sample::demo_world();
        for k in 0..5 {
            d.meta.name = format!("第 {k} 版");
            save_with_backups(&path, &d, 3).unwrap();
        }
        let name = |p: &Path| load(p).unwrap().meta.name;
        assert_eq!(name(&path), "第 4 版");
        assert_eq!((name(&backup_path(&path, 1)), name(&backup_path(&path, 2)), name(&backup_path(&path, 3))), ("第 3 版".into(), "第 2 版".into(), "第 1 版".into()));
        assert!(!backup_path(&path, 4).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
    use map_core::{EntityKind, FaceKind, FieldKind, LonLat, Value};

    #[test]
    fn roundtrip_is_lossless() {
        let mut d = Document::new("测试世界", 3000.0);
        let sq = [LonLat::new(170.0, 0.0), LonLat::new(-175.0, 0.0), LonLat::new(-175.0, 9.0), LonLat::new(170.0, 9.0)];
        let f = d.add_ring_face(&sq, FaceKind::Land).unwrap();
        let (a, b) = d.split_face(&[LonLat::new(178.0, 0.0), LonLat::new(178.0, 9.0)]).unwrap();
        let c = d.add_entity(EntityKind::Country, "甲国");
        d.assign(a, Some(c)).unwrap();
        let fid = d.add_field("pop", "人口", FieldKind::Number, None).unwrap();
        d.set_value(c, fid, Some(1900), Some(Value::Num(12.5)));
        d.add_city(LonLat::new(179.0, 4.0), "首都");
        assert_eq!(a, f);
        let _ = b;
        let bytes = to_bytes(&d).unwrap();
        let back = from_bytes(&bytes).unwrap();
        assert_eq!(without_revs(&back), without_revs(&d));
        // fresh revisions, unique in this process
        assert!(back.topo.faces.iter().all(|(f, x)| x.rev > d.topo.faces[f].rev));
        back.topo.validate().unwrap();
        // corrupt / foreign files are rejected cleanly
        assert!(from_bytes(b"hello world, not a map").is_err());
        let mut bad = bytes.clone();
        bad[8] = 99;
        assert!(from_bytes(&bad).unwrap_err().contains("版本"));
    }

    #[test]
    fn interrupted_save_keeps_the_previous_file() {
        let dir = std::env::temp_dir().join(format!("atlas_save_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("w.atlas");
        let v1 = map_core::sample::demo_world();
        save(&path, &v1).unwrap();
        // a crash in the middle of the next save leaves a truncated temp file behind
        let mut v2 = v1.clone();
        v2.meta.name = "第二版".into();
        let full = to_bytes(&v2).unwrap();
        std::fs::write(path.with_extension("atlas.tmp"), &full[..full.len() / 3]).unwrap();
        assert_eq!(without_revs(&load(&path).unwrap()), without_revs(&v1), "the committed file must be untouched");
        // the next save simply replaces the stale temp file
        save(&path, &v2).unwrap();
        assert_eq!(without_revs(&load(&path).unwrap()), without_revs(&v2));
        assert!(!path.with_extension("atlas.tmp").exists());
        // a truncated project file is rejected, not half-loaded
        std::fs::write(&path, &full[..full.len() / 2]).unwrap();
        assert!(load(&path).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn default_geographic_view_survives_saving_and_reopening() {
        use map_core::{doc::HomeView, projection::Projection};
        let view = HomeView {
            projection: Projection::EqualEarth,
            central_meridian: 128.0,
            globe_latitude: 41.0,
            flat_extent: None,
        };
        let mut doc = Document::new("远洋洲", 9650.0);
        doc.home_view = Some(view);
        let reopened = from_bytes(&to_bytes(&doc).unwrap()).unwrap();
        assert_eq!(reopened.home_view, Some(view));
        doc.home_view = Some(HomeView { central_meridian: f64::NAN, ..view });
        assert!(from_bytes(&to_bytes(&doc).unwrap()).is_err());
    }
}


/// The local authoring fixture used for desktop performance testing.
pub fn ithaer_test_project() -> Option<std::path::PathBuf> {
    let name = "伊瑟尔星2897_远洋洲与伊瑟尔三国.atlas";
    let path = std::env::current_dir().ok().into_iter()
        .chain(std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf)));
    for directory in path {
        for ancestor in directory.ancestors() {
            let candidate = ancestor.join("worlds").join("ithaer-2897").join("revised").join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
