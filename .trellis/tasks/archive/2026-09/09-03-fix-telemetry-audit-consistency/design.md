# Technical Design: Telemetry and Audit Consistency

## 1. Boundary and data flow

The authoritative lifecycle for an inference request becomes:

```text
Gateway exact /v1/messages classification
  -> create UsageAttempt + internal correlation id
  -> register sanitized pending telemetry profile
  -> send upstream request
  -> response headers complete status/request id
  -> UsageService observes stream bytes and finalizes actual usage
  -> completed observation channel
  -> TelemetryService matches pending profile and queues completed event
  -> periodic HTTP event batch carries multiple query/success pairs
  -> FingerprintAudit aggregates lifecycle counters without persisting ids
```

`/v1/messages/count_tokens` follows the ordinary forwarding/rewriting path but never enters the inference telemetry lifecycle. It receives a separate audit counter.

The task remains one gateway-backend task because all changes share `UsageAttempt` and the response stream lifecycle. No frontend, schema migration, TLS or craftls change is required.

## 2. Endpoint classification

Add one pure classifier used consistently by gateway audit, telemetry activation and usage observation:

```rust
enum MessageEndpoint {
    Inference,
    CountTokens,
    Other,
}
```

Classification is based on the normalized URI path:

- exact `/v1/messages` -> `Inference`
- exact `/v1/messages/count_tokens` -> `CountTokens`
- anything else -> `Other`

Do not use `starts_with("/v1/messages")` for telemetry activation. Existing request body rewriting can remain shared where protocol-compatible, while CCH attestation and inference-only actions use exact classification.

## 3. Model capability rules

Replace string lists embedded in `compute_betas_for_request` with pure model capability helpers:

- legacy Claude 3 -> no interleaved thinking or context management
- Claude Opus 4/5, Sonnet 4/5, Haiku 4 and Fable 5 -> context management
- Claude 4.5 Haiku -> `claude-code-20250219` in the captured late position
- unknown incoming beta values -> preserve original order
- `redact-thinking` -> preserve incoming; do not force or strip

Tests use production model ids from the audit: `claude-opus-5`, `claude-sonnet-5`, `claude-fable-5-1`.

## 4. Real completion observation

### 4.1 Shared observation type

Add a non-serialized internal type under `model::usage` or `service::usage`:

```rust
struct CompletedInferenceObservation {
    correlation_id: String,
    account_id: i64,
    model: String,
    upstream_request_id: Option<String>,
    tokens: UsageTokens,
    cost_nano_usd: Option<i64>,
    duration_ms: i64,
    ttft_ms: Option<i64>,
    stop_reason: Option<String>,
    http_status: u16,
    is_stream: bool,
    client_dropped: bool,
}
```

The correlation id stays in memory and is never written to fingerprint audit. The cost is present only when pricing is complete; incomplete pricing omits `costUSD` rather than inventing it.

### 4.2 Timing

`UsageAttempt` keeps a monotonic `Instant` start. `ResponseObserver` records the first non-empty response chunk for TTFT. Final duration is measured when the parser finalizes on message completion, EOF or client drop with usable usage.

### 4.3 Parsing

Extend SSE/JSON parsing to capture a validated stop reason when present. Existing token parsing and compressed/client-drop guarantees remain unchanged. A parsed response with usage produces both:

1. the existing persisted `UsageEvent`;
2. a best-effort `CompletedInferenceObservation` on a bounded channel.

Persistence and telemetry observation failures remain independent: neither can block or alter the streamed response.

## 5. Usage-to-telemetry channel

Create a bounded mpsc channel in `main` before constructing the services:

- sender owned by `UsageService`;
- receiver owned by a TelemetryService completion worker;
- channel full increments a dedicated counter and emits a sanitized audit observation;
- use `try_send`, never await from response stream polling/drop code.

This avoids a service dependency cycle and keeps the response observer synchronous/non-blocking.

## 6. Telemetry session state

Replace `pending_events: i32` plus one latest profile with:

- `pending: HashMap<correlation_id, PendingInferenceProfile>`
- `ready: VecDeque<CompletedTelemetryEvent>`
- `startup_sent`
- process simulation state retained for env/process metrics only

Registration stores the real request timestamp, model, beta, entrypoint, thinking, effort, message count and token-cache mode. Completion matches by correlation id and combines the request profile with the real response observation.

Session expiry rules:

- idle TTL does not remove a session while `pending` or `ready` is non-empty;
- a hard pending timeout prevents permanently hung requests;
- timed-out pending entries are removed with an audit counter, not converted to fake success;
- no new telemetry request is generated solely to drain an empty session.

## 7. Batched event sending

At each allowed send interval, take up to a bounded number of ready observations (recommended 64) and build one event batch:

- one optional `tengu_startup` per session;
- one `tengu_api_query` and one `tengu_api_success` per completed observation;
- individual query/success timestamps reflect request start and completion;
- success fields use actual tokens, cache tokens, duration, TTFT, request id, stop reason and cost when known;
- unknown optional fields are omitted;
- no random token/cost/timing/request-id/tool events.

Use at-most-once delivery for automatic telemetry. A failed HTTP send is audited and the batch is not automatically retried, avoiding duplicate telemetry or traffic amplification when the server accepted the request but the response was lost.

## 8. GrowthBook cadence

Move GrowthBook attempt state outside `TelemetrySession` into a TelemetryService account map:

```rust
HashMap<account_id, Instant>
```

The timestamp is recorded when an attempt is scheduled, regardless of success, so a failing endpoint cannot create a retry storm. Session restart within six hours does not resend. Process restart may send once again; no database migration is introduced.

## 9. Fingerprint audit v2

Bump new records to schema version 2 while remaining able to append after v1 records.

Keep `last_profiles` outside hourly counters. Hourly flush resets interval counts but not profile state.

New counters:

- `inference_requests`
- `count_token_requests`
- `responses_2xx`, `responses_3xx`, `responses_403`, `responses_429`, `responses_4xx_other`, `responses_5xx`
- `send_failed`
- `in_flight_at_end`
- `telemetry_registered`, `telemetry_completed`, `telemetry_batched`
- `telemetry_pending_timed_out`, `telemetry_observation_dropped`
- event-batch and GrowthBook success/failure

Correlation ids may exist in worker memory to maintain in-flight counts but must never be serialized. Request/response observations crossing an hour boundary are represented by `in_flight_at_end`, not treated as mismatches.

## 10. Compatibility and rollout

- `auto_telemetry` values and defaults remain unchanged.
- No database schema or API response change.
- Existing audit v1 files remain readable; v2 records are appended to the same rotated file.
- Billing rewrite, account selection, quota absorption, 429/5xx wrappers and SlotHeldStream ownership remain unchanged.
- The completed observation path is best effort; usage persistence remains authoritative for billing/reporting.

## 11. Rollback

All state is in-memory or file output. Rollback to v1.8.3-qbn.6 requires only the previous image; no database rollback is needed. Audit v2 lines may remain in the file and should be handled by schema version during offline analysis.
