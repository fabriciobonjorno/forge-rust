use core::fmt;

use serde::{Deserialize, Serialize};

use crate::{ErrorCategory, ForgeError};

/// Optimistic-locking version of a persisted aggregate.
///
/// Versions start at [`Version::INITIAL`] and only move forward through
/// [`Version::next`]. A repository update typically reads
/// `WHERE id = $1 AND version = $2`, writes `version = next`, and maps zero
/// affected rows to a conflict. The wire and persistence representation is a
/// plain positive integer (`BIGINT`).
///
/// ```
/// use forge_core::Version;
///
/// let version = Version::INITIAL;
/// assert_eq!(version.get(), 1);
/// assert_eq!(version.next()?.get(), 2);
/// assert!(Version::new(0).is_err());
/// # Ok::<(), forge_core::ForgeError>(())
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "i64", into = "i64")]
pub struct Version(i64);

impl Version {
    /// The version of a newly created aggregate.
    pub const INITIAL: Self = Self(1);

    /// Validates that `value` is a version number (at least 1).
    pub fn new(value: i64) -> Result<Self, ForgeError> {
        if value < Self::INITIAL.0 {
            return Err(ForgeError::new(
                ErrorCategory::Validation,
                "invalid_version",
                "version must be a positive integer",
            ));
        }
        Ok(Self(value))
    }

    /// Returns the numeric version.
    #[must_use]
    pub const fn get(self) -> i64 {
        self.0
    }

    /// Returns the version that follows this one.
    ///
    /// Fails instead of wrapping when the version space is exhausted.
    pub fn next(self) -> Result<Self, ForgeError> {
        self.0.checked_add(1).map(Self).ok_or_else(|| {
            ForgeError::new(
                ErrorCategory::Internal,
                "version_overflow",
                "version counter is exhausted",
            )
        })
    }
}

impl Default for Version {
    fn default() -> Self {
        Self::INITIAL
    }
}

impl fmt::Display for Version {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl TryFrom<i64> for Version {
    type Error = ForgeError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<Version> for i64 {
    fn from(value: Version) -> Self {
        value.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_version_is_one_and_default() {
        assert_eq!(Version::INITIAL.get(), 1);
        assert_eq!(Version::default(), Version::INITIAL);
    }

    #[test]
    fn non_positive_versions_are_rejected() {
        for invalid in [0, -1, i64::MIN] {
            let error = Version::new(invalid).err();
            assert_eq!(
                error.as_ref().map(ForgeError::code),
                Some("invalid_version"),
                "{invalid}"
            );
            assert_eq!(
                error.map(|error| error.category()),
                Some(ErrorCategory::Validation)
            );
        }
    }

    #[test]
    fn next_increments_and_detects_overflow() -> Result<(), ForgeError> {
        assert_eq!(Version::INITIAL.next()?.get(), 2);

        let last = Version::new(i64::MAX)?;
        let error = last.next().err();
        assert_eq!(
            error.as_ref().map(ForgeError::code),
            Some("version_overflow")
        );
        Ok(())
    }

    #[test]
    fn serializes_as_plain_integer_and_validates_on_deserialize()
    -> Result<(), Box<dyn std::error::Error>> {
        let version = Version::new(7)?;
        assert_eq!(serde_json::to_string(&version)?, "7");
        assert_eq!(serde_json::from_str::<Version>("7")?, version);
        assert!(serde_json::from_str::<Version>("0").is_err());
        assert_eq!(version.to_string(), "7");
        Ok(())
    }

    #[test]
    fn ordering_follows_numeric_value() -> Result<(), ForgeError> {
        assert!(Version::INITIAL < Version::INITIAL.next()?);
        Ok(())
    }
}
