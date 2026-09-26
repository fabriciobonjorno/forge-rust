use core::{fmt, marker::PhantomData, str::FromStr};

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use uuid::{Uuid, Version};

/// A UUIDv7 identifier associated with the entity marker `T`.
///
/// The marker prevents identifiers for different entity types from being
/// interchanged at compile time while having no runtime size or performance
/// cost.
///
/// ```
/// use forge_core::Id;
///
/// enum User {}
/// enum Organization {}
///
/// let user_id = Id::<User>::new();
/// let organization_id = Id::<Organization>::new();
/// assert_ne!(user_id.to_string(), organization_id.to_string());
/// ```
///
/// Identifiers for different marker types cannot be mixed:
///
/// ```compile_fail
/// use forge_core::Id;
/// enum User {}
/// enum Organization {}
/// fn load_user(_: Id<User>) {}
/// load_user(Id::<Organization>::new());
/// ```
#[repr(transparent)]
pub struct Id<T> {
    value: Uuid,
    marker: PhantomData<fn() -> T>,
}

impl<T> Id<T> {
    /// Generates a time-sortable UUIDv7 identifier.
    #[must_use]
    pub fn new() -> Self {
        Self {
            value: Uuid::now_v7(),
            marker: PhantomData,
        }
    }

    /// Constructs a typed identifier after validating that `value` is UUIDv7.
    pub fn from_uuid(value: Uuid) -> Result<Self, InvalidId> {
        if value.get_version() != Some(Version::SortRand) {
            return Err(InvalidId::WrongVersion {
                actual: value.get_version_num(),
            });
        }

        Ok(Self {
            value,
            marker: PhantomData,
        })
    }

    /// Returns the underlying UUID value.
    #[must_use]
    pub const fn as_uuid(&self) -> &Uuid {
        &self.value
    }

    /// Consumes the typed identifier and returns its underlying UUID.
    #[must_use]
    pub const fn into_uuid(self) -> Uuid {
        self.value
    }
}

impl<T> Default for Id<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> Clone for Id<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Id<T> {}

impl<T> PartialEq for Id<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T> Eq for Id<T> {}

impl<T> PartialOrd for Id<T> {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl<T> Ord for Id<T> {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.value.cmp(&other.value)
    }
}

impl<T> core::hash::Hash for Id<T> {
    fn hash<H: core::hash::Hasher>(&self, state: &mut H) {
        self.value.hash(state);
    }
}

impl<T> fmt::Debug for Id<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_tuple("Id").field(&self.value).finish()
    }
}

impl<T> fmt::Display for Id<T> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.value.fmt(formatter)
    }
}

impl<T> FromStr for Id<T> {
    type Err = InvalidId;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let uuid = Uuid::parse_str(value).map_err(InvalidId::Malformed)?;
        Self::from_uuid(uuid)
    }
}

impl<T> TryFrom<Uuid> for Id<T> {
    type Error = InvalidId;

    fn try_from(value: Uuid) -> Result<Self, Self::Error> {
        Self::from_uuid(value)
    }
}

impl<T> From<Id<T>> for Uuid {
    fn from(value: Id<T>) -> Self {
        value.into_uuid()
    }
}

impl<T> Serialize for Id<T> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(&self.value)
    }
}

impl<'de, T> Deserialize<'de> for Id<T> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct IdVisitor<T>(PhantomData<fn() -> T>);

        impl<'de, T> de::Visitor<'de> for IdVisitor<T> {
            type Value = Id<T>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a hyphenated UUIDv7 string")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                value.parse().map_err(E::custom)
            }
        }

        deserializer.deserialize_str(IdVisitor(PhantomData))
    }
}

/// The reason a value could not be used as a typed UUIDv7 identifier.
#[derive(Debug, thiserror::Error)]
pub enum InvalidId {
    /// The input was not a syntactically valid UUID.
    #[error("invalid UUID: {0}")]
    Malformed(#[source] uuid::Error),

    /// The UUID was valid but did not use the mandatory version 7 layout.
    #[error("expected UUID version 7, got version {actual}")]
    WrongVersion {
        /// The version nibble found in the UUID.
        actual: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    enum User {}

    #[test]
    fn generated_ids_are_version_seven() {
        let id = Id::<User>::new();

        assert_eq!(id.as_uuid().get_version(), Some(Version::SortRand));
    }

    #[test]
    fn rejects_non_v7_uuid() {
        let result = Id::<User>::from_uuid(Uuid::nil());

        assert!(matches!(result, Err(InvalidId::WrongVersion { actual: 0 })));
    }

    #[test]
    fn serde_round_trip_preserves_id() -> Result<(), Box<dyn std::error::Error>> {
        let original = Id::<User>::new();
        let encoded = serde_json::to_string(&original)?;
        let decoded: Id<User> = serde_json::from_str(&encoded)?;

        assert_eq!(decoded, original);
        Ok(())
    }

    #[test]
    fn deserialization_rejects_non_v7_uuid() {
        let decoded = serde_json::from_str::<Id<User>>("\"00000000-0000-0000-0000-000000000000\"");

        assert!(decoded.is_err());
    }
}
