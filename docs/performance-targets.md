# Performance Targets

Status: Initial budgets for design and acceptance. No measurements have been run,
and these numbers are not benchmark results.

## Measurement principles

Correctness, tenant isolation, authorization, observability, cancellation, and
backpressure remain enabled during representative benchmarks. A faster insecure or
unobservable configuration is not the product target. Optimize only after profiles
identify a bottleneck.

Every published result records revision, Rust toolchain, feature set, optimization
profile, allocator, OS/kernel, CPU model/count, memory, network topology, database
version/configuration, dataset, concurrency, duration, warm-up, and load-generator
location. Results report confidence/variance and raw artifacts, not only a peak.

## Reference profiles

Two profiles prevent one machine from defining the architecture:

- `ci-small`: 4 dedicated x86-64 or arm64 vCPU, 8 GiB RAM; server and load
  generator separated; loopback results are labeled diagnostic only.
- `reference-prod`: 8 dedicated x86-64 vCPU, 16 GiB RAM, Linux, local-zone managed
  PostgreSQL with at least 4 vCPU; load generator on a separate host.

Before the first benchmark release, the project will pin exact public/reproducible
machine images and store their configuration with benchmark artifacts.

## Initial service-level performance budgets

These targets apply to optimized release builds on `reference-prod` after a
10-minute warm-up, over a 15-minute steady-state run, below 70% sustained CPU and
without errors above 0.1%.

| Scenario | Load | Initial target |
| --- | --- | --- |
| Plain HTTP health response | 256 concurrent keep-alive clients, HTTP/1.1, telemetry export enabled | at least 100,000 req/s; p50 <= 1 ms, p95 <= 3 ms, p99 <= 8 ms |
| JSON route with validation | 1 KiB request and 1 KiB response, 256 clients | at least 60,000 req/s; p95 <= 5 ms, p99 <= 12 ms |
| PostgreSQL single-row read | indexed tenant-scoped lookup, pool warm, 128 clients | p50 <= 3 ms, p95 <= 10 ms, p99 <= 25 ms; at least 15,000 ops/s where DB permits |
| Transactional create + outbox | insert aggregate and outbox row, 128 clients | p95 <= 18 ms, p99 <= 40 ms; at least 6,000 tx/s where DB permits |
| PostgreSQL job claim/complete | no handler work, 32 workers, batches of 100 | at least 20,000 jobs/min; p95 enqueue-to-claim <= 250 ms at steady state |
| SSE fan-out | 10,000 connected clients, 1 event/s/client, 256-byte event | p99 framework-added delivery delay <= 50 ms; bounded memory, zero dropped events before configured backpressure limit |
| AI token proxy stream | upstream replay fixture, 1,000 concurrent streams | framework p95 time-to-first-byte overhead <= 10 ms; p99 inter-chunk overhead <= 2 ms |

Network and database limits are reported separately. A target blocked by external
capacity is rerun with the bottleneck scaled; it is not silently marked achieved.

## Resource budgets

On `reference-prod`:

- Idle minimal HTTP application RSS: <= 25 MiB after allocator stabilization.
- Incremental framework memory for 10,000 idle SSE connections: <= 40 MiB beyond
  socket/TLS buffers measured in the same environment.
- No queue may grow without a configured bound. At saturation, memory reaches a
  plateau and callers receive backpressure or explicit overload errors.
- Minimal release container compressed size: <= 25 MiB, excluding optional CA and
  timezone datasets; exact contents are reported.
- Graceful shutdown: stop admission within 100 ms of signal and drain ordinary
  requests within configured deadline; tests use a 10-second budget.

Allocation targets are workload-specific and established by the Phase 1 baseline.
The first baseline must publish allocations/request for health and JSON routes;
later regressions greater than 10% require explanation or remediation.

## Compile-time and developer-loop budgets

Measured on a pinned 8-core developer reference machine:

- Clean build of the Phase 1 workspace: <= 120 seconds.
- Incremental `cargo check` after editing a generated handler: <= 3 seconds.
- `forge generate resource` for a small resource: <= 500 ms excluding an optional
  formatter run; output is deterministic.
- Minimal generated application release binary: <= 15 MiB stripped, before adding
  database, TLS, or AI features.

These are feature-budget signals, not permission to bypass type or security checks.

## Benchmark suites

- Criterion-style microbenchmarks cover UUIDv7 generation/parsing, route matching,
  error mapping, validation, JSON serialization, policy evaluation, and AI stream
  framing.
- Closed-loop and open-loop HTTP load tests report coordinated-omission-safe
  histograms, throughput, errors, CPU, RSS, allocations, and context switches.
- Database benchmarks use seeded tenant-skewed data, realistic indexes, fixed pool
  sizes, cold and warm variants, and query plans.
- Job tests measure enqueue, claim, retry/dead-letter, fairness, and recovery after
  worker death.
- AI tests replay deterministic upstream streams so framework overhead is separated
  from provider latency and cost.
- Saturation/soak tests run for at least two hours before production certification
  and look for memory growth, task leaks, queue growth, and tail-latency collapse.

## Regression policy

PR CI runs stable microbenchmarks for signal but does not fail on noisy single
samples. A controlled scheduled runner compares at least 20 samples. A regression
is actionable when median throughput worsens by more than 5%, p99 latency by more
than 10%, allocation count/bytes by more than 10%, or idle/steady RSS by more than
10%, with statistical confidence defined in the benchmark harness. Security or
correctness fixes may accept a regression only with recorded rationale and a new
budget or follow-up owner.

Phase 9 may revise numerical targets using evidence. Changes preserve historical
results and explain whether hardware, workload, implementation, or product
requirements changed.
