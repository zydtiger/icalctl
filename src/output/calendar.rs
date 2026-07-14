use super::*;

pub(super) fn print_dry_run(draft: &EventDraftReport) {
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

pub(super) fn print_calendar(calendar: &CalendarReport) {
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

pub(super) fn print_events(events: &[EventReport]) {
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

pub(super) fn print_event_detail(event: &EventReport) {
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
            crate::models::CalendarSelection::ConfiguredDefault => "configured default",
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

pub(super) fn event_recurrence_label(rule: &EventRecurrenceReport) -> String {
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

pub(super) fn append_number_rule_part(
    parts: &mut Vec<String>,
    label: &str,
    values: Option<&[i32]>,
) {
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

pub(super) fn event_weekday_name(value: isize) -> &'static str {
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

pub(super) fn alarm_label(alarm: &AlarmReport) -> String {
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
