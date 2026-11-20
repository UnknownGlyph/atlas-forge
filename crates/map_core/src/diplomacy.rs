//! Historical relations and agreements between political entities.
use crate::doc::{Document, EntityId, EntityKind, POLITICAL};
use imbl::OrdMap;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, sync::Arc};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Alliance,
    #[default]
    Cooperation,
    Trade,
    Neutral,
    Tension,
    War,
    Other,
}

impl Kind {
    pub const ALL: [Self; 7] = [
        Self::Alliance,
        Self::Cooperation,
        Self::Trade,
        Self::Neutral,
        Self::Tension,
        Self::War,
        Self::Other,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Alliance => "同盟",
            Self::Cooperation => "合作",
            Self::Trade => "贸易",
            Self::Neutral => "中立",
            Self::Tension => "紧张",
            Self::War => "战争",
            Self::Other => "自定义",
        }
    }
    pub fn color(self) -> [u8; 3] {
        match self {
            Self::Alliance => [27, 118, 109],
            Self::Cooperation | Self::Trade => [33, 99, 155],
            Self::Neutral | Self::Other => [90, 95, 104],
            Self::Tension => [168, 104, 34],
            Self::War => [176, 52, 60],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Agreement {
    pub name: String,
    pub kind: Kind,
    pub parties: BTreeSet<EntityId>,
    pub from: Option<i32>,
    pub until: Option<i32>,
    pub event: Option<u32>,
    pub note: String,
}

impl Agreement {
    pub fn visible(&self, year: Option<i32>) -> bool {
        year.is_none_or(|y| {
            self.from.is_none_or(|from| y >= from) && self.until.is_none_or(|end| y <= end)
        })
    }

    fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() {
            return Err("外交关系需要名称".into());
        }
        if self.parties.len() < 2 {
            return Err("至少选择两个国家".into());
        }
        if matches!((self.from, self.until), (Some(a), Some(b)) if a > b) {
            return Err("结束年不得早于起始年".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Diplomacy {
    pub records: OrdMap<u64, Agreement>,
    next: u64,
}

impl Diplomacy {
    pub fn validate(&self) -> Result<(), String> {
        for agreement in self.records.values() {
            agreement.validate()?;
        }
        if self.records.keys().any(|id| *id >= self.next) {
            return Err("外交记录标识序列无效".into());
        }
        Ok(())
    }
}

impl Document {
    pub fn save_agreement(&mut self, id: Option<u64>, agreement: Agreement) -> Result<u64, String> {
        agreement.validate()?;
        let previous = self
            .diplomacy
            .as_ref()
            .and_then(|d| id.and_then(|id| d.records.get(&id)));
        if id.is_some() && previous.is_none() {
            return Err("外交关系已不存在".into());
        }
        for party in &agreement.parties {
            if previous.is_some_and(|old| old.parties.contains(party)) {
                continue;
            }
            if self
                .entities
                .get(party)
                .is_none_or(|e| e.kind != EntityKind::Country || e.layer != POLITICAL)
            {
                return Err("请选择政治归属层中的国家".into());
            }
        }
        let diplomacy = Arc::make_mut(
            self.diplomacy
                .get_or_insert_with(|| Arc::new(Diplomacy::default())),
        );
        let id = match id {
            Some(id) => id,
            None => {
                let id = diplomacy.next;
                diplomacy.next = id.checked_add(1).ok_or("外交记录标识已用尽")?;
                id
            }
        };
        diplomacy.records.insert(id, agreement);
        Ok(id)
    }

    pub fn remove_agreement(&mut self, id: u64) -> Result<(), String> {
        self.diplomacy
            .as_mut()
            .and_then(|d| Arc::make_mut(d).records.remove(&id))
            .map(|_| ())
            .ok_or_else(|| "外交关系已不存在".into())
    }

    pub fn diplomatic_name(&self, id: EntityId) -> String {
        self.entities
            .get(&id)
            .map_or_else(|| format!("已删除国家 #{}", id.0), |e| e.name.clone())
    }
}
