use super::*;

pub(super) fn duplicate_recurrence_policy_error(
    input: &AddEventInput,
    duplicates: &[EventItem],
) -> Result<Option<String>> {
    let [existing] = duplicates else {
        return Ok(None);
    };
    let details = read_event_details(&existing.identifier, Some(existing.start_date))?;
    let recurrence_matches =
        recurrence_rules_match(input.recurrence.as_ref(), &details.recurrence_rules);
    if !recurrence_matches {
        return Ok(Some(format!(
            "matching event [{}] has a different recurrence rule; refusing to treat it as the same event",
            existing.identifier
        )));
    }
    if input.if_exists == IfExistsArg::Update && !details.recurrence_rules.is_empty() {
        return Ok(Some(
            "--if-exists update for a recurring match requires explicit series scope; use skip for an identical rule or use the update command with an explicit occurrence and scope"
                .to_string(),
        ));
    }
    Ok(None)
}

pub(super) struct DuplicateQuery<'a> {
    pub(super) title: &'a str,
    pub(super) start: DateTime<Local>,
    pub(super) end: DateTime<Local>,
    pub(super) all_day: bool,
    pub(super) calendar_id: &'a str,
    pub(super) excluded_event_id: Option<&'a str>,
    pub(super) window_seconds: i64,
}

pub(super) fn matching_events(
    events: &EventsManager,
    query: &DuplicateQuery<'_>,
) -> Result<Vec<EventItem>> {
    let fetch_padding = query.window_seconds.max(1);
    let candidates = events
        .fetch_events(
            query.start - chrono::Duration::seconds(fetch_padding),
            query.end + chrono::Duration::seconds(fetch_padding),
            None,
        )
        .context("failed to check for duplicate events")?;

    Ok(candidates
        .into_iter()
        .filter(|event| query.excluded_event_id != Some(event.identifier.as_str()))
        .filter(|event| event.title == query.title)
        .filter(|event| event.all_day == query.all_day)
        .filter(|event| {
            datetime_within_window(event.start_date, query.start, query.window_seconds)
                && datetime_within_window(event.end_date, query.end, query.window_seconds)
        })
        .filter(|event| event.calendar_id.as_deref() == Some(query.calendar_id))
        .collect())
}

pub(super) fn datetime_within_window(
    candidate: DateTime<Local>,
    expected: DateTime<Local>,
    window_seconds: i64,
) -> bool {
    let limit_milliseconds = window_seconds.checked_mul(1_000).unwrap_or(i64::MAX);
    (candidate - expected).num_milliseconds().abs() <= limit_milliseconds
}

pub(super) fn duplicate_warnings(events: &[EventItem], window_seconds: i64) -> Vec<String> {
    events
        .iter()
        .map(|event| {
            format!(
                "possible duplicate within {window_seconds} seconds: {:?} at {} [{}]",
                event.title,
                event.start_date.to_rfc3339(),
                event.identifier
            )
        })
        .collect()
}
