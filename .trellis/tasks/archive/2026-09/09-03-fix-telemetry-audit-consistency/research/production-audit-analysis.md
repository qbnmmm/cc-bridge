# Production fingerprint audit analysis

Source: `/Users/qiubingnan/RustProject/cc-bridge/fingerprint-audit.jsonl`
Analyzed: 2026-09-03

## Dataset

- 28 valid NDJSON records, schema version 1, account 1 only.
- UTC range: 2026-09-02 05:27:13 to 2026-09-03 00:50:41.
- Events: 9 profile snapshots, 11 telemetry session transitions, 8 hourly summaries, 0 anomalies.
- No email, token, UUID, URL or Authorization values were found.

## Aggregates

- Requests: 1,774.
- 2xx: 1,766; 403: 0; 429: 0; 5xx: 7.
- Stream: 1,714; thinking: 1,703; effort: 1,722.
- Session ID present: 1,774; client request ID present: 1,774.
- Telemetry sends: 1,460 event batches, 5 GrowthBook evals, 0 send failures.
- Audit queue drops: 0.
- Models: Opus 5 = 1,147; Sonnet 5 = 605; Fable 5.1 = 22.
- Token-counting beta: 52 requests.
- Redact-thinking beta: 1,722 requests; it is incoming production behavior and must continue to be preserved.

## Confirmed defects / blind spots

1. `/v1/messages/count_tokens` matches `starts_with("/v1/messages")`, so count-token traffic is audited and activates automatic inference telemetry like a model call.
2. Beta capability detection recognizes Opus 4/Sonnet 4-5/Haiku 4 but not production `claude-opus-5` or `claude-fable-5-1`. Claude Code traffic is currently protected by incoming beta preservation; generic API injection is not.
3. `last_growthbook_at` lives inside `TelemetrySession`; an idle expiry resets it, causing a new GrowthBook request before the documented six-hour interval. Production sent 5 evals across session restarts.
4. Hourly audit clears the complete per-account counter, including last profile; identical profile snapshots are emitted after each flush.
5. Audit status buckets omit 3xx and ordinary 4xx, and request/response observations are not correlated across hourly boundaries. Production shows 403 requests vs 401 2xx in one window and 129 requests vs 130 2xx in the next.
6. One synthetic query/success pair is generated per telemetry HTTP batch. The 13-22 second send cadence cannot drain high request rates before session expiry, so pending request observations can be discarded. Production saw 1,460 event batches for 1,774 message-path observations.
7. `tengu_api_success` still invents token counts, duration, TTFT, cost and request ID instead of consuming the response data already parsed by `UsageService`.

## Healthy signals

- Effective 2.1.258 release profile stayed constant.
- No fingerprint mismatch events.
- No audit drops or telemetry send failures.
- No 403 or 429.
- Seven 5xx were isolated to one hourly window and later returned to zero.
