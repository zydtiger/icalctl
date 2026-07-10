use anyhow::{Context, Result, anyhow};
use eventkit::EventDraft;
use objc2_event_kit::{EKCalendarItem, EKEvent, EKEventStore, EKSpan};
use objc2_foundation::{NSDate, NSString, NSTimeZone, NSURL};

pub fn create_event_in_calendar(
    draft: &EventDraft<'_>,
    calendar_id: &str,
    time_zone: Option<&str>,
) -> Result<String> {
    let store = unsafe { EKEventStore::new() };
    let calendar_id = NSString::from_str(calendar_id);
    let calendar = unsafe { store.calendarWithIdentifier(&calendar_id) }
        .context("selected calendar is no longer available")?;
    let event = unsafe { EKEvent::eventWithEventStore(&store) };

    let title = NSString::from_str(draft.title);
    unsafe { event.setTitle(Some(&title)) };

    let start = draft
        .start
        .ok_or_else(|| anyhow!("event start is required"))?;
    let end = draft.end.ok_or_else(|| anyhow!("event end is required"))?;
    let start = NSDate::dateWithTimeIntervalSince1970(start.timestamp() as f64);
    let end = NSDate::dateWithTimeIntervalSince1970(end.timestamp() as f64);
    unsafe {
        event.setStartDate(Some(&start));
        event.setEndDate(Some(&end));
        event.setAllDay(draft.all_day);
        event.setCalendar(Some(&calendar));
    }

    if let Some(notes) = draft.notes {
        let notes = NSString::from_str(notes);
        unsafe { event.setNotes(Some(&notes)) };
    }
    if let Some(location) = draft.location {
        let location = NSString::from_str(location);
        unsafe { event.setLocation(Some(&location)) };
    }
    if let Some(url) = draft.URL {
        set_url(&event, url)?;
    }
    if let Some(availability) = draft.availability {
        unsafe { event.setAvailability(availability.to_ek()) };
    }
    if let Some(time_zone) = time_zone {
        set_time_zone(&event, time_zone)?;
    }

    unsafe {
        store
            .saveEvent_span_commit_error(&event, EKSpan::ThisEvent, true)
            .map_err(|error| anyhow!("failed to save event: {error:?}"))?;
        store.refreshSourcesIfNecessary();
    }

    unsafe { event.eventIdentifier() }
        .map(|id| id.to_string())
        .ok_or_else(|| anyhow!("EventKit did not return an id for the created event"))
}

pub fn update_event_calendar_metadata(
    event_id: &str,
    calendar_id: Option<&str>,
    time_zone: Option<Option<&str>>,
) -> Result<String> {
    let store = unsafe { EKEventStore::new() };
    unsafe { store.refreshSourcesIfNecessary() };

    let event_id = NSString::from_str(event_id);
    let event =
        unsafe { store.eventWithIdentifier(&event_id) }.context("event is no longer available")?;

    unsafe {
        if let Some(calendar_id) = calendar_id {
            let calendar_id = NSString::from_str(calendar_id);
            let calendar = store
                .calendarWithIdentifier(&calendar_id)
                .context("selected calendar is no longer available")?;
            event.setCalendar(Some(&calendar));
        }
        match time_zone {
            Some(Some(value)) => set_time_zone(&event, value)?,
            Some(None) => event.setTimeZone(None),
            None => {}
        }
        store
            .saveEvent_span_commit_error(&event, EKSpan::ThisEvent, true)
            .map_err(|error| anyhow!("failed to update event calendar metadata: {error:?}"))?;
        store.refreshSourcesIfNecessary();
    }

    unsafe { event.eventIdentifier() }
        .map(|id| id.to_string())
        .ok_or_else(|| anyhow!("EventKit did not return an id for the moved event"))
}

fn set_url(item: &EKCalendarItem, value: &str) -> Result<()> {
    let ns_value = NSString::from_str(value);
    let url = NSURL::URLWithString_encodingInvalidCharacters(&ns_value, false)
        .ok_or_else(|| anyhow!("invalid URL: {value}"))?;
    unsafe { item.setURL(Some(&url)) };
    Ok(())
}

pub fn validate_event_url(value: &str) -> Result<()> {
    let ns_value = NSString::from_str(value);
    NSURL::URLWithString_encodingInvalidCharacters(&ns_value, false)
        .map(|_| ())
        .ok_or_else(|| anyhow!("invalid URL: {value}"))
}

pub fn validate_event_time_zone(value: &str) -> Result<()> {
    let value = NSString::from_str(value);
    NSTimeZone::timeZoneWithName(&value)
        .map(|_| ())
        .ok_or_else(|| anyhow!("unknown IANA time zone: {value}"))
}

fn set_time_zone(item: &EKCalendarItem, value: &str) -> Result<()> {
    let time_zone = NSString::from_str(value);
    let time_zone = NSTimeZone::timeZoneWithName(&time_zone)
        .ok_or_else(|| anyhow!("unknown IANA time zone: {value}"))?;
    unsafe { item.setTimeZone(Some(&time_zone)) };
    Ok(())
}
