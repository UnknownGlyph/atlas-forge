//! Saved layouts ("版式"): a named set of map export settings (page size,
//! projection, extent, title, what is drawn) kept in the project, so the
//! same page can be made again for any year and an atlas can be made of
//! several pages. A layout is layout only: it never changes the map.

use crate::doc::Document;
use crate::projection::Projection;
use crate::topo::TResult;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub projection: Projection,
    /// Central meridian.
    pub lon0: f64,
    /// Region in projected units [xmin, ymin, xmax, ymax]; None = whole world.
    pub bounds: Option<[f64; 4]>,
    /// Title; `{year}` is replaced by the year of each page.
    pub title: String,
    pub labels: bool,
    pub legend: bool,
    pub graticule: bool,
    pub scale_bar: bool,
    pub north_arrow: bool,
    /// Where the title, legend, scale bar and north arrow sit, and the
    /// inset maps (set in the layout view; older layouts: the usual places).
    #[serde(default)]
    pub placement: Placement,
}

/// Where a page's furniture sits: each element's box, its top-left corner
/// as fractions of the page (0–1, from the top left); None keeps the
/// element in its usual place. And the inset maps on the page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Placement {
    #[serde(default)]
    pub title: Option<[f32; 2]>,
    #[serde(default)]
    pub legend: Option<[f32; 2]>,
    #[serde(default)]
    pub scale_bar: Option<[f32; 2]>,
    #[serde(default)]
    pub north_arrow: Option<[f32; 2]>,
    #[serde(default)]
    pub insets: Vec<Inset>,
}

/// A small map of another region drawn in a frame on the page (an island
/// group far away, a detail of a crowded coast).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Inset {
    /// The frame on the page: [x, y, w, h] as fractions of the page.
    pub rect: [f32; 4],
    /// The region shown, in the page's projection's units [xmin, ymin, xmax, ymax].
    pub bounds: [f64; 4],
    /// Written in the frame's top-left corner (may be empty).
    #[serde(default)]
    pub title: String,
}

impl Placement {
    /// Whether everything sits in its usual place and there are no insets.
    pub fn is_default(&self) -> bool {
        *self == Placement::default()
    }
}

impl Document {
    /// Save a layout under its name, replacing one of the same name.
    pub fn save_layout(&mut self, l: Layout) -> TResult<()> {
        let name = l.name.trim().to_string();
        if name.is_empty() {
            return Err("版式需要名称".into());
        }
        if l.width < 16 || l.height < 16 {
            return Err("版式尺寸太小".into());
        }
        if let Some(b) = l.bounds {
            if !(b.iter().all(|v| v.is_finite()) && b[2] > b[0] && b[3] > b[1]) {
                return Err("版式范围无效".into());
            }
        }
        for i in &l.placement.insets {
            let (r, b) = (i.rect, i.bounds);
            if !(r[2] > 0.0 && r[3] > 0.0 && r[0] >= -0.01 && r[1] >= -0.01 && r[0] + r[2] <= 1.01 && r[1] + r[3] <= 1.01) {
                return Err("插图框超出了页面".into());
            }
            if !(b.iter().all(|v| v.is_finite()) && b[2] > b[0] && b[3] > b[1]) {
                return Err("插图的范围无效".into());
            }
        }
        let l = Layout { name: name.clone(), ..l };
        match self.layouts.iter_mut().find(|x| x.name == name) {
            Some(x) => *x = l,
            None => self.layouts.push(l),
        }
        Ok(())
    }

    pub fn delete_layout(&mut self, name: &str) -> TResult<()> {
        let n = self.layouts.len();
        self.layouts.retain(|x| x.name != name);
        if self.layouts.len() == n {
            return Err(format!("没有名为“{name}”的版式"));
        }
        Ok(())
    }

    pub fn layout(&self, name: &str) -> Option<&Layout> {
        self.layouts.iter().find(|x| x.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(name: &str, w: u32) -> Layout {
        Layout {
            name: name.into(),
            width: w,
            height: 600,
            projection: Projection::EqualEarth,
            lon0: 0.0,
            bounds: None,
            title: "{year}".into(),
            labels: true,
            legend: true,
            graticule: false,
            scale_bar: true,
            north_arrow: false,
            placement: Placement::default(),
        }
    }

    #[test]
    fn save_replace_and_delete() {
        let mut d = Document::default();
        d.save_layout(l(" 全图 ", 800)).unwrap();
        d.save_layout(l("欧洲", 800)).unwrap();
        // same name: replaced in place
        d.save_layout(l("全图", 1200)).unwrap();
        assert_eq!(d.layouts.iter().map(|x| (x.name.as_str(), x.width)).collect::<Vec<_>>(), [("全图", 1200), ("欧洲", 800)]);
        assert!(d.save_layout(l("  ", 800)).is_err());
        assert!(d.save_layout(l("小", 4)).is_err());
        assert!(d.save_layout(Layout { bounds: Some([1.0, 0.0, 0.0, 1.0]), ..l("坏", 800) }).is_err());
        d.delete_layout("欧洲").unwrap();
        assert!(d.delete_layout("欧洲").is_err());
        assert_eq!(d.layout("全图").unwrap().width, 1200);
        // saved with the project; older projects have none
        let v = serde_json::to_value(&d).unwrap();
        let back: Document = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(back.layouts, d.layouts);
        let mut o = v.as_object().unwrap().clone();
        o.remove("layouts");
        let old: Document = serde_json::from_value(serde_json::Value::Object(o)).unwrap();
        assert!(old.layouts.is_empty());
    }
}
