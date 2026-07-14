use super::*;

pub(super) fn reminder_mutation_dry_run(
    operation: &str,
    changed_fields: Vec<String>,
    before: ReminderReport,
    result: ReminderReport,
) -> JsonOutput {
    JsonOutput::ReminderMutationDryRun {
        would_write: false,
        draft: Box::new(ReminderMutationDraftReport {
            operation: operation.to_string(),
            reminder_id: before.id.clone(),
            changed_fields,
            before: Box::new(before),
            result: Box::new(result),
        }),
    }
}

pub(super) fn ensure_reminder_writable(reminder: &ReminderReport, operation: &str) -> Result<()> {
    let list = reminder.list.as_deref().unwrap_or("unknown list");
    let list_id = reminder.list_id.as_deref().unwrap_or("unknown id");
    match reminder.allows_list_modifications {
        Some(true) => Ok(()),
        Some(false) => bail!("cannot {operation} reminder in read-only list {list} [{list_id}]"),
        None => bail!(
            "cannot {operation} reminder because its list is unavailable or writability is unknown: {list} [{list_id}]"
        ),
    }
}

pub(super) fn resolve_parent(
    store: &impl ReminderStore,
    parent_id: &str,
) -> Result<ReminderReport> {
    let parent = store
        .get(parent_id)
        .with_context(|| format!("parent reminder not found: {parent_id}"))?;
    ensure_reminder_writable(&parent, "use as a parent")?;
    if parent.completed {
        bail!(
            "completed reminder {} [{}] cannot be used as a parent",
            parent.title,
            parent.id
        );
    }
    Ok(parent)
}

pub(super) fn reminder_parent_list(
    lists: &[ReminderListReport],
    parent: &ReminderReport,
) -> Result<ReminderListReport> {
    let list_id = parent
        .list_id
        .as_deref()
        .context("parent reminder list is unavailable")?;
    lists
        .iter()
        .find(|list| list.id == list_id)
        .cloned()
        .with_context(|| format!("parent reminder list is no longer available: {list_id}"))
}

pub(super) fn validate_parent_assignment(
    store: &impl ReminderStore,
    child_id: &str,
    child_list_id: &str,
    parent: &ReminderReport,
) -> Result<()> {
    if parent.id == child_id {
        bail!("a reminder cannot be its own parent: {child_id}");
    }
    let parent_list_id = parent
        .list_id
        .as_deref()
        .context("parent reminder list is unavailable")?;
    if parent_list_id != child_list_id {
        bail!(
            "parent reminder {} [{}] is in list {}, but child {} is in list {}; parent and child must use the same reminder list",
            parent.title,
            parent.id,
            parent_list_id,
            child_id,
            child_list_id
        );
    }

    let mut seen = BTreeSet::new();
    let mut current = Some(parent.clone());
    while let Some(reminder) = current {
        if reminder.id == child_id {
            bail!("parent assignment would create a reminder hierarchy cycle involving {child_id}");
        }
        if !seen.insert(reminder.id.clone()) {
            bail!(
                "existing reminder hierarchy contains a cycle at {} [{}]",
                reminder.title,
                reminder.id
            );
        }
        current = reminder
            .parent_id
            .as_deref()
            .map(|id| resolve_parent(store, id))
            .transpose()?;
    }
    Ok(())
}

pub(super) fn confirm_reminder_delete(reminder: &ReminderReport) -> Result<()> {
    let mut stderr = io::stderr();
    let list = reminder.list.as_deref().unwrap_or("unknown list");
    writeln!(
        stderr,
        "Delete reminder \"{}\" from {} [{}]?",
        reminder.title,
        list,
        reminder.list_id.as_deref().unwrap_or("unknown id")
    )?;
    write!(stderr, "Type delete to confirm: ")?;
    stderr.flush()?;
    let mut input = String::new();
    io::stdin()
        .read_line(&mut input)
        .context("failed to read delete confirmation")?;
    if input.trim() == "delete" {
        Ok(())
    } else {
        bail!("delete cancelled")
    }
}

pub(super) fn lifecycle_changed_fields(patch: &ReminderLifecyclePatch) -> Vec<String> {
    [
        (patch.title.is_some(), "title"),
        (patch.parent_id.is_some(), "parent"),
        (patch.list_id.is_some(), "list"),
        (patch.due.is_some(), "due"),
        (patch.start.is_some(), "start"),
        (patch.notes.is_some(), "notes"),
        (patch.location.is_some(), "location"),
        (patch.url.is_some(), "url"),
        (patch.priority_value.is_some(), "priority"),
        (
            patch.notifications.is_some() || patch.geofence.is_some(),
            "alarms",
        ),
        (patch.recurrence.is_some(), "recurrence"),
    ]
    .into_iter()
    .filter(|(changed, _)| *changed)
    .map(|(_, name)| name.to_string())
    .collect()
}

