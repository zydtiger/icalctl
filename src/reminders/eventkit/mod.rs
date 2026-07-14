mod conversion;

pub(super) use conversion::*;

use super::{
    ReminderAddPatch, ReminderLifecyclePatch, ReminderSaveDraft, ReminderStore,
    reminders_with_hierarchy, set_reminderkit_parent,
};
use crate::models::{ReminderListReport, ReminderReport};
use anyhow::{Context, Result, anyhow, bail};
use block2::RcBlock;
use chrono::{DateTime, Utc};
use objc2::Message;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2_event_kit::{EKAuthorizationStatus, EKCalendar, EKEntityType, EKEventStore, EKReminder};
use objc2_foundation::{NSArray, NSError, NSString};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReminderAuthorization {
    NotDetermined,
    Restricted,
    Denied,
    FullAccess,
    WriteOnly,
    Unknown,
}

impl ReminderAuthorization {
    pub(crate) fn current() -> Self {
        let status =
            unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Reminder) };
        Self::from_ek(status)
    }

    pub(super) fn from_ek(status: EKAuthorizationStatus) -> Self {
        if status == EKAuthorizationStatus::NotDetermined {
            Self::NotDetermined
        } else if status == EKAuthorizationStatus::Restricted {
            Self::Restricted
        } else if status == EKAuthorizationStatus::Denied {
            Self::Denied
        } else if status == EKAuthorizationStatus::FullAccess {
            Self::FullAccess
        } else if status == EKAuthorizationStatus::WriteOnly {
            Self::WriteOnly
        } else {
            Self::Unknown
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NotDetermined => "NotDetermined",
            Self::Restricted => "Restricted",
            Self::Denied => "Denied",
            Self::FullAccess => "FullAccess",
            Self::WriteOnly => "WriteOnly",
            Self::Unknown => "Unknown",
        }
    }
}

pub(super) struct EventKitReminderStore {
    store: Retained<EKEventStore>,
}

impl EventKitReminderStore {
    pub(super) fn new() -> Self {
        Self {
            store: unsafe { EKEventStore::new() },
        }
    }

    fn request_access(&self) -> Result<bool> {
        let result = Arc::new((Mutex::new(None::<(bool, Option<String>)>), Condvar::new()));
        let callback_result = Arc::clone(&result);
        let completion = RcBlock::new(move |granted: Bool, error: *mut NSError| {
            let error = if error.is_null() {
                None
            } else {
                Some(format!("{:?}", unsafe { &*error }))
            };
            let (lock, cvar) = &*callback_result;
            let mut state = lock.lock().expect("reminder authorization lock poisoned");
            *state = Some((granted.as_bool(), error));
            cvar.notify_one();
        });
        unsafe {
            let block_ptr = &*completion as *const _ as *mut _;
            self.store
                .requestFullAccessToRemindersWithCompletion(block_ptr);
        }
        let (lock, cvar) = &*result;
        let mut state = lock
            .lock()
            .map_err(|_| anyhow!("reminder authorization lock poisoned"))?;
        while state.is_none() {
            state = cvar
                .wait(state)
                .map_err(|_| anyhow!("reminder authorization lock poisoned"))?;
        }
        match state.take() {
            Some((granted, None)) => Ok(granted),
            Some((_, Some(error))) => bail!("failed to request full Reminders access: {error}"),
            None => bail!("failed to request full Reminders access"),
        }
    }

    fn ek_lists(&self) -> Vec<Retained<EKCalendar>> {
        unsafe { self.store.calendarsForEntityType(EKEntityType::Reminder) }
            .iter()
            .map(|list| list.retain())
            .collect()
    }

