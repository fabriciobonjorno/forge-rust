//! Rule violations of the product aggregate and its value objects.
//!
//! The conversion into [`ForgeError`] lives here, next to the enum, so the
//! mapping stays exhaustive and every rule has exactly one stable code.
//! `forge::core` is a transport-neutral contract crate (no HTTP, database or
//! runtime types), the same crate that provides the domain's `Id` and
//! `Version`; the HTTP status for the `Validation` category is chosen by the
//! HTTP adapter, not here.

use core::{error::Error, fmt};

use forge::core::{ErrorCategory, ForgeError};

/// A value or state transition rejected by a product business rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProductError {
    /// The SKU is not 3–32 ASCII letters, digits or `-`.
    InvalidSku,
    /// The name is empty, longer than 120 characters or contains control
    /// characters.
    InvalidName,
    /// The price amount is negative.
    InvalidPrice,
    /// The currency is not a three-letter ISO 4217 style code.
    InvalidCurrency,
    /// A new price uses a different currency than the product.
    CurrencyMismatch,
}

impl ProductError {
    /// Stable machine-readable code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidSku => "invalid_sku",
            Self::InvalidName => "invalid_name",
            Self::InvalidPrice => "invalid_price",
            Self::InvalidCurrency => "invalid_currency",
            Self::CurrencyMismatch => "currency_mismatch",
        }
    }

    /// Name of the offending input field, as seen by API clients.
    #[must_use]
    pub const fn field(self) -> &'static str {
        match self {
            Self::InvalidSku => "sku",
            Self::InvalidName => "name",
            Self::InvalidPrice => "price.cents",
            Self::InvalidCurrency | Self::CurrencyMismatch => "price.currency",
        }
    }

    /// Caller-safe description of the violated rule.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidSku => {
                "SKU must be 3 to 32 characters of ASCII letters, digits or hyphens"
            }
            Self::InvalidName => "name must be 1 to 120 characters without control characters",
            Self::InvalidPrice => "price must not be negative",
            Self::InvalidCurrency => "currency must be a three-letter code such as EUR",
            Self::CurrencyMismatch => "a product's price cannot change currency",
        }
    }
}

impl fmt::Display for ProductError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl Error for ProductError {}

impl From<ProductError> for ForgeError {
    fn from(error: ProductError) -> Self {
        Self::new(ErrorCategory::Validation, error.code(), error.message())
            .with_metadata("field", error.field())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [ProductError; 5] = [
        ProductError::InvalidSku,
        ProductError::InvalidName,
        ProductError::InvalidPrice,
        ProductError::InvalidCurrency,
        ProductError::CurrencyMismatch,
    ];

    #[test]
    fn every_rule_maps_to_a_validation_error_with_its_field() {
        for rule in ALL {
            let error = ForgeError::from(rule);

            assert_eq!(error.category(), ErrorCategory::Validation, "{rule:?}");
            assert_eq!(error.code(), rule.code());
            assert_eq!(error.message(), rule.to_string());
            assert_eq!(
                error
                    .metadata()
                    .get("field")
                    .and_then(|field| field.as_str()),
                Some(rule.field())
            );
            assert!(!error.is_retryable());
        }
    }

    #[test]
    fn codes_are_unique() {
        for (index, rule) in ALL.iter().enumerate() {
            assert!(
                ALL[index + 1..]
                    .iter()
                    .all(|other| other.code() != rule.code()),
                "{rule:?}"
            );
        }
    }
}
