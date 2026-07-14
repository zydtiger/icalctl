use super::*;

pub(super) fn print_reminder_batch(batch: &ReminderBatchReport) {
    if batch.dry_run {
        println!("Reminder batch dry run: no Reminders changes were made");
    } else {
        println!("Reminder batch result");
    }
    println!(
        "total={} created={} skipped={} updated={} failed={} not_attempted={} would_create={} would_skip={} would_update={}",
        batch.summary.total,
        batch.summary.created,
        batch.summary.skipped,
        batch.summary.updated,
        batch.summary.failed,
        batch.summary.not_attempted,
        batch.summary.would_create,
        batch.summary.would_skip,
        batch.summary.would_update,
    );
    if !batch.can_write {
        println!("reminder batch is blocked by preflight errors");
    }
    for item in &batch.items {
        let client_id = item
            .client_id
            .as_deref()
            .map(|value| format!(" client_id={value:?}"))
            .unwrap_or_default();
        let reminder_id = item
            .reminder_id
            .as_deref()
            .or(item.matched_reminder_id.as_deref())
            .map(|value| format!(" reminder_id={value}"))
            .unwrap_or_default();
        let error = item
            .error
            .as_ref()
            .map(|value| format!(" error={:?}", value.message))
            .unwrap_or_default();
        println!(
            "{}. {}{}{}{}",
            item.index + 1,
            item.status,
            client_id,
            reminder_id,
            error
        );
    }
}

pub(super) fn print_reminder_mutation_dry_run(draft: &ReminderMutationDraftReport) {
    println!("Dry run: no Reminders changes were made");
    println!("operation: {}", draft.operation);
    println!("reminder id: {}", draft.reminder_id);
    println!("changed fields: {}", draft.changed_fields.join(", "));
    println!("before:");
    print_reminder_detail(&draft.before);
    println!("result:");
    print_reminder_detail(&draft.result);
}

pub(super) fn print_reminder_dry_run(draft: &ReminderDraftReport) {
    println!("Dry run: no Reminders changes were made");
    println!("operation: {}", draft.operation);
    if let Some(id) = &draft.matched_reminder_id {
        println!("matched reminder id: {id}");
    }
    println!("title: {}", draft.title);
    if let Some(parent_id) = &draft.parent_id {
        println!("parent id: {parent_id}");
    }
    println!("list: {} [{}]", draft.list, draft.list_id);
    if let Some(source) = &draft.list_source {
        if let Some(source_id) = &draft.list_source_id {
            println!("list source: {source} [{source_id}]");
        } else {
            println!("list source: {source}");
        }
    }
    println!(
        "list selection: {}",
        match draft.list_selection {
            crate::models::ReminderListSelection::Explicit => "explicit",
            crate::models::ReminderListSelection::ConfiguredDefault => "configured default",
            crate::models::ReminderListSelection::EventkitDefault => "EventKit default",
        }
    );
    println!(
        "due: {}",
        draft
            .due
            .as_ref()
            .map(reminder_date_label)
            .unwrap_or_else(|| "undated".to_string())
    );
    if let Some(input) = &draft.due_input {
        println!("due input: {input}");
    }
    if let Some(start) = &draft.start {
        println!("start: {}", reminder_date_label(start));
    }
    if let Some(input) = &draft.start_input {
        println!("start input: {input}");
    }
    println!(
        "priority: {} ({})",
        reminder_priority_label(&draft.priority),
        draft.priority_value
    );
    println!("notifications: {}", draft.notification_count);
    for notification in &draft.notifications {
        if notification.minutes_before == 0 {
            println!("- at due: {}", notification.absolute_in_due_time_zone);
        } else {
            println!(
                "- {} minutes before: {}",
                notification.minutes_before, notification.absolute_in_due_time_zone
            );
        }
    }
    println!("planned alarms: {}", draft.planned_alarm_count);
    for alarm in &draft.planned_alarms {
        match alarm.kind.as_str() {
            "geofence" => {
                let location = alarm.structured_location.as_ref();
                println!(
                    "- {} at {} ({}, {}) radius={}m",
                    alarm.proximity.as_deref().unwrap_or("location"),
                    location
                        .and_then(|value| value.title.as_deref())
                        .unwrap_or("location"),
                    location
                        .and_then(|value| value.latitude)
                        .unwrap_or_default(),
                    location
                        .and_then(|value| value.longitude)
                        .unwrap_or_default(),
                    location
                        .map(|value| value.radius_meters)
                        .unwrap_or_default()
                );
            }
            _ => println!(
                "- {}: {}",
                alarm.kind,
                alarm.absolute_utc.as_deref().unwrap_or("unknown time")
            ),
        }
    }
    if let Some(recurrence) = &draft.recurrence {
        println!("recurrence: {}", reminder_recurrence_label(recurrence));
    }
    println!(
        "fields: notes={} location={} url={}",
        draft.has_notes, draft.has_location, draft.has_url
    );
    println!(
        "duplicate policy: {} window={} seconds",
        draft.if_exists, draft.duplicate_window_seconds
    );
    for warning in &draft.duplicate_warnings {
        println!("warning: {warning}");
    }
}

