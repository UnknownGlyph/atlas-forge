use crate::i18n::tr;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke, Vec2};
use map_core::visualization::{Dataset, Plot, Spec};

pub struct Index {
    order: Vec<usize>,
    edges: Vec<usize>,
    x: [f64; 2],
    y: [f64; 2],
    colors: [f64; 2],
}

impl Index {
    pub fn new(spec: &Spec, dataset: &Dataset) -> Self {
        let rows = &dataset.rows;
        let mut order: Vec<_> = (0..rows.len()).collect();
        let mut edges = Vec::new();
        match spec.plot {
            Plot::Bars | Plot::Relations => {
                let limit = (if spec.plot == Plot::Bars { 90 } else { 140 }).min(order.len());
                let compare = |&a: &usize, &b: &usize| {
                    rows[b].value.total_cmp(&rows[a].value).then(a.cmp(&b))
                };
                if limit < order.len() {
                    order.select_nth_unstable_by(limit, compare);
                    order.truncate(limit);
                }
                order.sort_unstable_by(compare);
                if spec.plot == Plot::Relations {
                    let selected: std::collections::HashSet<_> = order.iter().copied().collect();
                    edges.extend(
                        dataset
                            .edges
                            .iter()
                            .enumerate()
                            .filter_map(|(i, (a, b, _))| {
                                (selected.contains(a) && selected.contains(b)).then_some(i)
                            }),
                    );
                }
            }
            Plot::Scatter => {
                let every = order.len().div_ceil(6000).max(1);
                if every > 1 {
                    order = order.into_iter().step_by(every).collect();
                }
            }
            _ => {
                order.sort_unstable_by(|a, b| rows[*a].x.total_cmp(&rows[*b].x).then(a.cmp(b)));
                if order.len() > 1300 {
                    let mut sampled = Vec::new();
                    for bucket in order.chunks(order.len().div_ceil(530)) {
                        let mut indices = [
                            0,
                            bucket
                                .iter()
                                .enumerate()
                                .min_by(|a, b| rows[*a.1].value.total_cmp(&rows[*b.1].value))
                                .unwrap()
                                .0,
                            bucket
                                .iter()
                                .enumerate()
                                .max_by(|a, b| rows[*a.1].value.total_cmp(&rows[*b.1].value))
                                .unwrap()
                                .0,
                            bucket.len() - 1,
                        ];
                        indices.sort_unstable();
                        sampled.extend(indices.into_iter().enumerate().filter_map(|(i, j)| {
                            (i == 0 || j > indices[i - 1]).then_some(bucket[j])
                        }));
                    }
                    order = sampled;
                }
            }
        }
        let extent = |values: &mut dyn Iterator<Item = f64>| {
            let [a, b] = values.fold([f64::INFINITY, f64::NEG_INFINITY], |[a, b], v| {
                [a.min(v), b.max(v)]
            });
            if !a.is_finite() {
                [0.0, 1.0]
            } else if a == b {
                [a - 0.5, b + 0.5]
            } else {
                [a, b]
            }
        };
        let x = if spec.plot == Plot::Bars {
            [0.0, order.len() as f64]
        } else {
            extent(&mut rows.iter().map(|r| r.x))
        };
        let mut y = extent(&mut rows.iter().map(|r| r.value));
        if spec.plot == Plot::Bars {
            y[0] = y[0].min(0.0);
            y[1] = y[1].max(0.0);
        }
        Self {
            order,
            edges,
            x,
            y,
            colors: spec.rule.scale(rows),
        }
    }
}

