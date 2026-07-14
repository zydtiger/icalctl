use super::*;

pub(super) fn add_reminder_from_cli(
    store: &impl ReminderStore,
    input: AddReminderCliCommand,
) -> Result<JsonOutput> {
    let command = if let Some(path) = input.json_file.as_deref() {
        if add_cli_has_individual_fields(&input) {
            bail!("--json-file cannot be combined with a title or individual reminder fields");
        }
        let contents = read_notes_file(path)
            .with_context(|| format!("failed to read reminder JSON file {}", path.display()))?;
        let draft: ReminderJsonDraft = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse reminder JSON file {}", path.display()))?;
        if draft.client_id.is_some() {
            bail!("client_id is only valid in reminder batch files");
        }
        json_draft_to_command(
            &ReminderBatchDefaults::default(),
            draft,
            input.if_exists,
            input.duplicate_window_seconds,
            input.dry_run,
        )?
    } else {
        AddReminderCommand {
            title: input
                .title
                .context("reminder add requires TITLE or --json-file")?,
            list_selector: input.list_selector,
            parent_id: input.parent_id,
            due: input.due,
            start: input.start,
            time_zone: input.time_zone,
            notes: input.notes,
            notes_file: input.notes_file,
            url: input.url,
            location: input.location,
            priority: input.priority,
            notify_at_due: input.notify_at_due,
            notify_minutes_before: input.notify_minutes_before,
            schedule: input.schedule,
            if_exists: input.if_exists,
            duplicate_window_seconds: input.duplicate_window_seconds,
            dry_run: input.dry_run,
        }
    };
    add_reminder(store, command)
}

pub(super) fn add_cli_has_individual_fields(input: &AddReminderCliCommand) -> bool {
    input.title.is_some()
        || !write_list_selector_is_empty(&input.list_selector)
        || input.parent_id.is_some()
        || input.due.is_some()
        || input.start.is_some()
        || input.time_zone.is_some()
        || input.notes.is_some()
        || input.notes_file.is_some()
        || input.url.is_some()
        || input.location.is_some()
        || input.priority.is_some()
        || input.notify_at_due
        || !input.notify_minutes_before.is_empty()
        || !advanced_schedule_is_empty(&input.schedule)
}

pub(super) fn advanced_schedule_is_empty(schedule: &ReminderAdvancedScheduleArgs) -> bool {
    schedule.notify_at.is_empty()
        && schedule.geofence_title.is_none()
        && schedule.geofence_latitude.is_none()
        && schedule.geofence_longitude.is_none()
        && schedule.geofence_radius_meters.is_none()
        && schedule.geofence_proximity.is_none()
        && schedule.repeat.is_none()
        && schedule.repeat_interval.is_none()
        && schedule.repeat_count.is_none()
        && schedule.repeat_until.is_none()
}

