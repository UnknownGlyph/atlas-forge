//! Coupled, editable environment layers sampled in geographic coordinates.
mod simulate;
pub use simulate::generate;

use crate::{
    climate::Region,
    geom::{wrap_lon, LonLat},
    Document,
};
use imbl::OrdMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[repr(usize)]
pub enum Field {
    #[default]
    Elevation,
    Slope,
    Landform,
    Drainage,
    Runoff,
    Temperature,
    Rain,
    Wetness,
    Biome,
    Vegetation,
    Diversity,
}

impl Field {
    pub const ALL: [Self; 11] = [
        Self::Elevation,
        Self::Slope,
        Self::Landform,
        Self::Drainage,
        Self::Runoff,
        Self::Temperature,
        Self::Rain,
        Self::Wetness,
        Self::Biome,
        Self::Vegetation,
        Self::Diversity,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::Elevation => "海拔 m",
            Self::Slope => "坡度 °",
            Self::Landform => "地形",
            Self::Drainage => "上游集水面积 km²",
            Self::Runoff => "河网流量 m³/s",
            Self::Temperature => "年平均气温 °C",
            Self::Rain => "年降水量 mm",
            Self::Wetness => "土壤湿润度 %",
            Self::Biome => "生态带",
            Self::Vegetation => "植被生产力 %",
            Self::Diversity => "生物多样性潜势 %",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Elevation => "elevation",
            Self::Slope => "slope",
            Self::Landform => "landform",
            Self::Drainage => "drainage",
            Self::Runoff => "runoff",
            Self::Temperature => "temperature",
            Self::Rain => "rain",
            Self::Wetness => "wetness",
            Self::Biome => "biome",
            Self::Vegetation => "vegetation",
            Self::Diversity => "diversity",
        }
    }

    fn bounds(self) -> (f32, f32) {
        match self {
            Self::Elevation => (-16_000.0, 30_000.0),
            Self::Slope => (0.0, 90.0),
            Self::Landform | Self::Biome => (0.0, 7.0),
            Self::Drainage => (0.0, 1.0e12),
            Self::Runoff => (0.0, 1.0e9),
            Self::Temperature => (-273.15, 500.0),
            Self::Rain => (0.0, 100_000.0),
            Self::Wetness | Self::Vegetation | Self::Diversity => (0.0, 100.0),
        }
    }

    fn normalize(self, value: f32) -> f32 {
        let (low, high) = self.bounds();
        let value = value.clamp(low, high);
        if matches!(self, Self::Landform | Self::Biome) {
            value.round()
        } else {
            value
        }
    }

    pub fn category(self, value: f32) -> Option<&'static str> {
        let labels = match self {
            Self::Landform => [
                "海底", "湖泊", "平原", "丘陵", "高原", "山地", "陡坡", "冰原",
            ],
            Self::Biome => [
                "海洋",
                "淡水",
                "冰原 / 苔原",
                "荒漠",
                "草原",
                "寒温带林",
                "温带林",
                "热带林",
            ],
            _ => return None,
        };
        labels.get(value.round().clamp(0.0, 7.0) as usize).copied()
    }

    pub fn color(self, value: f32) -> [u8; 4] {
        if matches!(self, Self::Landform | Self::Biome) {
            let palette = if self == Self::Landform {
                [
                    [39, 90, 137],
                    [84, 157, 187],
                    [150, 181, 126],
                    [183, 176, 110],
                    [205, 158, 107],
                    [154, 111, 93],
                    [111, 95, 94],
                    [225, 238, 247],
                ]
            } else {
                [
                    [43, 91, 135],
                    [65, 151, 161],
                    [213, 227, 238],
                    [218, 175, 90],
                    [185, 201, 113],
                    [92, 157, 146],
                    [75, 153, 104],
                    [25, 111, 70],
                ]
            };
            let [r, g, b] = palette[value.round().clamp(0.0, 7.0) as usize];
            return [r, g, b, 255];
        }
        let (fraction, colors) = match self {
            Self::Elevation => {
                let value = if value < 0.0 {
                    0.30 + value / 20_000.0
                } else {
                    0.34 + value / 8200.0
                };
                (
                    value,
                    [
                        [28, 67, 127],
                        [99, 153, 187],
                        [120, 164, 102],
                        [185, 146, 100],
                        [241, 235, 230],
                    ],
                )
            }
            Self::Temperature => {
                return crate::climate::Field::Temperature
                    .color(value, crate::climate::Period::Annual)
            }
            Self::Rain => {
                return crate::climate::Field::Rain.color(value, crate::climate::Period::Annual)
            }
            Self::Drainage => (value.ln_1p() / 15., BLUE),
            Self::Runoff => (value.ln_1p() / 9., BLUE),
            Self::Slope => (
                value / 45.,
                [
                    [242, 243, 221],
                    [222, 195, 112],
                    [217, 135, 75],
                    [162, 88, 85],
                    [92, 43, 80],
                ],
            ),
            Self::Wetness => (value / 100.0, BLUE),
            _ => (value / 100.0, GREEN),
        };
        gradient(fraction, colors)
    }
}

