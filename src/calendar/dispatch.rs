use super::*;

pub fn run(command: Command) -> Result<JsonOutput> {
    match command {
        Command::Config { command } => crate::config::run(command),
        Command::Version => Ok(JsonOutput::Version {
            version: crate::version::report(),
        }),
        Command::Status => Ok(JsonOutput::Status(StatusReport {
            authorization: authorization_string(),
        })),
        Command::Doctor => Ok(JsonOutput::Doctor {
            doctor: crate::doctor::doctor_report(),
        }),
        Command::Calendars {
            source,
            writable_only,
        } => {
            let events = authorized_events_manager()?;
            let default_id = match events.default_calendar() {
                Ok(calendar) => Some(calendar.identifier),
                Err(EventKitError::NoDefaultCalendar) => None,
                Err(error) => return Err(error).context("failed to read default calendar"),
            };
            let calendars = events
                .list_calendars()
                .context("failed to list Calendar calendars through EventKit")?;
            let calendars = filter_calendar_list(calendars, source.as_deref(), writable_only);
            let calendars = calendar_reports(&calendars, default_id.as_deref());
            Ok(JsonOutput::Calendars { calendars })
        }
        Command::DefaultCalendar => {
            let events = authorized_events_manager()?;
            let calendar = events
                .default_calendar()
                .context("no default calendar is available for new events")?;
            let mut calendar = CalendarReport::from(&calendar);
            calendar.is_default_for_new_events = true;
            Ok(JsonOutput::DefaultCalendar { calendar })
        }
        Command::List {
            from,
            to,
            calendar_selector,
        } => {
            let events = fetch_range(&from, &to, &calendar_selector)
                .with_context(|| format!("failed to list events from {from} to {to}"))?;
            Ok(JsonOutput::Events { events })
        }
        Command::Today { calendar_selector } => {
            let (start, end) = today_range()?;
            let events =
                fetch_events(start, end, &calendar_selector).context("failed to list today")?;
            Ok(JsonOutput::Events { events })
        }
        Command::Upcoming {
            days,
            calendar_selector,
        } => {
            if days <= 0 {
                bail!("--days must be greater than zero");
            }
            let start = Local::now();
            let end = start + chrono::Duration::days(days);
            let events = fetch_events(start, end, &calendar_selector)
                .context("failed to list upcoming events")?;
            Ok(JsonOutput::Events { events })
        }
        Command::Show {
            id,
            occurrence_start,
        } => {
            let reference = resolve_event_show_ref(&id, occurrence_start)?;
            let events = authorized_events_manager()?;
            Ok(JsonOutput::Event {
                event: Box::new(event_report_with_alarms(
                    &events,
                    &reference.id,
                    reference.occurrence_start.as_deref(),
                )?),
            })
        }
        Command::Search {
            query,
            from,
            to,
            calendar_selector,
        } => {
            let query = query.to_lowercase();
            let events = fetch_range(&from, &to, &calendar_selector)
                .with_context(|| format!("failed to search events from {from} to {to}"))?
                .into_iter()
                .filter(|event| event_matches(event, &query))
                .collect();
            Ok(JsonOutput::Events { events })
        }
        Command::Add {
            title,
            start,
            end,
            calendar_selector,
            notes,
            notes_file,
            json_file,
            location,
            url,
            all_day,
            availability,
            time_zone,
            alarm_minutes_before,
            recurrence,
            if_exists,
            duplicate_window_seconds,
            dry_run,
        } => {
            let input = resolve_add_command(AddCommandInput {
                title,
                start,
                end,
                calendar_selector,
                notes,
                notes_file,
                json_file,
                location,
                url,
                all_day,
                availability,
                time_zone,
                alarm_minutes_before,
                recurrence,
                if_exists,
                duplicate_window_seconds,
                dry_run,
            })?;
            let result = add_event(input)?;
            Ok(write_result_output(result))
        }
        Command::Update {
            id,
            occurrence_start,
            scope,
            title,
            start,
            end,
            calendar_selector,
            notes,
            clear_notes,
            location,
            clear_location,
            url,
            clear_url,
            all_day,
            timed,
            availability,
            time_zone,
            clear_time_zone,
            add_alarm_minutes_before,
            dry_run,
        } => {
            let result = update_event(UpdateEventInput {
                id,
                occurrence_start,
                scope,
                title,
                start,
                end,
                calendar_selector,
                notes,
                clear_notes,
                location,
                clear_location,
                url,
                clear_url,
                all_day,
                timed,
                availability,
                time_zone,
                clear_time_zone,
                add_alarm_minutes_before,
                dry_run,
            })?;
            Ok(write_result_output(result))
        }
        Command::Batch { command } => match command {
            BatchCommand::Add {
                file,
                if_exists,
                dry_run,
                continue_on_error,
            } => Ok(JsonOutput::Batch {
                batch: run_batch_add(&file, if_exists, dry_run, continue_on_error)?,
            }),
        },
        Command::Travel { command } => match command {
            TravelCommand::Serve { .. } => {
                unreachable!("travel serve is handled before calendar command dispatch")
            }
            TravelCommand::Flight {
                flight_number,
                from_airport,
                to_airport,
                departure,
                arrival,
                calendar_selector,
                notes,
                notes_file,
                url,
                availability,
                alarm_minutes_before,
                if_exists,
                duplicate_window_seconds,
                dry_run,
            } => {
                let extra_notes = match notes_file {
                    Some(path) => Some(read_notes_file(&path)?),
                    None => notes,
                };
                let flight = crate::travel::format_flight(crate::travel::FlightInput {
                    flight_number: &flight_number,
                    from_airport: &from_airport,
                    to_airport: &to_airport,
                    departure: &departure,
                    arrival: &arrival,
                    extra_notes: extra_notes.as_deref(),
                })?;
                let result = add_event(AddEventInput {
                    title: flight.title,
                    start: flight.start,
                    end: flight.end,
                    calendar_selector,
                    notes: Some(flight.notes),
                    location: Some(flight.location),
                    url,
                    all_day: false,
                    availability: Some(availability),
                    time_zone: None,
                    alarm_minutes_before,
                    recurrence: None,
                    if_exists,
                    duplicate_window_seconds,
                    dry_run,
                })?;
                Ok(write_result_output(result))
            }
        },
        Command::Reminders { command } => crate::reminders::run(command),
        Command::Delete {
            id,
            occurrence_start,
            scope,
            force,
        } => {
            let deleted = delete_event(&id, occurrence_start, scope, force)?;
            Ok(JsonOutput::Deleted { deleted })
        }
        Command::Completions { .. } => unreachable!("completions are handled before calendar run"),
    }
}
