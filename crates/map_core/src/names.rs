//! Name generator ("名称生成器"): made-up names for countries, cities,
//! regions, seas, rivers, mountains and people, offline, in a handful of
//! styles. A name is put together from syllables (or characters) of its
//! style and, for some kinds, a suffix ("王国", "城", "河"…). The same seed
//! gives the same names; `unique` leaves out names already taken.

use crate::sample::Rng;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NameStyle {
    /// Two classical characters, as in Chinese place names.
    Huaxia,
    /// Transliterated European-sounding names.
    Western,
    /// Transliterated northern names.
    Nordic,
    /// Transliterated desert and steppe names.
    Desert,
    /// Two characters in an eastern-island manner.
    Eastern,
    /// Latin letters.
    Latin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NameKind {
    Country,
    City,
    Region,
    Sea,
    River,
    Mountain,
    Person,
}

impl NameStyle {
    pub const ALL: [NameStyle; 6] = [NameStyle::Huaxia, NameStyle::Western, NameStyle::Nordic, NameStyle::Desert, NameStyle::Eastern, NameStyle::Latin];

    pub fn name(self) -> &'static str {
        match self {
            NameStyle::Huaxia => "华夏古风",
            NameStyle::Western => "西式音译",
            NameStyle::Nordic => "北地音译",
            NameStyle::Desert => "沙海音译",
            NameStyle::Eastern => "东方岛国",
            NameStyle::Latin => "拉丁字母",
        }
    }
}

impl NameKind {
    pub const ALL: [NameKind; 7] = [NameKind::Country, NameKind::City, NameKind::Region, NameKind::Sea, NameKind::River, NameKind::Mountain, NameKind::Person];

    pub fn name(self) -> &'static str {
        match self {
            NameKind::Country => "国家",
            NameKind::City => "城市",
            NameKind::Region => "地区",
            NameKind::Sea => "海洋",
            NameKind::River => "河流",
            NameKind::Mountain => "山脉",
            NameKind::Person => "人名",
        }
    }
}

// characters and syllables of each style
const HUAXIA_A: &[&str] = &["青", "云", "安", "永", "长", "临", "东", "西", "南", "北", "平", "清", "白", "赤", "玄", "苍", "丹", "金", "玉", "华", "明", "宁", "昌", "广", "定", "兴", "镇", "靖", "怀", "绥", "凤", "龙", "鹤", "鹿", "雁", "松", "柏", "桐", "梧", "兰", "芷", "沧", "澜", "汀", "洛", "渭", "泾", "汶", "沂", "岷", "岱", "嵩", "衡", "峨", "崇", "朔", "幽", "瀚", "嘉", "宜"];
const HUAXIA_B: &[&str] = &["川", "原", "阳", "陵", "江", "州", "泽", "岭", "峰", "林", "关", "塞", "宁", "安", "平", "丰", "和", "庆", "泉", "溪", "湖", "海", "山", "岗", "谷", "源", "浦", "滨", "台", "城", "都", "京", "邑", "门", "梁", "桥", "渡", "汉", "夏", "晋", "楚", "越", "吴", "齐", "燕", "赵", "徽", "秦", "雍", "冀"];
const HUAXIA_GIVEN: &[&str] = &["子", "文", "武", "仲", "伯", "季", "思", "明", "允", "承", "景", "怀", "德", "仁", "义", "礼", "智", "信", "安", "宁", "嘉", "敬", "远", "清", "云", "霄", "瑾", "瑜", "琰", "璋", "昭", "晟", "曦", "澄", "渊", "岳", "峻", "珩", "瑶", "婉", "姝", "蓁", "芸", "蘅", "若", "衡"];
const HUAXIA_FAMILY: &[&str] = &["李", "王", "张", "刘", "陈", "杨", "赵", "周", "吴", "孙", "沈", "韩", "萧", "裴", "崔", "卢", "谢", "陆", "顾", "司马", "上官", "欧阳", "慕容", "宇文", "独孤"];

