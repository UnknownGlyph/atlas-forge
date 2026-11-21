//! Map symbols: point markers for peaks, battles, capitals, ports, forts
//! and sites. Each kind is a small vector drawing in a unit box; the app
//! and every exporter draw it from the same `marks`, so they look alike.
//! A symbol is layout like map text, but may carry a year range (a battle
//! belongs to its year): with a year selected it shows only inside its
//! range; with no year selected every symbol shows.

use crate::geom::LonLat;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Peak,
    Battle,
    Capital,
    Port,
    Fort,
    Site,
    Volcano,
    Temple,
    Lighthouse,
    Mine,
    Pass,
    Camp,
    /// Any character as the symbol (a letter, 寺, ★, ✠...).
    Glyph,
}

impl SymbolKind {
    pub const ALL: [SymbolKind; 13] = [
        SymbolKind::Peak,
        SymbolKind::Volcano,
        SymbolKind::Pass,
        SymbolKind::Battle,
        SymbolKind::Camp,
        SymbolKind::Fort,
        SymbolKind::Capital,
        SymbolKind::Port,
        SymbolKind::Lighthouse,
        SymbolKind::Temple,
        SymbolKind::Mine,
        SymbolKind::Site,
        SymbolKind::Glyph,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SymbolKind::Peak => "山峰",
            SymbolKind::Battle => "战役",
            SymbolKind::Capital => "首都",
            SymbolKind::Port => "港口",
            SymbolKind::Fort => "要塞",
            SymbolKind::Site => "遗址 / 地点",
            SymbolKind::Volcano => "火山",
            SymbolKind::Temple => "寺庙",
            SymbolKind::Lighthouse => "灯塔",
            SymbolKind::Mine => "矿山",
            SymbolKind::Pass => "关隘",
            SymbolKind::Camp => "营地",
            SymbolKind::Glyph => "字符",
        }
    }

    /// The colour a new symbol of this kind starts with.
    pub fn default_color(self) -> [u8; 4] {
        match self {
            SymbolKind::Peak => [110, 80, 50, 255],
            SymbolKind::Battle => [170, 30, 30, 255],
            SymbolKind::Capital => [40, 40, 40, 255],
            SymbolKind::Port => [30, 70, 130, 255],
            SymbolKind::Fort => [70, 70, 70, 255],
            SymbolKind::Site => [120, 60, 120, 255],
            SymbolKind::Volcano => [160, 50, 30, 255],
            SymbolKind::Temple => [130, 95, 35, 255],
            SymbolKind::Lighthouse => [30, 70, 130, 255],
            SymbolKind::Mine => [80, 80, 80, 255],
            SymbolKind::Pass => [110, 80, 50, 255],
            SymbolKind::Camp => [85, 105, 40, 255],
            SymbolKind::Glyph => [40, 40, 40, 255],
        }
    }
}

/// Where a symbol's name is written, beside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamePlace {
    #[default]
    Right,
    Left,
    Above,
    Below,
}

impl NamePlace {
    pub const ALL: [NamePlace; 4] = [NamePlace::Right, NamePlace::Left, NamePlace::Above, NamePlace::Below];
    pub fn name(self) -> &'static str {
        match self {
            NamePlace::Right => "右",
            NamePlace::Left => "左",
            NamePlace::Above => "上",
            NamePlace::Below => "下",
        }
    }

    /// The top left of a name `w` × `h` pixels beside a symbol of
    /// half-height `size` centred at `c` (screen pixels, y down), with a
    /// `gap` between them.
    pub fn top_left(self, c: [f32; 2], size: f32, w: f32, h: f32, gap: f32) -> [f32; 2] {
        match self {
            NamePlace::Right => [c[0] + size + gap, c[1] - h * 0.5],
            NamePlace::Left => [c[0] - size - gap - w, c[1] - h * 0.5],
            NamePlace::Above => [c[0] - w * 0.5, c[1] - size - gap - h],
            NamePlace::Below => [c[0] - w * 0.5, c[1] + size + gap],
        }
    }
}

