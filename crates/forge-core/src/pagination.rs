//! Keyset (cursor) pagination over UUIDv7 identifiers.
//!
//! UUIDv7 identifiers are time-ordered, so `WHERE id < $after ORDER BY id DESC
//! LIMIT $fetch_size` is a stable, index-friendly cursor: rows inserted while a
//! client pages do not shift or duplicate already-returned items. Repositories
//! fetch one row more than the page size ([`PageLimit::fetch_size`]) and build
//! the page with [`Page::from_overfetched`], which uses the extra row only to
//! detect whether a next page exists.
//!
//! ```
//! use forge_core::{Id, Page, PageLimit};
//!
//! enum Note {}
//! struct NoteRow { id: Id<Note> }
//!
//! let limit = PageLimit::new(2)?;
//! // Pretend these came from `... ORDER BY id DESC LIMIT limit.fetch_size()`.
//! let mut ids = vec![Id::<Note>::new(), Id::new(), Id::new()];
//! ids.sort_by(|a, b| b.cmp(a));
//! let rows: Vec<NoteRow> = ids.iter().map(|id| NoteRow { id: *id }).collect();
//!
//! let page = Page::from_overfetched(rows, limit, |row| row.id);
//! assert_eq!(page.items.len(), 2);
//! assert_eq!(page.next_cursor, Some(ids[1]));
//! # Ok::<(), forge_core::ForgeError>(())
//! ```

use core::fmt;

use serde::Serialize;

use crate::{ErrorCategory, ForgeError, Id};

/// Number of items requested for one page, between 1 and [`PageLimit::MAX`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PageLimit(u16);

impl PageLimit {
    /// Page size used when a client does not ask for one.
    pub const DEFAULT: Self = Self(20);

    /// Largest accepted page size.
    pub const MAX: u16 = 100;

    /// Validates a client-provided page size.
    ///
    /// Fails with a [`ErrorCategory::Validation`] error (code
    /// `invalid_page_limit`) unless `1 <= value <= MAX`.
    pub fn new(value: u16) -> Result<Self, ForgeError> {
        if value == 0 || value > Self::MAX {
            return Err(ForgeError::new(
                ErrorCategory::Validation,
                "invalid_page_limit",
                "page limit must be between 1 and 100",
            )
            .with_metadata("min", 1)
            .with_metadata("max", Self::MAX));
        }
        Ok(Self(value))
    }

    /// Returns the page size.
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    /// Returns the SQL `LIMIT` to use: one more than the page size, so the
    /// presence of an extra row reveals that a next page exists.
    #[must_use]
    pub const fn fetch_size(self) -> i64 {
        // Lossless widening; `i64::from` is not usable in a `const fn`.
        self.0 as i64 + 1
    }
}

impl Default for PageLimit {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A request for one page of `Id<T>`-keyed items, newest first.
pub struct PageRequest<T> {
    /// Maximum number of items to return.
    pub limit: PageLimit,
    /// Cursor from the previous page's [`Page::next_cursor`]; `None` requests
    /// the first page.
    pub after: Option<Id<T>>,
}

impl<T> Clone for PageRequest<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for PageRequest<T> {}

impl<T> fmt::Debug for PageRequest<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PageRequest")
            .field("limit", &self.limit)
            .field("after", &self.after)
            .finish()
    }
}

impl<T> Default for PageRequest<T> {
    fn default() -> Self {
        Self {
            limit: PageLimit::DEFAULT,
            after: None,
        }
    }
}

/// One page of items plus the cursor for the following page.
///
/// Serializes as `{"items":[...],"next_cursor":"<uuid>"|null}`.
#[derive(Serialize)]
#[serde(bound(serialize = "I: Serialize"))]
pub struct Page<T, I> {
    /// Items of this page, in query order (newest first).
    pub items: Vec<I>,
    /// Identifier to pass as [`PageRequest::after`] to fetch the next page;
    /// `None` when this is the last page.
    pub next_cursor: Option<Id<T>>,
}

impl<T, I> Page<T, I> {
    /// Builds a page from rows fetched with `LIMIT limit.fetch_size()`
    /// ordered by id `DESC`.
    ///
    /// When more than `limit` rows are present the surplus is discarded and
    /// the id of the last kept item becomes [`Page::next_cursor`].
    pub fn from_overfetched(
        mut rows: Vec<I>,
        limit: PageLimit,
        id_of: impl Fn(&I) -> Id<T>,
    ) -> Self {
        let page_size = usize::from(limit.get());
        if rows.len() <= page_size {
            return Self {
                items: rows,
                next_cursor: None,
            };
        }
        rows.truncate(page_size);
        let next_cursor = rows.last().map(id_of);
        Self {
            items: rows,
            next_cursor,
        }
    }
}

