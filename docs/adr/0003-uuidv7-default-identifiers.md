# ADR 0003: UUIDv7 typed identifiers by default

- Status: Accepted
- Date: 2026-09-26

## Context

Applications need globally unique identifiers that are efficient to index and do
not confuse entity types. UUIDv4 has poor insertion locality and untyped UUIDs can
be accidentally exchanged.

## Decision

All generators produce nominal entity ID newtypes backed by UUIDv7. New IDs are
generated through Forge's clock/randomness abstraction, serialize using canonical
UUID text, map to PostgreSQL `uuid`, and validate version 7 when parsed through the
default constructor. Test helpers can inject deterministic time and entropy.

Legacy UUID versions can enter only through a visibly named compatibility
constructor. UUID timestamps are ordering hints, not trusted authorization,
business time, or proof of creation time.

## Consequences

- Index locality and roughly time-ordered cursors improve.
- IDs expose approximate generation time and remain guess-resistant rather than
  secret; authorization must never rely on obscurity.
- Clock rollback and same-millisecond generation need conformance and monotonicity
  tests.
- Typed IDs add small wrapper and conversion costs at boundaries.