    fn fetch_reports(&self, list_ids: &[String], details: bool) -> Result<Vec<ReminderReport>> {
        let selected: Vec<Retained<EKCalendar>> = if list_ids.is_empty() {
            self.ek_lists()
        } else {
            let all = self.ek_lists();
            let mut selected = Vec::new();
            for id in list_ids {
                let list = all
                    .iter()
                    .find(|list| unsafe { list.calendarIdentifier() }.to_string() == *id)
                    .with_context(|| format!("reminder list is no longer available: {id}"))?;
                selected.push(list.clone());
            }
            selected
        };
        let array = NSArray::from_retained_slice(&selected);
        let predicate = unsafe { self.store.predicateForRemindersInCalendars(Some(&array)) };
        let result = Arc::new((Mutex::new(None::<Vec<ReminderReport>>), Condvar::new()));
        let callback_result = Arc::clone(&result);
        let completion = RcBlock::new(move |reminders: *mut NSArray<EKReminder>| {
            let reports = if reminders.is_null() {
                Vec::new()
            } else {
                let reminders = unsafe { Retained::retain(reminders) }
                    .expect("EventKit returned a dangling reminders array");
                let reminders = reminders
                    .iter()
                    .map(|reminder| reminder.retain())
                    .collect::<Vec<_>>();
                reminders_with_hierarchy(&reminders, details)
            };
            let (lock, cvar) = &*callback_result;
            let mut state = lock.lock().expect("reminder fetch lock poisoned");
            *state = Some(reports);
            cvar.notify_one();
        });
        unsafe {
            self.store
                .fetchRemindersMatchingPredicate_completion(&predicate, &completion);
        }
        let (lock, cvar) = &*result;
        let mut state = lock
            .lock()
            .map_err(|_| anyhow!("reminder fetch lock poisoned"))?;
        while state.is_none() {
            state = cvar
                .wait(state)
                .map_err(|_| anyhow!("reminder fetch lock poisoned"))?;
        }
        state
            .take()
            .ok_or_else(|| anyhow!("EventKit reminder fetch returned no result"))
    }

    fn report_by_id(&self, id: &str) -> Result<ReminderReport> {
        self.fetch_reports(&[], true)?
            .into_iter()
            .find(|reminder| reminder.id == id)
            .with_context(|| format!("reminder is no longer available: {id}"))
    }
}

impl ReminderStore for EventKitReminderStore {
    fn authorization_status(&self) -> ReminderAuthorization {
        ReminderAuthorization::current()
    }

    fn ensure_authorized(&self) -> Result<()> {
        match self.authorization_status() {
            ReminderAuthorization::FullAccess => Ok(()),
            ReminderAuthorization::NotDetermined => {
                if self.request_access()? {
                    Ok(())
                } else {
                    bail!("full Reminders access was denied")
                }
            }
            ReminderAuthorization::Denied => bail!(
                "Reminders access is denied; enable it in System Settings > Privacy & Security > Reminders"
            ),
            ReminderAuthorization::Restricted => {
                bail!("Reminders access is restricted by system policy")
            }
            ReminderAuthorization::WriteOnly => {
                bail!("full Reminders access is required for reminder reads")
            }
            ReminderAuthorization::Unknown => bail!("unknown Reminders authorization status"),
        }
    }

    fn lists(&self) -> Result<Vec<ReminderListReport>> {
        Ok(self
            .ek_lists()
            .iter()
            .map(|list| reminder_list_report(list, false))
            .collect())
    }

    fn default_list(&self) -> Result<ReminderListReport> {
        let list = unsafe { self.store.defaultCalendarForNewReminders() }
            .context("EventKit did not return a default reminder list")?;
        Ok(reminder_list_report(&list, true))
    }

    fn fetch(&self, list_ids: &[String]) -> Result<Vec<ReminderReport>> {
        self.fetch_reports(list_ids, false)
            .context("failed to fetch reminders through EventKit")
    }

    fn get(&self, id: &str) -> Result<ReminderReport> {
        self.report_by_id(id)
    }

