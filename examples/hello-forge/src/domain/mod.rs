//! Business rules. This ring must not depend on HTTP, databases, queues,
//! filesystems, cloud vendors or AI providers. Entity identifiers use
//! `forge::core::Id<T>`, a typed UUIDv7.

pub mod entities;
pub mod errors;
pub mod events;
pub mod services;
pub mod value_objects;
