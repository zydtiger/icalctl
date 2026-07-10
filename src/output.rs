use crate::models::{AlarmReport, CalendarReport, EventDraftReport, EventReport, JsonOutput};
use chrono::DateTime;

pub fn print_human_output(output: &JsonOutput) {
    match output {
        JsonOutput::Status(status) => {
            println!("Calendar authorization: {}", status.authorization);
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
        JsonOutput::Events { events } => print_events(events),
        JsonOutput::Event { event } => print_event_detail(event),
        JsonOutput::DryRun { draft, .. } => print_dry_run(draft),
        JsonOutput::Deleted { deleted } => {
            println!("Deleted event: {} [{}]", deleted.title, deleted.id);
        }
    }
}

fn print_dry_run(draft: &EventDraftReport) {
    println!("Dry run: no Calendar changes were made");
    println!("operation: {}", draft.operation);
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
    if let Some(selection) = event.calendar_selection {
        let selection = match selection {
            crate::models::CalendarSelection::Explicit => "explicit",
            crate::models::CalendarSelection::EventkitDefault => "EventKit default",
        };
        println!("calendar selection: {selection}");
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
    if let Some(notes) = &event.notes {
        println!();
        println!("{notes}");
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
}