/// A symbol on the map. `size` is its half-height in screen pixels (in
/// export, at a 1600 px wide image, scaled like line widths). `name` is
/// written beside it when not empty.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MapSymbol {
    pub kind: SymbolKind,
    pub at: LonLat,
    pub size: f32,
    pub color: [u8; 4],
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub name_place: NamePlace,
    /// The character drawn for a `Glyph` symbol.
    #[serde(default)]
    pub glyph: String,
    /// Optional custom artwork. The underlying kind is restored if removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pixels: Option<std::sync::Arc<crate::pixel_art::PixelArt>>,
    /// What it stands for in the export's legend; empty: a built-in kind's
    /// name (a `Glyph` symbol then stays out of the legend).
    #[serde(default)]
    pub legend: String,
    /// Hidden: not drawn, exported or put in the legend, but kept (and
    /// listed in the 要素 tab, where it is shown again).
    #[serde(default)]
    pub hidden: bool,
    /// First and last year it is shown in (inclusive); `None` is open.
    #[serde(default)]
    pub from: Option<i32>,
    #[serde(default)]
    pub to: Option<i32>,
}

impl MapSymbol {
    pub fn new(kind: SymbolKind, at: LonLat) -> Self {
        Self { kind, at, size: 7.0, color: kind.default_color(), name: String::new(), name_place: NamePlace::Right, glyph: if kind == SymbolKind::Glyph { "◆".into() } else { String::new() }, pixels: None, legend: String::new(), hidden: false, from: None, to: None }
    }

    pub fn drawn_size(&self, scale: f32) -> [f32; 2] {
        let extent = self.size * 2.0 * scale;
        self.pixels.as_ref().map_or([extent; 2], |art| art.drawn_size(extent))
    }

    pub fn kind_name(&self) -> &'static str {
        if self.pixels.is_some() { "像素标志" } else { self.kind.name() }
    }

    /// Whether it shows when viewing `year` (`None`: no year selected, all show).
    pub fn shown_in(&self, year: Option<i32>) -> bool {
        !self.hidden && in_years(self.from, self.to, year)
    }

    /// Its legend row's text: its own, else a built-in kind's name; `None`
    /// for a character symbol without one.
    pub fn legend_text(&self) -> Option<String> {
        let own = self.legend.trim();
        if !own.is_empty() {
            Some(own.to_string())
        } else if self.pixels.is_some() || self.kind == SymbolKind::Glyph {
            None
        } else {
            Some(self.kind.name().to_string())
        }
    }

    /// The year range as text, empty when it always shows.
    pub fn years_text(&self) -> String {
        years_text(self.from, self.to)
    }
}

/// Whether something shown from `from` to `to` (inclusive, `None` open)
/// shows when viewing `year`; with no year selected (`None`) everything
/// shows, so maps without years work as before.
pub fn in_years(from: Option<i32>, to: Option<i32>, year: Option<i32>) -> bool {
    year.is_none_or(|y| from.is_none_or(|f| y >= f) && to.is_none_or(|t| y <= t))
}

/// A year range as text, empty when open at both ends.
pub fn years_text(from: Option<i32>, to: Option<i32>) -> String {
    match (from, to) {
        (None, None) => String::new(),
        (Some(f), None) => format!("{f} 年起"),
        (None, Some(t)) => format!("至 {t} 年"),
        (Some(f), Some(t)) if f == t => format!("{f} 年"),
        (Some(f), Some(t)) => format!("{f}–{t} 年"),
    }
}

/// One part of a symbol drawing, in pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    /// A filled area: its outline (may be concave) for renderers that
    /// fill any polygon, and the same area as convex pieces for those
    /// that only fill convex ones.
    Fill { outline: Vec<[f32; 2]>, pieces: Vec<Vec<[f32; 2]>>, color: [u8; 4] },
    /// A stroked line (round caps and joins), closed or open.
    Stroke { pts: Vec<[f32; 2]>, closed: bool, width: f32, color: [u8; 4] },
}

/// Line width of stroked parts, in units of the symbol's half-height.
const STROKE: f32 = 0.2;
const HALO: [u8; 4] = [255, 255, 255, 220];

fn circle(c: [f32; 2], r: f32, n: usize) -> Vec<[f32; 2]> {
    (0..n).map(|i| {
        let t = i as f32 / n as f32 * std::f32::consts::TAU;
        [c[0] + r * t.cos(), c[1] + r * t.sin()]
    })
    .collect()
}

fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Vec<[f32; 2]> {
    vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]
}

enum Part {
    Fill(Vec<[f32; 2]>, Vec<Vec<[f32; 2]>>),
    Stroke(Vec<[f32; 2]>, bool),
}

/// The drawing of a kind in a unit box (y down, about -1..1).
/// The font size of a character symbol of half-height `size`: the glyph
/// about as tall as the drawn symbols.
pub fn glyph_px(size: f32) -> f32 {
    size * 2.2
}

fn parts(kind: SymbolKind) -> Vec<Part> {
    match kind {
        // drawn as text by each renderer
        SymbolKind::Glyph => vec![],
        SymbolKind::Peak => {
            let t = vec![[0.0, -0.85], [0.95, 0.8], [-0.95, 0.8]];
            vec![Part::Fill(t.clone(), vec![t])]
        }
        SymbolKind::Battle => vec![
            // two crossed swords, each with a crossguard near its hilt
            Part::Stroke(vec![[-0.8, -0.8], [0.8, 0.8]], false),
            Part::Stroke(vec![[0.8, -0.8], [-0.8, 0.8]], false),
            Part::Stroke(vec![[0.25, 0.75], [0.75, 0.25]], false),
            Part::Stroke(vec![[-0.25, 0.75], [-0.75, 0.25]], false),
        ],
        SymbolKind::Capital => {
            // five-pointed star: an inner pentagon and five tips
            let pt = |i: usize, r: f32| {
                let t = (-90.0 + 36.0 * i as f32).to_radians();
                [r * t.cos(), r * t.sin() + 0.08]
            };
            let outline: Vec<[f32; 2]> = (0..10).map(|i| pt(i, if i % 2 == 0 { 1.0 } else { 0.42 })).collect();
            let mut pieces = vec![(0..5).map(|k| pt(2 * k + 1, 0.42)).collect::<Vec<_>>()];
            for k in 0..5 {
                pieces.push(vec![pt(2 * k, 1.0), pt(2 * k + 1, 0.42), pt((2 * k + 9) % 10, 0.42)]);
            }
            vec![Part::Fill(outline, pieces)]
        }
        SymbolKind::Port => {
            // an anchor: ring, shank, stock and the curved arms
            let arc: Vec<[f32; 2]> = (0..=12).map(|i| {
                let t = (20.0 + 140.0 * i as f32 / 12.0).to_radians();
                [0.72 * t.cos(), 0.15 + 0.72 * t.sin()]
            })
            .collect();
            vec![
                Part::Stroke(circle([0.0, -0.68], 0.2, 12), true),
                Part::Stroke(vec![[0.0, -0.48], [0.0, 0.86]], false),
                Part::Stroke(vec![[-0.42, -0.25], [0.42, -0.25]], false),
                Part::Stroke(arc, false),
            ]
        }
        SymbolKind::Fort => {
            // a wall with three merlons
            let outline = vec![
                [-0.9, 0.85],
                [-0.9, -0.75],
                [-0.55, -0.75],
                [-0.55, -0.4],
                [-0.18, -0.4],
                [-0.18, -0.75],
                [0.18, -0.75],
                [0.18, -0.4],
                [0.55, -0.4],
                [0.55, -0.75],
                [0.9, -0.75],
                [0.9, 0.85],
            ];
            let pieces = vec![rect(-0.9, -0.4, 0.9, 0.85), rect(-0.9, -0.75, -0.55, -0.4), rect(-0.18, -0.75, 0.18, -0.4), rect(0.55, -0.75, 0.9, -0.4)];
            vec![Part::Fill(outline, pieces)]
        }
        SymbolKind::Site => {
            let dot = circle([0.0, 0.0], 0.32, 16);
            vec![Part::Stroke(circle([0.0, 0.0], 0.8, 24), true), Part::Fill(dot.clone(), vec![dot])]
        }
        SymbolKind::Volcano => {
            // a cut-off cone with smoke rising from the crater
            let cone = vec![[-0.95, 0.85], [0.95, 0.85], [0.32, -0.3], [-0.32, -0.3]];
            vec![
                Part::Fill(cone.clone(), vec![cone]),
                Part::Stroke(vec![[0.0, -0.42], [0.0, -0.92]], false),
                Part::Stroke(vec![[-0.3, -0.45], [-0.55, -0.85]], false),
                Part::Stroke(vec![[0.3, -0.45], [0.55, -0.85]], false),
            ]
        }
        SymbolKind::Temple => {
            // a pediment on three columns over a step
            let roof = vec![[-0.95, -0.38], [0.95, -0.38], [0.0, -0.95]];
            let step = rect(-0.95, 0.62, 0.95, 0.88);
            let mut parts = vec![Part::Fill(roof.clone(), vec![roof]), Part::Fill(step.clone(), vec![step])];
            for x in [-0.72, -0.12, 0.48] {
                let col = rect(x, -0.3, x + 0.24, 0.62);
                parts.push(Part::Fill(col.clone(), vec![col]));
            }
            parts
        }
        SymbolKind::Lighthouse => {
            // a tapering tower, its lamp, and two rays
            let tower = vec![[-0.42, 0.88], [0.42, 0.88], [0.22, -0.42], [-0.22, -0.42]];
            let lamp = rect(-0.28, -0.78, 0.28, -0.5);
            vec![
                Part::Fill(tower.clone(), vec![tower]),
                Part::Fill(lamp.clone(), vec![lamp]),
                Part::Stroke(vec![[-0.45, -0.66], [-0.92, -0.8]], false),
                Part::Stroke(vec![[0.45, -0.66], [0.92, -0.8]], false),
            ]
        }
        SymbolKind::Mine => vec![
            // two crossed hammers: handles, and heads at their top ends
            Part::Stroke(vec![[-0.75, 0.82], [0.5, -0.45]], false),
            Part::Stroke(vec![[0.75, 0.82], [-0.5, -0.45]], false),
            Part::Stroke(vec![[0.25, -0.78], [0.8, -0.23]], false),
            Part::Stroke(vec![[-0.25, -0.78], [-0.8, -0.23]], false),
        ],
        SymbolKind::Pass => {
            // two banks facing each other: ")("
            let arc = |side: f32| -> Vec<[f32; 2]> {
                (0..=10)
                    .map(|i| {
                        let t = (-60.0 + 12.0 * i as f32).to_radians();
                        [side * (1.35 - 1.05 * t.cos()), 1.05 * t.sin()]
                    })
                    .collect()
            };
            vec![Part::Stroke(arc(-1.0), false), Part::Stroke(arc(1.0), false)]
        }
        SymbolKind::Camp => {
            // a tent with its door open
            let left = vec![[-0.92, 0.82], [0.0, -0.88], [-0.14, 0.82]];
            let right = vec![[0.14, 0.82], [0.0, -0.88], [0.92, 0.82]];
            vec![Part::Fill(left.clone(), vec![left]), Part::Fill(right.clone(), vec![right])]
        }
    }
}

/// The symbol drawn at `centre` (pixels), `size` its half-height: a white
/// halo under every part first, then the parts in `color`.
pub fn marks(kind: SymbolKind, centre: [f32; 2], size: f32, color: [u8; 4]) -> Vec<Mark> {
    let at = |p: &[f32; 2]| [centre[0] + p[0] * size, centre[1] + p[1] * size];
    let tr = |v: &[[f32; 2]]| v.iter().map(at).collect::<Vec<_>>();
    let width = (STROKE * size).max(1.0);
    let mut halo = Vec::new();
    let mut body = Vec::new();
    for part in parts(kind) {
        match part {
            Part::Fill(outline, pieces) => {
                let outline = tr(&outline);
                halo.push(Mark::Stroke { pts: outline.clone(), closed: true, width: 3.0, color: HALO });
                body.push(Mark::Fill { outline, pieces: pieces.iter().map(|p| tr(p)).collect(), color });
            }
            Part::Stroke(pts, closed) => {
                let pts = tr(&pts);
                halo.push(Mark::Stroke { pts: pts.clone(), closed, width: width + 3.0, color: HALO });
                body.push(Mark::Stroke { pts, closed, width, color });
            }
        }
    }
    halo.extend(body);
    halo
}