pub fn draw(ui: &mut egui::Ui, spec: &Spec, dataset: &Dataset, index: &Index) {
    if dataset.rows.is_empty() {
        ui.label(tr("当前数据来源没有可显示的值。"));
        return;
    }
    let size = Vec2::new(
        ui.available_width().max(200.0),
        ui.available_height().clamp(250.0, 720.0),
    );
    let (response, painter) = ui.allocate_painter(size, egui::Sense::hover());
    let rect = response.rect;
    let ink = ui.visuals().text_color();
    painter.rect_filled(rect, 5.0, ui.visuals().extreme_bg_color);
    if spec.plot == Plot::Relations {
        draw_network(ui, &response, &painter, dataset, index, spec, ink);
        return;
    }
    let area = Rect::from_min_max(
        rect.min + Vec2::new(65.0, 17.0),
        rect.max - Vec2::new(20.0, 45.0),
    );
    if area.width() <= 0.0 {
        return;
    }
    let category = spec.plot == Plot::Bars;
    let [min_x, max_x] = index.x;
    let [min_y, max_y] = index.y;
    let labels: Vec<_> = index.order.iter().map(|&i| &dataset.rows[i]).collect();
    let x_px = |x: f64| area.left() + ((x - min_x) / (max_x - min_x)) as f32 * area.width();
    let y_px = |y: f64| area.bottom() - ((y - min_y) / (max_y - min_y)) as f32 * area.height();
    let label = FontId::proportional(11.0);
    for i in 0..=5 {
        let f = f64::from(i) / 5.0;
        let y = min_y + (max_y - min_y) * f;
        let py = y_px(y);
        painter.line_segment(
            [Pos2::new(area.left(), py), Pos2::new(area.right(), py)],
            Stroke::new(0.65, ink.gamma_multiply(0.3)),
        );
        painter.text(
            Pos2::new(area.left() - 8.0, py),
            Align2::RIGHT_CENTER,
            tick(y),
            label.clone(),
            ink,
        );
        if !category {
            let x = min_x + (max_x - min_x) * f;
            painter.text(
                Pos2::new(x_px(x), area.bottom() + 10.0),
                Align2::CENTER_TOP,
                tick(x),
                label.clone(),
                ink,
            );
        }
    }
    if !spec.rule.unit.is_empty() {
        painter.text(
            Pos2::new(area.left(), rect.top() + 2.0),
            Align2::LEFT_TOP,
            &spec.rule.unit,
            label.clone(),
            ink,
        );
    }
    if spec.plot == Plot::Profile {
        painter.text(
            Pos2::new(area.center().x, rect.bottom() - 6.0),
            Align2::CENTER_BOTTOM,
            tr("沿线距离 km"),
            label.clone(),
            ink,
        );
    }
    let scale = index.colors;
    let plotted: Vec<_> = labels
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let x = if category { i as f64 + 0.5 } else { row.x };
            (Pos2::new(x_px(x), y_px(row.value)), *row)
        })
        .collect();
    if category {
        let width = (area.width() / labels.len() as f32 * 0.70).clamp(1.0, 100.0) * 0.5;
        let zero = y_px(0.0);
        for (i, (p, row)) in plotted.iter().enumerate() {
            let color = color(spec.rule.color(row.value, scale));
            painter.rect_filled(
                Rect::from_min_max(
                    Pos2::new(p.x - width, p.y.min(zero)),
                    Pos2::new(p.x + width, p.y.max(zero)),
                ),
                1.0,
                color,
            );
            if i % labels.len().div_ceil(7) == 0 {
                let text = row.label.chars().take(12).collect::<String>();
                painter.text(
                    Pos2::new(p.x, area.bottom() + 9.0),
                    Align2::CENTER_TOP,
                    text,
                    label.clone(),
                    ink,
                );
            }
        }
    } else if spec.plot == Plot::Scatter {
        for (p, row) in &plotted {
            painter.circle_filled(*p, 3.2, color(spec.rule.color(row.value, scale)));
        }
    } else {
        for pair in plotted.windows(2) {
            let [(a, r0), (b, r1)] = pair else {
                unreachable!()
            };
            let stroke = Stroke::new(
                2.2,
                color(spec.rule.color((r0.value + r1.value) * 0.5, scale)),
            );
            if spec.plot == Plot::Curve && b.x > a.x + 2.0 {
                let delta = (b.x - a.x) * 0.36;
                painter.add(egui::epaint::CubicBezierShape::from_points_stroke(
                    [
                        *a,
                        *a + Vec2::new(delta, 0.0),
                        *b - Vec2::new(delta, 0.0),
                        *b,
                    ],
                    false,
                    Color32::TRANSPARENT,
                    stroke,
                ));
            } else {
                painter.line_segment([*a, *b], stroke);
            }
        }
        if plotted.len() <= 24 {
            for (p, row) in &plotted {
                painter.circle_filled(*p, 3.4, color(spec.rule.color(row.value, scale)));
            }
        }
    }
    if let Some(at) = response.hover_pos().filter(|p| area.contains(*p)) {
        let distance = |p: Pos2| {
            if category {
                (p.x - at.x).abs()
            } else {
                p.distance(at)
            }
        };
        if let Some((p, row)) = plotted
            .iter()
            .min_by(|(a, _), (b, _)| distance(*a).total_cmp(&distance(*b)))
        {
            if category || distance(*p) < 28.0 {
                painter.circle_stroke(*p, 5.0, Stroke::new(1.4, ink));
                let text = format!("{} · {} {}", row.label, tick(row.value), spec.rule.unit);
                response.clone().on_hover_text_at_pointer(text);
            }
        }
    }
    if category && dataset.rows.len() > plotted.len() {
        ui.weak(format!(
            "{} {}/{}",
            tr("按数值显示"),
            plotted.len(),
            dataset.rows.len()
        ));
    }
}

