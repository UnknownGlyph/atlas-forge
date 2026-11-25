//! Pointer interactions belong to the current tool and board. IDs can be
//! reused on other boards, so discard these handles together at a transition.
use eframe::egui::{Pos2, Vec2};
use map_core::{doc::LineId, redraw::{DragFrom, Plan}, EntityId, LonLat, VertexRef};

#[derive(Default)]
pub(crate) struct Gestures {
    pub city: Option<EntityId>,
    pub vertex: Option<VertexRef>,
    pub river_point: Option<(LineId, usize)>,
    pub area_point: Option<(LineId, usize)>,
    pub label: Option<(EntityId, Vec2)>,
    pub annotation: Option<(LineId, Vec2)>,
    pub symbol: Option<(LineId, Vec2)>,
    pub curve: Option<crate::curve_ui::CurveDrag>,
    pub features: Option<crate::select_ui::MoveDrag>,
    pub dated_vertex: Option<(DragFrom, Option<(LonLat, Result<Plan, String>)>)>,
    pub draft_point: Option<crate::draw_ui::DraftDrag>,
    pub brush_stroke: Vec<Pos2>,
    pub marquee: Option<(usize, Pos2, Pos2)>,
}
