use super::*;

pub(super) fn json_draft_to_command(
    defaults: &ReminderBatchDefaults,
    draft: ReminderJsonDraft,
    if_exists: IfExistsArg,
    duplicate_window_seconds: i64,
    dry_run: bool,
) -> Result<AddReminderCommand> {
    let has_timezone_less_timed_date = reminder_has_timezone_less_timed_date(&draft);
    let item_has_selector = draft.list.is_some()
        || draft.list_id.is_some()
        || draft.list_source.is_some()
        || draft.source_id.is_some();
    let (list, list_id, list_source, source_id) = if item_has_selector {
        (
            draft.list,
            draft.list_id,
            draft.list_source,
            draft.source_id,
        )
    } else {
        (
            defaults.list.clone(),
            defaults.list_id.clone(),
            defaults.list_source.clone(),
            defaults.source_id.clone(),
        )
    };
    validate_json_list_selector(
        list.as_deref(),
        list_id.as_deref(),
        list_source.as_deref(),
        source_id.as_deref(),
    )?;
    let geofence = reminder_json_override(draft.geofence, &defaults.geofence);
    let recurrence = reminder_json_override(draft.recurrence, &defaults.recurrence);
    let time_zone = match draft.time_zone {
        ReminderJsonOverride::Value(value) => Some(value),
        ReminderJsonOverride::Null => None,
        ReminderJsonOverride::Missing => has_timezone_less_timed_date
            .then(|| defaults.time_zone.clone())
            .flatten(),
    };
    let schedule = ReminderAdvancedScheduleArgs {
        notify_at: draft
            .notify_at
            .or_else(|| defaults.notify_at.clone())
            .unwrap_or_default(),
        geofence_title: geofence.as_ref().map(|value| value.title.clone()),
        geofence_latitude: geofence.as_ref().map(|value| value.latitude),
        geofence_longitude: geofence.as_ref().map(|value| value.longitude),
        geofence_radius_meters: geofence.as_ref().map(|value| value.radius_meters),
        geofence_proximity: geofence.as_ref().map(|value| value.proximity),
        repeat: recurrence.as_ref().map(|value| value.frequency),
        repeat_interval: recurrence.as_ref().and_then(|value| value.interval),
        repeat_count: recurrence.as_ref().and_then(|value| value.count),
        repeat_until: recurrence.and_then(|value| value.until),
    };
    Ok(AddReminderCommand {
        title: draft.title,
        list_selector: WriteReminderListSelectorArgs {
            list,
            list_id,
            list_source,
            source_id,
        },
        parent_id: draft.parent_id.or_else(|| defaults.parent_id.clone()),
        due: draft.due,
        start: draft.start,
        time_zone,
        notes: draft.notes,
        notes_file: None,
        url: draft.url,
        location: draft.location,
        priority: draft.priority.or(defaults.priority),
        notify_at_due: draft
            .notify_at_due
            .or(defaults.notify_at_due)
            .unwrap_or(false),
        notify_minutes_before: draft
            .notify_minutes_before
            .or_else(|| defaults.notify_minutes_before.clone())
            .unwrap_or_default(),
        schedule,
        if_exists,
        duplicate_window_seconds,
        dry_run,
    })
}

fn reminder_json_override<T: Clone>(
    value: ReminderJsonOverride<T>,
    default: &Option<T>,
) -> Option<T> {
    match value {
        ReminderJsonOverride::Missing => default.clone(),
        ReminderJsonOverride::Null => None,
        ReminderJsonOverride::Value(value) => Some(value),
    }
}

fn reminder_has_timezone_less_timed_date(draft: &ReminderJsonDraft) -> bool {
    [draft.due.as_deref(), draft.start.as_deref()]
        .into_iter()
        .flatten()
        .any(|value| {
            NaiveDate::parse_from_str(value, "%Y-%m-%d").is_err()
                && DateTime::parse_from_rfc3339(value).is_err()
        })
}

