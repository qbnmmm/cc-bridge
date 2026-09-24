# Implementation Plan

## Phase A: endpoint and beta correctness

- [x] Add a pure exact message-endpoint classifier and replace telemetry/audit inference `starts_with` checks.
- [x] Count `/v1/messages/count_tokens` separately without registering inference telemetry.
- [x] Extract model capability helpers and cover Opus 5, Sonnet 5, Fable 5.1, Claude 4 families and Claude 3 legacy behavior.
- [x] Preserve incoming beta order, including production `redact-thinking`, fallback-credit, token-counting and structured-output values.

## Phase B: real usage completion bridge

- [x] Define the internal completed inference observation and bounded UsageService -> TelemetryService channel.
- [x] Add monotonic request start, first-byte TTFT and client-drop state to UsageAttempt/ResponseObserver.
- [x] Extend SSE/JSON parsers to capture validated stop reason when available.
- [x] Produce the existing UsageEvent and a best-effort completed observation from the same parsed response and pricing result.
- [x] Add channel-full/drop health and audit accounting without awaiting in stream poll/drop paths.

## Phase C: telemetry queues and batching

- [x] Replace scalar pending count/latest profile with correlation-keyed pending profiles and a ready queue.
- [x] Register only exact inference requests; match completed observations by internal correlation id.
- [x] Prevent idle expiry while pending/ready work exists; add a bounded hard timeout with explicit audit counters.
- [x] Build up to 64 completed request pairs into one event HTTP batch.
- [x] Populate query/success with actual request timestamps, message count, model/beta/thinking/effort and actual response tokens/cache/cost/duration/TTFT/request ID/stop reason.
- [x] Remove all remaining random query/success token, cost, timing and request-id generation.
- [x] Keep telemetry sends at-most-once and non-blocking to the inference response.

## Phase D: GrowthBook and audit v2

- [x] Move GrowthBook throttle into account-level TelemetryService state surviving session expiry; record attempts to avoid failure storms.
- [x] Separate persistent last-profile state from hourly counters so unchanged profiles do not repeat snapshots.
- [x] Add v2 inference/count-token/status/send-failure/in-flight/pending/completed/batched/drop counters.
- [x] Keep internal correlation ids out of every serialized audit record.
- [x] Update README audit collection notes with v2 fields and expected 24-hour checks.

## Validation

- [x] Exact path tests prove count-token requests neither activate nor create inference telemetry.
- [x] Beta tests cover audited Opus 5/Fable 5.1 traffic and legacy models.
- [x] Usage parser tests cover JSON, SSE, compression, early completion, client drop, TTFT and stop reason.
- [x] Batching tests prove multiple completed requests share one HTTP payload and retain per-request values.
- [x] Session tests cover pending work across idle TTL, hard timeout and GrowthBook restarts within six hours.
- [x] Audit tests cover schema v2, cross-hour in-flight accounting, stable profile snapshots, status classes, queue drops and forbidden-value scans.
- [x] Existing slot/stream lifecycle, 429/5xx wrappers, usage persistence and telemetry tests pass.
- [x] `cargo test --lib` with loopback permission.
- [x] `cargo check`.
- [x] Modified files pass rustfmt; do not reformat unrelated baseline files.
- [x] Final diff contains no downloaded `fingerprint-audit.jsonl`, token, email, UUID, URL, raw request/response or TLS change.

## Rollback points

1. Endpoint/beta changes are independently revertible before lifecycle wiring.
2. Usage completion bridge remains best-effort and can be disabled without changing usage persistence.
3. Telemetry batching/audit v2 are in-memory/file-only and require no schema rollback.