pub(super) fn print_reminder_lists(lists: &[ReminderListReport]) {
    println!("Reminder lists ({})", lists.len());
    for list in lists {
        print_reminder_list(list);
    }
}

pub(super) fn print_reminder_list(list: &ReminderListReport) {
    let source = list.source.as_deref().unwrap_or("unknown source");
    let default_marker = if list.is_default_for_new_reminders {
        " default-for-new-reminders=true"
    } else {
        ""
    };
    println!(
        "- {} [{}] source={} writable={} subscribed={} id={}{}",
        list.title,
        list.list_type,
        source,
        list.allows_modifications,
        list.is_subscribed,
        list.id,
        default_marker
    );
}

pub(super) fn print_reminders(reminders: &[ReminderReport]) {
    println!("Reminders ({})", reminders.len());
    if reminders.is_empty() {
        println!("- no reminders found");
        return;
    }
    for (index, reminder) in reminders.iter().enumerate() {
        let state = if reminder.completed {
            "completed"
        } else {
            "incomplete"
        };
        let list = reminder.list.as_deref().unwrap_or("unknown list");
        let due = reminder
            .due
            .as_ref()
            .map(reminder_date_label)
            .unwrap_or_else(|| "undated".to_string());
        let hierarchy = if let Some(parent_id) = &reminder.parent_id {
            format!(" parent={parent_id}")
        } else if reminder.child_count > 0 {
            format!(" children={}", reminder.child_count)
        } else {
            String::new()
        };
        println!(
            "{}. {} [{}] due={} ({}) [{}]{}",
            index + 1,
            reminder.title,
            state,
            due,
            list,
            reminder.id,
            hierarchy
        );
    }
}

