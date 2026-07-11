use crate::models::{
    AlarmReport, BatchReport, CalendarReport, EventDraftReport, EventRecurrenceReport, EventReport,
    JsonOutput, ReminderAlarmReport, ReminderBatchReport, ReminderDateKind, ReminderDateReport,
    ReminderDraftReport, ReminderListReport, ReminderMutationDraftReport,
    ReminderRecurrenceEndReport, ReminderRecurrenceReport, ReminderReport,
};
use chrono::DateTime;

pub fn print_human_output(output: &JsonOutput) {
    match output {
        JsonOutput::Status(status) => {
            println!("Calendar authorization: {}", status.authorization);
        }
        JsonOutput::Doctor { doctor } => {
            println!("EventKit diagnostics");
            println!("Calendar authorization: {}", doctor.authorization);
            println!(
                "Reminders authorization: {}",
                doctor.reminders_authorization
            );
            println!("Location authorization: {}", doctor.location_authorization);
            println!(
                "Location services enabled: {}",
                doctor.location_services_enabled
            );
            println!(
                "process: {} [{}]",
                doctor.process.executable, doctor.process.pid
            );
            if let Some(terminal) = &doctor.process.terminal_program {
                println!("terminal program: {terminal}");
            }
            println!(
                "embedded Info.plist: {} bundle={}",
                doctor.info_plist.embedded, doctor.info_plist.bundle_identifier
            );
            println!("recommended command: {}", doctor.recommended_command);
            for step in &doctor.remediation {
                println!("- {step}");
            }
            println!(
                "recommended Reminders command: {}",
                doctor.recommended_reminders_command
            );
            for step in &doctor.reminders_remediation {
                println!("- {step}");
            }
            for step in &doctor.location_remediation {
                println!("- {step}");
            }
        }
        JsonOutput::Calendars { calendars } => {
            println!("Calendars ({})", calendars.len());
            for calendar in calendars {
                print_calendar(calendar);
            }
        }
        JsonOutput::DefaultCalendar { calendar } => {
            println!("Default calendar for new events");
            print_calendar(calendar);
        }
        JsonOutput::ReminderStatus(status) => {
            println!("Reminders authorization: {}", status.authorization);
        }
        JsonOutput::ReminderLists { lists } => print_reminder_lists(lists),
        JsonOutput::DefaultReminderList { list } => {
            println!("Default list for new reminders");
            print_reminder_list(list);
        }
        JsonOutput::Reminders { reminders } => print_reminders(reminders),
        JsonOutput::Reminder { reminder } => print_reminder_detail(reminder),
        JsonOutput::ReminderDryRun { draft, .. } => print_reminder_dry_run(draft),
        JsonOutput::ReminderMutationDryRun { draft, .. } => print_reminder_mutation_dry_run(draft),
        JsonOutput::ReminderDeleted { deleted } => {
            println!("Deleted reminder: {} [{}]", deleted.title, deleted.id);
        }
        JsonOutput::ReminderBatch { batch } => print_reminder_batch(batch),
        JsonOutput::Events { events } => print_events(events),
        JsonOutput::Event { event } => print_event_detail(event),
        JsonOutput::DryRun { draft, .. } => print_dry_run(draft),
        JsonOutput::Batch { batch } => print_batch(batch),
        JsonOutput::Deleted { deleted } => {
            println!("Deleted event: {} [{}]", deleted.title, deleted.id);
            if let Some(scope) = &deleted.scope {
                println!("scope: {scope}");
            }
        }
    }
}

