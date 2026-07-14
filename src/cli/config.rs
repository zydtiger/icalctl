use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum ConfigCommand {
    /// Print the configuration file path.
    Path,
    /// Create a documented configuration file with safe defaults.
    Init,
    /// Print the effective configuration with secrets redacted.
    Show,
    /// Validate the configuration file.
    Validate,
    /// Open the configuration file in $VISUAL or $EDITOR.
    Edit,
    /// Print one effective configuration value.
    Get {
        /// Dotted configuration key, for example calendar.default_calendar_id.
        key: String,
    },
    /// Set one configuration value.
    Set {
        /// Dotted configuration key, for example calendar.default_calendar_id.
        key: String,
        /// New value. Omit to be prompted; arrays use TOML syntax.
        value: Option<String>,
        /// Read the value from standard input.
        #[arg(long, conflicts_with = "value")]
        stdin: bool,
    },
    /// Remove one configuration value so its built-in default applies.
    Unset {
        /// Dotted configuration key.
        key: String,
    },
}
