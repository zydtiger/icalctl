use crate::models::VersionReport;

pub fn report() -> VersionReport {
    VersionReport {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        git_commit: option_env!("ICALCTL_GIT_COMMIT").map(str::to_string),
        target: env!("ICALCTL_BUILD_TARGET").to_string(),
        profile: env!("ICALCTL_BUILD_PROFILE").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_report_uses_cargo_package_version() {
        let report = report();
        assert_eq!(report.name, "icalctl");
        assert_eq!(report.version, env!("CARGO_PKG_VERSION"));
        assert!(!report.target.is_empty());
        assert!(!report.profile.is_empty());
    }
}