pub(super) fn preview_lifecycle_patch(
    before: &ReminderReport,
    patch: &ReminderLifecyclePatch,
    moved_list: Option<&ReminderListReport>,
) -> ReminderReport {
    let mut result = before.clone();
    if let Some(title) = &patch.title {
        result.title = title.clone();
    }
    if let Some(parent_id) = &patch.parent_id {
        result.parent_id = parent_id.clone();
    }
    if let Some(list) = moved_list {
        result.list = Some(list.title.clone());
        result.list_id = Some(list.id.clone());
        result.list_source = list.source.clone();
        result.list_source_id = list.source_id.clone();
        result.list_type = Some(list.list_type.clone());
        result.allows_list_modifications = Some(list.allows_modifications);
        result.list_selection = Some(ReminderListSelection::Explicit);
    }
    if let Some(due) = &patch.due {
        result.due = due.as_ref().map(|value| value.report.clone());
        result.due_input = due.as_ref().map(|value| value.input.clone());
    }
    if let Some(start) = &patch.start {
        result.start = start.as_ref().map(|value| value.report.clone());
        result.start_input = start.as_ref().map(|value| value.input.clone());
    }
    if let Some(notes) = &patch.notes {
        result.notes = notes.clone();
        result.has_notes = notes.is_some();
    }
    if let Some(location) = &patch.location {
        result.location = location.clone();
    }
    if let Some(url) = &patch.url {
        result.url = url.clone();
        result.has_url = url.is_some();
    }
    if let Some(priority) = patch.priority_value {
        result.priority_value = priority;
        result.priority = priority_name(priority);
    }
    if patch.notifications.is_some() || patch.geofence.is_some() {
        let notifications = patch.notifications.as_deref().unwrap_or_default();
        let geofence = patch.geofence.as_ref().and_then(Option::as_ref);
        let alarms = alarm_reports_from_parsed(notifications, geofence);
        result.alarm_count = Some(alarms.len());
        result.alarms = Some(alarms);
    }
    if let Some(recurrence) = &patch.recurrence {
        let rules = recurrence
            .as_ref()
            .map(recurrence_report_from_parsed)
            .into_iter()
            .collect::<Vec<_>>();
        result.recurrence_count = Some(rules.len());
        result.recurrence_rules = Some(rules);
    }
    result
}

pub(super) fn find_duplicates<'a>(
    reminders: &'a [ReminderReport],
    title: &str,
    due: Option<&ParsedReminderDate>,
    parent_id: Option<&str>,
    window_seconds: i64,
) -> Vec<&'a ReminderReport> {
    reminders
        .iter()
        .filter(|reminder| {
            reminder.title == title
                && reminder.parent_id.as_deref() == parent_id
                && due_matches(reminder.due.as_ref(), due, window_seconds)
        })
        .collect()
}

pub(super) fn due_matches(
    existing: Option<&ReminderDateReport>,
    requested: Option<&ParsedReminderDate>,
    window_seconds: i64,
) -> bool {
    match (existing, requested) {
        (None, None) => true,
        (Some(existing), Some(requested))
            if existing.kind == ReminderDateKind::Date
                && requested.report.kind == ReminderDateKind::Date =>
        {
            existing.date == requested.report.date
        }
        (Some(existing), Some(requested))
            if existing.kind == ReminderDateKind::Datetime
                && requested.report.kind == ReminderDateKind::Datetime =>
        {
            match (
                existing
                    .normalized
                    .as_deref()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok()),
                requested
                    .report
                    .normalized
                    .as_deref()
                    .and_then(|value| DateTime::parse_from_rfc3339(value).ok()),
            ) {
                (Some(existing), Some(requested)) => {
                    (existing - requested).num_seconds().abs() <= window_seconds
                }
                _ => {
                    existing.local == requested.report.local
                        && existing.time_zone == requested.report.time_zone
                }
            }
        }
        _ => false,
    }
}

pub(super) fn priority_arg_value(priority: ReminderPriorityArg) -> usize {
    match priority {
        ReminderPriorityArg::None => 0,
        ReminderPriorityArg::High => 1,
        ReminderPriorityArg::Medium => 5,
        ReminderPriorityArg::Low => 9,
    }
}

pub(super) fn if_exists_name(value: IfExistsArg) -> &'static str {
    match value {
        IfExistsArg::Error => "error",
        IfExistsArg::Skip => "skip",
        IfExistsArg::Update => "update",
    }
}

pub(super) fn validate_reminder_url(value: &str) -> Result<()> {
    let value = NSString::from_str(value);
    NSURL::URLWithString_encodingInvalidCharacters(&value, false)
        .map(|_| ())
        .ok_or_else(|| anyhow!("invalid URL: {value}"))
}

pub(super) fn read_notes_file(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut notes = String::new();
        io::stdin()
            .read_to_string(&mut notes)
            .context("failed to read reminder notes from stdin")?;
        return Ok(notes);
    }
    fs::read_to_string(path)
        .with_context(|| format!("failed to read reminder notes from {}", path.display()))
}
