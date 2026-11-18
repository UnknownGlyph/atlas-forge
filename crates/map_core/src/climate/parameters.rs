//! Positive physical quantities use log-linear relations so locked and manual
//! inputs can never drift during a solve. Free dimensions retain their values.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Control {
    Manual,
    Fixed,
    Auto,
}

macro_rules! parameters {
    ($( $id:ident => ($label:literal, $value:expr, $min:expr, $max:expr, $role:ident) ),+ $(,)?) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        #[repr(usize)]
        pub enum Param { $($id),+ }
        pub const PARAMS: &[Param] = &[$(Param::$id),+];
        impl Param {
            pub fn spec(self) -> (&'static str, f64, f64, f64, Control) {
                match self { $(Param::$id => ($label,$value,$min,$max,Control::$role)),+ }
            }
        }
    }
}
parameters! {
    Gravity => ("表面重力（m/s²）",9.80665,0.05,150.0,Auto),
    Density => ("行星平均密度（g/cm³）",5.514,0.1,40.0,Auto),
    Flux => ("轨道半长轴处辐照（W/m²）",1361.0,1.0,30000.0,Auto),
    Period => ("公转周期（地球日）",365.256,0.1,100000.0,Auto),
    Lum => ("恒星光度（太阳倍数）",1.0,0.00001,10000.0,Auto),
    AirDensity => ("近地大气密度（kg/m³，288 K）",1.225,0.001,250.0,Auto),
    Mass => ("行星质量（地球倍数）",1.0,0.001,300.0,Auto),
    Distance => ("轨道半长轴（AU）",1.0,0.015,200.0,Auto),
    StarRadius => ("恒星半径（太阳倍数）",1.0,0.03,20.0,Auto),
    Radius => ("行星半径（地球倍数）",1.0,0.05,12.0,Manual),
    StarTemp => ("恒星表面温度（K）",5772.0,2200.0,30000.0,Manual),
    StarMass => ("恒星质量（太阳倍数）",1.0,0.06,30.0,Manual),
    Pressure => ("地表气压（bar）",1.01325,0.001,120.0,Manual),
    Spin => ("自转周期（小时）",23.934,0.5,2400000.0,Auto),
    Tilt => ("地轴倾角（°）",23.44,0.0,180.0,Manual),
    Ecc => ("轨道偏心率",0.0167,0.0,0.88,Manual),
    Albedo => ("行星反照率",0.30,0.01,0.95,Manual),
    Greenhouse => ("温室增温系数",1.0,0.0,5.0,Auto),
    CO2 => ("二氧化碳（ppm）",420.0,0.0,950000.0,Manual),
    Methane => ("甲烷（ppm）",1.9,0.0,20000.0,Manual),
    Transport => ("大气热量输送（相对值）",1.0,0.03,4.0,Auto),
    Vapour => ("可用水汽（地球倍数）",1.0,0.0,4.0,Auto),
    Ocean => ("未知地表的水体比例",0.50,0.0,1.0,Manual),
    Substellar => ("星下点经度（°）",0.0,-180.0,180.0,Manual),
}
pub const N: usize = PARAMS.len();
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Gas {
    Nitrogen,
    CarbonDioxide,
    Hydrogen,
    Argon,
}
impl Gas {
    pub fn name(self) -> &'static str {
        match self {
            Self::Nitrogen => "氮氧混合",
            Self::CarbonDioxide => "二氧化碳",
            Self::Hydrogen => "氢氦混合",
            Self::Argon => "氩气",
        }
    }
    pub fn molar(self) -> f64 {
        match self {
            Self::Nitrogen => 28.97,
            Self::CarbonDioxide => 44.01,
            Self::Hydrogen => 2.3,
            Self::Argon => 39.95,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Planet {
    pub values: [f64; N],
    pub controls: [Control; N],
    pub locked_rotation: bool,
    pub retrograde: bool,
    pub gas: Gas,
    pub linked: bool,
}
impl Default for Planet {
    fn default() -> Self {
        Self {
            values: std::array::from_fn(|i| PARAMS[i].spec().1),
            controls: std::array::from_fn(|i| PARAMS[i].spec().4),
            locked_rotation: false,
            retrograde: false,
            gas: Gas::Nitrogen,
            linked: true,
        }
    }
}
impl std::ops::Index<Param> for Planet {
    type Output = f64;
    fn index(&self, k: Param) -> &f64 {
        &self.values[k as usize]
    }
}
impl std::ops::IndexMut<Param> for Planet {
    fn index_mut(&mut self, k: Param) -> &mut f64 {
        &mut self.values[k as usize]
    }
}

impl Planet {
    pub fn resolve(&self) -> Result<Self, String> {
        use Param::*;
        for &p in PARAMS {
            let (name, _, lo, hi, _) = p.spec();
            if !self[p].is_finite() || !(lo..=hi).contains(&self[p]) {
                return Err(format!("{name} 超出范围 {lo} … {hi}"));
            }
        }
        if !self.linked {
            return Ok(self.clone());
        }
        // Sum of (coefficient × ln(value / reference)) equals the RHS.
        type LogRelation = (&'static str, Vec<(Param, f64)>, f64);
        let mut equations: Vec<LogRelation> = vec![
            (
                "表面重力",
                vec![(Gravity, 1.), (Mass, -1.), (Radius, 2.)],
                0.,
            ),
            (
                "平均密度",
                vec![(Density, 1.), (Mass, -1.), (Radius, 3.)],
                0.,
            ),
            ("恒星辐照", vec![(Flux, 1.), (Lum, -1.), (Distance, 2.)], 0.),
            (
                "公转周期",
                vec![(Period, 2.), (Distance, -3.), (StarMass, 1.)],
                0.,
            ),
            (
                "恒星光度",
                vec![(Lum, 1.), (StarRadius, -2.), (StarTemp, -4.)],
                0.,
            ),
            (
                "大气密度",
                vec![(AirDensity, 1.), (Pressure, -1.)],
                (self.gas.molar() / 28.97).ln(),
            ),
        ];
        if self.locked_rotation {
            equations.push((
                "同步自转",
                vec![(Spin, 1.), (Period, -1.)],
                (365.256_f64 * 24. / 23.934).ln(),
            ));
        }
        let mut rows = Vec::new();
        for (name, terms, target) in equations {
            let mut row = [0.; N + 1];
            row[N] = target;
            for (p, c) in terms {
                row[N] -= c * (self[p] / p.spec().1).ln();
                if self.controls[p as usize] == Control::Auto {
                    row[p as usize] = c;
                }
            }
            rows.push((name, row));
        }
        let mut pivots = Vec::new();
        // Synchronizing a freely rotating world should change its spin before
        // it changes the orbit. Pivot order is independent of file format.
        for col in std::iter::once(Spin as usize).chain((0..N).filter(|&i| i != Spin as usize)) {
            let n = pivots.len();
            let Some(i) = (n..rows.len()).find(|&i| rows[i].1[col].abs() > 1e-9) else {
                continue;
            };
            rows.swap(n, i);
            let div = rows[n].1[col];
            for v in &mut rows[n].1 {
                *v /= div;
            }
            let pivot = rows[n].1;
            for (i, (_, row)) in rows.iter_mut().enumerate() {
                if i == n {
                    continue;
                }
                let a = row[col];
                for j in 0..=N {
                    row[j] -= a * pivot[j];
                }
            }
            pivots.push(col);
        }
        if let Some((name, _)) = rows[pivots.len()..]
            .iter()
            .find(|(_, row)| row[N].abs() > 1e-6)
        {
            return Err(format!(
                "约束冲突（{name}）。将相关量设为自动、修改固定值，或关闭物理联动。"
            ));
        }
        let mut out = self.clone();
        for (i, col) in pivots.into_iter().enumerate() {
            let p = PARAMS[col];
            let v = self[p] * rows[i].1[N].exp();
            let (name, _, lo, hi, _) = p.spec();
            if !v.is_finite() || v < lo * (1. - 1e-8) || v > hi * (1. + 1e-8) {
                return Err(format!("约束冲突：{name} 将超出可用范围。"));
            }
            out[p] = v.clamp(lo, hi);
        }
        Ok(out)
    }
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Targets {
    pub temperature: Option<f64>,
    pub rain: Option<f64>,
    pub ice: Option<f64>,
    pub dry: Option<f64>,
    pub season: Option<f64>,
}
impl Default for Targets {
    fn default() -> Self {
        Self {
            temperature: Some(15.),
            rain: None,
            ice: None,
            dry: None,
            season: None,
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    pub planet: Planet,
    pub goals: Targets,
}
pub const PRESETS: [&str; 8] = [
    "标准地球",
    "地球重力",
    "地球气候",
    "理想化行星",
    "冰封行星",
    "干旱行星",
    "海洋行星",
    "潮汐锁定行星",
];
pub fn preset(index: usize) -> Settings {
    use Param::*;
    let mut s = Settings::default();
    let p = &mut s.planet;
    match index {
        1 => p.controls[Gravity as usize] = Control::Fixed,
        2 => {
            s.goals = Targets {
                temperature: Some(15.),
                rain: Some(990.),
                ice: Some(0.10),
                dry: Some(0.18),
                season: Some(12.),
            }
        }
        3 => {
            p[Ecc] = 0.;
            p[Ocean] = 0.5;
        }
        4 => {
            p[Albedo] = 0.62;
            p[Greenhouse] = 0.55;
            p[Vapour] = 0.55;
        }
        5 => {
            p[Vapour] = 0.18;
            p[Ocean] = 0.05;
            p[Albedo] = 0.35;
        }
        6 => {
            p[Ocean] = 0.97;
            p[Vapour] = 1.3;
        }
        7 => {
            p.locked_rotation = true;
            p[Tilt] = 0.;
            p[Ecc] = 0.;
        }
        _ => {}
    }
    s.planet = p.resolve().unwrap_or_else(|_| p.clone());
    s
}
