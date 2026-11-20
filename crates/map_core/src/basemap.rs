//! Map themes ("地图主题"): a whole look for the base map — sea, land,
//! coasts, borders, rivers, the country names' colour and font, the
//! country colours' tone and the bands along the coasts — chosen in one
//! go. A theme is only a set of style values: applying one changes the
//! document's [`Style`] (one undo step), and every value can still be
//! changed by hand afterwards. Drawn features keep their own styles.

use crate::doc::Style;

/// One map theme.
pub struct MapTheme {
    pub name: &'static str,
    pub about: &'static str,
    /// Fonts for the country names, the first this computer has (none: the default font).
    pub fonts: &'static [&'static str],
    look: fn() -> Style,
}

impl MapTheme {
    /// The theme's look (every value it sets; the rest default).
    pub fn style(&self) -> Style {
        (self.look)()
    }

    /// Put the theme's look into `st`, keeping what is not a look (what is
    /// shown, the overlay layer). `font`: the font chosen from its list.
    pub fn apply(&self, st: &mut Style, font: Option<String>) {
        let t = self.style();
        let keep = (st.show_labels, st.show_cities, st.overlay_layer, st.overlay_color, st.overlay_width);
        *st = Style { label_font: font, admin_typography: st.admin_typography, ..t };
        (st.show_labels, st.show_cities, st.overlay_layer, st.overlay_color, st.overlay_width) = keep;
    }

    /// Whether `st` looks as this theme does (fonts aside).
    pub fn matches(&self, st: &Style) -> bool {
        let t = self.style();
        let close = |a: f32, b: f32| (a - b).abs() < 1e-4;
        t.ocean == st.ocean
            && t.land == st.land
            && t.lake == st.lake
            && t.coast == st.coast
            && t.border == st.border
            && t.internal == st.internal
            && t.river == st.river
            && t.label == st.label
            && t.tone == st.tone
            && t.coast_glow == st.coast_glow
            && close(t.tone_mix, st.tone_mix)
            && close(t.coast_glow_width, st.coast_glow_width)
            && close(t.coast_width, st.coast_width)
            && close(t.border_width, st.border_width)
    }
}

fn modern() -> Style {
    Style::default()
}

fn atlas() -> Style {
    Style {
        ocean: [192, 218, 228, 255],
        land: [244, 239, 224, 255],
        lake: [192, 218, 228, 255],
        coast: [66, 104, 134, 255],
        coast_width: 1.0,
        border: [112, 78, 82, 255],
        border_width: 1.3,
        internal: [150, 132, 122, 150],
        internal_width: 0.6,
        river: [86, 134, 184, 255],
        river_width: 1.1,
        label: [62, 42, 34, 255],
        tone: [252, 248, 236, 255],
        tone_mix: 0.45,
        coast_glow: [128, 168, 196, 255],
        coast_glow_width: 9.0,
        ..Style::default()
    }
}

fn parchment() -> Style {
    Style {
        ocean: [206, 192, 154, 255],
        land: [236, 222, 184, 255],
        lake: [192, 176, 138, 255],
        coast: [92, 62, 34, 255],
        coast_width: 1.4,
        border: [112, 72, 42, 255],
        border_width: 1.3,
        internal: [142, 112, 82, 150],
        internal_width: 0.6,
        river: [108, 92, 64, 255],
        river_width: 1.1,
        label: [72, 42, 22, 255],
        tone: [226, 202, 152, 255],
        tone_mix: 0.62,
        coast_glow: [150, 126, 86, 255],
        coast_glow_width: 12.0,
        ..Style::default()
    }
}

fn dark() -> Style {
    Style {
        ocean: [24, 32, 46, 255],
        land: [62, 66, 74, 255],
        lake: [24, 32, 46, 255],
        coast: [122, 162, 200, 255],
        coast_width: 1.0,
        border: [196, 198, 210, 255],
        border_width: 1.0,
        internal: [150, 152, 164, 110],
        internal_width: 0.5,
        river: [78, 138, 206, 255],
        river_width: 1.0,
        label: [234, 236, 242, 255],
        tone: [28, 32, 42, 255],
        tone_mix: 0.42,
        coast_glow: [44, 74, 112, 255],
        coast_glow_width: 8.0,
        ..Style::default()
    }
}

/// The map themes, in the order they are offered.
pub const THEMES: [MapTheme; 4] = [
    MapTheme { name: "现代", about: "清爽的现代地图：浅蓝海洋、原色国家、灰色边界（默认）", fonts: &[], look: modern },
    MapTheme { name: "古典图集", about: "传统地图集：柔和的国家颜色、褐色国界、沿海的蓝色晕带、宋体名称", fonts: &["SimSun", "STSong", "Songti SC", "Noto Serif CJK SC", "Source Han Serif SC"], look: atlas },
    MapTheme { name: "羊皮纸", about: "旧地图：黄褐色纸面、棕色线条与文字、楷体名称", fonts: &["KaiTi", "STKaiti", "Kaiti SC", "AR PL UKai CN"], look: parchment },
    MapTheme { name: "深色", about: "深色背景：暗色海洋、浅色线条与文字，适合屏幕展示", fonts: &[], look: dark },
];

/// The theme `st` looks like, if any.
pub fn current(st: &Style) -> Option<&'static MapTheme> {
    THEMES.iter().find(|t| t.matches(st))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn themes_apply_and_are_recognised() {
        let mut st = Style { show_cities: false, ..Style::default() };
        assert_eq!(current(&st).map(|t| t.name), Some("现代"));
        for t in &THEMES {
            t.apply(&mut st, t.fonts.first().map(|f| f.to_string()));
            assert_eq!(current(&st).map(|x| x.name), Some(t.name));
            assert!(!st.show_cities, "what is shown is kept");
            assert_eq!(st.label_font.as_deref(), t.fonts.first().copied());
        }
        // changed by hand: no longer the theme
        st.border_width += 1.0;
        assert!(current(&st).is_none());
        // the dark theme's names get a dark halo, the others a white one
        assert_eq!(crate::doc::halo_for(THEMES[3].style().label), [20, 22, 28, 255]);
        assert!(THEMES[..3].iter().all(|t| crate::doc::halo_for(t.style().label) == [255, 255, 255, 255]));
    }
}
