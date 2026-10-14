//! Stock keeping unit.

use core::fmt;

use crate::domain::errors::ProductError;

/// A stock keeping unit: 3–32 ASCII letters, digits or `-`, normalized to
/// upper case (`abc-1` and `ABC-1` are the same SKU).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Sku(String);

impl Sku {
    /// Shortest accepted SKU.
    pub const MIN_LEN: usize = 3;
    /// Longest accepted SKU.
    pub const MAX_LEN: usize = 32;

    /// Validates and normalizes a SKU.
    ///
    /// Surrounding whitespace is not trimmed: it is rejected like any other
    /// character outside the allowed set.
    pub fn parse(value: &str) -> Result<Self, ProductError> {
        // Every allowed character is one byte, so the byte length is the
        // character count whenever the character check passes.
        let valid_length = (Self::MIN_LEN..=Self::MAX_LEN).contains(&value.len());
        let valid_characters = value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-');
        if !(valid_length && valid_characters) {
            return Err(ProductError::InvalidSku);
        }
        Ok(Self(value.to_ascii_uppercase()))
    }

    /// Returns the normalized SKU.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Sku {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value_objects::generated;

    #[test]
    fn accepts_and_normalizes_valid_skus() -> Result<(), ProductError> {
        for (input, expected) in [
            ("abc", "ABC"),
            ("ABC-123", "ABC-123"),
            ("mug-red-xl", "MUG-RED-XL"),
            ("---", "---"),
            (
                "a1b2c3d4e5f6g7h8i9j0k1l2m3n4o5p6",
                "A1B2C3D4E5F6G7H8I9J0K1L2M3N4O5P6",
            ),
        ] {
            assert_eq!(Sku::parse(input)?.as_str(), expected);
        }
        Ok(())
    }

    #[test]
    fn rejects_invalid_skus() {
        let too_long = "A".repeat(Sku::MAX_LEN + 1);
        for input in [
            "", "AB", " ABC", "ABC ", "AB C", "ABC_1", "ÄBC", "ABC\n", &too_long,
        ] {
            assert_eq!(
                Sku::parse(input),
                Err(ProductError::InvalidSku),
                "{input:?}"
            );
        }
    }

    #[test]
    fn property_accepts_exactly_the_documented_language() {
        for input in generated::strings(5_000, Sku::MAX_LEN + 4) {
            let expected_valid = (Sku::MIN_LEN..=Sku::MAX_LEN).contains(&input.chars().count())
                && input.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');

            match Sku::parse(&input) {
                Ok(sku) => {
                    assert!(expected_valid, "{input:?} must be rejected");
                    assert_eq!(sku.as_str(), input.to_ascii_uppercase());
                    // Normalization is idempotent.
                    assert_eq!(Sku::parse(sku.as_str()).as_ref(), Ok(&sku));
                }
                Err(error) => {
                    assert!(!expected_valid, "{input:?} must be accepted");
                    assert_eq!(error, ProductError::InvalidSku);
                }
            }
        }
    }
}
