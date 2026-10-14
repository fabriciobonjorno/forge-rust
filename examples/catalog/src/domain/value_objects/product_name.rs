//! Display name of a product.

use core::fmt;

use crate::domain::errors::ProductError;

/// A product name: 1–120 characters after trimming surrounding whitespace,
/// without control characters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProductName(String);

impl ProductName {
    /// Longest accepted name, in Unicode scalar values (PostgreSQL
    /// `char_length`).
    pub const MAX_CHARS: usize = 120;

    /// Trims and validates a name.
    pub fn parse(value: &str) -> Result<Self, ProductError> {
        let trimmed = value.trim();
        let length = trimmed.chars().count();
        if length == 0 || length > Self::MAX_CHARS || trimmed.chars().any(char::is_control) {
            return Err(ProductError::InvalidName);
        }
        Ok(Self(trimmed.to_owned()))
    }

    /// Returns the trimmed name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProductName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value_objects::generated;

    #[test]
    fn trims_and_accepts_valid_names() -> Result<(), ProductError> {
        let longest = "é".repeat(ProductName::MAX_CHARS);
        for (input, expected) in [
            ("Mug", "Mug"),
            ("  Espresso cup  ", "Espresso cup"),
            ("\u{a0}Tasse à café\u{a0}", "Tasse à café"),
            ("x", "x"),
            (longest.as_str(), longest.as_str()),
        ] {
            assert_eq!(ProductName::parse(input)?.as_str(), expected);
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_names() {
        let too_long = "a".repeat(ProductName::MAX_CHARS + 1);
        for input in ["", "   ", "\t\n", "Mug\nRed", "Mug\u{7f}", &too_long] {
            assert_eq!(
                ProductName::parse(input),
                Err(ProductError::InvalidName),
                "{input:?}"
            );
        }
    }

    #[test]
    fn property_accepts_exactly_the_documented_language() {
        for input in generated::strings(5_000, ProductName::MAX_CHARS + 10) {
            let trimmed = input.trim();
            let length = trimmed.chars().count();
            let expected_valid = (1..=ProductName::MAX_CHARS).contains(&length)
                && !trimmed.chars().any(char::is_control);

            match ProductName::parse(&input) {
                Ok(name) => {
                    assert!(expected_valid, "{input:?} must be rejected");
                    assert_eq!(name.as_str(), trimmed);
                    assert_eq!(ProductName::parse(name.as_str()).as_ref(), Ok(&name));
                }
                Err(error) => {
                    assert!(!expected_valid, "{input:?} must be accepted");
                    assert_eq!(error, ProductError::InvalidName);
                }
            }
        }
    }
}
