use anyhow::{Context, Result, anyhow};
use eventkit::EventDraft;
use objc2_event_kit::{EKCalendarItem, EKEvent, EKEventStore, EKSpan};
use objc2_foundation::{NSDate, NSString, NSURL};

pub fn create_event_in_calendar(draft: &EventDraft<'_>, calendar_id: &str) -> Result<String> {
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

pub fn move_event_to_calendar(event_id: &str, calendar_id: &str) -> Result<String> {
    let store = unsafe { EKEventStore::new() };
    unsafe { store.refreshSourcesIfNecessary() };

    let event_id = NSString::from_str(event_id);
    let event =
        unsafe { store.eventWithIdentifier(&event_id) }.context("event is no longer available")?;
    let calendar_id = NSString::from_str(calendar_id);
    let calendar = unsafe { store.calendarWithIdentifier(&calendar_id) }
        .context("selected calendar is no longer available")?;

    unsafe {
        event.setCalendar(Some(&calendar));
        store
            .saveEvent_span_commit_error(&event, EKSpan::ThisEvent, true)
            .map_err(|error| anyhow!("failed to move event: {error:?}"))?;
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
