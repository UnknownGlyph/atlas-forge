//! Drawn features as clipboard text, so they can be copied from one window
//! of the program and pasted into another.

use map_core::arrange::Clip;

const MARK: &str = "{\"atlas_forge_features\"";

pub fn to_text(clip: &Clip) -> String {
    serde_json::to_string(clip).unwrap_or_default()
}

/// Features from clipboard text; None when the text is something else
/// (any other text on the clipboard is left alone).
pub fn from_text(text: &str) -> Option<Clip> {
    let t = text.trim_start();
    if !t.starts_with(MARK) {
        return None;
    }
    serde_json::from_str::<Clip>(t).ok().filter(|c| c.atlas_forge_features == 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use map_core::features::FeatureRef;

    #[test]
    fn features_survive_the_clipboard_and_other_text_is_ignored() {
        let (d, _) = map_core::testworld::history_world(&map_core::testworld::TestWorldSpec { nx: 3, ny: 2, events: 6, ..Default::default() });
        let all: Vec<FeatureRef> = d.features().into_iter().map(|f| f.what).collect();
        assert!(all.len() >= 10);
        let clip = d.copy_features(&all);
        let text = to_text(&clip);
        // the same features come back (numbers to within rounding of the text)
        for t in [text.clone(), format!("  {text}")] {
            let back = from_text(&t).expect("our own text");
            assert_eq!(back.items.len(), clip.items.len());
            let (mut x, mut y) = (map_core::Document::default(), map_core::Document::default());
            let (fx, fy) = (x.paste_features(&clip, None).unwrap(), y.paste_features(&back, None).unwrap());
            for (p, q) in fx.iter().zip(&fy) {
                assert_eq!(std::mem::discriminant(p), std::mem::discriminant(q));
                let (pp, qq) = (x.feature_points(*p), y.feature_points(*q));
                assert!(pp.len() == qq.len() && pp.iter().zip(&qq).all(|(a, b)| (a.lon - b.lon).abs() < 1e-9 && (a.lat - b.lat).abs() < 1e-9));
            }
        }
        for other in ["", "hello", "{}", "{\"atlas_forge_features\":2,\"items\":[]}", "{\"items\":[]}"] {
            assert!(from_text(other).is_none(), "{other}");
        }
    }
}
