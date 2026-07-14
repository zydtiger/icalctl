use super::*;

pub(crate) fn print_human_output(output: &JsonOutput) {
    match output {
        JsonOutput::Config { config } => {
            if let Some(contents) = &config.contents {
                print!("{contents}");
                if !contents.ends_with('\n') {
                    println!();
                }
            } else if let Some(value) = &config.value {
                println!("{value}");
            } else {
                println!("{}: {}", config.action, config.path);
            }
        }
        JsonOutput::Version { version } => {
            println!("{} {}", version.name, version.version);
            println!(
                "git commit: {}",
                version.git_commit.as_deref().unwrap_or("unknown")
            );
            println!("target: {}", version.target);
            println!("profile: {}", version.profile);
        }
        JsonOutput::Status(status) => {
            println!("Calendar authorization: {}", status.authorization);
        }
        JsonOutput::Doctor { doctor } => {
            println!("EventKit diagnostics");
            println!("Calendar authorization: {}", doctor.authorization);
            println!(
                "Reminders authorization: {}",
                doctor.reminders_authorization
            );
            println!("Location authorization: {}", doctor.location_authorization);
            println!(
                "Location services enabled: {}",
                doctor.location_services_enabled
            );
            println!(
                "process: {} [{}]",
                doctor.process.executable, doctor.process.pid
            );
            if let Some(terminal) = &doctor.process.terminal_program {
                println!("terminal program: {terminal}");
            }
            println!(
                "embedded Info.plist: {} bundle={}",
                doctor.info_plist.embedded, doctor.info_plist.bundle_identifier
            );
            println!("recommended command: {}", doctor.recommended_command);
            for step in &doctor.remediation {
                println!("- {step}");
            }
            println!(
                "recommended Reminders command: {}",
                doctor.recommended_reminders_command
            );
            for step in &doctor.reminders_remediation {
                println!("- {step}");
            }
            for step in &doctor.location_remediation {
                println!("- {step}");
            }
        }
        JsonOutput::Calendars { calendars } => {
            println!("Calendars ({})", calendars.len());
            for calendar in calendars {
                print_calendar(calendar);
            }
        }
        JsonOutput::DefaultCalendar { calendar } => {
            println!("Default calendar for new events");
            print_calendar(calendar);
        }
        JsonOutput::ReminderStatus(status) => {
            println!("Reminders authorization: {}", status.authorization);
        }
        JsonOutput::ReminderLists { lists } => print_reminder_lists(lists),
        JsonOutput::DefaultReminderList { list } => {
            println!("Default list for new reminders");
            print_reminder_list(list);
        }
        JsonOutput::Reminders { reminders } => print_reminders(reminders),
        JsonOutput::Reminder { reminder } => print_reminder_detail(reminder),
        JsonOutput::ReminderDryRun { draft, .. } => print_reminder_dry_run(draft),
        JsonOutput::ReminderMutationDryRun { draft, .. } => print_reminder_mutation_dry_run(draft),
        JsonOutput::ReminderDeleted { deleted } => {
            println!("Deleted reminder: {} [{}]", deleted.title, deleted.id);
        }
        JsonOutput::ReminderBatch { batch } => print_reminder_batch(batch),
        JsonOutput::Events { events } => print_events(events),
        JsonOutput::Event { event } => print_event_detail(event),
        JsonOutput::DryRun { draft, .. } => print_dry_run(draft),
        JsonOutput::Batch { batch } => print_batch(batch),
        JsonOutput::Deleted { deleted } => {
            println!("Deleted event: {} [{}]", deleted.title, deleted.id);
            if let Some(scope) = &deleted.scope {
                println!("scope: {scope}");
            }
        }
    }
}
