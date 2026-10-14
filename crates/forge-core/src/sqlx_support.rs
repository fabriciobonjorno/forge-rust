//! Driver-agnostic SQLx support for [`Id`] and [`Version`].
//!
//! Both types are encoded exactly like the primitive they wrap, so the column
//! type is whatever SQLx maps that primitive to on the active backend:
//!
//! | Rust | PostgreSQL | MySQL / MariaDB | SQLite |
//! | --- | --- | --- | --- |
//! | `Id<T>` | `uuid` | `BINARY(16)` | `BLOB` |
//! | `Version` | `BIGINT` | `BIGINT` | `INTEGER` |
//!
//! Decoding re-validates: a UUID that is not version 7, or a version below 1,
//! is a decode error on every backend instead of silently entering the domain.

use sqlx::{Database, Decode, Encode, Type, encode::IsNull, error::BoxDynError};
use uuid::Uuid;

use crate::{Id, Version};

impl<T, DB: Database> Type<DB> for Id<T>
where
    Uuid: Type<DB>,
{
    fn type_info() -> DB::TypeInfo {
        <Uuid as Type<DB>>::type_info()
    }

    fn compatible(ty: &DB::TypeInfo) -> bool {
        <Uuid as Type<DB>>::compatible(ty)
    }
}

impl<'q, T, DB: Database> Encode<'q, DB> for Id<T>
where
    Uuid: Encode<'q, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as Database>::ArgumentBuffer,
    ) -> Result<IsNull, BoxDynError> {
        <Uuid as Encode<'q, DB>>::encode_by_ref(self.as_uuid(), buf)
    }

    fn produces(&self) -> Option<DB::TypeInfo> {
        <Uuid as Encode<'q, DB>>::produces(self.as_uuid())
    }

    fn size_hint(&self) -> usize {
        <Uuid as Encode<'q, DB>>::size_hint(self.as_uuid())
    }
}

impl<'r, T, DB: Database> Decode<'r, DB> for Id<T>
where
    Uuid: Decode<'r, DB>,
{
    fn decode(value: <DB as Database>::ValueRef<'r>) -> Result<Self, BoxDynError> {
        let uuid = <Uuid as Decode<'r, DB>>::decode(value)?;
        Ok(Self::from_uuid(uuid)?)
    }
}

impl<DB: Database> Type<DB> for Version
where
    i64: Type<DB>,
{
    fn type_info() -> DB::TypeInfo {
        <i64 as Type<DB>>::type_info()
    }

    fn compatible(ty: &DB::TypeInfo) -> bool {
        <i64 as Type<DB>>::compatible(ty)
    }
}

impl<'q, DB: Database> Encode<'q, DB> for Version
where
    i64: Encode<'q, DB>,
{
    fn encode_by_ref(
        &self,
        buf: &mut <DB as Database>::ArgumentBuffer,
    ) -> Result<IsNull, BoxDynError> {
        <i64 as Encode<'q, DB>>::encode_by_ref(&self.get(), buf)
    }

    fn produces(&self) -> Option<DB::TypeInfo> {
        <i64 as Encode<'q, DB>>::produces(&self.get())
    }

    fn size_hint(&self) -> usize {
        <i64 as Encode<'q, DB>>::size_hint(&self.get())
    }
}

impl<'r, DB: Database> Decode<'r, DB> for Version
where
    i64: Decode<'r, DB>,
{
    fn decode(value: <DB as Database>::ValueRef<'r>) -> Result<Self, BoxDynError> {
        let raw = <i64 as Decode<'r, DB>>::decode(value)?;
        Ok(Self::new(raw)?)
    }
}

#[cfg(feature = "postgres")]
mod postgres {
    use sqlx::postgres::{PgHasArrayType, PgTypeInfo};
    use uuid::Uuid;

    use crate::{Id, Version};

    impl<T> PgHasArrayType for Id<T> {
        fn array_type_info() -> PgTypeInfo {
            <Uuid as PgHasArrayType>::array_type_info()
        }

        fn array_compatible(ty: &PgTypeInfo) -> bool {
            <Uuid as PgHasArrayType>::array_compatible(ty)
        }
    }

    impl PgHasArrayType for Version {
        fn array_type_info() -> PgTypeInfo {
            <i64 as PgHasArrayType>::array_type_info()
        }

        fn array_compatible(ty: &PgTypeInfo) -> bool {
            <i64 as PgHasArrayType>::array_compatible(ty)
        }
    }
}
