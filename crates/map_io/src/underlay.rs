//! Reference image files: import (keeping the original encoded bytes) and
//! decoding for display.

use map_core::underlay::Underlay;
use std::io::Cursor;
use std::path::Path;

/// Read an image file into a new underlay placed as a whole-world
/// equirectangular map (the user aligns it afterwards).
pub fn import(path: &Path) -> Result<Underlay, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取失败: {e}"))?;
    let (w, h) = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|e| format!("无法识别图片格式: {e}"))?
        .into_dimensions()
        .map_err(|e| format!("无法读取图片: {e}"))?;
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "参考图片".into());
    Ok(Underlay::new(&name, bytes, [w, h]))
}

/// Decoded straight-alpha RGBA8 pixels.
pub struct Decoded {
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
    /// Downscaled to fit the limits (the georeference still uses the
    /// original pixel size).
    pub scaled: bool,
}

/// Decode, downscaling (aspect kept) to at most `max_dim` per side and
/// `max_pixels` in total.
pub fn decode(bytes: &[u8], max_dim: u32, max_pixels: u64) -> Result<Decoded, String> {
    let img = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| format!("无法识别图片格式: {e}"))?
        .decode()
        .map_err(|e| format!("图片解码失败: {e}"))?;
    let (w, h) = (img.width() as f64, img.height() as f64);
    let k = (max_dim as f64 / w).min(max_dim as f64 / h).min((max_pixels as f64 / (w * h)).sqrt()).min(1.0);
    let (img, scaled) = if k < 1.0 {
        let (nw, nh) = (((w * k) as u32).max(1), ((h * k) as u32).max(1));
        (img.resize_exact(nw, nh, image::imageops::FilterType::Triangle), true)
    } else {
        (img, false)
    };
    let rgba = img.into_rgba8();
    Ok(Decoded { w: rgba.width(), h: rgba.height(), pixels: rgba.into_raw(), scaled })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png(w: u32, h: u32) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| image::Rgba([(x % 256) as u8, (y % 256) as u8, 7, 255]));
        let mut out = Vec::new();
        img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn import_decode_and_project_roundtrip() {
        let dir = std::env::temp_dir().join(format!("atlas_underlay_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("老地图.png");
        std::fs::write(&path, png(300, 150)).unwrap();
        let u = import(&path).unwrap();
        assert_eq!((u.name.as_str(), u.size), ("老地图", [300, 150]));
        let d = decode(&u.image.0, 8192, 1 << 30).unwrap();
        assert_eq!((d.w, d.h, d.scaled), (300, 150, false));
        assert_eq!(&d.pixels[4 * 301..4 * 301 + 4], &[1, 1, 7, 255]);
        // limits downscale, keeping the aspect ratio
        let d = decode(&u.image.0, 100, 1 << 30).unwrap();
        assert_eq!((d.w, d.h, d.scaled), (100, 50, true));
        let d = decode(&u.image.0, 8192, 300 * 150 / 4).unwrap();
        assert_eq!((d.w, d.h), (150, 75));
        // the image travels inside the project file
        let mut doc = map_core::sample::demo_world();
        doc.underlays.push(u.clone());
        let back = crate::project::from_bytes(&crate::project::to_bytes(&doc).unwrap()).unwrap();
        assert_eq!(back.underlays, vec![u]);
        assert!(import(&dir.join("missing.png")).is_err());
        std::fs::write(dir.join("bad.png"), b"not an image").unwrap();
        assert!(import(&dir.join("bad.png")).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
