use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::CliError;

/// Creates `migrations/<UTC timestamp>_<name>.up.sql` and `.down.sql`.
///
/// The version is the UTC time as `YYYYMMDDHHMMSS`, the format understood by
/// SQLx migrators. Existing files are never overwritten.
pub(crate) fn create(
    application: &Path,
    name: &str,
    now: SystemTime,
) -> Result<Vec<PathBuf>, CliError> {
    validate_name(name)?;
    if !application.join("Cargo.toml").is_file() {
        return Err(CliError::NotAnApplication(application.to_path_buf()));
    }

    let directory = application.join("migrations");
    fs::create_dir_all(&directory).map_err(|source| CliError::CreateDirectory {
        path: directory.clone(),
        source,
    })?;

    let version = version(now)?;
    let up = directory.join(format!("{version}_{name}.up.sql"));
    let down = directory.join(format!("{version}_{name}.down.sql"));
    if let Some(existing) = [&up, &down].into_iter().find(|path| path.exists()) {
        return Err(CliError::MigrationExists(existing.clone()));
    }

    create_new(
        &up,
        &format!(
            "-- {name}: forward migration.\n\
             -- Released migrations are immutable; change the schema with a new migration.\n"
        ),
    )?;
    if let Err(error) = create_new(
        &down,
        &format!("-- {name}: reverts {version}_{name}.up.sql for `rollback`.\n"),
    ) {
        let _cleanup_result = fs::remove_file(&up);
        return Err(error);
    }
    Ok(vec![up, down])
}

fn validate_name(name: &str) -> Result<(), CliError> {
    let reason = if name.is_empty() {
        Some("the name cannot be empty")
    } else if name.len() > 64 {
        Some("use at most 64 characters")
    } else if !name.as_bytes()[0].is_ascii_lowercase() {
        Some("start with an ASCII lowercase letter")
    } else if !name
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        Some("use snake_case: lowercase letters, digits and `_`")
    } else if name.ends_with('_') || name.contains("__") {
        Some("do not end with `_` or repeat `_`")
    } else {
        None
    };
    match reason {
        Some(reason) => Err(CliError::InvalidMigrationName {
            name: name.to_owned(),
            reason,
        }),
        None => Ok(()),
    }
}

fn create_new(path: &Path, content: &str) -> Result<(), CliError> {
    let write = || -> std::io::Result<()> {
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        file.write_all(content.as_bytes())
    };
    write().map_err(|source| {
        if source.kind() == std::io::ErrorKind::AlreadyExists {
            CliError::MigrationExists(path.to_path_buf())
        } else {
            CliError::WriteFile {
                path: path.to_path_buf(),
                source,
            }
        }
    })
}

/// Formats `now` as a UTC `YYYYMMDDHHMMSS` migration version.
fn version(now: SystemTime) -> Result<String, CliError> {
    let seconds = now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| CliError::Clock)?
        .as_secs();
    let days = i64::try_from(seconds / 86_400).map_err(|_| CliError::Clock)?;
    let second_of_day = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    Ok(format!(
        "{year:04}{month:02}{day:02}{:02}{:02}{:02}",
        second_of_day / 3_600,
        second_of_day % 3_600 / 60,
        second_of_day % 60
    ))
}

/// Converts days since 1970-01-01 into a proleptic Gregorian date
/// (Howard Hinnant's `civil_from_days` algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    // `day` is in 1..=31 and `month` in 1..=12 by construction.
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn at(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn versions_are_utc_timestamps() {
        assert_eq!(version(at(0)).expect("valid"), "19700101000000");
        assert_eq!(version(at(951_782_400)).expect("valid"), "20000229000000");
        assert_eq!(version(at(1_790_427_845)).expect("valid"), "20260926130405");
        assert_eq!(version(at(4_102_444_799)).expect("valid"), "20991231235959");
    }

    #[test]
    fn versions_sort_chronologically() {
        let mut previous = version(at(0)).expect("valid");
        for seconds in (0..4_102_444_800_u64).step_by(9_999_991) {
            let current = version(at(seconds)).expect("valid");
            assert!(current >= previous, "{current} < {previous}");
            previous = current;
        }
    }

    #[test]
    fn validates_migration_names() {
        for name in ["create_customers", "add_index2", "a"] {
            assert!(validate_name(name).is_ok(), "rejected {name}");
        }
        for name in [
            "",
            "Create",
            "2add",
            "add-index",
            "add__x",
            "add_",
            "../x",
            "a b",
            "ção",
        ] {
            assert!(validate_name(name).is_err(), "accepted {name}");
        }
    }

    #[test]
    fn creates_reversible_pair_and_refuses_overwrite() {
        let directory = tempfile::tempdir().expect("temporary directory should be available");
        fs::write(directory.path().join("Cargo.toml"), "").expect("manifest fixture");
        let now = at(1_790_427_845);

        let files = create(directory.path(), "create_customers", now).expect("created");

        let migrations = directory.path().join("migrations");
        assert_eq!(
            files,
            vec![
                migrations.join("20260926130405_create_customers.up.sql"),
                migrations.join("20260926130405_create_customers.down.sql"),
            ]
        );
        let error = create(directory.path(), "create_customers", now).expect_err("duplicate");
        assert!(matches!(error, CliError::MigrationExists(_)));
    }

    #[test]
    fn requires_an_application_directory() {
        let directory = tempfile::tempdir().expect("temporary directory should be available");

        let error = create(directory.path(), "create_customers", at(0)).expect_err("no manifest");

        assert!(matches!(error, CliError::NotAnApplication(_)));
        assert!(!directory.path().join("migrations").exists());
    }
}