fn validate_json_list_selector(
    list: Option<&str>,
    list_id: Option<&str>,
    list_source: Option<&str>,
    source_id: Option<&str>,
) -> Result<()> {
    if list_id.is_some() && (list.is_some() || list_source.is_some() || source_id.is_some()) {
        bail!("list_id cannot be combined with list, list_source, or source_id");
    }
    if list_source.is_some() && source_id.is_some() {
        bail!("list_source and source_id cannot be combined");
    }
    if (list_source.is_some() || source_id.is_some()) && list.is_none() {
        bail!("list_source and source_id require list");
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct ReminderBatchIdentity {
    list_id: String,
    parent_id: Option<String>,
    title: String,
    due: String,
}

struct ReminderBatchSlot {
    index: usize,
    client_id: Option<String>,
    command: Option<AddReminderCommand>,
    draft: Option<ReminderDraftReport>,
    error: Option<String>,
}

pub(super) fn reminder_batch_add(
    store: &impl ReminderStore,
    path: &Path,
    if_exists: IfExistsArg,
    dry_run: bool,
    continue_on_error: bool,
) -> Result<JsonOutput> {
    let contents = fs::read(path)
        .with_context(|| format!("failed to read reminder batch file {}", path.display()))?;
    let envelope: ReminderBatchEnvelope = serde_json::from_slice(&contents)
        .with_context(|| format!("failed to parse reminder batch file {}", path.display()))?;
    if envelope.version != 1 {
        bail!(
            "unsupported reminder batch version {}; expected 1",
            envelope.version
        );
    }
    if envelope.reminders.is_empty() {
        bail!("reminder batch must contain at least one reminder");
    }
    validate_json_list_selector(
        envelope.defaults.list.as_deref(),
        envelope.defaults.list_id.as_deref(),
        envelope.defaults.list_source.as_deref(),
        envelope.defaults.source_id.as_deref(),
    )?;
    store.ensure_authorized()?;

    let mut slots = envelope
        .reminders
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let client_id = value
                .get("client_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            match serde_json::from_value::<ReminderJsonDraft>(value) {
                Ok(draft) => {
                    let client_id = draft.client_id.clone();
                    match json_draft_to_command(&envelope.defaults, draft, if_exists, 0, true) {
                        Ok(command) => ReminderBatchSlot {
                            index,
                            client_id,
                            command: Some(command),
                            draft: None,
                            error: None,
                        },
                        Err(error) => ReminderBatchSlot {
                            index,
                            client_id,
                            command: None,
                            draft: None,
                            error: Some(format!("invalid reminder: {error:#}")),
                        },
                    }
                }
                Err(error) => ReminderBatchSlot {
                    index,
                    client_id,
                    command: None,
                    draft: None,
                    error: Some(format!("invalid reminder: {error}")),
                },
            }
        })
        .collect::<Vec<_>>();
    mark_duplicate_reminder_client_ids(&mut slots);

    for slot in &mut slots {
        if slot.error.is_some() {
            continue;
        }
        let command = slot
            .command
            .as_ref()
            .expect("batch command is present")
            .clone();
        match add_reminder(store, command) {
            Ok(JsonOutput::ReminderDryRun { draft, .. }) => {
                if let Some(command) = slot.command.as_mut() {
                    command.list_selector = WriteReminderListSelectorArgs {
                        list: None,
                        list_id: Some(draft.list_id.clone()),
                        list_source: None,
                        source_id: None,
                    };
                }
                slot.draft = Some(*draft);
            }
            Ok(_) => slot.error = Some("reminder preflight returned unexpected output".to_string()),
            Err(error) => slot.error = Some(format!("{error:#}")),
        }
    }
    mark_duplicate_reminder_identities(&mut slots);

    let has_preflight_errors = slots.iter().any(|slot| slot.error.is_some());
    let can_write = !has_preflight_errors || continue_on_error;
    let items = if dry_run {
        reminder_batch_dry_reports(&slots)
    } else if has_preflight_errors && !continue_on_error {
        reminder_batch_blocked_reports(&slots)
    } else {
        reminder_batch_execute(store, &slots, continue_on_error)
    };
    let summary = summarize_reminder_batch(&items);
    Ok(JsonOutput::ReminderBatch {
        batch: ReminderBatchReport {
            version: 1,
            dry_run,
            can_write,
            if_exists: if_exists_name(if_exists).to_string(),
            continue_on_error,
            summary,
            items,
        },
    })
}

fn mark_duplicate_reminder_client_ids(slots: &mut [ReminderBatchSlot]) {
    let mut positions: HashMap<String, Vec<usize>> = HashMap::new();
    for (position, slot) in slots.iter().enumerate() {
        if let Some(client_id) = &slot.client_id {
            positions
                .entry(client_id.clone())
                .or_default()
                .push(position);
        }
    }
    for (client_id, positions) in positions {
        if positions.len() > 1 {
            for position in positions {
                slots[position].error = Some(format!("duplicate client_id: {client_id:?}"));
            }
        }
    }
}

fn mark_duplicate_reminder_identities(slots: &mut [ReminderBatchSlot]) {
    let mut positions: HashMap<ReminderBatchIdentity, Vec<usize>> = HashMap::new();
    for (position, slot) in slots.iter().enumerate() {
        if slot.error.is_none()
            && let Some(draft) = &slot.draft
        {
            let due = match draft.due.as_ref() {
                None => "undated".to_string(),
                Some(due) if due.kind == ReminderDateKind::Date => {
                    format!("date:{}", due.date.as_deref().unwrap_or("invalid"))
                }
                Some(due) => format!("datetime:{}", due.utc.as_deref().unwrap_or("invalid")),
            };
            positions
                .entry(ReminderBatchIdentity {
                    list_id: draft.list_id.clone(),
                    parent_id: draft.parent_id.clone(),
                    title: draft.title.clone(),
                    due,
                })
                .or_default()
                .push(position);
        }
    }
    for positions in positions.into_values() {
        if positions.len() > 1 {
            let rows = positions
                .iter()
                .map(|position| (slots[*position].index + 1).to_string())
                .collect::<Vec<_>>()
                .join(", ");
            for position in positions {
                slots[position].error = Some(format!(
                    "duplicate reminder identity within batch at rows {rows}"
                ));
            }
        }
    }
}

