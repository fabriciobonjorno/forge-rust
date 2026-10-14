//! Percent-decoding shared by path parameters and query strings.

use std::borrow::Cow;

/// Why a percent-encoded string could not be decoded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DecodeError {
    /// A `%` not followed by two hexadecimal digits.
    InvalidEscape,
    /// The decoded bytes are not UTF-8.
    InvalidUtf8,
}

/// Decodes `%XX` escapes (and `+` as a space when `plus_as_space` is set, as
/// `application/x-www-form-urlencoded` requires). Borrows when there is
/// nothing to decode.
pub(crate) fn percent_decode(
    input: &str,
    plus_as_space: bool,
) -> Result<Cow<'_, str>, DecodeError> {
    let needs_decoding = input
        .bytes()
        .any(|byte| byte == b'%' || (plus_as_space && byte == b'+'));
    if !needs_decoding {
        return Ok(Cow::Borrowed(input));
    }
    let mut decoded = Vec::with_capacity(input.len());
    let mut bytes = input.bytes();
    while let Some(byte) = bytes.next() {
        match byte {
            b'%' => {
                let high = bytes.next().and_then(hex_value);
                let low = bytes.next().and_then(hex_value);
                let (Some(high), Some(low)) = (high, low) else {
                    return Err(DecodeError::InvalidEscape);
                };
                decoded.push((high << 4) | low);
            }
            b'+' if plus_as_space => decoded.push(b' '),
            other => decoded.push(other),
        }
    }
    String::from_utf8(decoded)
        .map(Cow::Owned)
        .map_err(|_| DecodeError::InvalidUtf8)
}

fn hex_value(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        b'A'..=b'F' => Some(digit - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_input_is_borrowed() {
        assert!(matches!(
            percent_decode("abc+d", false),
            Ok(Cow::Borrowed("abc+d"))
        ));
        assert!(matches!(
            percent_decode("abc", true),
            Ok(Cow::Borrowed("abc"))
        ));
    }

    #[test]
    fn escapes_and_plus_are_decoded() {
        assert_eq!(percent_decode("caf%C3%A9", false).as_deref(), Ok("café"));
        assert_eq!(percent_decode("a%2fb%2F", false).as_deref(), Ok("a/b/"));
        assert_eq!(percent_decode("a+b%2B", true).as_deref(), Ok("a b+"));
        assert_eq!(percent_decode("a+b", false).as_deref(), Ok("a+b"));
    }

    #[test]
    fn malformed_escapes_and_invalid_utf8_are_rejected() {
        for input in ["%", "%4", "%zz", "%4g", "a%", "%%41"] {
            assert_eq!(
                percent_decode(input, true),
                Err(DecodeError::InvalidEscape),
                "{input:?}"
            );
        }
        for input in ["%FF", "%C3", "%C3%28", "%ED%A0%80"] {
            assert_eq!(
                percent_decode(input, true),
                Err(DecodeError::InvalidUtf8),
                "{input:?}"
            );
        }
    }

    #[test]
    fn every_byte_round_trips_through_its_escape() {
        for byte in 0_u8..=0x7F {
            let encoded = format!("%{byte:02X}");
            let lower = format!("%{byte:02x}");
            let expected = char::from(byte).to_string();
            assert_eq!(percent_decode(&encoded, false).as_deref(), Ok(&*expected));
            assert_eq!(percent_decode(&lower, true).as_deref(), Ok(&*expected));
        }
    }
}
