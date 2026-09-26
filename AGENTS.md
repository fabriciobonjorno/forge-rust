# AGENTS.md

## Engineering Operating Mode

This repository uses the global Codex engineering instructions plus the
project-specific rules defined here.

### Mandatory workflow

UNDERSTAND → INSPECT → PLAN → IMPLEMENT → VERIFY → REVIEW

### Project rules

Add repository-specific rules below, for example:

-   architecture and module boundaries;
-   approved frameworks/libraries;
-   forbidden dependencies;
-   database access conventions;
-   API conventions;
-   security requirements;
-   naming conventions;
-   commands for lint/typecheck/test/build;
-   deployment constraints.

### Before changing code

Inspect the relevant repository context first. Determine requested
behavior, affected components, existing patterns, dependencies, tests,
and potential regressions.

Search before creating. Reuse existing components and abstractions when
appropriate.

### Scope

Make the smallest coherent change that fully solves the request.

Do not refactor unrelated code, rename unrelated symbols, change public
contracts unnecessarily, introduce abstractions without concrete need,
or silently expand scope.

### Debugging

REPRODUCE → INVESTIGATE → ROOT CAUSE → FIX → REGRESSION TEST → VERIFY

Do not patch symptoms when the underlying cause can reasonably be
identified.

### Validation

Run the relevant project checks after implementation:

1.  targeted tests
2.  formatter
3.  lint
4.  typecheck/static analysis
5.  broader tests when justified
6.  build when applicable

Then inspect version-control status and the final diff.

Never claim a check passed unless it was actually executed successfully.

### Safety

Never expose secrets or credentials. Preserve existing user changes.
Avoid destructive operations unless required and explicitly justified.

### Definition of Done

A task is complete only when the requested behavior is implemented,
relevant validation passes, the final diff is reviewed, accidental
changes are absent, and unresolved risks are disclosed.

### Final response

Report:

1.  Summary
2.  Files changed
3.  Key implementation decisions
4.  Tests/checks executed
5.  Remaining risks or follow-ups
