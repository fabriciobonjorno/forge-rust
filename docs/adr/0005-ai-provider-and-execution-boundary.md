# ADR 0005: Separate AI generation from controlled execution

- Status: Accepted
- Date: 2026-09-26

## Context

AI providers have incompatible protocols and model capabilities. Model output is
probabilistic and can contain prompt-injected or malicious commands. Treating a
tool call as authorization would give untrusted output ambient system access.

## Decision

Application code uses provider-neutral AI traits. API, local-server, and CLI
integrations are adapters. Provider responses and tool requests are untrusted data.

No provider may execute a tool. Every requested action traverses intent parsing,
policy evaluation, principal/tenant authorization, typed argument validation,
optional human approval, a capability-limited executor, output filtering, and an
append-only audit event. An immutable execution plan binds the authorized action
to its arguments, limits, policy version, expiry, and unique ID. Sandbox profiles
default to no network, a minimal filesystem view, command allowlists, bounded CPU,
memory, output, and wall time.

Fallback routing may change providers only when data-classification and residency
policy permit it. Retries do not repeat non-idempotent execution.

## Consequences

- Vendor adapters are replaceable and business code stays vendor-neutral.
- Tool use has a reviewable trust boundary and complete decision trail.
- Some provider-specific capabilities require explicit extension interfaces.
- Sandboxing varies by operating system; unsupported guarantees must fail closed,
  not silently degrade.
