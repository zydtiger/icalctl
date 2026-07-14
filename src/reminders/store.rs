use super::{ReminderAddPatch, ReminderAuthorization, ReminderLifecyclePatch, ReminderSaveDraft};
use crate::models::{ReminderListReport, ReminderReport};
use anyhow::Result;
use chrono::{DateTime, Utc};

pub(super) trait ReminderStore {
    fn authorization_status(&self) -> ReminderAuthorization;
    fn ensure_authorized(&self) -> Result<()>;
    fn lists(&self) -> Result<Vec<ReminderListReport>>;
    fn default_list(&self) -> Result<ReminderListReport>;
    fn fetch(&self, list_ids: &[String]) -> Result<Vec<ReminderReport>>;
    fn get(&self, id: &str) -> Result<ReminderReport>;
    fn create(&self, draft: &ReminderSaveDraft) -> Result<ReminderReport>;
    fn update_add_fields(&self, id: &str, patch: &ReminderAddPatch) -> Result<ReminderReport>;
    fn update(&self, id: &str, patch: &ReminderLifecyclePatch) -> Result<ReminderReport>;
    fn set_completion(
        &self,
        id: &str,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<ReminderReport>;
    fn delete(&self, id: &str) -> Result<()>;
}