    fn create(&self, draft: &ReminderSaveDraft) -> Result<ReminderReport> {
        self.ensure_authorized()?;
        let list_id = NSString::from_str(&draft.list_id);
        let list = unsafe { self.store.calendarWithIdentifier(&list_id) }
            .context("selected reminder list is no longer available")?;
        if !unsafe { list.allowsContentModifications() } {
            bail!("selected reminder list is read-only");
        }
        let reminder = unsafe { EKReminder::reminderWithEventStore(&self.store) };
        let title = NSString::from_str(&draft.title);
        unsafe {
            reminder.setTitle(Some(&title));
            reminder.setCalendar(Some(&list));
            reminder.setPriority(draft.priority_value);
        }
        if let Some(due) = &draft.due {
            let components = reminder_date_components(&due.components)?;
            unsafe { reminder.setDueDateComponents(Some(&components)) };
        }
        if let Some(start) = &draft.start {
            let components = reminder_date_components(&start.components)?;
            unsafe { reminder.setStartDateComponents(Some(&components)) };
        }
        if let Some(notes) = &draft.notes {
            let notes = NSString::from_str(notes);
            unsafe { reminder.setNotes(Some(&notes)) };
        }
        if let Some(location) = &draft.location {
            let location = NSString::from_str(location);
            unsafe { reminder.setLocation(Some(&location)) };
        }
        if let Some(url) = &draft.url {
            set_reminder_url(&reminder, url)?;
        }
        add_reminder_notifications(&reminder, &draft.notifications);
        if let Some(geofence) = &draft.geofence {
            add_reminder_geofence(&reminder, geofence);
        }
        if let Some(recurrence) = &draft.recurrence {
            set_reminder_recurrence(&reminder, Some(recurrence));
        }
        self.save_reminder(&reminder)?;
        let id = unsafe { reminder.calendarItemIdentifier() }.to_string();
        if let Some(parent_id) = &draft.parent_id
            && let Err(error) = self.set_parent_relationship(&id, Some(parent_id))
        {
            let rollback = self.remove_reminder_by_id(&id);
            return match rollback {
                Ok(()) => Err(error.context(
                    "failed to create native child reminder; the flat reminder was rolled back",
                )),
                Err(rollback_error) => Err(error.context(format!(
                    "failed to create native child reminder and failed to roll back flat reminder {id}: {rollback_error:#}"
                ))),
            };
        }
        self.report_by_id(&id)
    }

    fn update_add_fields(&self, id: &str, patch: &ReminderAddPatch) -> Result<ReminderReport> {
        self.ensure_authorized()?;
        let reminder = self.find_reminder(id)?;
        if let Some(start) = &patch.start {
            let components = reminder_date_components(&start.components)?;
            unsafe { reminder.setStartDateComponents(Some(&components)) };
        }
        if let Some(notes) = &patch.notes {
            let notes = NSString::from_str(notes);
            unsafe { reminder.setNotes(Some(&notes)) };
        }
        if let Some(location) = &patch.location {
            let location = NSString::from_str(location);
            unsafe { reminder.setLocation(Some(&location)) };
        }
        if let Some(url) = &patch.url {
            set_reminder_url(&reminder, url)?;
        }
        if let Some(priority) = patch.priority_value {
            unsafe { reminder.setPriority(priority) };
        }
        if let Some(notifications) = &patch.notifications {
            unsafe { reminder.setAlarms(None) };
            add_reminder_notifications(&reminder, notifications);
            if let Some(geofence) = patch.geofence.as_ref().and_then(Option::as_ref) {
                add_reminder_geofence(&reminder, geofence);
            }
        }
        if let Some(recurrence) = &patch.recurrence {
            set_reminder_recurrence(&reminder, Some(recurrence));
        }
        self.save_reminder(&reminder)?;
        if let Some(parent_id) = &patch.parent_id {
            self.set_parent_relationship(id, Some(parent_id))?;
        }
        self.report_by_id(id)
    }

