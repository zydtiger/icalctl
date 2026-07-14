use super::*;

pub fn run(command: RemindersCommand) -> Result<JsonOutput> {
    let store = EventKitReminderStore::new();
    run_with_store(&store, command)
}

pub(super) fn run_with_store(
    store: &impl ReminderStore,
    command: RemindersCommand,
) -> Result<JsonOutput> {
    match command {
        RemindersCommand::Status => Ok(JsonOutput::ReminderStatus(StatusReport {
            authorization: store.authorization_status().as_str().to_string(),
        })),
        RemindersCommand::Lists {
            source,
            writable_only,
        } => {
            store.ensure_authorized()?;
            let default_id = store.default_list().ok().map(|list| list.id);
            let mut lists = filter_list_discovery(store.lists()?, source.as_deref(), writable_only);
            mark_default_list(&mut lists, default_id.as_deref());
            sort_lists(&mut lists);
            Ok(JsonOutput::ReminderLists { lists })
        }
        RemindersCommand::DefaultList => {
            store.ensure_authorized()?;
            let mut list = store
                .default_list()
                .context("no default reminder list is available for new reminders")?;
            list.is_default_for_new_reminders = true;
            Ok(JsonOutput::DefaultReminderList { list })
        }
        RemindersCommand::List { filters } => list_reminders(store, filters, None),
        RemindersCommand::Search { query, filters } => {
            list_reminders(store, filters, Some(query.as_str()))
        }
        RemindersCommand::Show { id } => {
            store.ensure_authorized()?;
            let id = resolve_reminder_ref(&id)?;
            Ok(JsonOutput::Reminder {
                reminder: Box::new(store.get(&id)?),
            })
        }
        RemindersCommand::Add {
            title,
            json_file,
            list_selector,
            parent_id,
            due,
            start,
            time_zone,
            notes,
            notes_file,
            url,
            location,
            priority,
            notify_at_due,
            notify_minutes_before,
            schedule,
            if_exists,
            duplicate_window_seconds,
            dry_run,
        } => add_reminder_from_cli(
            store,
            AddReminderCliCommand {
                title,
                json_file,
                list_selector,
                parent_id,
                due,
                start,
                time_zone,
                notes,
                notes_file,
                url,
                location,
                priority,
                notify_at_due,
                notify_minutes_before,
                schedule,
                if_exists,
                duplicate_window_seconds,
                dry_run,
            },
        ),
        RemindersCommand::Batch { command } => match command {
            ReminderBatchCommand::Add {
                file,
                if_exists,
                dry_run,
                continue_on_error,
            } => reminder_batch_add(store, &file, if_exists, dry_run, continue_on_error),
        },
        RemindersCommand::Update {
            id,
            title,
            list_selector,
            parent_id,
            clear_parent,
            due,
            clear_due,
            start,
            clear_start,
            time_zone,
            clear_time_zone,
            notes,
            notes_file,
            clear_notes,
            url,
            clear_url,
            location,
            clear_location,
            priority,
            notify_at_due,
            notify_minutes_before,
            schedule,
            clear_notifications,
            clear_recurrence,
            dry_run,
        } => update_reminder(
            store,
            UpdateReminderCommand {
                id,
                title,
                list_selector,
                parent_id,
                clear_parent,
                due,
                clear_due,
                start,
                clear_start,
                time_zone,
                clear_time_zone,
                notes,
                notes_file,
                clear_notes,
                url,
                clear_url,
                location,
                clear_location,
                priority,
                notify_at_due,
                notify_minutes_before,
                schedule,
                clear_notifications,
                clear_recurrence,
                dry_run,
            },
        ),
        RemindersCommand::Complete {
            id,
            completed_at,
            dry_run,
        } => complete_reminder(store, &id, completed_at.as_deref(), dry_run),
        RemindersCommand::Uncomplete { id, dry_run } => uncomplete_reminder(store, &id, dry_run),
        RemindersCommand::Delete { id, force } => delete_reminder(store, &id, force),
    }
}
