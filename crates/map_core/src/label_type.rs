//! Shared map typography for the viewport and exporters.
use crate::doc::Style;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    Country,
    Province,
    District,
    Local,
    Capital,
    City,
}

impl Tier {
    pub const ALL: [Self; 6] = [
        Self::Country,
        Self::Province,
        Self::District,
        Self::Local,
        Self::Capital,
        Self::City,
    ];

    pub fn administrative(depth: usize) -> Self {
        match depth {
            0 => Self::Country,
            1 => Self::Province,
            2 => Self::District,
            _ => Self::Local,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LabelType<'a> {
    pub font: Option<&'a str>,
    pub bold: bool,
    pub size: f32,
}

impl Style {
    pub fn label_type(&self, tier: Tier) -> LabelType<'_> {
        if !self.admin_typography {
            let city = matches!(tier, Tier::City | Tier::Capital);
            return LabelType {
                font: if city {
                    None
                } else {
                    self.label_font.as_deref()
                },
                bold: !city && self.label_bold,
                size: match tier {
                    Tier::Country => 14.0,
                    Tier::Province => 13.0,
                    Tier::District => 12.0,
                    Tier::Local => 11.0,
                    _ => 12.0,
                },
            };
        }
        let (family, bold, size) = match tier {
            Tier::Country => (self.label_font.as_deref().unwrap_or("SimHei"), true, 23.0),
            Tier::Province => ("SimSun", false, 15.0),
            Tier::District => ("KaiTi", false, 13.0),
            Tier::Local => ("Microsoft YaHei", false, 11.0),
            Tier::Capital => ("SimHei", true, 13.0),
            Tier::City => ("Microsoft YaHei", false, 10.5),
        };
        LabelType {
            font: Some(family),
            bold,
            size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn political_names_have_distinct_faces_and_hierarchy() {
        let st = Style {
            admin_typography: true,
            ..Default::default()
        };
        let tiers = [Tier::Country, Tier::Province, Tier::District, Tier::Local]
            .map(|tier| st.label_type(tier));
        for i in 0..tiers.len() - 1 {
            assert_ne!(tiers[i].font, tiers[i + 1].font);
            assert!(tiers[i].size > tiers[i + 1].size);
        }
    }
}
