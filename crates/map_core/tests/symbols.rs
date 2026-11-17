//! Map symbols: their years and their drawings.

use map_core::geom::LonLat;
use map_core::symbol::{marks, MapSymbol, Mark, SymbolKind};

fn area(p: &[[f32; 2]]) -> f32 {
    let n = p.len();
    (0..n).map(|i| p[i][0] * p[(i + 1) % n][1] - p[(i + 1) % n][0] * p[i][1]).sum::<f32>().abs() * 0.5
}

/// No year selected: every symbol shows. A year selected: only those
/// whose range holds it (ends inclusive, open ends unbounded).
#[test]
fn symbols_show_in_their_years() {
    let mut s = MapSymbol::new(SymbolKind::Battle, LonLat::new(0.0, 0.0));
    assert!(s.shown_in(None) && s.shown_in(Some(-500)) && s.years_text().is_empty());
    s.from = Some(1815);
    s.to = Some(1815);
    assert!(s.shown_in(None) && s.shown_in(Some(1815)));
    assert!(!s.shown_in(Some(1814)) && !s.shown_in(Some(1816)));
    assert_eq!(s.years_text(), "1815 年");
    s.to = None;
    assert!(s.shown_in(Some(2000)) && !s.shown_in(Some(1800)));
    assert_eq!(s.years_text(), "1815 年起");
    s.from = None;
    s.to = Some(1900);
    assert!(s.shown_in(Some(1000)) && !s.shown_in(Some(1901)));
}

/// Every filled part's convex pieces cover exactly its outline (the app
/// fills the pieces, the exports the outline), each piece is convex, and
/// the halo comes first, one per part.
#[test]
fn symbol_drawings_agree() {
    for kind in SymbolKind::ALL {
        let ms = marks(kind, [100.0, 50.0], 10.0, [1, 2, 3, 255]);
        let n = ms.len() / 2;
        assert_eq!(ms.len(), 2 * n);
        for (i, m) in ms.iter().enumerate() {
            let halo = matches!(m, Mark::Stroke { color: [255, 255, 255, _], .. });
            assert_eq!(halo, i < n, "{kind:?}: halos first");
        }
        for m in &ms {
            if let Mark::Fill { outline, pieces, .. } = m {
                let sum: f32 = pieces.iter().map(|p| area(p)).sum();
                assert!((sum - area(outline)).abs() < 1e-2 * area(outline), "{kind:?}: {sum} vs {}", area(outline));
                for p in pieces {
                    let k = p.len();
                    let signs: Vec<bool> = (0..k)
                        .map(|i| {
                            let (a, b, c) = (p[i], p[(i + 1) % k], p[(i + 2) % k]);
                            (b[0] - a[0]) * (c[1] - b[1]) - (b[1] - a[1]) * (c[0] - b[0]) > 0.0
                        })
                        .collect();
                    assert!(signs.iter().all(|s| *s == signs[0]), "{kind:?}: a piece is not convex");
                }
            }
            // everything within the symbol's box (plus stroke)
            let pts = match m {
                Mark::Fill { outline, .. } => outline,
                Mark::Stroke { pts, .. } => pts,
            };
            assert!(pts.iter().all(|p| (p[0] - 100.0).abs() <= 10.0 && (p[1] - 50.0).abs() <= 10.0), "{kind:?}");
        }
    }
}

/// A symbol saved with only the required fields loads with no name and
/// no years.
#[test]
fn old_symbol_fields_default() {
    let s: MapSymbol = serde_json::from_str(r#"{"kind":"Port","at":{"lon":1.0,"lat":2.0},"size":7.0,"color":[1,2,3,255]}"#).unwrap();
    assert_eq!((s.name.as_str(), s.from, s.to), ("", None, None));
}
