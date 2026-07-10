use anyhow::{Result, anyhow, bail};
use eventkit::CalendarInfo;
use std::fmt::Write;

#[derive(Debug, Default)]
pub struct CalendarSelector<'a> {
    pub titles: &'a [String],
    pub ids: &'a [String],
    pub source: Option<&'a str>,
    pub source_id: Option<&'a str>,
}

impl CalendarSelector<'_> {
    pub fn is_empty(&self) -> bool {
        self.titles.is_empty() && self.ids.is_empty()
    }
}

pub fn resolve_calendars(
    calendars: &[CalendarInfo],
    selector: &CalendarSelector<'_>,
) -> Result<Vec<CalendarInfo>> {
    let mut resolved = Vec::new();

    for id in selector.ids {
        let calendar = calendars
            .iter()
            .find(|calendar| calendar.identifier == *id)
            .ok_or_else(|| anyhow!("calendar id not found: {id}"))?;
        push_unique(&mut resolved, calendar);
    }

    for title in selector.titles {
        let candidates: Vec<&CalendarInfo> = calendars
            .iter()
            .filter(|calendar| calendar.title == *title)
            .filter(|calendar| {
                selector
                    .source
                    .is_none_or(|source| calendar.source.as_deref() == Some(source))
            })
            .filter(|calendar| {
                selector
                    .source_id
                    .is_none_or(|source_id| calendar.source_id.as_deref() == Some(source_id))
            })
            .collect();

        match candidates.as_slice() {
            [] => bail!(missing_calendar_message(title, selector)),
            [calendar] => push_unique(&mut resolved, calendar),
            _ => bail!(ambiguous_calendar_message(title, &candidates)),
        }
    }

    Ok(resolved)
}

pub fn require_single_calendar(
    calendars: &[CalendarInfo],
    selector: &CalendarSelector<'_>,
) -> Result<CalendarInfo> {
    let resolved = resolve_calendars(calendars, selector)?;
    match resolved.as_slice() {
        [calendar] => Ok(calendar.clone()),
        [] => bail!("a calendar selector is required"),
        _ => bail!("calendar selector resolved to multiple calendars; select exactly one calendar"),
    }
}

fn push_unique(resolved: &mut Vec<CalendarInfo>, calendar: &CalendarInfo) {
    if !resolved
        .iter()
        .any(|item| item.identifier == calendar.identifier)
    {
        resolved.push(calendar.clone());
    }
}

fn missing_calendar_message(title: &str, selector: &CalendarSelector<'_>) -> String {
    let mut message = format!("calendar not found: {title:?}");
    if let Some(source) = selector.source {
        let _ = write!(message, " in source {source:?}");
    }
    if let Some(source_id) = selector.source_id {
        let _ = write!(message, " with source id {source_id:?}");
    }
    message
}

fn ambiguous_calendar_message(title: &str, candidates: &[&CalendarInfo]) -> String {
    let mut message = format!(
        "calendar title {title:?} is ambiguous; use --calendar-id, --calendar-source, or --source-id. Matches:"
    );
    for calendar in candidates {
        let source = calendar.source.as_deref().unwrap_or("unknown");
        let source_id = calendar.source_id.as_deref().unwrap_or("unknown");
        let _ = write!(
            message,
            "\n- title={:?} source={source:?} source_id={source_id:?} calendar_id={:?} writable={}",
            calendar.title, calendar.identifier, calendar.allows_modifications
        );
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use eventkit::CalendarType;

    fn calendar(id: &str, title: &str, source: &str, source_id: &str) -> CalendarInfo {
        CalendarInfo {
            identifier: id.to_string(),
            title: title.to_string(),
            source: Some(source.to_string()),
            source_id: Some(source_id.to_string()),
            calendar_type: CalendarType::CalDAV,
            allows_modifications: true,
            is_immutable: false,
            is_subscribed: false,
            color: None,
            allowed_entity_types: vec!["event".to_string()],
            supported_event_availabilities: Vec::new(),
        }
    }

    #[test]
    fn unique_title_resolves() {
        let calendars = vec![calendar("A", "Work", "iCloud", "S1")];
        let titles = vec!["Work".to_string()];
        let resolved = resolve_calendars(
            &calendars,
            &CalendarSelector {
                titles: &titles,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(resolved[0].identifier, "A");
    }

    #[test]
    fn duplicate_title_is_ambiguous_and_lists_candidates() {
        let calendars = vec![
            calendar("A", "Calendar", "iCloud", "S1"),
            calendar("B", "Calendar", "Exchange", "S2"),
        ];
        let titles = vec!["Calendar".to_string()];
        let error = resolve_calendars(
            &calendars,
            &CalendarSelector {
                titles: &titles,
                ..Default::default()
            },
        )
        .unwrap_err()
        .to_string();

        assert!(error.contains("calendar title \"Calendar\" is ambiguous"));
        assert!(error.contains("source=\"iCloud\""));
        assert!(error.contains("source_id=\"S2\""));
        assert!(error.contains("calendar_id=\"A\""));
        assert!(error.contains("writable=true"));
    }

    #[test]
    fn source_qualified_title_resolves() {
        let calendars = vec![
            calendar("A", "Calendar", "iCloud", "S1"),
            calendar("B", "Calendar", "Exchange", "S2"),
        ];
        let titles = vec!["Calendar".to_string()];
        let resolved = resolve_calendars(
            &calendars,
            &CalendarSelector {
                titles: &titles,
                source: Some("Exchange"),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(resolved[0].identifier, "B");
    }

    #[test]
    fn source_id_qualified_title_resolves() {
        let calendars = vec![
            calendar("A", "Calendar", "iCloud", "S1"),
            calendar("B", "Calendar", "Exchange", "S2"),
        ];
        let titles = vec!["Calendar".to_string()];
        let resolved = resolve_calendars(
            &calendars,
            &CalendarSelector {
                titles: &titles,
                source_id: Some("S1"),
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(resolved[0].identifier, "A");
    }

    #[test]
    fn exact_id_resolves_duplicate_title() {
        let calendars = vec![
            calendar("A", "Calendar", "iCloud", "S1"),
            calendar("B", "Calendar", "Exchange", "S2"),
        ];
        let ids = vec!["A".to_string()];
        let resolved = resolve_calendars(
            &calendars,
            &CalendarSelector {
                ids: &ids,
                ..Default::default()
            },
        )
        .unwrap();

        assert_eq!(resolved[0].source.as_deref(), Some("iCloud"));
    }
}
