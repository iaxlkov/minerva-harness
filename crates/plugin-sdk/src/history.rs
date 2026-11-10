//! Wire contracts for bounded history queries.
use crate::learning::SourceRef;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryHit {
    pub source_ref: SourceRef,
    pub snippet: String,
    pub surface: HistorySurface,
    pub current_revision: String,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub enum HistorySurface {
    Current,
    Superseded,
    LogOnly,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryPage {
    pub hits: Vec<HistoryHit>,
    pub cursor: Option<HistoryCursor>,
    pub has_more: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryCursor {
    pub generation: i64,
    pub offset: u32,
    pub binding: String,
}