const WESTERN: &[&str] = &["阿", "尔", "德", "兰", "维", "斯", "特", "里", "亚", "克", "罗", "诺", "卡", "萨", "莱", "恩", "托", "伦", "加", "巴", "马", "蒂", "奥", "埃", "菲", "格", "瑞", "琳", "娜", "瓦", "索", "贝", "塔", "西", "尼", "利", "芬", "波", "米", "雷", "伊", "安", "柯", "赛", "丹", "布", "泰", "拉"];
const WESTERN_END: &[&str] = &["亚", "尼亚", "里亚", "兰", "斯", "顿", "堡", "维尔", "加德", "伦", "提亚", "隆", "诺", "多", "萨"];
const NORDIC: &[&str] = &["斯", "卡", "格", "尔", "海", "姆", "维", "克", "约", "恩", "索", "瓦", "比", "约", "托", "拉", "弗", "雷", "伊", "芬", "布", "隆", "达", "霍", "冈", "奥", "瑟", "奥丁", "西", "吉", "赫", "伦", "峡", "冰"];
const NORDIC_END: &[&str] = &["海姆", "加德", "维克", "斯塔", "伦德", "堡", "冈", "斯卡", "约德", "霍尔姆", "菲约", "尔"];
const DESERT: &[&str] = &["阿", "尔", "萨", "哈", "拉", "卡", "米", "扎", "法", "穆", "苏", "巴", "达", "希", "贾", "纳", "库", "塔", "伊", "兹", "沙", "鲁", "哈", "马", "因", "奥", "尔", "迪", "塞", "乌", "耶", "卡"];
const DESERT_END: &[&str] = &["斯坦", "巴德", "卡尔", "扎尔", "拉", "汗", "沙", "兰", "迪尔", "布尔", "马", "亚"];
const EASTERN_A: &[&str] = &["朝", "夕", "春", "秋", "樱", "雾", "月", "星", "风", "雪", "桜", "藤", "松", "竹", "梅", "菊", "水", "岩", "石", "浅", "深", "高", "远", "千", "八", "常", "若", "白", "黑", "碧", "茜", "苍", "日", "天", "叶", "花"];
const EASTERN_B: &[&str] = &["野", "原", "津", "岛", "濑", "川", "崎", "冈", "泽", "宫", "井", "谷", "浦", "滨", "田", "木", "森", "山", "城", "畑", "江", "叶", "道", "关", "门"];
const LATIN_START: &[&str] = &["Al", "Ar", "Bel", "Cal", "Dor", "El", "Fal", "Gal", "Hal", "Is", "Kar", "Lor", "Mar", "Nor", "Or", "Pel", "Quel", "Ral", "Sar", "Tal", "Ul", "Val", "Wes", "Ys", "Zar", "Ther", "Bran", "Cor", "Dra", "Eld", "Fen", "Gil", "Hel", "Ith", "Lun", "Mor", "Ny", "Ost", "Rhy", "Syl", "Tor", "Vey"];
const LATIN_MID: &[&str] = &["a", "e", "i", "o", "u", "ae", "ia", "io", "ea", "an", "en", "in", "on", "ar", "er", "or", "el", "il", "ul", "and", "orn", "ess", "ith", "ar", "ad", "ev"];
const LATIN_END: &[&str] = &["ia", "or", "ar", "en", "on", "is", "us", "heim", "mark", "gard", "dale", "mere", "wick", "holt", "ford", "ton", "ia", "ea", "ane", "ost", "rin", "dor", "ath", "and"];

/// Syllables a transliterated name does not begin with.
const NO_START: &[&str] = &["尔", "恩", "姆"];

fn pick<'a>(rng: &mut Rng, from: &[&'a str]) -> &'a str {
    from[((rng.next_f64() * from.len() as f64) as usize).min(from.len() - 1)]
}

/// A word of `n` syllables from `syl`, ended by one of `end` (if any).
fn word(rng: &mut Rng, syl: &[&str], end: &[&str], n: usize) -> String {
    let mut w = String::new();
    let mut last = "";
    for i in 0..n {
        // no syllable twice in a row, and none that cannot begin a name
        let mut s = pick(rng, syl);
        for _ in 0..8 {
            if s != last && !(i == 0 && NO_START.contains(&s)) {
                break;
            }
            s = pick(rng, syl);
        }
        w.push_str(s);
        last = s;
    }
    if !end.is_empty() {
        let e = pick(rng, end);
        if !w.ends_with(e) {
            w.push_str(e);
        }
    }
    w
}

