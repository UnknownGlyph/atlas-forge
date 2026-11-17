//! The event timeline ("事件时间线"): named events of a world's history —
//! a war, a treaty, a succession — each a year (or a stretch of years),
//! with notes and, if wanted, the local view (`crate::focus`) where it
//! happened. Events only describe the history: the map's changes stay
//! where they are. A chronicle lists the events in order with what
//! changed in them (in their region, or on the whole map).

use crate::doc::Document;
use crate::focus::FocusReport;
use crate::topo::{FaceId, TResult, BACKGROUND};
use serde::{Deserialize, Serialize};
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub id: u32,
    pub name: String,
    /// Its year, and its last year when it lasted longer.
    pub year: i32,
    #[serde(default)]
    pub end: Option<i32>,
    #[serde(default)]
    pub note: String,
    /// The local view where it happened.
    #[serde(default)]
    pub focus: Option<u32>,
}

impl Event {
    /// Its last year.
    pub fn last(&self) -> i32 {
        self.end.unwrap_or(self.year).max(self.year)
    }
}

/// One entry of a chronicle: an event and what changed during it.
pub struct ChronicleEntry {
    pub event: Event,
    /// Changes from the year before it to its last year.
    pub report: FocusReport,
}

impl Document {
    pub fn event(&self, id: u32) -> Option<&Event> {
        self.events.iter().find(|e| e.id == id)
    }

    /// Add an event (an empty name: "事件 n"); returns its id.
    pub fn add_event(&mut self, name: &str, year: i32, end: Option<i32>) -> u32 {
        let id = self.events.iter().map(|e| e.id).max().map_or(1, |m| m + 1);
        let name = match name.trim() {
            "" => format!("事件 {}", self.events.len() + 1),
            n => n.to_string(),
        };
        self.events.push(Event { id, name, year, end: end.filter(|e| *e > year), note: String::new(), focus: None });
        id
    }

    /// Change an event (found by its id).
    pub fn update_event(&mut self, e: Event) -> TResult<()> {
        if e.name.trim().is_empty() {
            return Err("事件需要名称".into());
        }
        if let Some(f) = e.focus {
            if self.focus(f).is_none() {
                return Err("没有这个局部视图".into());
            }
        }
        let x = self.events.iter_mut().find(|x| x.id == e.id).ok_or("没有这个事件")?;
        *x = Event { name: e.name.trim().to_string(), end: e.end.filter(|z| *z > e.year), ..e };
        Ok(())
    }

    pub fn remove_event(&mut self, id: u32) -> TResult<()> {
        let n = self.events.len();
        self.events.retain(|e| e.id != id);
        if self.events.len() == n {
            return Err("没有这个事件".into());
        }
        Ok(())
    }

    /// The events in order of their years (then names).
    pub fn events_in_order(&self) -> Vec<&Event> {
        let mut v: Vec<&Event> = self.events.iter().collect();
        v.sort_by(|a, b| (a.year, a.last(), &a.name, a.id).cmp(&(b.year, b.last(), &b.name, b.id)));
        v
    }

    /// The chronicle: each event with what changed during it — in its
    /// local view's region when it has one, else on the whole map.
    pub fn chronicle(&self) -> Vec<ChronicleEntry> {
        let all: Vec<FaceId> = self.topo.faces.keys().copied().filter(|f| *f != BACKGROUND).collect();
        self.events_in_order()
            .into_iter()
            .map(|e| {
                let faces = e.focus.and_then(|f| self.focus(f)).map(|f| self.faces_in(f)).unwrap_or_else(|| all.clone());
                ChronicleEntry { event: e.clone(), report: self.change_report(&faces, e.year - 1, e.last()) }
            })
            .collect()
    }

    /// The chronicle as Markdown.
    pub fn chronicle_markdown(&self) -> String {
        let cal = &self.calendar;
        let name = |e: Option<crate::doc::EntityId>| e.and_then(|e| self.entities.get(&e)).map_or("无主".to_string(), |x| x.name.clone());
        let km = |v: f64| {
            let s = (v.round() as i64).abs().to_string();
            let mut o = String::new();
            for (i, c) in s.chars().enumerate() {
                if i > 0 && (s.len() - i).is_multiple_of(3) {
                    o.push(',');
                }
                o.push(c);
            }
            o
        };
        let mut out = format!("# {} 编年表\n\n", self.meta.name);
        for c in self.chronicle() {
            let e = &c.event;
            let when = match e.end {
                Some(z) if z > e.year => format!("{}—{}", cal.show(e.year), cal.show(z)),
                _ => cal.show(e.year),
            };
            let _ = writeln!(out, "## {when}　{}\n", e.name);
            if let Some(f) = e.focus.and_then(|f| self.focus(f)) {
                let _ = writeln!(out, "地区：{}\n", f.name);
            }
            if !e.note.trim().is_empty() {
                let _ = writeln!(out, "{}\n", e.note.trim());
            }
            let r = &c.report;
            if r.transfers.is_empty() && r.to_land == 0.0 && r.to_water == 0.0 {
                out.push_str("（期间领土没有变化）\n\n");
                continue;
            }
            for (a, b, k) in r.transfers.iter().take(12) {
                let _ = writeln!(out, "- {} → {}：{} km²", name(*a), name(*b), km(*k));
            }
            let gains: Vec<String> = r.areas.iter().take(8).map(|(e, a, b)| format!("{} {}{} km²", name(*e), if b > a { "+" } else { "−" }, km(b - a))).collect();
            if !gains.is_empty() {
                let _ = writeln!(out, "\n面积变化：{}", gains.join("，"));
            }
            if r.to_land > 0.0 || r.to_water > 0.0 {
                let _ = writeln!(out, "\n陆地变水域 {} km²，水域变陆地 {} km²", km(r.to_water), km(r.to_land));
            }
            out.push('\n');
        }
        out
    }
}
