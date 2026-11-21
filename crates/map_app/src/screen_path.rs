//! Keep repeated stroke work proportional to the visible path length.
use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke};

fn clip(a: Pos2, b: Pos2, view: Rect) -> Option<(f32, f32)> {
    let direction = b - a;
    let mut start = 0.0f32;
    let mut end = 1.0f32;
    for (d, margin) in [
        (-direction.x, a.x - view.left()),
        (direction.x, view.right() - a.x),
        (-direction.y, a.y - view.top()),
        (direction.y, view.bottom() - a.y),
    ] {
        if d < 0.0 {
            start = start.max(margin / d);
        } else if d > 0.0 {
            end = end.min(margin / d);
        } else if margin < 0.0 {
            return None;
        }
    }
    (start < end).then_some((start, end))
}

pub(crate) fn visible_runs(path: &[Pos2], view: Rect) -> Vec<(Vec<Pos2>, f64)> {
    let mut runs: Vec<(Vec<Pos2>, f64)> = Vec::new();
    let mut distance = 0.0;
    let mut continuing = false;
    for pair in path.windows(2) {
        let a = pair[0];
        let b = pair[1];
        let length = f64::from(a.distance(b));
        if length == 0.0 { continue; }
        if let Some((first, last)) = clip(a, b, view) {
            let start = a.lerp(b, first);
            let end = a.lerp(b, last);
            if continuing && first == 0.0 {
                runs.last_mut().unwrap().0.push(end);
            } else {
                runs.push((vec![start, end], distance + length * f64::from(first)));
            }
            continuing = last == 1.0;
        } else {
            continuing = false;
        }
        distance += length;
    }
    runs
}

pub(crate) fn dashed(painter: &egui::Painter, path: &[Pos2], stroke: Stroke, dash: f32, gap: f32) {
    let period = f64::from(dash + gap);
    for (points, distance) in visible_runs(path, painter.clip_rect().expand(stroke.width + 3.0)) {
        let phase = -distance.rem_euclid(period) as f32;
        painter.extend(Shape::dashed_line_with_offset(&points, stroke, &[dash], &[gap], phase));
    }
}

pub(crate) fn dotted(painter: &egui::Painter, path: &[Pos2], color: Color32, step: f32, radius: f32) {
    if step <= 0.0 { return; }
    let step = f64::from(step);
    for (points, distance) in visible_runs(path, painter.clip_rect().expand(radius + 3.0)) {
        let phase = distance.rem_euclid(step);
        let mut next = if phase <= 1.0e-6 { 0.0 } else { step - phase };
        let mut travelled = 0.0;
        for pair in points.windows(2) {
            let length = f64::from(pair[0].distance(pair[1]));
            if length <= 0.0 { continue; }
            while next < travelled + length {
                let at = pair[0].lerp(pair[1], ((next - travelled) / length).clamp(0.0, 1.0) as f32);
                painter.circle_filled(at, radius, color);
                next += step;
            }
            travelled += length;
        }
    }
}
