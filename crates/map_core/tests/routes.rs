//! Routes: rivers' styled siblings.

use map_core::doc::{arrow_end, Dash, LineFeature, RouteStyle};

/// The head points along the line's end, is as long as asked, and the
/// line stops inside it so no cap pokes past the tip.
#[test]
fn an_arrow_sits_on_the_end() {
    let pts = [[0.0, 0.0], [50.0, 0.0], [50.0, 50.0]];
    let (line, head) = arrow_end(&pts, 2.0).unwrap();
    // head length 2*3+7 = 13, half width 2*1.8+4 = 7.6, pointing down
    assert_eq!(head[0], [50.0, 50.0]);
    let base = [(head[1][0] + head[2][0]) * 0.5, (head[1][1] + head[2][1]) * 0.5];
    assert!((base[0] - 50.0).abs() < 1e-4 && (base[1] - 37.0).abs() < 1e-4, "{base:?}");
    assert!(((head[1][0] - head[2][0]).abs() - 15.2).abs() < 1e-4);
    assert_eq!(&line[..2], &pts[..2]);
    let end = *line.last().unwrap();
    assert!(end[0] == 50.0 && end[1] > 37.0 && end[1] + 1.0 < 50.0, "{end:?}");
    // the head can cross a corner of the line
    let (line, head) = arrow_end(&[[0.0, 0.0], [100.0, 0.0], [100.0, 5.0]], 2.0).unwrap();
    assert_eq!(head[0], [100.0, 5.0]);
    assert_eq!(line.len(), 2);
    // too short to carry one
    assert!(arrow_end(&[[0.0, 0.0], [5.0, 0.0]], 2.0).is_none());
    assert!(arrow_end(&[[0.0, 0.0]], 2.0).is_none());
}

#[test]
fn dash_patterns() {
    let s = |dash| RouteStyle { dash, ..Default::default() };
    assert_eq!(s(Dash::Solid).pattern(2.0), None);
    assert_eq!(s(Dash::Dashed).pattern(2.0), Some([8.0, 7.0]));
    let dots = s(Dash::Dotted).pattern(2.0).unwrap();
    assert!(dots[0] < 0.1 && dots[1] == 5.0);
}

/// Lines saved before routes existed load as rivers.
#[test]
fn old_lines_are_rivers() {
    let r: LineFeature = serde_json::from_str(r#"{"name":"河","pts":[]}"#).unwrap();
    assert_eq!((r.route, r.kind_name()), (None, "河流"));
    let mut v = serde_json::to_value(&r).unwrap();
    v["route"] = serde_json::to_value(RouteStyle::default()).unwrap();
    let back: LineFeature = serde_json::from_value(v).unwrap();
    assert_eq!((back.route, back.kind_name()), (Some(RouteStyle::default()), "线路"));
}
