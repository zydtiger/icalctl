use super::*;

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

static TEMP_FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    static TEST_CONFIG_PATH: std::cell::RefCell<Option<PathBuf>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn with_test_config<T>(path: &Path, operation: impl FnOnce() -> T) -> T {
    TEST_CONFIG_PATH.with(|slot| {
        let previous = slot.replace(Some(path.to_path_buf()));
        let result = operation();
        slot.replace(previous);
        result
    })
}

#[cfg(test)]
pub(crate) fn with_test_config_contents<T>(
    name: &str,
    contents: &str,
    operation: impl FnOnce() -> T,
) -> T {
    let path = std::env::temp_dir()
        .join(format!(
            "icalctl-config-integration-{name}-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ))
        .join("config.toml");
    write_secure(&path, contents.as_bytes()).unwrap();
    let result = with_test_config(&path, operation);
    fs::remove_dir_all(path.parent().unwrap()).unwrap();
    result
}

pub(super) fn config_path() -> Result<PathBuf> {
    #[cfg(test)]
    if let Some(path) = TEST_CONFIG_PATH.with(|path| path.borrow().clone()) {
        return Ok(path);
    }
    #[cfg(test)]
    return Ok(std::env::temp_dir()
        .join("icalctl-test-default-config")
        .join(std::thread::current().name().unwrap_or("test"))
        .join("config.toml"));

    #[cfg(not(test))]
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    #[cfg(not(test))]
    Ok(PathBuf::from(home).join(".icalctl").join("config.toml"))
}

pub(super) fn load_from(path: &Path) -> Result<Config> {
    let contents = read_secure_string(path)?;
    let config: Config = toml::from_str(&contents).map_err(|error| {
        let location = error
            .span()
            .map(|span| line_and_column(&contents, span.start));
        match location {
            Some((line, column)) => anyhow!(
                "failed to parse configuration {} at line {line}, column {column}",
                path.display()
            ),
            None => anyhow!("failed to parse configuration {}", path.display()),
        }
    })?;
    validate(&config)?;
    Ok(config)
}

#[cfg(unix)]
pub(super) fn read_secure(path: &Path) -> Result<Vec<u8>> {
    let parent = path.parent().context("configuration path has no parent")?;
    validate_existing_directory(parent)?;
    let mut options = OpenOptions::new();
    options
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    let mut file = options
        .open(path)
        .with_context(|| format!("failed to open configuration {}", path.display()))?;
    let metadata = file
        .metadata()
        .with_context(|| format!("failed to inspect configuration {}", path.display()))?;
    if !metadata.is_file() {
        bail!("configuration is not a regular file: {}", path.display());
    }
    let expected_uid = unsafe { libc::geteuid() };
    if metadata.uid() != expected_uid {
        bail!(
            "configuration is not owned by the current user: {}",
            path.display()
        );
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        bail!(
            "configuration permissions are too broad: {}; run `chmod 600 {}`",
            path.display(),
            path.display()
        );
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        bail!(
            "configuration is too large: {} exceeds {} bytes",
            path.display(),
            MAX_CONFIG_BYTES
        );
    }
    let mut contents = Vec::with_capacity(metadata.len() as usize);
    file.read_to_end(&mut contents)
        .with_context(|| format!("failed to read configuration {}", path.display()))?;
    Ok(contents)
}

#[cfg(not(unix))]
pub(super) fn read_secure(path: &Path) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path)
        .with_context(|| format!("failed to inspect configuration {}", path.display()))?;
    if !metadata.is_file() {
        bail!("configuration is not a regular file: {}", path.display());
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        bail!(
            "configuration is too large: {} exceeds {} bytes",
            path.display(),
            MAX_CONFIG_BYTES
        );
    }
    fs::read(path).with_context(|| format!("failed to read configuration {}", path.display()))
}

pub(super) fn read_secure_string(path: &Path) -> Result<String> {
    String::from_utf8(read_secure(path)?)
        .with_context(|| format!("configuration is not valid UTF-8: {}", path.display()))
}

#[cfg(unix)]
pub(super) fn validate_existing_directory(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("failed to inspect {}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        bail!(
            "configuration directory must be a real directory: {}",
            path.display()
        );
    }
    let expected_uid = unsafe { libc::geteuid() };
    if metadata.uid() != expected_uid {
        bail!(
            "configuration directory is not owned by the current user: {}",
            path.display()
        );
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        bail!(
            "configuration directory permissions are too broad: {}; run `chmod 700 {}`",
            path.display(),
            path.display()
        );
    }
    Ok(())
}

#[cfg(not(unix))]
pub(super) fn validate_existing_directory(path: &Path) -> Result<()> {
    if !path.is_dir() {
        bail!(
            "configuration directory is not a directory: {}",
            path.display()
        );
    }
    Ok(())
}

pub(super) fn line_and_column(contents: &str, byte_offset: usize) -> (usize, usize) {
    let prefix = &contents[..byte_offset.min(contents.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.chars().count() + 1, |(_, tail)| {
            tail.chars().count() + 1
        });
    (line, column)
}

pub(super) fn write_secure(path: &Path, contents: &[u8]) -> Result<()> {
    let parent = path.parent().context("configuration path has no parent")?;
    if parent.exists() {
        validate_existing_directory(parent)?;
    } else {
        fs::create_dir(parent).with_context(|| format!("failed to create {}", parent.display()))?;
        #[cfg(unix)]
        fs::set_permissions(parent, fs::Permissions::from_mode(0o700))
            .with_context(|| format!("failed to secure {}", parent.display()))?;
        validate_existing_directory(parent)?;
    }

    for _ in 0..100 {
        let suffix = TEMP_FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = parent.join(format!(
            ".config.toml.{}.{}.tmp",
            std::process::id(),
            suffix
        ));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
        let mut file = match options.open(&temporary) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("failed to create {}", temporary.display()));
            }
        };
        let write_result = (|| -> Result<()> {
            file.write_all(contents)
                .with_context(|| format!("failed to write {}", temporary.display()))?;
            file.sync_all()
                .with_context(|| format!("failed to sync {}", temporary.display()))?;
            drop(file);
            fs::rename(&temporary, path)
                .with_context(|| format!("failed to replace {}", path.display()))?;
            Ok(())
        })();
        if write_result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        return write_result;
    }
    bail!("failed to create a unique temporary configuration file")
}
