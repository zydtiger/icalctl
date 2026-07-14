use super::EventDraftReport;
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct BatchReport {
    pub dry_run: bool,
    pub can_write: bool,
    pub if_exists: String,
    pub continue_on_error: bool,
    pub summary: BatchSummaryReport,
    pub items: Vec<BatchItemReport>,
}

#[derive(Debug, Default, Serialize)]
pub struct BatchSummaryReport {
    pub total: usize,
    pub created: usize,
    pub skipped: usize,
    pub updated: usize,
    pub failed: usize,
    pub not_attempted: usize,
    pub would_create: usize,
    pub would_skip: usize,
    pub would_update: usize,
}

#[derive(Debug, Serialize)]
pub struct BatchItemReport {
    pub index: usize,
    pub client_id: Option<String>,
    pub status: String,
    pub event_id: Option<String>,
    pub matched_event_id: Option<String>,
    pub draft: Option<Box<EventDraftReport>>,
    pub error: Option<BatchErrorReport>,
}

#[derive(Debug, Serialize)]
pub struct BatchErrorReport {
    pub message: String,
}
