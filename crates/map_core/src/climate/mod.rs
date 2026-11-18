//! Optional climate references for worldbuilding. Geography and authored
//! corrections are kept separate from natural simulation results.
mod fit;
mod geography;
mod model;
mod output;
mod parameters;
use crate::LonLat;
pub use fit::fit;
pub use geography::*;
use imbl::OrdMap;
pub use model::*;
pub use parameters::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const W: usize = 96;
pub const H: usize = 48;
pub fn center(i: usize) -> LonLat {
    LonLat::new(
        -180. + (i % W) as f64 * 360. / W as f64 + 180. / W as f64,
        90. - ((i / W) as f64 + 0.5) * 180. / H as f64,
    )
}
pub fn index(p: LonLat) -> usize {
    let x = (((p.lon + 180.).rem_euclid(360.)) * W as f64 / 360.) as usize;
    let y = ((90. - p.lat.clamp(-90., 90.)) * H as f64 / 180.) as usize;
    y.min(H - 1) * W + x.min(W - 1)
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub west: f64,
    pub east: f64,
    pub south: f64,
    pub north: f64,
}
impl Default for Region {
    fn default() -> Self {
        Self {
            west: -180.,
            east: 180.,
            south: -90.,
            north: 90.,
        }
    }
}
impl Region {
    pub fn contains(self, p: LonLat) -> bool {
        p.lat >= self.south
            && p.lat <= self.north
            && if self.west <= self.east {
                p.lon >= self.west && p.lon <= self.east
            } else {
                p.lon >= self.west || p.lon <= self.east
            }
    }
    pub fn validate(self) -> Result<(), String> {
        if self.north < self.south || region_cells(self).next().is_none() {
            Err("所选区域没有气候网格中心，请扩大范围。".into())
        } else {
            Ok(())
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Field {
    #[default]
    Temperature,
    Rain,
    Humidity,
    Wind,
    Ice,
    Range,
    Zone,
    Vegetation,
    Aridity,
}
pub const ZONES: [&str; 7] = [
    "极地",
    "高山",
    "荒漠",
    "草原 / 半干旱",
    "寒温带",
    "温带",
    "热带湿润",
];
pub const VEG: [&str; 10] = [
    "冰原 / 苔原",
    "高山草甸",
    "荒漠植被",
    "草地 / 稀树草原",
    "针叶林",
    "温带森林",
    "热带森林",
    "海洋生态",
    "淡水生态",
    "冰下水域",
];
impl Field {
    pub const ALL: [Self; 9] = [
        Self::Temperature,
        Self::Rain,
        Self::Humidity,
        Self::Wind,
        Self::Ice,
        Self::Range,
        Self::Zone,
        Self::Vegetation,
        Self::Aridity,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Temperature => "温度 °C",
            Self::Rain => "累计降水 mm",
            Self::Humidity => "相对湿度 %",
            Self::Wind => "风 / 水汽输送去向 °",
            Self::Ice => "冰雪覆盖 %",
            Self::Range => "年温差 °C",
            Self::Zone => "气候类型",
            Self::Vegetation => "植被适宜性",
            Self::Aridity => "干湿指数 P/PET",
        }
    }
    pub fn scale(self, period: Period) -> (f32, f32) {
        match self {
            Self::Temperature => (-50., 40.),
            Self::Rain => (0., 2000. * period.month_count() as f32 / 12.),
            Self::Wind => (0., 360.),
            Self::Ice | Self::Humidity => (0., 100.),
            Self::Zone => (0., (ZONES.len() - 1) as f32),
            Self::Vegetation => (0., (VEG.len() - 1) as f32),
            Self::Range => (0., 65.),
            Self::Aridity => (0., 2.),
        }
    }
    pub fn format(self, v: f32) -> String {
        match self {
            Self::Zone => ZONES[(v.round() as usize).min(6)].into(),
            Self::Vegetation => VEG[(v.round() as usize).min(VEG.len() - 1)].into(),
            _ => format!("{v:.1}"),
        }
    }
    pub fn color(self, v: f32, period: Period) -> [u8; 4] {
        let (lo, hi) = self.scale(period);
        let x = ((v - lo) / (hi - lo)).clamp(0., 1.);
        const CLASSES: [[u8; 3]; VEG.len()] = [
            [213, 232, 242],
            [156, 143, 164],
            [216, 171, 92],
            [189, 191, 104],
            [95, 159, 151],
            [78, 159, 99],
            [26, 107, 75],
            [57, 120, 161],
            [85, 174, 187],
            [166, 208, 228],
        ];
        let stops = match self {
            Self::Zone | Self::Vegetation => {
                let [r, g, b] = CLASSES[(v.round() as usize).min(hi as usize)];
                return [r, g, b, 255];
            }
            Self::Wind => [
                [49, 109, 177],
                [81, 186, 133],
                [229, 189, 83],
                [191, 100, 161],
                [49, 109, 177],
            ],
            Self::Temperature | Self::Range => [
                [47, 64, 136],
                [93, 158, 200],
                [224, 232, 201],
                [242, 177, 103],
                [173, 58, 49],
            ],
            Self::Ice => [
                [66, 87, 108],
                [91, 141, 166],
                [137, 188, 203],
                [193, 224, 233],
                [250, 252, 253],
            ],
            _ => [
                [191, 149, 86],
                [221, 205, 151],
                [143, 196, 173],
                [71, 147, 161],
                [43, 79, 129],
            ],
        };
        let f = x * 4.;
        let k = (f as usize).min(3);
        let mut c = [255; 4];
        for (j, v) in c[..3].iter_mut().enumerate() {
            *v = (stops[k][j] as f32 * (1. - (f - k as f32))
                + stops[k + 1][j] as f32 * (f - k as f32)) as u8;
        }
        c
    }
}
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Period {
    #[default]
    Annual,
    Month(u8),
    Season(u8),
}
impl Period {
    pub fn month_count(self) -> usize {
        match self {
            Self::Annual => 12,
            Self::Month(_) => 1,
            Self::Season(_) => 3,
        }
    }
    pub fn months(self) -> impl Iterator<Item = usize> {
        let start = match self {
            Self::Month(m) => usize::from(m) % 12,
            Self::Season(s) => ((usize::from(s) % 4) * 3 + 11) % 12,
            _ => 0,
        };
        (0..self.month_count()).map(move |n| (start + n) % 12)
    }
    pub fn name(self) -> String {
        match self {
            Self::Annual => "全年".into(),
            Self::Month(n) => format!("{} 月", n + 1),
            Self::Season(n) => {
                ["12–2 月", "3–5 月", "6–8 月", "9–11 月"][usize::from(n) % 4].into()
            }
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Override {
    pub values: [f32; 12],
    pub locked: bool,
    pub authored: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Scenario {
    pub settings: Settings,
    pub year: Option<i32>,
    pub region: Region,
    pub cells: Arc<Vec<Cell>>,
    pub edits: OrdMap<(usize, Field), Override>,
}
impl Scenario {
    pub fn new(
        settings: Settings,
        year: Option<i32>,
        region: Region,
        ground: &[Ground],
    ) -> Result<Self, String> {
        region.validate()?;
        let planet = settings.planet.resolve()?;
        Ok(Self {
            cells: Arc::new(calculate(&planet, ground)),
            settings: Settings { planet, ..settings },
            year,
            region,
            edits: Default::default(),
        })
    }
    pub fn retain_locks(&mut self, old: &Self) {
        self.edits = old
            .edits
            .iter()
            .filter(|(_, v)| v.locked)
            .map(|(k, v)| (*k, v.clone()))
            .collect();
    }
    fn month(&self, i: usize, f: Field, m: usize) -> f32 {
        if let Some(v) = self.edits.get(&(i, f)) {
            return v.values[m];
        }
        let cell = &self.cells[i];
        let w = cell.months[m];
        match f {
            Field::Temperature => w.temp,
            Field::Rain => w.rain,
            Field::Humidity => w.humidity,
            Field::Ice => w.ice,
            Field::Wind => w.east.atan2(w.north).to_degrees().rem_euclid(360.),
            Field::Range => {
                let temps = (0..12).map(|n| self.month(i, Field::Temperature, n));
                temps.clone().fold(f32::NEG_INFINITY, f32::max)
                    - temps.fold(f32::INFINITY, f32::min)
            }
            Field::Aridity => {
                let daily = self.month(i, Field::Rain, m) * 12.
                    / self.settings.planet[Param::Period] as f32;
                daily / (1.25 + 0.12 * self.month(i, Field::Temperature, m)).max(0.25)
            }
            Field::Zone | Field::Vegetation => {
                // Broad annual classes; the vegetation overlay describes climatic suitability.
                let t = self.value(i, Field::Temperature, Period::Annual);
                if f == Field::Vegetation
                    && matches!(cell.ground.surface, Surface::Water | Surface::Lake)
                {
                    return if t < -3. {
                        9.
                    } else if cell.ground.surface == Surface::Lake {
                        8.
                    } else {
                        7.
                    };
                }
                let wet = self.value(i, Field::Aridity, Period::Annual);
                if t < -5. {
                    0.
                } else if cell.ground.height > 2400. && t < 12. {
                    1.
                } else if wet < 0.23 {
                    2.
                } else if wet < 0.55 {
                    3.
                } else if t < 6. {
                    4.
                } else if t < 20. {
                    5.
                } else {
                    6.
                }
            }
        }
    }
    pub fn value(&self, i: usize, f: Field, p: Period) -> f32 {
        if matches!(f, Field::Zone | Field::Vegetation | Field::Range)
            && !self.edits.contains_key(&(i, f))
        {
            return self.month(i, f, 0);
        }
        if f == Field::Wind {
            let (mut x, mut y) = (0., 0.);
            for m in p.months() {
                let a = self.month(i, f, m).to_radians();
                let w = self.cells[i].months[m];
                let speed = w.east.hypot(w.north).max(0.01);
                x += a.sin() * speed;
                y += a.cos() * speed;
            }
            return x.atan2(y).to_degrees().rem_euclid(360.);
        }
        if matches!(f, Field::Zone | Field::Vegetation) {
            let mut counts = [0; VEG.len()];
            for m in p.months() {
                counts[(self.month(i, f, m).round() as usize).min(f.scale(p).1 as usize)] += 1;
            }
            return (0..counts.len()).max_by_key(|&k| counts[k]).unwrap() as f32;
        }
        let sum: f32 = p.months().map(|m| self.month(i, f, m)).sum();
        if f == Field::Rain {
            sum
        } else {
            sum / p.month_count() as f32
        }
    }
    pub fn paint(
        &mut self,
        indices: &[usize],
        f: Field,
        period: Period,
        value: Option<f32>,
        lock: bool,
    ) {
        for &i in indices
            .iter()
            .filter(|&&i| i < self.cells.len() && self.region.contains(center(i)))
        {
            let mut values = std::array::from_fn(|m| self.month(i, f, m));
            if let Some(v) = value {
                if !v.is_finite() {
                    continue;
                }
                let v = match f {
                    Field::Rain => v.max(0.) / period.month_count() as f32,
                    Field::Wind => v.rem_euclid(360.),
                    Field::Humidity | Field::Ice => v.clamp(0., 100.),
                    Field::Zone | Field::Vegetation => v.round().clamp(0., f.scale(period).1),
                    Field::Temperature => v.max(-273.15),
                    Field::Range | Field::Aridity => v.max(0.),
                };
                for m in period.months() {
                    values[m] = v;
                }
            }
            let authored = value.is_some() || self.edits.get(&(i, f)).is_some_and(|e| e.authored);
            self.edits.insert(
                (i, f),
                Override {
                    values,
                    locked: lock,
                    authored,
                },
            );
        }
    }
    pub fn image(&self, f: Field, p: Period) -> Vec<u8> {
        let mut out = vec![0; W * H * 4];
        for i in region_cells(self.region) {
            out[i * 4..i * 4 + 4].copy_from_slice(&f.color(self.value(i, f, p), p));
        }
        out
    }
    pub fn summary(&self, area: Region) -> [f64; 5] {
        let mut sums = [0.; 5];
        let mut total = 0.;
        for i in region_cells(area).filter(|&i| self.region.contains(center(i))) {
            let weight = center(i).lat.to_radians().cos();
            total += weight;
            for (k, f) in [
                Field::Temperature,
                Field::Rain,
                Field::Ice,
                Field::Zone,
                Field::Range,
            ]
            .into_iter()
            .enumerate()
            {
                let value = f64::from(self.value(i, f, Period::Annual));
                sums[k] += weight
                    * match f {
                        Field::Ice => value / 100.,
                        Field::Zone => {
                            if value == 2. || value == 3. {
                                1.
                            } else {
                                0.
                            }
                        }
                        _ => value,
                    };
            }
        }
        sums.map(|s| s / total.max(1e-12))
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub settings: Settings,
    pub live: bool,
    pub visible: bool,
    pub opacity: f32,
    pub field: Field,
    pub period: Period,
    pub region: Region,
    pub background: Surface,
    pub surface_edits: OrdMap<usize, SurfaceEdit>,
    pub current: Option<Arc<Scenario>>,
    pub saved: Vec<(String, Arc<Scenario>)>,
    #[serde(default)]
    pub selected: Option<usize>,
}
impl Project {
    pub fn shown(&self) -> Option<&Arc<Scenario>> {
        self.selected
            .and_then(|i| self.saved.get(i).map(|(_, s)| s))
            .or(self.current.as_ref())
    }
    pub fn scenarios(&self) -> impl Iterator<Item = &Arc<Scenario>> {
        self.current.iter().chain(self.saved.iter().map(|(_, s)| s))
    }
    pub fn validate_storage(&self) -> Result<(), String> {
        if self.scenarios().any(|s| {
            s.cells.len() != W * H
                || s.region.validate().is_err()
                || s.edits.keys().any(|(i, _)| *i >= W * H)
        }) {
            return Err("气候参考图层的数据尺寸或范围无效".into());
        }
        Ok(())
    }
}
impl Default for Project {
    fn default() -> Self {
        Self {
            settings: Default::default(),
            live: false,
            visible: true,
            opacity: 0.68,
            field: Default::default(),
            period: Default::default(),
            region: Default::default(),
            background: Default::default(),
            surface_edits: Default::default(),
            current: None,
            saved: vec![],
            selected: None,
        }
    }
}