fn draw_network(
    ui: &mut egui::Ui,
    response: &egui::Response,
    painter: &egui::Painter,
    dataset: &Dataset,
    index: &Index,
    spec: &Spec,
    ink: Color32,
) {
    let rect = response.rect.shrink(38.0);
    let center = rect.center();
    let visible = &index.order;
    let mut positions = std::collections::HashMap::new();
    let first_ring = visible.len().min(13);
    let rings = if visible.len() > first_ring { 2 } else { 1 };
    for (rank, &id) in visible.iter().enumerate() {
        let (ring, index, len) = if rank < first_ring {
            (0, rank, first_ring)
        } else {
            (1, rank - first_ring, visible.len() - first_ring)
        };
        let phase = std::f32::consts::TAU * index as f32 / len.max(1) as f32;
        let reach = if visible.len() == 1 {
            0.0
        } else {
            (ring as f32 + 1.0) / rings as f32
        };
        let pos = center
            + Vec2::new(
                phase.sin() * rect.width() * 0.46,
                -phase.cos() * rect.height() * 0.45,
            ) * reach;
        positions.insert(id, pos);
    }
    let edge_color = ink.gamma_multiply(0.34);
    for &i in &index.edges {
        let (from, to, _) = dataset.edges[i];
        let (Some(a), Some(b)) = (positions.get(&from), positions.get(&to)) else {
            continue;
        };
        let delta = *b - *a;
        if delta.length_sq() < 1.0 {
            painter.circle_stroke(*a + Vec2::new(8.0, -8.0), 8.0, Stroke::new(0.9, edge_color));
            continue;
        }
        let dir = delta.normalized();
        let end = *b - dir * 8.0;
        painter.line_segment([*a + dir * 7.0, end], Stroke::new(0.9, edge_color));
        let side = Vec2::new(-dir.y, dir.x);
        for sign in [-1.0, 1.0] {
            painter.line_segment(
                [end, end - dir * 5.0 + side * sign * 2.8],
                Stroke::new(0.9, edge_color),
            );
        }
    }
    for id in visible.iter().copied() {
        let pos = positions[&id];
        let row = &dataset.rows[id];
        let radius = (4.0 + (row.value as f32).sqrt()).min(10.0);
        painter.circle_filled(pos, radius, color(spec.rule.color(row.value, index.colors)));
        if visible.len() <= 30
            || response
                .hover_pos()
                .is_some_and(|p| p.distance(pos) < radius + 7.0)
        {
            painter.text(
                pos + Vec2::new(0.0, -radius - 2.0),
                Align2::CENTER_BOTTOM,
                &row.label,
                FontId::proportional(11.0),
                ink,
            );
        }
    }
    if visible.len() < dataset.rows.len() {
        ui.weak(format!(
            "{} {}/{}",
            tr("按连接数显示"),
            visible.len(),
            dataset.rows.len()
        ));
    }
}

fn tick(value: f64) -> String {
    if value.abs() >= 1e6 || (value.abs() > 0.0 && value.abs() < 0.01) {
        format!("{value:.2e}")
    } else if value >= 100.0 || value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

fn color([r, g, b, a]: [u8; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}