fn reminder_batch_dry_reports(slots: &[ReminderBatchSlot]) -> Vec<ReminderBatchItemReport> {
    slots
        .iter()
        .map(|slot| {
            if let Some(error) = &slot.error {
                return reminder_batch_error_report(slot, "failed", error.clone());
            }
            let draft = slot.draft.as_ref().expect("batch draft is present");
            let status = match draft.operation.as_str() {
                "create" => "would_create",
                "skip" => "would_skip",
                "update" => "would_update",
                _ => "failed",
            };
            reminder_batch_planned_report(slot, status, None)
        })
        .collect()
}

fn reminder_batch_blocked_reports(slots: &[ReminderBatchSlot]) -> Vec<ReminderBatchItemReport> {
    slots
        .iter()
        .map(|slot| {
            if let Some(error) = &slot.error {
                reminder_batch_error_report(slot, "failed", error.clone())
            } else {
                reminder_batch_planned_report(
                    slot,
                    "not_attempted",
                    Some("reminder batch blocked by preflight errors".to_string()),
                )
            }
        })
        .collect()
}

fn reminder_batch_execute(
    store: &impl ReminderStore,
    slots: &[ReminderBatchSlot],
    continue_on_error: bool,
) -> Vec<ReminderBatchItemReport> {
    let mut stopped = false;
    let mut reports = Vec::with_capacity(slots.len());
    for slot in slots {
        if let Some(error) = &slot.error {
            reports.push(reminder_batch_error_report(slot, "failed", error.clone()));
            continue;
        }
        if stopped {
            reports.push(reminder_batch_planned_report(
                slot,
                "not_attempted",
                Some("not attempted after an earlier reminder write failure".to_string()),
            ));
            continue;
        }
        let mut command = slot
            .command
            .as_ref()
            .expect("batch command is present")
            .clone();
        command.dry_run = false;
        match add_reminder(store, command) {
            Ok(JsonOutput::Reminder { reminder }) => {
                let status = reminder
                    .write_action
                    .as_deref()
                    .unwrap_or("written")
                    .to_string();
                let mut executed_draft = slot.draft.clone();
                if let Some(draft) = executed_draft.as_mut() {
                    draft.operation = match status.as_str() {
                        "created" => "create",
                        "skipped" => "skip",
                        "updated" => "update",
                        other => other,
                    }
                    .to_string();
                    draft.matched_reminder_id = matches!(status.as_str(), "skipped" | "updated")
                        .then(|| reminder.id.clone());
                }
                let matched_reminder_id =
                    matches!(status.as_str(), "skipped" | "updated").then(|| reminder.id.clone());
                reports.push(ReminderBatchItemReport {
                    index: slot.index,
                    client_id: slot.client_id.clone(),
                    status,
                    reminder_id: Some(reminder.id.clone()),
                    matched_reminder_id,
                    draft: executed_draft.map(Box::new),
                    error: None,
                });
            }
            Ok(_) => {
                reports.push(reminder_batch_error_report(
                    slot,
                    "failed",
                    "reminder write returned unexpected output".to_string(),
                ));
                if !continue_on_error {
                    stopped = true;
                }
            }
            Err(error) => {
                reports.push(reminder_batch_error_report(
                    slot,
                    "failed",
                    format!("{error:#}"),
                ));
                if !continue_on_error {
                    stopped = true;
                }
            }
        }
    }
    reports
}

fn reminder_batch_planned_report(
    slot: &ReminderBatchSlot,
    status: &str,
    error: Option<String>,
) -> ReminderBatchItemReport {
    let draft = slot.draft.as_ref();
    ReminderBatchItemReport {
        index: slot.index,
        client_id: slot.client_id.clone(),
        status: status.to_string(),
        reminder_id: None,
        matched_reminder_id: draft.and_then(|draft| draft.matched_reminder_id.clone()),
        draft: draft.cloned().map(Box::new),
        error: error.map(|message| BatchErrorReport { message }),
    }
}

fn reminder_batch_error_report(
    slot: &ReminderBatchSlot,
    status: &str,
    message: String,
) -> ReminderBatchItemReport {
    ReminderBatchItemReport {
        index: slot.index,
        client_id: slot.client_id.clone(),
        status: status.to_string(),
        reminder_id: None,
        matched_reminder_id: slot
            .draft
            .as_ref()
            .and_then(|draft| draft.matched_reminder_id.clone()),
        draft: slot.draft.clone().map(Box::new),
        error: Some(BatchErrorReport { message }),
    }
}

fn summarize_reminder_batch(items: &[ReminderBatchItemReport]) -> BatchSummaryReport {
    let mut summary = BatchSummaryReport {
        total: items.len(),
        ..Default::default()
    };
    for item in items {
        match item.status.as_str() {
            "created" => summary.created += 1,
            "skipped" => summary.skipped += 1,
            "updated" => summary.updated += 1,
            "failed" => summary.failed += 1,
            "not_attempted" => summary.not_attempted += 1,
            "would_create" => summary.would_create += 1,
            "would_skip" => summary.would_skip += 1,
            "would_update" => summary.would_update += 1,
            _ => summary.failed += 1,
        }
    }
    summary
}