/// The bare name in a style (no suffix).
fn stem(style: NameStyle, rng: &mut Rng) -> String {
    match style {
        NameStyle::Huaxia => {
            let a = pick(rng, HUAXIA_A);
            let mut b = pick(rng, HUAXIA_B);
            while b == a {
                b = pick(rng, HUAXIA_B);
            }
            format!("{a}{b}")
        }
        NameStyle::Western => {
            let n = 1 + (rng.next_f64() * 2.0) as usize;
            word(rng, WESTERN, WESTERN_END, n)
        }
        NameStyle::Nordic => {
            let n = 1 + (rng.next_f64() * 2.0) as usize;
            word(rng, NORDIC, NORDIC_END, n)
        }
        NameStyle::Desert => {
            let n = 1 + (rng.next_f64() * 2.0) as usize;
            word(rng, DESERT, DESERT_END, n)
        }
        NameStyle::Eastern => {
            let a = pick(rng, EASTERN_A);
            let b = pick(rng, EASTERN_B);
            format!("{a}{b}")
        }
        NameStyle::Latin => {
            let mut w = pick(rng, LATIN_START).to_string();
            if rng.next_f64() < 0.6 {
                w.push_str(pick(rng, LATIN_MID));
            }
            w.push_str(pick(rng, LATIN_END));
            w
        }
    }
}

/// One name of a kind in a style.
pub fn generate(style: NameStyle, kind: NameKind, rng: &mut Rng) -> String {
    let latin = style == NameStyle::Latin;
    let suffix = |rng: &mut Rng, cn: &[&'static str], en: &[&'static str]| -> &'static str {
        if latin {
            pick(rng, en)
        } else {
            pick(rng, cn)
        }
    };
    match kind {
        NameKind::Person => match style {
            NameStyle::Huaxia => {
                let f = pick(rng, HUAXIA_FAMILY);
                let g = pick(rng, HUAXIA_GIVEN);
                if rng.next_f64() < 0.6 {
                    format!("{f}{g}{}", pick(rng, HUAXIA_GIVEN))
                } else {
                    format!("{f}{g}")
                }
            }
            NameStyle::Eastern => format!("{}{}·{}{}", pick(rng, EASTERN_A), pick(rng, EASTERN_B), pick(rng, EASTERN_A), pick(rng, EASTERN_A)),
            NameStyle::Latin => format!("{} {}", stem(style, rng), stem(style, rng)),
            // given name and family name of plain syllables (no place endings)
            _ => {
                let syl = match style {
                    NameStyle::Nordic => NORDIC,
                    NameStyle::Desert => DESERT,
                    _ => WESTERN,
                };
                let (a, b) = (2 + (rng.next_f64() * 2.0) as usize, 2 + (rng.next_f64() * 2.0) as usize);
                format!("{}·{}", word(rng, syl, &[], a), word(rng, syl, &[], b))
            }
        },
        NameKind::Country => {
            let s = stem(style, rng);
            let kinds_cn: &[&str] = match style {
                NameStyle::Huaxia | NameStyle::Eastern => &["国", "国", "国", "朝", "邦", "王国"],
                NameStyle::Desert => &["王国", "汗国", "苏丹国", "酋长国", "联邦"],
                NameStyle::Nordic => &["王国", "公国", "联盟", "共和国"],
                _ => &["王国", "公国", "共和国", "联邦", "帝国", "大公国"],
            };
            if latin {
                // "Kingdom of …", or the bare name
                let pre = pick(rng, &["", "", "Kingdom of ", "Republic of ", "Duchy of "]);
                format!("{pre}{s}")
            } else {
                format!("{s}{}", pick(rng, kinds_cn))
            }
        }
        NameKind::City => {
            let s = stem(style, rng);
            if latin || rng.next_f64() < 0.5 {
                return s;
            }
            match style {
                NameStyle::Huaxia => format!("{s}{}", pick(rng, &["城", "州", "府", "镇"])),
                NameStyle::Eastern => format!("{s}{}", pick(rng, &["町", "城", "港"])),
                _ => format!("{s}{}", pick(rng, &["堡", "港", "城"])),
            }
        }
        NameKind::Region => format!("{}{}", stem(style, rng), suffix(rng, &["省", "郡", "道", "州", "地区", "领"], &[""])),
        NameKind::Sea => {
            let s = stem(style, rng);
            if latin {
                format!("Sea of {s}")
            } else {
                format!("{s}{}", pick(rng, &["海", "洋", "湾", "海峡"]))
            }
        }
        NameKind::River => {
            let s = stem(style, rng);
            if latin {
                format!("{s} River")
            } else {
                format!("{s}{}", pick(rng, &["河", "江", "水"]))
            }
        }
        NameKind::Mountain => {
            let s = stem(style, rng);
            if latin {
                format!("{s} Mountains")
            } else {
                format!("{s}{}", pick(rng, &["山脉", "山", "岭"]))
            }
        }
    }
}

