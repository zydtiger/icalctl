use crate::models::{DoctorReport, InfoPlistReport, ProcessReport};
use crate::reminders::ReminderAuthorization;
use eventkit::{AuthorizationStatus, EventsManager};
use objc2_core_location::{CLAuthorizationStatus, CLLocationManager};
use std::fmt::Debug;

const BUNDLE_IDENTIFIER: &str = "dev.zyd.icalctl";
const INFO_PLIST: &str = include_str!("../Info.plist");

pub fn doctor_report() -> DoctorReport {
    let authorization = EventsManager::authorization_status();
    let reminders_authorization = ReminderAuthorization::current();
    let location = location_capability();
    let executable = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|error| format!("unknown ({error})"));
    let terminal_program = std::env::var("TERM_PROGRAM").ok();

    DoctorReport {
        authorization: authorization_name(authorization).to_string(),
        reminders_authorization: reminders_authorization.as_str().to_string(),
        location_authorization: location.authorization.clone(),
        location_services_enabled: location.services_enabled,
        process: ProcessReport {
            pid: std::process::id(),
            executable,
            terminal_program,
        },
        info_plist: InfoPlistReport {
            embedded: cfg!(target_os = "macos"),
            bundle_identifier: BUNDLE_IDENTIFIER.to_string(),
            has_full_access_usage_description: INFO_PLIST
                .contains("NSCalendarsFullAccessUsageDescription"),
            has_legacy_usage_description: INFO_PLIST.contains("NSCalendarsUsageDescription"),
            has_write_only_usage_description: INFO_PLIST
                .contains("NSCalendarsWriteOnlyAccessUsageDescription"),
            has_reminders_full_access_usage_description: INFO_PLIST
                .contains("NSRemindersFullAccessUsageDescription"),
            has_reminders_legacy_usage_description: INFO_PLIST
                .contains("NSRemindersUsageDescription"),
            has_location_usage_description: INFO_PLIST
                .contains("NSLocationWhenInUseUsageDescription"),
        },
        recommended_command: recommended_command(authorization).to_string(),
        recommended_reminders_command: recommended_reminders_command(reminders_authorization)
            .to_string(),
        remediation: remediation(authorization),
        reminders_remediation: reminders_remediation(reminders_authorization),
        location_remediation: location_remediation(&location),
    }
}

#[derive(Clone, Debug)]
pub struct LocationCapability {
    pub authorization: String,
    pub services_enabled: bool,
}

pub fn location_capability() -> LocationCapability {
    let manager = unsafe { CLLocationManager::new() };
    let status = unsafe { manager.authorizationStatus() };
    LocationCapability {
        authorization: location_authorization_name(status).to_string(),
        services_enabled: unsafe { CLLocationManager::locationServicesEnabled_class() },
    }
}

fn location_authorization_name(status: CLAuthorizationStatus) -> &'static str {
    if status == CLAuthorizationStatus::AuthorizedAlways {
        "AuthorizedAlways"
    } else if status == CLAuthorizationStatus::AuthorizedWhenInUse {
        "AuthorizedWhenInUse"
    } else if status == CLAuthorizationStatus::NotDetermined {
        "NotDetermined"
    } else if status == CLAuthorizationStatus::Restricted {
        "Restricted"
    } else if status == CLAuthorizationStatus::Denied {
        "Denied"
    } else {
        "Unknown"
    }
}

fn location_remediation(capability: &LocationCapability) -> Vec<String> {
    if !capability.services_enabled {
        return vec![
            "Location Services are disabled. This does not block writing an explicit-coordinate EventKit geofence, but macOS may not deliver location triggers until services are enabled."
                .to_string(),
        ];
    }
    match capability.authorization.as_str() {
        "AuthorizedAlways" | "AuthorizedWhenInUse" => vec![
            "Location access is authorized. Explicit-coordinate EventKit geofence creation does not read the device's current location."
                .to_string(),
        ],
        "NotDetermined" => vec![
            "Location access has not been decided. Explicit-coordinate EventKit geofence creation does not request the device's current location; this status is informational."
                .to_string(),
        ],
        "Denied" => vec![
            "Location access is denied. This does not block writing an explicit-coordinate EventKit geofence because icalctl does not read the device's current location."
                .to_string(),
        ],
        "Restricted" => vec![
            "Location access is restricted by system policy. Explicit-coordinate EventKit geofence creation does not consume current location, though macOS delivery remains system-controlled."
                .to_string(),
        ],
        _ => vec![
            "macOS returned an unknown Location authorization status; inspect System Settings before using reminder geofences."
                .to_string(),
        ],
    }
}

fn recommended_reminders_command(status: ReminderAuthorization) -> &'static str {
    match status {
        ReminderAuthorization::FullAccess => "icalctl reminders lists --json",
        ReminderAuthorization::NotDetermined => "icalctl reminders lists",
        ReminderAuthorization::WriteOnly
        | ReminderAuthorization::Denied
        | ReminderAuthorization::Restricted
        | ReminderAuthorization::Unknown => "icalctl doctor --json",
    }
}

