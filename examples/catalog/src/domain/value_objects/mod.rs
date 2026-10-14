//! Value objects: immutable, self-validating domain values.

mod money;
mod product_name;
mod sku;

pub use money::{Currency, Money};
pub use product_name::ProductName;
pub use sku::Sku;

/// Deterministic pseudo-random inputs for property-style tests, without a
/// property-testing dependency.
#[cfg(test)]
mod generated {
    /// Characters mixing valid and invalid input for every value object:
    /// ASCII letters in both cases, digits, punctuation, whitespace, control
    /// characters and multi-byte letters.
    const ALPHABET: &[char] = &[
        'a', 'b', 'k', 'x', 'z', 'A', 'B', 'K', 'X', 'Z', '0', '1', '5', '9', '-', '_', ' ', '.',
        '/', '\t', '\n', '\u{7f}', 'é', 'ß', 'Ж', '中', '\u{a0}',
    ];

    /// Returns `count` strings of length `0..=max_len` drawn from
    /// [`ALPHABET`] with a fixed-seed xorshift generator, so failures are
    /// reproducible.
    pub(crate) fn strings(count: usize, max_len: usize) -> Vec<String> {
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            usize::try_from(state % (bound as u64)).unwrap_or(0)
        };
        (0..count)
            .map(|_| {
                let length = next(max_len + 1);
                (0..length)
                    .map(|_| ALPHABET[next(ALPHABET.len())])
                    .collect()
            })
            .collect()
    }
}