/// `n` different names not in `taken`, from a seed (fewer if the style
/// runs out of names after many tries).
pub fn unique(style: NameStyle, kind: NameKind, n: usize, seed: u64, taken: &HashSet<String>) -> Vec<String> {
    let mut rng = Rng::new(seed.wrapping_add(style as u64 * 1_000_003 + kind as u64 * 7919));
    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = taken.clone();
    let mut tries = 0;
    while out.len() < n && tries < n * 200 + 200 {
        tries += 1;
        let name = generate(style, kind, &mut rng);
        if seen.insert(name.clone()) {
            out.push(name);
        }
    }
    out
}

impl crate::doc::Document {
    /// Give unit `e` a name style (None: none of its own).
    pub fn set_name_style(&mut self, e: crate::doc::EntityId, style: Option<NameStyle>) -> crate::topo::TResult<()> {
        if !self.entities.contains_key(&e) {
            return Err("实体不存在".into());
        }
        match style {
            Some(s) => self.name_styles.insert(e, s),
            None => self.name_styles.remove(&e),
        };
        Ok(())
    }

    /// The style new names for `e` should follow, in the selected year: its
    /// own; else that of a unit with a style covering it in another layer
    /// (a culture): for a city the unit where it stands, for a country or
    /// region the unit over most of its land. None when nothing says.
    pub fn name_style_for(&self, e: crate::doc::EntityId) -> Option<NameStyle> {
        if let Some(s) = self.name_styles.get(&e) {
            return Some(*s);
        }
        let x = self.entities.get(&e)?;
        let mut layers: Vec<crate::doc::LayerId> = self.name_styles.keys().filter(|u| **u != e).filter_map(|u| self.entities.get(u)).map(|u| u.layer).filter(|l| *l != x.layer).collect();
        layers.sort();
        layers.dedup();
        let r = self.radius();
        for l in layers {
            let owners = self.layer_owner(l);
            let styled = |f: &crate::topo::FaceId| owners.get(f).and_then(|u| self.name_styles.get(u).map(|s| (*u, *s)));
            let found = if x.kind == crate::doc::EntityKind::City {
                x.pos.and_then(|p| styled(&self.topo.face_at(p))).map(|(_, s)| s)
            } else {
                let mut tally: std::collections::BTreeMap<crate::doc::EntityId, (f64, NameStyle)> = std::collections::BTreeMap::new();
                for f in self.faces_of_tree(e) {
                    if let Some((u, s)) = styled(&f) {
                        tally.entry(u).or_insert((0.0, s)).0 += self.topo.face_area(f, r);
                    }
                }
                tally.into_values().max_by(|a, b| a.0.total_cmp(&b.0)).map(|v| v.1)
            };
            if found.is_some() {
                return found;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_of_every_style_and_kind() {
        for style in NameStyle::ALL {
            for kind in NameKind::ALL {
                let v = unique(style, kind, 30, 7, &HashSet::new());
                assert_eq!(v.len(), 30, "{} {}", style.name(), kind.name());
                assert!(v.iter().all(|n| !n.trim().is_empty() && n.chars().count() <= 24), "{v:?}");
                // the same seed, the same names
                assert_eq!(v, unique(style, kind, 30, 7, &HashSet::new()));
            }
        }
        // suffixes of their kind
        let rivers = unique(NameStyle::Huaxia, NameKind::River, 20, 3, &HashSet::new());
        assert!(rivers.iter().all(|n| n.ends_with('河') || n.ends_with('江') || n.ends_with('水')));
        let latin = unique(NameStyle::Latin, NameKind::City, 20, 3, &HashSet::new());
        assert!(latin.iter().all(|n| n.is_ascii()));
        // names already taken are left out
        let first = unique(NameStyle::Western, NameKind::Country, 10, 1, &HashSet::new());
        let taken: HashSet<String> = first[..5].iter().cloned().collect();
        let again = unique(NameStyle::Western, NameKind::Country, 10, 1, &taken);
        assert!(again.iter().all(|n| !taken.contains(n)));
    }
}