pub(super) fn print_reminder_detail(reminder: &ReminderReport) {
    println!("{}", reminder.title);
    println!("id: {}", reminder.id);
    if let Some(parent_id) = &reminder.parent_id {
        println!("parent id: {parent_id}");
    }
    println!("direct children: {}", reminder.child_count);
    if let Some(child_ids) = &reminder.child_ids
        && !child_ids.is_empty()
    {
        println!("child ids:");
        for child_id in child_ids {
            println!("- {child_id}");
        }
    }
    println!(
        "state: {}",
        if reminder.completed {
            "completed"
        } else {
            "incomplete"
        }
    );
    if let Some(completion_date) = &reminder.completion_date {
        println!("completed at: {completion_date}");
    }
    println!(
        "priority: {} ({})",
        reminder_priority_label(&reminder.priority),
        reminder.priority_value
    );
    if let Some(list) = &reminder.list {
        println!("list: {list}");
    }
    if let Some(list_id) = &reminder.list_id {
        println!("list id: {list_id}");
    }
    if let Some(source) = &reminder.list_source {
        if let Some(source_id) = &reminder.list_source_id {
            println!("list source: {source} [{source_id}]");
        } else {
            println!("list source: {source}");
        }
    }
    if let Some(selection) = reminder.list_selection {
        println!(
            "list selection: {}",
            match selection {
                crate::models::ReminderListSelection::Explicit => "explicit",
                crate::models::ReminderListSelection::ConfiguredDefault => "configured default",
                crate::models::ReminderListSelection::EventkitDefault => "EventKit default",
            }
        );
    }
    if let Some(action) = &reminder.write_action {
        println!("write action: {action}");
    }
    if let Some(due) = &reminder.due {
        println!("due: {}", reminder_date_label(due));
    } else {
        println!("due: undated");
    }
    if let Some(input) = &reminder.due_input {
        println!("due input: {input}");
    }
    if let Some(start) = &reminder.start {
        println!("start: {}", reminder_date_label(start));
    }
    if let Some(input) = &reminder.start_input {
        println!("start input: {input}");
    }
    if let Some(location) = &reminder.location {
        println!("location: {location}");
    }
    if let Some(url) = &reminder.url {
        println!("url: {url}");
    }
    if let Some(alarms) = &reminder.alarms {
        println!("alarms: {}", alarms.len());
        for alarm in alarms {
            println!("- {}", reminder_alarm_label(alarm));
        }
    }
    if let Some(rules) = &reminder.recurrence_rules {
        println!("recurrence rules: {}", rules.len());
        for rule in rules {
            println!("- {}", reminder_recurrence_label(rule));
        }
    }
    if let Some(notes) = &reminder.notes {
        println!();
        println!("{notes}");
    }
}

pub(super) fn reminder_date_label(value: &ReminderDateReport) -> String {
    match value.kind {
        ReminderDateKind::Date => value
            .date
            .clone()
            .unwrap_or_else(|| "invalid date".to_string()),
        ReminderDateKind::Datetime => {
            let display = value
                .normalized
                .as_ref()
                .or(value.local.as_ref())
                .cloned()
                .unwrap_or_else(|| "invalid datetime".to_string());
            match &value.time_zone {
                Some(time_zone) => format!("{display} [{time_zone}]"),
                None => display,
            }
        }
    }
}

pub(super) fn reminder_alarm_label(alarm: &ReminderAlarmReport) -> String {
    if let Some(date) = &alarm.absolute_date {
        return format!("{} at {date}", alarm.alarm_type);
    }
    if alarm.proximity != "none" {
        let location = alarm
            .structured_location
            .as_ref()
            .and_then(|location| location.title.as_deref())
            .unwrap_or("location");
        return format!("{} {location}", alarm.proximity);
    }
    if let Some(offset) = alarm.relative_offset_seconds {
        return format!("{} at relative offset {} seconds", alarm.alarm_type, offset);
    }
    alarm.alarm_type.clone()
}

pub(super) fn reminder_priority_label(priority: &crate::models::ReminderPriority) -> &'static str {
    match priority {
        crate::models::ReminderPriority::None => "none",
        crate::models::ReminderPriority::High => "high",
        crate::models::ReminderPriority::Medium => "medium",
        crate::models::ReminderPriority::Low => "low",
    }
}

pub(super) fn reminder_recurrence_label(rule: &ReminderRecurrenceReport) -> String {
    let end = match &rule.end {
        ReminderRecurrenceEndReport {
            occurrence_count: Some(count),
            ..
        } => format!("count={count}"),
        ReminderRecurrenceEndReport {
            end_date: Some(until),
            ..
        } => format!("until={until}"),
        _ => "never".to_string(),
    };
    format!("every {} {} ({end})", rule.interval, rule.frequency)
}
