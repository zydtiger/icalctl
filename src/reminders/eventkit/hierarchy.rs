use super::conversion::reminder_to_report;
use crate::models::ReminderReport;
use anyhow::{Context, Result, bail};
use objc2::msg_send;
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_event_kit::EKReminder;
use objc2_foundation::{NSError, NSString};

pub(super) fn reminders_with_hierarchy(
    reminders: &[Retained<EKReminder>],
    details: bool,
) -> Vec<ReminderReport> {
    let public_ids = reminders
        .iter()
        .map(|reminder| unsafe { reminder.calendarItemIdentifier() }.to_string())
        .collect::<Vec<_>>();
    let parent_ids = public_ids
        .iter()
        .map(|id| reminderkit_parent_id(id).ok().flatten())
        .collect::<Vec<_>>();
    let mut children = vec![Vec::<String>::new(); reminders.len()];
    for (child_index, parent_id) in parent_ids.iter().enumerate() {
        if let Some(parent_index) = parent_id
            .as_deref()
            .and_then(|parent_id| public_ids.iter().position(|id| id == parent_id))
        {
            children[parent_index].push(public_ids[child_index].clone());
        }
    }
    for child_ids in &mut children {
        child_ids.sort();
    }

    reminders
        .iter()
        .enumerate()
        .map(|(index, reminder)| {
            let mut report = reminder_to_report(reminder, details);
            report.parent_id = parent_ids[index].clone();
            report.child_count = children[index].len();
            report.child_ids = details.then(|| children[index].clone());
            report
        })
        .collect()
}

pub(in crate::reminders) fn private_class(name: &str) -> Result<&'static AnyClass> {
    let class = match name {
        "NSUUID" => AnyClass::get(c"NSUUID"),
        "REMReminder" => AnyClass::get(c"REMReminder"),
        "REMSaveRequest" => AnyClass::get(c"REMSaveRequest"),
        "REMStore" => AnyClass::get(c"REMStore"),
        _ => None,
    };
    class.with_context(|| format!("private macOS class {name} is unavailable"))
}

pub(super) fn reminderkit_store() -> Result<Retained<AnyObject>> {
    let class = private_class("REMStore")?;
    Ok(unsafe { msg_send![class, new] })
}

pub(super) fn reminderkit_reminder(store: &AnyObject, id: &str) -> Result<Retained<AnyObject>> {
    let uuid_class = private_class("NSUUID")?;
    let id = NSString::from_str(id);
    let uuid: Retained<AnyObject> =
        unsafe { msg_send![msg_send![uuid_class, alloc], initWithUUIDString: &*id] };
    let reminder_class = private_class("REMReminder")?;
    let object_id: Retained<AnyObject> =
        unsafe { msg_send![reminder_class, objectIDWithUUID: &*uuid] };
    let reminder: Option<Retained<AnyObject>> = unsafe {
        msg_send![store, fetchReminderWithObjectID: &*object_id, error: std::ptr::null_mut::<*mut NSError>()]
    };
    reminder.with_context(|| format!("ReminderKit could not find reminder {id}"))
}

pub(super) fn reminderkit_parent_id(id: &str) -> Result<Option<String>> {
    let store = reminderkit_store()?;
    let reminder = reminderkit_reminder(&store, id)?;
    let parent: Option<Retained<AnyObject>> = unsafe { msg_send![&*reminder, parentReminder] };
    let Some(parent) = parent else {
        return Ok(None);
    };
    let storage: Retained<AnyObject> = unsafe { msg_send![&*parent, storage] };
    let object_id: Retained<AnyObject> = unsafe { msg_send![&*storage, objectID] };
    let uuid: Retained<AnyObject> = unsafe { msg_send![&*object_id, uuid] };
    let uuid_string: Retained<NSString> = unsafe { msg_send![&*uuid, UUIDString] };
    Ok(Some(uuid_string.to_string()))
}

pub(super) fn set_reminderkit_parent(child_id: &str, parent_id: Option<&str>) -> Result<()> {
    let store = reminderkit_store()?;
    let child = reminderkit_reminder(&store, child_id)?;
    let save_class = private_class("REMSaveRequest")?;
    let request: Retained<AnyObject> =
        unsafe { msg_send![msg_send![save_class, alloc], initWithStore: &*store] };
    let child_change: Retained<AnyObject> =
        unsafe { msg_send![&*request, updateReminder: &*child] };
    if let Some(parent_id) = parent_id {
        let parent = reminderkit_reminder(&store, parent_id)?;
        let parent_change: Retained<AnyObject> =
            unsafe { msg_send![&*request, updateReminder: &*parent] };
        let context: Retained<AnyObject> = unsafe { msg_send![&*parent_change, subtaskContext] };
        let _: () = unsafe { msg_send![&*context, addReminderChangeItem: &*child_change] };
    } else {
        let _: () = unsafe { msg_send![&*child_change, removeFromParentReminder] };
    }
    let saved: Bool = unsafe {
        msg_send![&*request, saveSynchronouslyWithError: std::ptr::null_mut::<*mut NSError>()]
    };
    if !saved.as_bool() {
        bail!("ReminderKit failed to save the reminder hierarchy");
    }
    Ok(())
}