    fn update(&self, id: &str, patch: &ReminderLifecyclePatch) -> Result<ReminderReport> {
        self.ensure_authorized()?;
        let reminder = self.find_reminder(id)?;
        if !unsafe { reminder.calendar() }
            .as_ref()
            .is_some_and(|list| unsafe { list.allowsContentModifications() })
        {
            bail!("the reminder's current list is read-only");
        }
        if let Some(title) = &patch.title {
            let title = NSString::from_str(title);
            unsafe { reminder.setTitle(Some(&title)) };
        }
        if let Some(list_id) = &patch.list_id {
            let list_id = NSString::from_str(list_id);
            let list = unsafe { self.store.calendarWithIdentifier(&list_id) }
                .context("selected reminder list is no longer available")?;
            if !unsafe { list.allowsContentModifications() } {
                bail!("selected reminder list is read-only");
            }
            unsafe { reminder.setCalendar(Some(&list)) };
        }
        if let Some(due) = &patch.due {
            let components = due
                .as_ref()
                .map(|value| reminder_date_components(&value.components))
                .transpose()?;
            unsafe { reminder.setDueDateComponents(components.as_deref()) };
        }
        if let Some(start) = &patch.start {
            let components = start
                .as_ref()
                .map(|value| reminder_date_components(&value.components))
                .transpose()?;
            unsafe { reminder.setStartDateComponents(components.as_deref()) };
        }
        if let Some(notes) = &patch.notes {
            let notes = notes.as_ref().map(|value| NSString::from_str(value));
            unsafe { reminder.setNotes(notes.as_deref()) };
        }
        if let Some(location) = &patch.location {
            let location = location.as_ref().map(|value| NSString::from_str(value));
            unsafe { reminder.setLocation(location.as_deref()) };
        }
        if let Some(url) = &patch.url {
            match url {
                Some(url) => set_reminder_url(&reminder, url)?,
                None => unsafe { reminder.setURL(None) },
            }
        }
        if let Some(priority) = patch.priority_value {
            unsafe { reminder.setPriority(priority) };
        }
        if let Some(notifications) = &patch.notifications {
            unsafe { reminder.setAlarms(None) };
            add_reminder_notifications(&reminder, notifications);
            if let Some(geofence) = patch.geofence.as_ref().and_then(Option::as_ref) {
                add_reminder_geofence(&reminder, geofence);
            }
        }
        if let Some(recurrence) = &patch.recurrence {
            set_reminder_recurrence(&reminder, recurrence.as_ref());
        }
        self.save_reminder(&reminder)?;
        if let Some(parent_id) = &patch.parent_id {
            self.set_parent_relationship(id, parent_id.as_deref())?;
        }
        self.report_by_id(id)
    }

    fn set_completion(
        &self,
        id: &str,
        completed_at: Option<DateTime<Utc>>,
    ) -> Result<ReminderReport> {
        self.ensure_authorized()?;
        let reminder = self.find_reminder(id)?;
        if !unsafe { reminder.calendar() }
            .as_ref()
            .is_some_and(|list| unsafe { list.allowsContentModifications() })
        {
            bail!("the reminder's current list is read-only");
        }
        match completed_at {
            Some(completed_at) => {
                let date = nsdate_from_utc(completed_at);
                unsafe { reminder.setCompletionDate(Some(&date)) };
            }
            None => unsafe { reminder.setCompletionDate(None) },
        }
        self.save_reminder(&reminder)?;
        self.report_by_id(id)
    }

    fn delete(&self, id: &str) -> Result<()> {
        self.ensure_authorized()?;
        let reminder = self.find_reminder(id)?;
        if !unsafe { reminder.calendar() }
            .as_ref()
            .is_some_and(|list| unsafe { list.allowsContentModifications() })
        {
            bail!("the reminder's current list is read-only");
        }
        unsafe {
            self.store
                .removeReminder_commit_error(&reminder, true)
                .map_err(|error| anyhow!("failed to delete reminder: {error:?}"))?;
            self.store.refreshSourcesIfNecessary();
        }
        Ok(())
    }
}

impl EventKitReminderStore {
    fn set_parent_relationship(&self, child_id: &str, parent_id: Option<&str>) -> Result<()> {
        set_reminderkit_parent(child_id, parent_id)
    }

    fn remove_reminder_by_id(&self, id: &str) -> Result<()> {
        let reminder = self.find_reminder(id)?;
        unsafe {
            self.store
                .removeReminder_commit_error(&reminder, true)
                .map_err(|error| anyhow!("failed to remove reminder: {error:?}"))?;
            self.store.refreshSourcesIfNecessary();
        }
        Ok(())
    }

    fn find_reminder(&self, id: &str) -> Result<Retained<EKReminder>> {
        unsafe { self.store.refreshSourcesIfNecessary() };
        let id = NSString::from_str(id);
        let item = unsafe { self.store.calendarItemWithIdentifier(&id) }
            .context("reminder is no longer available")?;
        item.downcast_ref::<EKReminder>()
            .map(Message::retain)
            .context("the selected EventKit item is an event, not a reminder")
    }

    fn save_reminder(&self, reminder: &EKReminder) -> Result<()> {
        unsafe {
            self.store
                .saveReminder_commit_error(reminder, true)
                .map_err(|error| anyhow!("failed to save reminder: {error:?}"))?;
            self.store.refreshSourcesIfNecessary();
        }
        Ok(())
    }
}
