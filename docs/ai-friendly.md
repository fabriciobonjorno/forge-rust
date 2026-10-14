# AI-friendly authoring

Forge must make it straightforward for a coding agent to understand, change,
generate, and verify an application without relying on undocumented conventions.
This describes a product requirement and delivery checklist; unchecked items are
not implemented merely because they appear here.

## Principles

- **Discoverable:** expose commands, project capabilities, required inputs, and
  next steps through stable CLI help and machine-readable output.
- **Deterministic:** generators produce stable output, show a preview/diff, reject
  conflicting or unsafe overwrites, and identify every changed path.
- **Context-rich:** generated projects explain their architecture, commands,
  domain boundaries, data model, and relevant security constraints in concise
  `AGENTS.md` / `docs/for-ai.md` files that match the actual generated features.
- **Verifiable:** every generated or modified artifact has a documented check;
  diagnostics identify the file, field or resource, stable error code, and a safe
  corrective action. Machine output is versioned and does not require parsing
  human prose.
- **Least authority:** AI suggestions are untrusted. Planning, policy,
  authorization, typed validation, approval where required, bounded execution,
  and audit remain separate stages as defined in ADR 0005.
- **Provider-neutral:** provider/model choice is configuration. Local and remote
  adapters may advertise different capabilities; routing must honor policy,
  data classification, deadlines, and cost/availability limits.

## Delivery checklist

### CLI and project context

- [x] Add `forge capabilities --format json` with a versioned schema describing
  enabled framework features and their configured adapters, without values of
  secrets or connection strings.
- [x] Add static `forge doctor --format json` checks for project/environment
  configuration and distinguish these categories from unprobed services.
- [x] Add opt-in PostgreSQL TCP reachability probes with bounded timeouts; they
  do not authenticate or query data and never report the connection URL.
- [x] Generate `AGENTS.md` and `docs/for-ai.md` from the same feature manifest as
  the application so the instructions cannot promise disabled capabilities.
- [x] Add opt-in JSON execution diagnostics with `code`, `message`, `path`,
  `hint`, and schema version; Forge returns exit code 1 for execution errors.
- [x] Add JSON diagnostics for argument-parser errors when `--json-errors` is
  present; help/version retain normal output and syntax errors exit with code 2.

### Safe generation and edits

- [x] Add non-mutating JSON `--dry-run` previews for migrations and resources,
  including paths, operation counts and content. Timestamped paths are marked
  provisional; byte-for-byte deterministic output remains open.
- [x] Provide an executable `forge generate page` vertical slice with a JSON
  preview, create-new output, automatic route registration and escaped dynamic
  text rendering.
- [ ] Ensure repeated generation is deterministic and safe; require explicit
  opt-in before replacing user-owned files.
  Migration paths are now repeatable when `--version` is explicit, and resource
  and page generators reject symlink traversal below the application root.
  The checklist remains open until stable defaults/golden output and safe,
  explicit replacement semantics are complete.
- [x] Add a golden fixture that compares the generated application and proves the
  declared architecture and enabled feature set.
- [ ] Complete focused generators for supported vertical slices: the page
  generator is executable, while resource HTTP, API contracts and jobs/events
  remain unfinished.

### AI integration and execution

- [ ] Implement provider adapters and model capability discovery behind
  provider-neutral contracts; support local endpoints as first-class providers.
- [ ] Add policy-aware routing, streaming, usage accounting, bounded retries,
  and explicit fallback behavior that never repeats non-idempotent actions.
- [ ] Keep tools disabled by default. Add typed, scoped capabilities, sandbox
  profiles, approvals, redaction, audit, cancellation, and resource limits before
  any AI-triggered command or code execution is supported.
- [ ] Add adversarial tests for prompt injection, secret exfiltration, path
  traversal, privilege escalation, resource exhaustion, and provider fallback.

## Current repository evidence

The framework has an accepted security architecture for separating AI output
from controlled execution in [ADR 0005](adr/0005-ai-provider-and-execution-boundary.md).
That is a design contract, not proof that the AI provider/router/executor product
has been implemented. `capabilities`, static `doctor`, dry-run generation, and
opt-in CLI errors have versioned JSON output. Doctor's optional database probe
checks TCP reachability only; it does not validate credentials or database
health. The resource generator is partial; the remaining checklist items stay
open until implemented and verified.
