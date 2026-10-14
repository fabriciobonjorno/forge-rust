//! Monetary amounts.

use core::fmt;

use crate::domain::errors::ProductError;

/// A currency, written as a three-letter ISO 4217 style code in upper case.
///
/// Only the shape is validated; the list of active ISO codes changes over
/// time and is not a catalog rule.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Currency(String);

impl Currency {
    /// Validates a three-letter code and normalizes it to upper case.
    pub fn parse(value: &str) -> Result<Self, ProductError> {
        if value.len() != 3 || !value.bytes().all(|byte| byte.is_ascii_alphabetic()) {
            return Err(ProductError::InvalidCurrency);
        }
        Ok(Self(value.to_ascii_uppercase()))
    }

    /// Returns the upper-case code.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A non-negative amount in minor units (cents) of a currency.
///
/// Integer minor units avoid floating-point rounding; `i64` matches the
/// PostgreSQL `BIGINT` column.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Money {
    cents: i64,
    currency: Currency,
}

impl Money {
    /// Validates that `cents` is not negative.
    pub fn new(cents: i64, currency: Currency) -> Result<Self, ProductError> {
        if cents < 0 {
            return Err(ProductError::InvalidPrice);
        }
        Ok(Self { cents, currency })
    }

    /// Amount in minor units.
    #[must_use]
    pub const fn cents(&self) -> i64 {
        self.cents
    }

    /// Currency of the amount.
    #[must_use]
    pub const fn currency(&self) -> &Currency {
        &self.currency
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value_objects::generated;

    #[test]
    fn currency_is_normalized_to_upper_case() -> Result<(), ProductError> {
        for (input, expected) in [("EUR", "EUR"), ("usd", "USD"), ("bRl", "BRL")] {
            assert_eq!(Currency::parse(input)?.as_str(), expected);
        }
        Ok(())
    }

    #[test]
    fn invalid_currencies_are_rejected() {
        for input in ["", "EU", "EURO", "E1R", "EU ", "ÉUR", "€"] {
            assert_eq!(
                Currency::parse(input),
                Err(ProductError::InvalidCurrency),
                "{input:?}"
            );
        }
    }

    #[test]
    fn property_currency_accepts_exactly_three_ascii_letters() {
        for input in generated::strings(5_000, 5) {
            let expected_valid =
                input.chars().count() == 3 && input.chars().all(|c| c.is_ascii_alphabetic());

            match Currency::parse(&input) {
                Ok(currency) => {
                    assert!(expected_valid, "{input:?} must be rejected");
                    assert_eq!(currency.as_str(), input.to_ascii_uppercase());
                }
                Err(error) => {
                    assert!(!expected_valid, "{input:?} must be accepted");
                    assert_eq!(error, ProductError::InvalidCurrency);
                }
            }
        }
    }

    #[test]
    fn negative_amounts_are_rejected() -> Result<(), ProductError> {
        let euro = Currency::parse("EUR")?;
        for cents in [-1, -100, i64::MIN] {
            assert_eq!(
                Money::new(cents, euro.clone()),
                Err(ProductError::InvalidPrice)
            );
        }
        for cents in [0, 1, 1_299, i64::MAX] {
            let money = Money::new(cents, euro.clone())?;
            assert_eq!(money.cents(), cents);
            assert_eq!(money.currency(), &euro);
        }
        Ok(())
    }
}