fn reminders_remediation(status: ReminderAuthorization) -> Vec<String> {
    match status {
        ReminderAuthorization::FullAccess => vec![
            "Reminders full access is ready; prefer exact list ids for automation.".to_string(),
        ],
        ReminderAuthorization::NotDetermined => vec![
            "Open Terminal.app, iTerm, or Ghostty outside an embedded tool sandbox.".to_string(),
            "Run `icalctl reminders lists` and approve the macOS Reminders prompt.".to_string(),
            "Run `icalctl doctor --json` again to verify Reminders FullAccess.".to_string(),
        ],
        ReminderAuthorization::WriteOnly => vec![
            "Open System Settings > Privacy & Security > Reminders.".to_string(),
            "Enable full Reminders access for icalctl or its launching terminal.".to_string(),
        ],
        ReminderAuthorization::Denied => vec![
            "Open System Settings > Privacy & Security > Reminders and enable access.".to_string(),
            format!(
                "If the entry is stale, run `tccutil reset Reminders {BUNDLE_IDENTIFIER}` from a terminal, then rerun `icalctl reminders lists`."
            ),
        ],
        ReminderAuthorization::Restricted => vec![
            "Reminders access is restricted by system policy; contact the device administrator."
                .to_string(),
        ],
        ReminderAuthorization::Unknown => vec![
            "macOS returned an unknown Reminders authorization status; rerun from a normal terminal and inspect `icalctl doctor --json`."
                .to_string(),
        ],
    }
}

pub fn access_request_error_message(error: &impl Debug) -> String {
    let detail = format!("{error:?}");
    let mach_hint = if detail.contains("NSMachErrorDomain") || detail.contains("4099") {
        " The failure looks like the known NSMachErrorDomain/Mach 4099 launch-context problem."
    } else {
        ""
    };
    format!(
        "failed to request full Calendar access (authorization=NotDetermined).{mach_hint} Run `icalctl calendars` from Terminal.app, iTerm, or Ghostty and approve the macOS Calendar prompt; then run `icalctl doctor --json`. Original error: {detail}"
    )
}

fn authorization_name(status: AuthorizationStatus) -> &'static str {
    match status {
        AuthorizationStatus::FullAccess => "FullAccess",
        AuthorizationStatus::WriteOnly => "WriteOnly",
        AuthorizationStatus::NotDetermined => "NotDetermined",
        AuthorizationStatus::Denied => "Denied",
        AuthorizationStatus::Restricted => "Restricted",
    }
}

fn recommended_command(status: AuthorizationStatus) -> &'static str {
    match status {
        AuthorizationStatus::FullAccess => "icalctl calendars --json",
        AuthorizationStatus::NotDetermined => "icalctl calendars",
        AuthorizationStatus::WriteOnly
        | AuthorizationStatus::Denied
        | AuthorizationStatus::Restricted => "icalctl doctor --json",
    }
}

fn remediation(status: AuthorizationStatus) -> Vec<String> {
    match status {
        AuthorizationStatus::FullAccess => vec![
            "Calendar full access is ready; use exact calendar ids for automated writes."
                .to_string(),
        ],
        AuthorizationStatus::NotDetermined => vec![
            "Open Terminal.app, iTerm, or Ghostty outside an embedded tool sandbox.".to_string(),
            "Run `icalctl calendars` and approve the macOS Calendar prompt.".to_string(),
            "Run `icalctl doctor --json` again to verify FullAccess.".to_string(),
        ],
        AuthorizationStatus::WriteOnly => vec![
            "Open System Settings > Privacy & Security > Calendars.".to_string(),
            "Enable Full Calendar Access for icalctl or its launching terminal.".to_string(),
        ],
        AuthorizationStatus::Denied => vec![
            "Open System Settings > Privacy & Security > Calendars and enable access.".to_string(),
            format!(
                "If the entry is stale, run `tccutil reset Calendar {BUNDLE_IDENTIFIER}` from a terminal, then rerun `icalctl calendars`."
            ),
        ],
        AuthorizationStatus::Restricted => vec![
            "Calendar access is restricted by system policy; contact the device administrator."
                .to_string(),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_determined_recommends_a_real_calendar_command() {
        assert_eq!(
            recommended_command(AuthorizationStatus::NotDetermined),
            "icalctl calendars"
        );
        assert!(
            remediation(AuthorizationStatus::NotDetermined)
                .iter()
                .any(|step| step.contains("Terminal.app"))
        );
    }

    #[test]
    fn mach_errors_include_status_and_specific_remediation() {
        let message = access_request_error_message(&"NSMachErrorDomain Code=4099");

        assert!(message.contains("authorization=NotDetermined"));
        assert!(message.contains("Mach 4099"));
        assert!(message.contains("icalctl calendars"));
        assert!(message.contains("Terminal.app"));
    }

    #[test]
    fn plist_diagnostics_find_required_calendar_keys() {
        let report = doctor_report();

        assert_eq!(report.info_plist.bundle_identifier, BUNDLE_IDENTIFIER);
        assert!(report.info_plist.has_full_access_usage_description);
        assert!(report.info_plist.has_legacy_usage_description);
        assert!(report.info_plist.has_write_only_usage_description);
        assert!(
            report
                .info_plist
                .has_reminders_full_access_usage_description
        );
        assert!(report.info_plist.has_reminders_legacy_usage_description);
        assert!(report.info_plist.has_location_usage_description);
    }
}