fn print_reminder_batch(batch: &ReminderBatchReport) {
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

fn print_reminder_mutation_dry_run(draft: &ReminderMutationDraftReport) {
    println!("Dry run: no Reminders changes were made");
    println!("operation: {}", draft.operation);
    println!("reminder id: {}", draft.reminder_id);
    println!("changed fields: {}", draft.changed_fields.join(", "));
    println!("before:");
    print_reminder_detail(&draft.before);
    println!("result:");
    print_reminder_detail(&draft.result);
}

fn print_reminder_dry_run(draft: &ReminderDraftReport) {
    println!("Dry run: no Reminders changes were made");
    println!("operation: {}", draft.operation);
    if let Some(id) = &draft.matched_reminder_id {
        println!("matched reminder id: {id}");
    }
    println!("title: {}", draft.title);
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

fn print_reminder_lists(lists: &[ReminderListReport]) {
    println!("Reminder lists ({})", lists.len());
    for list in lists {
        print_reminder_list(list);
    }
}

fn print_reminder_list(list: &ReminderListReport) {
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

fn print_reminders(reminders: &[ReminderReport]) {
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
        println!(
            "{}. {} [{}] due={} ({}) [{}]",
            index + 1,
            reminder.title,
            state,
            due,
            list,
            reminder.id
        );
    }
}

fn print_reminder_detail(reminder: &ReminderReport) {
    println!("{}", reminder.title);
    println!("id: {}", reminder.id);
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

fn reminder_date_label(value: &ReminderDateReport) -> String {
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

fn reminder_alarm_label(alarm: &ReminderAlarmReport) -> String {
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

fn reminder_priority_label(priority: &crate::models::ReminderPriority) -> &'static str {
    match priority {
        crate::models::ReminderPriority::None => "none",
        crate::models::ReminderPriority::High => "high",
        crate::models::ReminderPriority::Medium => "medium",
        crate::models::ReminderPriority::Low => "low",
    }
}

fn reminder_recurrence_label(rule: &ReminderRecurrenceReport) -> String {
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

fn print_batch(batch: &BatchReport) {
    if batch.dry_run {
        println!("Batch dry run: no Calendar changes were made");
    } else {
        println!("Batch result");
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
        println!("batch is blocked by preflight errors");
    }

    for item in &batch.items {
        let client_id = item
            .client_id
            .as_deref()
            .map(|value| format!(" client_id={value:?}"))
            .unwrap_or_default();
        let event_id = item
            .event_id
            .as_deref()
            .or(item.matched_event_id.as_deref())
            .map(|value| format!(" event_id={value}"))
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
            event_id,
            error
        );
    }
}

fn print_dry_run(draft: &EventDraftReport) {
    println!("Dry run: no Calendar changes were made");
    println!("operation: {}", draft.operation);
    if let Some(scope) = &draft.scope {
        println!("scope: {scope}");
    }
    if let Some(event_id) = &draft.event_id {
        println!("event id: {event_id}");
    }
    println!("title: {}", draft.title);
    println!("time: {} to {}", draft.start, draft.end);
    if let (Some(start_input), Some(end_input)) = (&draft.start_input, &draft.end_input) {
        println!("input time: {start_input} to {end_input}");
    }
    println!("UTC: {} to {}", draft.start_utc, draft.end_utc);
    if let (Some(start), Some(end), Some(time_zone)) = (
        &draft.start_in_event_time_zone,
        &draft.end_in_event_time_zone,
        &draft.time_zone,
    ) {
        println!("event time zone ({time_zone}): {start} to {end}");
    }
    println!("duration: {} seconds", draft.duration_seconds);
    println!("kind: {}", if draft.all_day { "all-day" } else { "timed" });
    println!("calendar: {} [{}]", draft.calendar, draft.calendar_id);
    if let Some(source) = &draft.calendar_source {
        if let Some(source_id) = &draft.calendar_source_id {
            println!("calendar source: {source} [{source_id}]");
        } else {
            println!("calendar source: {source}");
        }
    }
    println!("availability: {}", draft.availability);
    println!("alarms: {}", draft.alarm_count);
    if let Some(recurrence) = &draft.recurrence {
        println!("recurrence: {}", event_recurrence_label(recurrence));
    }
    println!(
        "fields: notes={} location={} url={}",
        draft.has_notes, draft.has_location, draft.has_url
    );
    for warning in &draft.duplicate_warnings {
        println!("warning: {warning}");
    }
}

fn print_calendar(calendar: &CalendarReport) {
    let source = calendar.source.as_deref().unwrap_or("unknown source");
    let default_marker = if calendar.is_default_for_new_events {
        " default-for-new-events=true"
    } else {
        ""
    };
    println!(
        "- {} [{}] source={} writable={} subscribed={} id={}{}",
        calendar.title,
        calendar.calendar_type,
        source,
        calendar.allows_modifications,
        calendar.is_subscribed,
        calendar.id,
        default_marker
    );
}

fn print_events(events: &[EventReport]) {
    println!("Events ({})", events.len());
    if events.is_empty() {
        println!("- no events found");
        return;
    }

    for (index, event) in events.iter().enumerate() {
        let calendar = event.calendar.as_deref().unwrap_or("unknown calendar");
        println!(
            "{}. {} {} ({}) [{}]",
            index + 1,
            event_time_range(event),
            event.title,
            calendar,
            event.id
        );
    }
}

fn print_event_detail(event: &EventReport) {
    println!("{}", event.title);
    println!("id: {}", event.id);
    println!("time: {}", event_time_range(event));
    if let (Some(start_input), Some(end_input)) = (&event.start_input, &event.end_input) {
        println!("input time: {start_input} to {end_input}");
    }
    println!("UTC: {} to {}", event.start_utc, event.end_utc);
    if let (Some(start), Some(end), Some(time_zone)) = (
        &event.start_in_event_time_zone,
        &event.end_in_event_time_zone,
        &event.timezone,
    ) {
        println!("event time zone ({time_zone}): {start} to {end}");
    }
    println!("duration: {} seconds", event.duration_seconds);

    if let Some(calendar) = &event.calendar {
        println!("calendar: {calendar}");
    }
    if let Some(calendar_id) = &event.calendar_id {
        println!("calendar id: {calendar_id}");
    }
    if let Some(source) = &event.calendar_source {
        if let Some(source_id) = &event.calendar_source_id {
            println!("calendar source: {source} [{source_id}]");
        } else {
            println!("calendar source: {source}");
        }
    }
    if let Some(calendar_type) = &event.calendar_type {
        println!("calendar type: {calendar_type}");
    }
    if let Some(writable) = event.allows_calendar_modifications {
        println!("calendar allows modifications: {writable}");
    }
    if let Some(selection) = event.calendar_selection {
        let selection = match selection {
            crate::models::CalendarSelection::Explicit => "explicit",
            crate::models::CalendarSelection::EventkitDefault => "EventKit default",
        };
        println!("calendar selection: {selection}");
    }
    if let Some(action) = &event.write_action {
        println!("write action: {action}");
    }
    if let Some(scope) = &event.write_scope {
        println!("write scope: {scope}");
    }
    if let Some(location) = &event.location {
        println!("location: {location}");
    }
    if let Some(url) = &event.url {
        println!("url: {url}");
    }
    if let Some(alarms) = &event.alarms
        && !alarms.is_empty()
    {
        println!("alarms:");
        for alarm in alarms {
            println!("- {}", alarm_label(alarm));
        }
    }
    if let Some(alarm_count) = event.alarm_count {
        println!("alarm count: {alarm_count}");
    }
    if let Some(recurrence_count) = event.recurrence_count {
        println!("recurrence count: {recurrence_count}");
    }
    if let Some(rules) = &event.recurrence_rules
        && !rules.is_empty()
    {
        println!("recurrence rules:");
        for rule in rules {
            println!("- {}", event_recurrence_label(rule));
        }
    }
    println!(
        "recurrence exception: {}",
        if event.is_detached {
            "detached"
        } else {
            "none"
        }
    );
    if let Some(occurrence_date) = &event.occurrence_date {
        println!("original occurrence: {occurrence_date}");
    }
    if let Some(notes) = &event.notes {
        println!();
        println!("{notes}");
    }
}

fn event_recurrence_label(rule: &EventRecurrenceReport) -> String {
    let mut parts = vec![format!("every {} {}", rule.interval, rule.frequency)];
    if rule.first_day_of_week != 0 {
        parts.push(format!(
            "week starts {}",
            event_weekday_name(rule.first_day_of_week)
        ));
    }
    if let Some(days) = &rule.days_of_week {
        parts.push(format!(
            "weekdays {}",
            days.iter()
                .map(|day| {
                    let weekday = event_weekday_name(day.weekday);
                    if day.week_number == 0 {
                        weekday.to_string()
                    } else {
                        format!("{weekday}(week={})", day.week_number)
                    }
                })
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
    append_number_rule_part(&mut parts, "month days", rule.days_of_month.as_deref());
    append_number_rule_part(&mut parts, "months", rule.months_of_year.as_deref());
    append_number_rule_part(&mut parts, "year weeks", rule.weeks_of_year.as_deref());
    append_number_rule_part(&mut parts, "year days", rule.days_of_year.as_deref());
    append_number_rule_part(&mut parts, "set positions", rule.set_positions.as_deref());
    parts.push(
        match (
            &rule.end.kind[..],
            rule.end.occurrence_count,
            &rule.end.end_date,
        ) {
            ("count", Some(count), _) => format!("ends after {count} occurrences"),
            ("date", _, Some(date)) => format!("ends {date}"),
            _ => "never ends".to_string(),
        },
    );
    parts.join("; ")
}

fn append_number_rule_part(parts: &mut Vec<String>, label: &str, values: Option<&[i32]>) {
    if let Some(values) = values {
        parts.push(format!(
            "{label} {}",
            values
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        ));
    }
}

fn event_weekday_name(value: isize) -> &'static str {
    match value {
        1 => "sun",
        2 => "mon",
        3 => "tue",
        4 => "wed",
        5 => "thu",
        6 => "fri",
        7 => "sat",
        _ => "unknown",
    }
}

pub(crate) fn alarm_label(alarm: &AlarmReport) -> String {
    if let Some(offset) = alarm.relative_offset_seconds {
        if offset <= 0.0 {
            return format!("{} minutes before", (-offset / 60.0).round());
        }
        return format!("{} minutes after", (offset / 60.0).round());
    }

    if let Some(date) = &alarm.absolute_date {
        return date.clone();
    }

    alarm.alarm_type.clone()
}

pub(crate) fn event_time_range(event: &EventReport) -> String {
    let start_value = event
        .start_in_event_time_zone
        .as_deref()
        .unwrap_or(&event.start);
    let end_value = event
        .end_in_event_time_zone
        .as_deref()
        .unwrap_or(&event.end);
    if event.all_day {
        return format!("{} all-day", date_part(start_value));
    }

    let start_date = date_part(start_value);
    let end_date = date_part(end_value);
    let start_time = time_with_offset(start_value);
    let end_time = time_with_offset(end_value);

    if start_date == end_date {
        format!("{start_date} {start_time}-{end_time}")
    } else {
        format!("{start_date} {start_time} to {end_date} {end_time}")
    }
}

fn date_part(value: &str) -> &str {
    value.get(0..10).unwrap_or(value)
}

fn time_with_offset(value: &str) -> String {
    DateTime::parse_from_rfc3339(value)
        .map(|datetime| datetime.format("%H:%M%:z").to_string())
        .unwrap_or_else(|_| value.get(11..16).unwrap_or(value).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alarm_label_formats_relative_offsets() {
        let before = AlarmReport {
            relative_offset_seconds: Some(-600.0),
            absolute_date: None,
            proximity: "None".to_string(),
            alarm_type: "Display".to_string(),
        };
        let after = AlarmReport {
            relative_offset_seconds: Some(300.0),
            absolute_date: None,
            proximity: "None".to_string(),
            alarm_type: "Display".to_string(),
        };

        assert_eq!(alarm_label(&before), "10 minutes before");
        assert_eq!(alarm_label(&after), "5 minutes after");
    }

    #[test]
    fn time_format_includes_rfc3339_offset() {
        assert_eq!(time_with_offset("2026-07-12T15:55:00+03:00"), "15:55+03:00");
        assert_eq!(time_with_offset("2026-07-12T15:55:00+02:00"), "15:55+02:00");
    }

    #[test]
    fn reminder_recurrence_label_includes_termination() {
        let mut rule = ReminderRecurrenceReport {
            frequency: "monthly".to_string(),
            interval: 2,
            first_day_of_week: 0,
            end: ReminderRecurrenceEndReport {
                kind: "count".to_string(),
                occurrence_count: Some(12),
                end_date: None,
            },
            days_of_week: None,
            days_of_month: None,
            months_of_year: None,
            weeks_of_year: None,
            days_of_year: None,
            set_positions: None,
        };
        assert_eq!(
            reminder_recurrence_label(&rule),
            "every 2 monthly (count=12)"
        );
        rule.end = ReminderRecurrenceEndReport {
            kind: "date".to_string(),
            occurrence_count: None,
            end_date: Some("2026-12-31T21:59:00+00:00".to_string()),
        };
        assert_eq!(
            reminder_recurrence_label(&rule),
            "every 2 monthly (until=2026-12-31T21:59:00+00:00)"
        );
        rule.end = ReminderRecurrenceEndReport {
            kind: "never".to_string(),
            occurrence_count: None,
            end_date: None,
        };
        assert_eq!(reminder_recurrence_label(&rule), "every 2 monthly (never)");
    }

    #[test]
    fn event_recurrence_label_includes_components_and_termination() {
        let rule = EventRecurrenceReport {
            frequency: "monthly".to_string(),
            interval: 2,
            first_day_of_week: 2,
            end: crate::models::EventRecurrenceEndReport {
                kind: "count".to_string(),
                occurrence_count: Some(6),
                end_date: None,
            },
            days_of_week: Some(vec![
                crate::models::EventRecurrenceWeekdayReport {
                    weekday: 2,
                    week_number: 1,
                },
                crate::models::EventRecurrenceWeekdayReport {
                    weekday: 4,
                    week_number: -1,
                },
            ]),
            days_of_month: Some(vec![1, -1]),
            months_of_year: None,
            weeks_of_year: None,
            days_of_year: None,
            set_positions: Some(vec![1]),
        };

        assert_eq!(
            event_recurrence_label(&rule),
            "every 2 monthly; week starts mon; weekdays mon(week=1),wed(week=-1); month days 1,-1; set positions 1; ends after 6 occurrences"
        );
    }
}
