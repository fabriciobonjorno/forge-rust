use std::{borrow::Cow, collections::BTreeMap, error::Error as StdError};

use serde::{Deserialize, Serialize};

/// Structured, deterministic metadata attached to an error.
pub type ErrorMetadata = BTreeMap<String, serde_json::Value>;

/// Broad error classes used for transport mapping and operational policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCategory {
    /// User-provided input is invalid.
    Validation,
    /// A requested resource does not exist.
    NotFound,
    /// The request conflicts with current state.
    Conflict,
    /// Authentication is missing or invalid.
    Unauthenticated,
    /// The caller lacks permission.
    Forbidden,
    /// A request limit has been exceeded.
    RateLimited,
    /// A dependency or other infrastructure component failed.
    Infrastructure,
    /// An unexpected internal failure occurred.
    Internal,
}

impl ErrorCategory {
    /// Returns the conventional HTTP status associated with this category.
    ///
    /// The primitive status keeps the core crate independent of an HTTP stack.
    #[must_use]
    pub const fn http_status(self) -> u16 {
        match self {
            Self::Validation => 422,
            Self::NotFound => 404,
            Self::Conflict => 409,
            Self::Unauthenticated => 401,
            Self::Forbidden => 403,
            Self::RateLimited => 429,
            Self::Infrastructure | Self::Internal => 500,
        }
    }
}

/// The framework's typed application error.
///
/// `message` is safe to expose to a caller. A diagnostic source may be retained
/// for logs and tracing, but is intentionally omitted from [`ErrorReport`].
#[derive(Debug, thiserror::Error)]
#[error("{code}: {message}")]
pub struct ForgeError {
    code: Cow<'static, str>,
    category: ErrorCategory,
    message: Cow<'static, str>,
    metadata: ErrorMetadata,
    retryable: bool,
    #[source]
    source: Option<Box<dyn StdError + Send + Sync + 'static>>,
}

impl ForgeError {
    /// Creates an error with a stable machine-readable code and safe message.
    #[must_use]
    pub fn new(
        category: ErrorCategory,
        code: impl Into<Cow<'static, str>>,
        message: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            code: code.into(),
            category,
            message: message.into(),
            metadata: ErrorMetadata::new(),
            retryable: false,
            source: None,
        }
    }

    /// Returns the stable machine-readable error code.
    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    /// Returns this error's broad category.
    #[must_use]
    pub const fn category(&self) -> ErrorCategory {
        self.category
    }

    /// Returns the caller-safe message.
    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Returns structured caller-safe metadata.
    #[must_use]
    pub const fn metadata(&self) -> &ErrorMetadata {
        &self.metadata
    }

    /// Returns whether retrying this operation may succeed.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        self.retryable
    }

    /// Attaches structured caller-safe metadata.
    #[must_use]
    pub fn with_metadata(
        mut self,
        key: impl Into<String>,
        value: impl Into<serde_json::Value>,
    ) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Marks whether retrying this operation may succeed.
    #[must_use]
    pub const fn with_retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    /// Attaches an internal diagnostic cause.
    ///
    /// The cause remains accessible through [`std::error::Error::source`] and
    /// is never included in the public report.
    #[must_use]
    pub fn with_source(mut self, source: impl StdError + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Returns the framework-neutral HTTP status mapping.
    #[must_use]
    pub const fn http_status(&self) -> u16 {
        self.category.http_status()
    }

    /// Produces a serializable projection safe for transport responses.
    #[must_use]
    pub fn report(&self) -> ErrorReport<'_> {
        ErrorReport {
            code: self.code(),
            category: self.category,
            message: self.message(),
            metadata: &self.metadata,
            retryable: self.retryable,
        }
    }
}

/// A serialized, caller-safe representation of a [`ForgeError`].
#[derive(Debug, Serialize)]
pub struct ErrorReport<'a> {
    /// Stable machine-readable error code.
    pub code: &'a str,
    /// Broad error category.
    pub category: ErrorCategory,
    /// Human-readable message safe to expose to a caller.
    pub message: &'a str,
    /// Structured caller-safe context.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: &'a ErrorMetadata,
    /// Whether retrying may succeed.
    pub retryable: bool,
}

/// A Forge operation result.
pub type Result<T, E = ForgeError> = std::result::Result<T, E>;

#[cfg(test)]
mod tests {
    use std::{error::Error, io};

    use super::*;

    #[test]
    fn report_omits_internal_source() -> Result<(), serde_json::Error> {
        let error = ForgeError::new(
            ErrorCategory::Infrastructure,
            "database_unavailable",
            "service temporarily unavailable",
        )
        .with_retryable(true)
        .with_metadata("attempt", 2)
        .with_source(io::Error::other("password=secret"));

        let encoded = serde_json::to_string(&error.report())?;

        assert!(!encoded.contains("password"));
        assert!(encoded.contains("database_unavailable"));
        assert_eq!(error.http_status(), 500);
        assert!(error.source().is_some());
        Ok(())
    }

    #[test]
    fn categories_have_expected_http_mapping() {
        assert_eq!(ErrorCategory::Validation.http_status(), 422);
        assert_eq!(ErrorCategory::NotFound.http_status(), 404);
        assert_eq!(ErrorCategory::Conflict.http_status(), 409);
        assert_eq!(ErrorCategory::Unauthenticated.http_status(), 401);
        assert_eq!(ErrorCategory::Forbidden.http_status(), 403);
        assert_eq!(ErrorCategory::RateLimited.http_status(), 429);
    }
}