const BLUE: [[u8; 3]; 5] = [
    [246, 238, 209],
    [193, 209, 181],
    [122, 183, 178],
    [55, 141, 175],
    [36, 78, 131],
];
const GREEN: [[u8; 3]; 5] = [
    [242, 231, 184],
    [194, 213, 139],
    [125, 181, 127],
    [55, 147, 111],
    [20, 88, 83],
];

fn gradient(t: f32, stops: [[u8; 3]; 5]) -> [u8; 4] {
    let scale = t.clamp(0., 1.) * 4.;
    let idx = (scale as usize).min(3);
    let f = scale - idx as f32;
    let [r, g, b] = std::array::from_fn(|n| {
        (f32::from(stops[idx][n]) * (1. - f) + f32::from(stops[idx + 1][n]) * f).round() as u8
    });
    [r, g, b, 255]
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum Layer {
    Standard(Field),
    Suitability(String),
}

impl Default for Layer {
    fn default() -> Self {
        Self::Standard(Field::default())
    }
}

impl Layer {
    pub fn name(&self) -> &str {
        match self {
            Self::Standard(field) => field.name(),
            Self::Suitability(name) => name,
        }
    }
    pub fn color(&self, value: f32) -> [u8; 4] {
        match self {
            Self::Standard(field) => field.color(value),
            Self::Suitability(_) => gradient(value / 100., GREEN),
        }
    }
    fn normalize(&self, value: f32) -> f32 {
        match self {
            Self::Standard(field) => field.normalize(value),
            Self::Suitability(_) => value.clamp(0., 100.),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Grid {
    pub region: Region,
    pub width: u16,
    pub height: u16,
}

impl Grid {
    pub fn new(region: Region, resolution: u16) -> Result<Self, String> {
        let (height, width) = (region.north - region.south, lon_span(region));
        let largest = height.max(width);
        if !(32..=768).contains(&resolution)
            || ![region.west, region.east]
                .iter()
                .all(|lon| (-180.0..=180.0).contains(lon))
            || ![region.south, region.north]
                .iter()
                .all(|lat| (-90.0..=90.0).contains(lat))
            || !(1e-4..=180.0).contains(&height)
            || !(1e-4..=360.0).contains(&width)
        {
            return Err("生成范围或网格分辨率无效。".into());
        }
        let scale = f64::from(resolution) / largest;
        Ok(Self {
            region,
            width: ((width * scale).round() as u16).max(4),
            height: ((height * scale).round() as u16).max(4),
        })
    }

    pub fn len(self) -> usize {
        self.width as usize * self.height as usize
    }

    pub fn is_empty(self) -> bool {
        self.len() == 0
    }

    pub fn whole_longitude(self) -> bool {
        lon_span(self.region) == 360.0
    }

    pub fn center(self, i: usize) -> LonLat {
        let (w, h) = (usize::from(self.width), usize::from(self.height));
        LonLat::new(
            wrap_lon(self.region.west + lon_span(self.region) * ((i % w) as f64 + 0.5) / w as f64),
            self.region.north
                - (self.region.north - self.region.south) * ((i / w) as f64 + 0.5) / h as f64,
        )
    }

    pub fn index(self, p: LonLat) -> Option<usize> {
        if self.is_empty() || !p.lon.is_finite() || !p.lat.is_finite() {
            return None;
        }
        let x = (p.lon - self.region.west).rem_euclid(360.0) / lon_span(self.region);
        let y = (self.region.north - p.lat) / (self.region.north - self.region.south);
        if !(0.0..=1.0).contains(&x) || !(0.0..=1.0).contains(&y) {
            return None;
        }
        let w = usize::from(self.width);
        let h = usize::from(self.height);
        Some(
            (y.mul_add(h as f64, 0.0) as usize).min(h - 1) * w
                + ((x * w as f64) as usize).min(w - 1),
        )
    }

    pub fn cell_area(self, i: usize, radius: f64) -> f64 {
        let lat = self.center(i).lat;
        let h = (self.region.north - self.region.south) / f64::from(self.height) * 0.5;
        radius.powi(2)
            * (lon_span(self.region) / f64::from(self.width)).to_radians()
            * ((lat + h).to_radians().sin() - (lat - h).to_radians().sin())
    }

    pub fn georef(self) -> crate::underlay::Georef {
        let lon = lon_span(self.region);
        let lon0 = wrap_lon(self.region.west + lon * 0.5);
        let x = f64::from(self.width) / lon.to_radians();
        let y = f64::from(self.height) / (self.region.north - self.region.south).to_radians();
        crate::underlay::Georef {
            proj: crate::projection::Projection::Equirectangular,
            lon0,
            m: [
                [x, 0.0, f64::from(self.width) * 0.5],
                [0.0, -y, y * self.region.north.to_radians()],
            ],
        }
    }
}

fn lon_span(r: Region) -> f64 {
    let delta = r.east - r.west;
    if delta < 0.0 {
        delta + 360.0
    } else {
        delta
    }
}

pub fn from_focus(f: &crate::focus::Focus) -> Region {
    let [west, south, east, north] = f.bounds;
    if east - west >= 360.0 {
        Region {
            south,
            north,
            ..Region::default()
        }
    } else {
        Region {
            west: wrap_lon(west),
            east: wrap_lon(east),
            south,
            north,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Parameter {
    Relief,
    SeaLevel,
    RainScale,
    SoilCapacity,
    HabitatPressure,
}

impl Parameter {
    pub const ALL: [Self; 5] = [
        Self::Relief,
        Self::SeaLevel,
        Self::RainScale,
        Self::SoilCapacity,
        Self::HabitatPressure,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Relief => "起伏尺度 m",
            Self::SeaLevel => "未知区海陆阈值",
            Self::RainScale => "降水倍率",
            Self::SoilCapacity => "土壤蓄水尺度 mm",
            Self::HabitatPressure => "人类扰动度",
        }
    }
    fn range(self) -> std::ops::RangeInclusive<f64> {
        match self {
            Self::Relief => 0.0..=18_000.0,
            Self::SeaLevel => -1.0..=1.0,
            Self::RainScale => 0.0..=10.0,
            Self::SoilCapacity => 10.0..=5000.0,
            Self::HabitatPressure => 0.0..=1.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub parameter: Parameter,
    pub record: crate::worldview::Id,
    pub key: String,
    pub scale: f64,
    pub offset: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Suitability {
    pub name: String,
    pub expression: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub region: Region,
    pub resolution: u16,
    pub seed: u64,
    pub generate_unknown: bool,
    pub relief: f64,
    pub sea_level: f64,
    pub rainfall: f64,
    pub soil_capacity: f64,
    pub habitat_pressure: f64,
    pub rules: Vec<Suitability>,
    pub bindings: Vec<Binding>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            region: Region::default(),
            resolution: 256,
            seed: 4011,
            generate_unknown: true,
            relief: 5700.0,
            sea_level: 0.08,
            rainfall: 1.0,
            soil_capacity: 950.0,
            habitat_pressure: 0.12,
            rules: vec![
                Suitability {
                    name: "淡水供应潜势".into(),
                    expression: "min(100, wetness * 0.55 + 4 * ln(1 + runoff)) * land".into(),
                },
                Suitability {
                    name: "山地矿产潜势".into(),
                    expression: "(min(60, elevation / 60) + noise * 36) * land".into(),
                },
                Suitability {
                    name: "温带森林动物适宜性".into(),
                    expression: "vegetation * (1 - min(1, abs(temperature - 13) / 26)) * land"
                        .into(),
                },
            ],
            bindings: vec![],
        }
    }
}

impl Settings {
    fn get_mut(&mut self, parameter: Parameter) -> &mut f64 {
        match parameter {
            Parameter::Relief => &mut self.relief,
            Parameter::SeaLevel => &mut self.sea_level,
            Parameter::RainScale => &mut self.rainfall,
            Parameter::SoilCapacity => &mut self.soil_capacity,
            Parameter::HabitatPressure => &mut self.habitat_pressure,
        }
    }

    fn resolve(&self, doc: &Document) -> Result<Self, String> {
        let mut settings = self.clone();
        let mut evaluation = crate::worldview::Evaluation::for_environment(doc, doc.year);
        for binding in &self.bindings {
            let resolved = evaluation
                .as_mut()
                .ok_or("关联的世界观设定不存在。")?
                .get(binding.record, &binding.key)?
                .and_then(|r| r.value.as_number())
                .ok_or_else(|| {
                    format!(
                        "设定条目 {} 的「{}」在当前年份没有数值。",
                        binding.record, binding.key
                    )
                })?;
            let value = resolved.mul_add(binding.scale, binding.offset);
            if !binding.parameter.range().contains(&value) {
                return Err(format!(
                    "联动参数「{}」的值 {value} 超出范围。",
                    binding.parameter.name()
                ));
            }
            *settings.get_mut(binding.parameter) = value;
        }
        Ok(settings)
    }

    pub fn validate(&self) -> Result<(), String> {
        Grid::new(self.region, self.resolution)?;
        let values = [
            self.relief,
            self.sea_level,
            self.rainfall,
            self.soil_capacity,
            self.habitat_pressure,
        ];
        if Parameter::ALL
            .into_iter()
            .zip(values)
            .any(|(param, value)| !param.range().contains(&value))
        {
            return Err("环境参数超出允许范围。".into());
        }
        let mut seen = std::collections::BTreeSet::new();
        for binding in &self.bindings {
            if !seen.insert(binding.parameter as u8)
                || binding.record == 0
                || binding.key.trim().is_empty()
                || !binding.scale.is_finite()
                || !binding.offset.is_finite()
            {
                return Err("环境的设定联动有重复参数或无效值。".into());
            }
        }
        if self.rules.len() > 32 {
            return Err("最多保存 32 条资源或物种规则。".into());
        }
        let mut names = std::collections::BTreeSet::new();
        for rule in &self.rules {
            if rule.name.trim().is_empty() || !names.insert(&rule.name) {
                return Err("资源或物种图层名称为空或重复。".into());
            }
            compile(rule)?;
        }
        Ok(())
    }
}

fn compile(rule: &Suitability) -> Result<crate::expr::Ast, String> {
    if rule.expression.len() > 512 {
        return Err("环境公式最多可输入 512 字节。".into());
    }
    let ast = crate::expr::parse(&rule.expression)?;
    let mut vars = Vec::new();
    crate::expr::vars(&ast, &mut vars);
    for var in vars {
        if !(matches!(var.as_str(), "land" | "latitude" | "noise")
            || Field::ALL.iter().any(|field| field.key() == var))
        {
            return Err(format!("环境公式「{}」的变量「{var}」未定义。", rule.name));
        }
    }
    Ok(ast)
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Edit {
    pub at: LonLat,
    pub layer: Layer,
    pub value: f32,
    pub pinned: bool,
}

type Cell = [f32; Field::ALL.len()];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Generation {
    pub grid: Grid,
    pub year: Option<i32>,
    pub settings: Settings,
    pub cells: Vec<Cell>,
    pub downstream: Vec<Option<u32>>,
    pub suitability: std::collections::BTreeMap<String, Vec<f32>>,
}

impl Generation {
    pub fn value(&self, layer: &Layer, i: usize) -> Option<f32> {
        match layer {
            Layer::Standard(field) => Some(self.cells.get(i)?[*field as usize]),
            Layer::Suitability(name) => self.suitability.get(name)?.get(i).copied(),
        }
    }

    pub fn at(&self, layer: &Layer, point: LonLat) -> Option<f32> {
        self.value(layer, self.grid.index(point)?)
    }

    pub fn layers(&self) -> impl Iterator<Item = Layer> + '_ {
        Field::ALL
            .into_iter()
            .map(Layer::Standard)
            .chain(self.suitability.keys().cloned().map(Layer::Suitability))
    }

    pub fn image(&self, layer: &Layer) -> Vec<u8> {
        (0..self.grid.len())
            .flat_map(|i| self.value(layer, i).map_or([0; 4], |x| layer.color(x)))
            .collect()
    }

    fn validate(&self) -> Result<(), String> {
        self.settings.validate()?;
        if self.grid != Grid::new(self.settings.region, self.settings.resolution)?
            || self.cells.len() != self.grid.len()
            || self.downstream.len() != self.grid.len()
            || self.cells.iter().flatten().any(|x| !x.is_finite())
            || self
                .downstream
                .iter()
                .flatten()
                .any(|&i| i as usize >= self.grid.len())
            || self.settings.rules.len() != self.suitability.len()
            || self
                .settings
                .rules
                .iter()
                .any(|r| !self.suitability.contains_key(&r.name))
            || self
                .suitability
                .values()
                .any(|xs| xs.len() != self.grid.len() || xs.iter().any(|x| !x.is_finite()))
        {
            return Err("环境结果的网格、流向或资源图层数据无效。".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Version {
    pub name: String,
    pub result: Arc<Generation>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub settings: Settings,
    pub live: bool,
    pub visible: bool,
    pub opacity: f32,
    pub layer: Layer,
    pub current: Option<Arc<Generation>>,
    pub edits: OrdMap<(u32, u32, Layer), Edit>,
    pub saved: Vec<Version>,
    pub selected: Option<usize>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            live: false,
            visible: true,
            opacity: 0.74,
            layer: Layer::default(),
            current: None,
            edits: OrdMap::new(),
            saved: Vec::new(),
            selected: None,
        }
    }
}

impl Project {
    pub fn shown(&self) -> Option<&Arc<Generation>> {
        self.selected
            .and_then(|i| self.saved.get(i).map(|saved| &saved.result))
            .or(self.current.as_ref())
    }

    pub fn edit_at(
        &mut self,
        at: LonLat,
        layer: Layer,
        value: Option<f32>,
        pin: bool,
    ) -> Result<(), String> {
        let result = self.current.as_ref().ok_or("请先生成环境图层。")?;
        let index = result.grid.index(at).ok_or("此位置不在结果范围内。")?;
        let at = result.grid.center(index);
        let key = (
            ((at.lon + 180.0) * 100_000.0).round() as u32,
            ((at.lat + 90.0) * 100_000.0).round() as u32,
            layer.clone(),
        );
        let value = value.or_else(|| pin.then(|| result.value(&layer, index)).flatten());
        if let Some(value) = value {
            if !value.is_finite() {
                return Err("编辑值无效。".into());
            }
            self.edits.insert(
                key,
                Edit {
                    at,
                    value: layer.normalize(value),
                    layer,
                    pinned: pin,
                },
            );
        } else {
            self.edits.remove(&key);
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), String> {
        self.settings.validate()?;
        if !(0.0..=1.0).contains(&self.opacity)
            || self.selected.is_some_and(|i| i >= self.saved.len())
            || self.edits.values().any(|e| {
                !e.value.is_finite()
                    || !(-180.0..=180.0).contains(&e.at.lon)
                    || !(-90.0..=90.0).contains(&e.at.lat)
            })
        {
            return Err("环境参考图层的编辑数据有误。".into());
        }
        for generation in self
            .current
            .iter()
            .chain(self.saved.iter().map(|v| &v.result))
        {
            generation.validate()?;
        }
        Ok(())
    }
}