pub(super) fn add_reminder(
    store: &impl ReminderStore,
    command: AddReminderCommand,
) -> Result<JsonOutput> {
    store.ensure_authorized()?;
    if command.title.trim().is_empty() {
        bail!("reminder title must not be empty");
    }
    if command.duplicate_window_seconds < 0 {
        bail!("--duplicate-window-seconds must not be negative");
    }
    if let Some(time_zone) = &command.time_zone {
        validate_time_zone(time_zone)?;
    }

    let notes = match command.notes_file.as_deref() {
        Some(path) => Some(read_notes_file(path)?),
        None => command.notes.clone(),
    };
    if let Some(url) = &command.url {
        validate_reminder_url(url)?;
    }

    let due = command
        .due
        .as_deref()
        .map(|value| parse_reminder_date(value, command.time_zone.as_deref()))
        .transpose()
        .context("invalid reminder due value")?;
    let start = command
        .start
        .as_deref()
        .map(|value| parse_reminder_date(value, command.time_zone.as_deref()))
        .transpose()
        .context("invalid reminder start value")?;
    if command.time_zone.is_some()
        && [due.as_ref(), start.as_ref()]
            .into_iter()
            .flatten()
            .all(|value| value.report.kind == ReminderDateKind::Date)
    {
        bail!("--time-zone requires at least one timed --due or --start value");
    }
    let notifications_supplied = command.notify_at_due
        || !command.notify_minutes_before.is_empty()
        || !command.schedule.notify_at.is_empty()
        || command.schedule.geofence_title.is_some();
    let mut notifications = build_notifications(
        due.as_ref().map(|value| &value.report),
        command.notify_at_due,
        &command.notify_minutes_before,
    )?;
    notifications.extend(parse_absolute_notifications(&command.schedule.notify_at)?);
    let geofence = parse_geofence(&command.schedule)?;
    let recurrence = parse_recurrence(&command.schedule)?;

    let parent = command
        .parent_id
        .as_deref()
        .map(|id| resolve_parent(store, id))
        .transpose()?;
    let lists = store.lists()?;
    let parent_list = parent
        .as_ref()
        .map(|parent| reminder_parent_list(&lists, parent))
        .transpose()?;
    let configured_default_id =
        if write_list_selector_is_empty(&command.list_selector) && parent_list.is_none() {
            crate::config::default_reminder_list_id()?
        } else {
            None
        };
    let (list, list_selection) = if write_list_selector_is_empty(&command.list_selector) {
        if let Some(parent_list) = parent_list.as_ref() {
            (parent_list.clone(), ReminderListSelection::Explicit)
        } else {
            let default_list = if configured_default_id.is_some() {
                None
            } else {
                Some(store.default_list()?)
            };
            resolve_write_list(
                &lists,
                default_list.as_ref(),
                configured_default_id.as_deref(),
                &command.list_selector,
            )?
        }
    } else {
        resolve_write_list(&lists, None, None, &command.list_selector)?
    };
    if let Some(parent_list) = parent_list.as_ref()
        && parent_list.id != list.id
    {
        bail!(
            "parent reminder {} [{}] is in {} [{}], but the child target is {} [{}]; parent and child must use the same reminder list",
            parent.as_ref().expect("parent is present").title,
            parent.as_ref().expect("parent is present").id,
            parent_list.title,
            parent_list.id,
            list.title,
            list.id
        );
    }
    let priority_arg = command.priority;
    let priority_value = priority_arg.map(priority_arg_value).unwrap_or(0);

    let save = ReminderSaveDraft {
        title: command.title.clone(),
        list_id: list.id.clone(),
        parent_id: command.parent_id.clone(),
        due: due.clone(),
        start: start.clone(),
        notes: notes.clone(),
        location: command.location.clone(),
        url: command.url.clone(),
        priority_value,
        notifications: notifications.clone(),
        geofence: geofence.clone(),
        recurrence: recurrence.clone(),
    };
    let patch = ReminderAddPatch {
        parent_id: command.parent_id.clone(),
        start: start.clone(),
        notes: notes.clone(),
        location: command.location.clone(),
        url: command.url.clone(),
        priority_value: priority_arg.map(priority_arg_value),
        notifications: notifications_supplied.then_some(notifications.clone()),
        geofence: notifications_supplied.then_some(geofence.clone()),
        recurrence: recurrence.clone(),
    };

    let existing = store.fetch(std::slice::from_ref(&list.id))?;
    let matches = find_duplicates(
        &existing,
        &command.title,
        due.as_ref(),
        command.parent_id.as_deref(),
        command.duplicate_window_seconds,
    );
    if matches.len() > 1 {
        let ids = matches
            .iter()
            .map(|reminder| reminder.id.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        bail!(
            "duplicate policy matched multiple reminders in {} [{}]: {ids}; use an exact due value or a smaller --duplicate-window-seconds",
            list.title,
            list.id
        );
    }
    let matched = matches.into_iter().next();
    if let (Some(matched), Some(parent)) = (matched.as_ref(), parent.as_ref()) {
        validate_parent_assignment(store, &matched.id, &list.id, parent)?;
    }
    if let Some(matched) = matched.as_ref()
        && command.if_exists == IfExistsArg::Error
    {
        bail!(
            "matching reminder already exists: {} [{}] in {} [{}]; use --if-exists skip or update",
            matched.title,
            matched.id,
            list.title,
            list.id
        );
    }

    let operation = match (&matched, command.if_exists) {
        (Some(_), IfExistsArg::Skip) => "skip",
        (Some(_), IfExistsArg::Update) => "update",
        _ => "create",
    };
    if let Some(recurrence) = &recurrence {
        let effective_anchor = if operation == "update" {
            matched
                .as_ref()
                .and_then(|reminder| reminder.due.as_ref())
                .or_else(|| start.as_ref().map(|value| &value.report))
                .or_else(|| {
                    matched
                        .as_ref()
                        .and_then(|reminder| reminder.start.as_ref())
                })
        } else {
            due.as_ref()
                .map(|value| &value.report)
                .or_else(|| start.as_ref().map(|value| &value.report))
        };
        let effective_anchor =
            effective_anchor.context("recurrence requires a due or start date")?;
        validate_recurrence_end_after_anchor(
            &recurrence_report_from_parsed(recurrence),
            Some(effective_anchor),
        )?;
    }
    let duplicate_warnings = (command.duplicate_window_seconds > 0)
        .then(|| {
            format!(
                "timed due duplicate matching uses a ±{} second window; date-only and undated identities remain exact",
                command.duplicate_window_seconds
            )
        })
        .into_iter()
        .collect();
    let draft = ReminderDraftReport {
        operation: operation.to_string(),
        matched_reminder_id: matched.as_ref().map(|reminder| reminder.id.clone()),
        title: command.title.clone(),
        parent_id: command.parent_id.clone(),
        list: list.title.clone(),
        list_id: list.id.clone(),
        list_source: list.source.clone(),
        list_source_id: list.source_id.clone(),
        list_selection,
        due: due.as_ref().map(|value| value.report.clone()),
        due_input: due.as_ref().map(|value| value.input.clone()),
        start: start.as_ref().map(|value| value.report.clone()),
        start_input: start.as_ref().map(|value| value.input.clone()),
        priority: priority_name(priority_value),
        priority_value,
        has_notes: notes.is_some(),
        has_location: command.location.is_some(),
        has_url: command.url.is_some(),
        notification_count: notifications
            .iter()
            .filter(|notification| notification.minutes_before.is_some())
            .count(),
        notifications: notification_reports(
            &notifications,
            due.as_ref().map(|value| &value.report),
        ),
        planned_alarm_count: notifications.len() + usize::from(geofence.is_some()),
        planned_alarms: planned_alarm_reports(&notifications, geofence.as_ref()),
        recurrence: recurrence.as_ref().map(recurrence_report_from_parsed),
        if_exists: if_exists_name(command.if_exists).to_string(),
        duplicate_window_seconds: command.duplicate_window_seconds,
        duplicate_warnings,
    };
    if command.dry_run {
        return Ok(JsonOutput::ReminderDryRun {
            would_write: false,
            draft: Box::new(draft),
        });
    }

    let mut reminder = match (operation, matched.as_ref()) {
        ("skip", Some(matched)) => store.get(&matched.id)?,
        ("update", Some(matched)) => store.update_add_fields(&matched.id, &patch)?,
        _ => store.create(&save)?,
    };
    reminder.list_selection = Some(list_selection);
    reminder.write_action = Some(
        match operation {
            "skip" => "skipped",
            "update" => "updated",
            _ => "created",
        }
        .to_string(),
    );
    reminder.due_input = due.map(|value| value.input);
    reminder.start_input = start.map(|value| value.input);
    Ok(JsonOutput::Reminder {
        reminder: Box::new(reminder),
    })
}

pub(super) fn update_reminder(
    store: &impl ReminderStore,
    command: UpdateReminderCommand,
) -> Result<JsonOutput> {
    store.ensure_authorized()?;
    let id = resolve_reminder_ref(&command.id)?;
    let before = store.get(&id)?;
    ensure_reminder_writable(&before, "update")?;

    if command
        .title
        .as_ref()
        .is_some_and(|title| title.trim().is_empty())
    {
        bail!("reminder title must not be empty");
    }
    if let Some(time_zone) = &command.time_zone {
        validate_time_zone(time_zone)?;
    }
    if let Some(url) = &command.url {
        validate_reminder_url(url)?;
    }

    let notes = match command.notes_file.as_deref() {
        Some(path) => Some(read_notes_file(path)?),
        None => command.notes.clone(),
    };
    let mut due = command
        .due
        .as_deref()
        .map(|value| parse_reminder_date(value, command.time_zone.as_deref()))
        .transpose()
        .context("invalid reminder due value")?;
    let mut start = command
        .start
        .as_deref()
        .map(|value| parse_reminder_date(value, command.time_zone.as_deref()))
        .transpose()
        .context("invalid reminder start value")?;
    if command.clear_time_zone {
        due = due.map(clear_parsed_time_zone).transpose()?;
        start = start.map(clear_parsed_time_zone).transpose()?;
    }

    let mut patch = ReminderLifecyclePatch {
        title: command.title.clone(),
        parent_id: if command.clear_parent {
            Some(None)
        } else {
            command.parent_id.clone().map(Some)
        },
        due: if command.clear_due {
            Some(None)
        } else {
            due.map(Some)
        },
        start: if command.clear_start {
            Some(None)
        } else {
            start.map(Some)
        },
        notes: if command.clear_notes {
            Some(None)
        } else {
            notes.map(Some)
        },
        location: if command.clear_location {
            Some(None)
        } else {
            command.location.clone().map(Some)
        },
        url: if command.clear_url {
            Some(None)
        } else {
            command.url.clone().map(Some)
        },
        priority_value: command.priority.map(priority_arg_value),
        ..Default::default()
    };
    let mut moved_list = None;
    if !write_list_selector_is_empty(&command.list_selector) {
        let lists = store.lists()?;
        let (list, _) = resolve_write_list(&lists, None, None, &command.list_selector)?;
        patch.list_id = Some(list.id.clone());
        moved_list = Some(list);
    }

    let resulting_list_id = moved_list
        .as_ref()
        .map(|list| list.id.as_str())
        .or(before.list_id.as_deref())
        .context("reminder list is unavailable")?;
    if let Some(parent_id) = command.parent_id.as_deref() {
        let parent = resolve_parent(store, parent_id)?;
        validate_parent_assignment(store, &id, resulting_list_id, &parent)?;
    } else if !command.clear_parent
        && let Some(parent_id) = before.parent_id.as_deref()
    {
        let parent = resolve_parent(store, parent_id)?;
        let parent_list_id = parent
            .list_id
            .as_deref()
            .context("parent reminder list is unavailable")?;
        if parent_list_id != resulting_list_id {
            bail!(
                "moving child reminder {} [{}] away from parent {} [{}] requires --clear-parent or --parent-id for a parent in the destination list",
                before.title,
                before.id,
                parent.title,
                parent.id
            );
        }
    }
    if moved_list.is_some() && before.child_count > 0 {
        bail!(
            "cannot move parent reminder {} [{}] while it has {} direct child reminder(s); reparent or clear those children first",
            before.title,
            before.id,
            before.child_count
        );
    }

    if command.time_zone.is_some() || command.clear_time_zone {
        rezone_unchanged_dates(
            &before,
            &mut patch,
            command.time_zone.as_deref(),
            command.clear_time_zone,
        )?;
        let result = preview_lifecycle_patch(&before, &patch, moved_list.as_ref());
        let has_timed_value = [result.due.as_ref(), result.start.as_ref()]
            .into_iter()
            .flatten()
            .any(|value| value.kind == ReminderDateKind::Datetime);
        if !has_timed_value {
            bail!("--time-zone and --clear-time-zone require a timed due or start value");
        }
    }

    let alarm_flags_supplied = command.notify_at_due
        || !command.notify_minutes_before.is_empty()
        || !command.schedule.notify_at.is_empty()
        || command.schedule.geofence_title.is_some();
    if command.clear_notifications && alarm_flags_supplied {
        bail!("--clear-notifications conflicts with notification and geofence options");
    }
    if command.clear_notifications {
        patch.notifications = Some(Vec::new());
        patch.geofence = Some(None);
    } else if alarm_flags_supplied {
        let result = preview_lifecycle_patch(&before, &patch, moved_list.as_ref());
        let mut notifications = build_notifications(
            result.due.as_ref(),
            command.notify_at_due,
            &command.notify_minutes_before,
        )?;
        notifications.extend(parse_absolute_notifications(&command.schedule.notify_at)?);
        let geofence = parse_geofence(&command.schedule)?;
        patch.notifications = Some(notifications);
        patch.geofence = Some(geofence);
    }

    if command.clear_recurrence && command.schedule.repeat.is_some() {
        bail!("--clear-recurrence conflicts with --repeat");
    }
    if command.clear_recurrence {
        patch.recurrence = Some(None);
    } else if let Some(recurrence) = parse_recurrence(&command.schedule)? {
        patch.recurrence = Some(Some(recurrence));
    }

    let final_preview = preview_lifecycle_patch(&before, &patch, moved_list.as_ref());
    let recurrence_rules = match &patch.recurrence {
        Some(Some(recurrence)) => vec![recurrence_report_from_parsed(recurrence)],
        Some(None) => Vec::new(),
        None => before.recurrence_rules.clone().unwrap_or_default(),
    };
    let recurrence_remains = !recurrence_rules.is_empty()
        || (patch.recurrence.is_none() && before.recurrence_count.unwrap_or_default() > 0);
    if recurrence_remains {
        let anchor = final_preview
            .due
            .as_ref()
            .or(final_preview.start.as_ref())
            .context(
                "recurrence requires a due or start date; clear recurrence when removing its final anchor",
            )?;
        for recurrence in &recurrence_rules {
            validate_recurrence_end_after_anchor(recurrence, Some(anchor))?;
        }
    }

    let changed_fields = lifecycle_changed_fields(&patch);
    if changed_fields.is_empty() {
        bail!("reminder update requires at least one field change");
    }
    let mut result = preview_lifecycle_patch(&before, &patch, moved_list.as_ref());
    result.due_input = command.due.clone();
    result.start_input = command.start.clone();
    result.write_action = Some("would_update".to_string());
    if command.dry_run {
        return Ok(reminder_mutation_dry_run(
            "update",
            changed_fields,
            before,
            result,
        ));
    }

    let mut reminder = store.update(&id, &patch)?;
    reminder.write_action = Some("updated".to_string());
    reminder.due_input = command.due;
    reminder.start_input = command.start;
    if moved_list.is_some() {
        reminder.list_selection = Some(ReminderListSelection::Explicit);
    }
    Ok(JsonOutput::Reminder {
        reminder: Box::new(reminder),
    })
}

pub(super) fn complete_reminder(
    store: &impl ReminderStore,
    reference: &str,
    completed_at: Option<&str>,
    dry_run: bool,
) -> Result<JsonOutput> {
    store.ensure_authorized()?;
    let id = resolve_reminder_ref(reference)?;
    let before = store.get(&id)?;
    ensure_reminder_writable(&before, "complete")?;
    if before.child_count > 0 {
        bail!(
            "cannot complete parent reminder {} [{}] while it has {} direct child reminder(s); complete or reparent the children first",
            before.title,
            before.id,
            before.child_count
        );
    }
    let completed_at = match completed_at {
        Some(value) => DateTime::parse_from_rfc3339(value)
            .with_context(|| {
                "--completed-at must be RFC3339 with an explicit UTC offset, for example 2026-07-11T14:00:00+03:00"
            })?
            .with_timezone(&Utc),
        None => Utc::now(),
    };
    let mut result = before.clone();
    result.completed = true;
    result.completion_date = Some(completed_at.to_rfc3339());
    result.write_action = Some("would_complete".to_string());
    if dry_run {
        return Ok(reminder_mutation_dry_run(
            "complete",
            vec!["completed".to_string(), "completion_date".to_string()],
            before,
            result,
        ));
    }
    let mut reminder = store.set_completion(&id, Some(completed_at))?;
    reminder.write_action = Some("completed".to_string());
    Ok(JsonOutput::Reminder {
        reminder: Box::new(reminder),
    })
}

pub(super) fn uncomplete_reminder(
    store: &impl ReminderStore,
    reference: &str,
    dry_run: bool,
) -> Result<JsonOutput> {
    store.ensure_authorized()?;
    let id = resolve_reminder_ref(reference)?;
    let before = store.get(&id)?;
    ensure_reminder_writable(&before, "uncomplete")?;
    let mut result = before.clone();
    result.completed = false;
    result.completion_date = None;
    result.write_action = Some("would_uncomplete".to_string());
    if dry_run {
        return Ok(reminder_mutation_dry_run(
            "uncomplete",
            vec!["completed".to_string(), "completion_date".to_string()],
            before,
            result,
        ));
    }
    let mut reminder = store.set_completion(&id, None)?;
    reminder.write_action = Some("uncompleted".to_string());
    Ok(JsonOutput::Reminder {
        reminder: Box::new(reminder),
    })
}

pub(super) fn delete_reminder(
    store: &impl ReminderStore,
    reference: &str,
    force: bool,
) -> Result<JsonOutput> {
    store.ensure_authorized()?;
    let id = resolve_reminder_ref(reference)?;
    let reminder = store.get(&id)?;
    ensure_reminder_writable(&reminder, "delete")?;
    if reminder.child_count > 0 {
        bail!(
            "cannot delete parent reminder {} [{}] while it has {} direct child reminder(s); reparent or delete the children first",
            reminder.title,
            reminder.id,
            reminder.child_count
        );
    }
    if !force {
        confirm_reminder_delete(&reminder)?;
    }
    store.delete(&id)?;
    Ok(JsonOutput::ReminderDeleted {
        deleted: ReminderDeletedReport {
            id,
            title: reminder.title,
            list: reminder.list,
            list_id: reminder.list_id,
        },
    })
}