impl<T, I: Clone> Clone for Page<T, I> {
    fn clone(&self) -> Self {
        Self {
            items: self.items.clone(),
            next_cursor: self.next_cursor,
        }
    }
}

impl<T, I: fmt::Debug> fmt::Debug for Page<T, I> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Page")
            .field("items", &self.items)
            .field("next_cursor", &self.next_cursor)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    enum Item {}

    /// Distinct ids in descending order, as a `ORDER BY id DESC` query yields.
    fn descending_ids(count: usize) -> Vec<Id<Item>> {
        let mut ids: Vec<Id<Item>> = (0..count).map(|_| Id::new()).collect();
        ids.sort_by(|left, right| right.cmp(left));
        ids
    }

    #[test]
    fn limit_bounds_are_enforced() {
        for invalid in [0, PageLimit::MAX + 1, u16::MAX] {
            let error = PageLimit::new(invalid).err();
            assert_eq!(
                error.as_ref().map(ForgeError::code),
                Some("invalid_page_limit"),
                "{invalid}"
            );
            assert_eq!(
                error.map(|error| error.category()),
                Some(ErrorCategory::Validation)
            );
        }
        for valid in [1, 20, PageLimit::MAX] {
            assert_eq!(PageLimit::new(valid).map(PageLimit::get).ok(), Some(valid));
        }
    }

    #[test]
    fn defaults() {
        assert_eq!(PageLimit::default().get(), 20);
        assert_eq!(PageLimit::DEFAULT.fetch_size(), 21);
        let request = PageRequest::<Item>::default();
        assert_eq!(request.limit, PageLimit::DEFAULT);
        assert!(request.after.is_none());
    }

    #[test]
    fn fetch_size_is_limit_plus_one() -> Result<(), ForgeError> {
        for value in 1..=PageLimit::MAX {
            let limit = PageLimit::new(value)?;
            assert_eq!(limit.fetch_size(), i64::from(value) + 1);
        }
        Ok(())
    }

    /// Exhaustively checks the page invariants for every limit and every
    /// row count from zero to a few rows past the over-fetch size.
    #[test]
    fn from_overfetched_invariants_hold_for_all_boundaries() -> Result<(), ForgeError> {
        for value in 1..=PageLimit::MAX {
            let limit = PageLimit::new(value)?;
            let page_size = usize::from(value);
            for row_count in 0..=page_size + 3 {
                let ids = descending_ids(row_count);
                let page = Page::from_overfetched(ids.clone(), limit, |id| *id);

                let expected_len = row_count.min(page_size);
                assert_eq!(
                    page.items.len(),
                    expected_len,
                    "limit {value}, rows {row_count}"
                );
                assert_eq!(page.items[..], ids[..expected_len]);
                if row_count > page_size {
                    assert_eq!(page.next_cursor, page.items.last().copied());
                    assert!(page.next_cursor.is_some());
                } else {
                    assert_eq!(page.next_cursor, None, "limit {value}, rows {row_count}");
                }
            }
        }
        Ok(())
    }

    #[test]
    fn cursor_chain_visits_every_row_exactly_once() -> Result<(), ForgeError> {
        let all = descending_ids(23);
        let limit = PageLimit::new(5)?;
        let mut seen = Vec::new();
        let mut after: Option<Id<Item>> = None;
        loop {
            // Simulates `WHERE id < $after ORDER BY id DESC LIMIT fetch_size`.
            let fetch = usize::try_from(limit.fetch_size()).unwrap_or(usize::MAX);
            let rows: Vec<Id<Item>> = all
                .iter()
                .copied()
                .filter(|id| after.is_none_or(|cursor| *id < cursor))
                .take(fetch)
                .collect();
            let page = Page::from_overfetched(rows, limit, |id| *id);
            seen.extend(page.items.iter().copied());
            match page.next_cursor {
                Some(cursor) => after = Some(cursor),
                None => break,
            }
        }
        assert_eq!(seen, all);
        Ok(())
    }

    #[test]
    fn page_serializes_items_and_cursor() -> Result<(), serde_json::Error> {
        let ids = descending_ids(2);
        let page = Page::from_overfetched(vec![1_u8, 2], PageLimit::DEFAULT, |_| ids[0]);
        assert_eq!(
            serde_json::to_string(&page)?,
            r#"{"items":[1,2],"next_cursor":null}"#
        );

        let limited = Page::<Item, u8> {
            items: vec![1],
            next_cursor: Some(ids[1]),
        };
        let encoded = serde_json::to_string(&limited)?;
        assert!(encoded.contains(&ids[1].to_string()), "{encoded}");
        Ok(())
    }
}
