use crate::models::{AlarmReport, CalendarReport, EventReport, JsonOutput};

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
        JsonOutput::Events { events } => print_events(events),
        JsonOutput::Event { event } => print_event_detail(event),
        JsonOutput::Deleted { deleted } => {
            println!("Deleted event: {} [{}]", deleted.title, deleted.id);
        }
    }
}

fn print_calendar(calendar: &CalendarReport) {
    let source = calendar.source.as_deref().unwrap_or("unknown source");
    println!(
        "- {} [{}] source={} writable={} subscribed={} id={}",
        calendar.title,
        calendar.calendar_type,
        source,
        calendar.allows_modifications,
        calendar.is_subscribed,
        calendar.id
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
    if event.all_day {
        return format!("{} all-day", date_part(&event.start));
    }

    let start_date = date_part(&event.start);
    let end_date = date_part(&event.end);
    let start_time = time_part(&event.start);
    let end_time = time_part(&event.end);

    if start_date == end_date {
        format!("{start_date} {start_time}-{end_time}")
    } else {
        format!("{start_date} {start_time} to {end_date} {end_time}")
    }
}

fn date_part(value: &str) -> &str {
    value.get(0..10).unwrap_or(value)
}

fn time_part(value: &str) -> &str {
    value.get(11..16).unwrap_or(value)
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
}
