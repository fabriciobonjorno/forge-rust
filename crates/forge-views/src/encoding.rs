//! Standard Base64 (RFC 4648 section 4, with padding) for CSP hash sources.
//!
//! This is an encoding, not cryptography; the digest itself comes from the
//! audited `sha2` crate.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Encodes `bytes` as padded standard Base64.
pub(crate) fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk.first().copied().unwrap_or(0);
        let second = chunk.get(1).copied();
        let third = chunk.get(2).copied();
        let block = (u32::from(first) << 16)
            | (u32::from(second.unwrap_or(0)) << 8)
            | u32::from(third.unwrap_or(0));
        out.push(sextet(block >> 18));
        out.push(sextet(block >> 12));
        out.push(if second.is_some() {
            sextet(block >> 6)
        } else {
            '='
        });
        out.push(if third.is_some() { sextet(block) } else { '=' });
    }
    out
}

/// The alphabet character for the low six bits of `value`.
fn sextet(value: u32) -> char {
    // The mask keeps the index below 64, so the lookup cannot miss; `'A'` is
    // never produced by the fallback.
    let index = (value & 0x3f) as usize;
    ALPHABET.get(index).copied().map_or('A', char::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_the_rfc_4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input.as_bytes()), expected, "{input:?}");
        }
    }

    #[test]
    fn encodes_high_bytes_with_the_url_unsafe_alphabet() {
        assert_eq!(base64(&[0xfb, 0xff, 0xbf]), "+/+/");
    }
}
