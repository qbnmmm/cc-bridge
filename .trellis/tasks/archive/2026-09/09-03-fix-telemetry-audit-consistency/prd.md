# 修复自动遥测与审计一致性

## Goal

根据 2026-09-02 至 2026-09-03 的线上 `fingerprint-audit.jsonl` 证据，修复 cc-bridge 自动遥测和脱敏审计中的结构性偏差，使 count-token、模型能力、GrowthBook 周期、事件批处理、小时统计和真实请求结果之间保持一致，同时不增加账号流量风险或记录敏感内容。

## Background

- 当前分支 `ccb` 已发布 `v1.8.3-qbn.6`，Claude Code release profile 为 2.1.258。
- 线上审计包含 1,774 次 message-path observation、1,460 次 event batch、5 次 GrowthBook eval、7 次 5xx、0 次 403/429、0 telemetry failure、0 audit drop、0 fingerprint anomaly。
- 52 次请求带 `token-counting-2024-11-01`，证明 `/v1/messages/count_tokens` 被当前 `starts_with("/v1/messages")` 逻辑纳入普通推理请求。
- 生产模型包含 `claude-opus-5` 和 `claude-fable-5-1`，且真实 Claude Code 请求携带 context-management；当前主动 beta capability 判断没有覆盖这两个模型。
- `redact-thinking-2026-02-12` 在 1,722 次请求中出现，说明它是实际生产客户端的条件性 incoming beta；必须保留，不能统一剥离或强制生成。
- `last_growthbook_at` 当前属于 10 分钟 TTL 的 `TelemetrySession`，session 重启会绕过六小时周期。
- Audit 小时 flush 清空完整 counter，导致相同 profile 重复 snapshot；状态分类也无法表示普通 4xx、3xx、send failure 或跨小时响应。
- 自动遥测仍按随机范围构造 token、duration、TTFT 和 cost；真实 usage 已由 `UsageService` 在流结束或客户端中断后解析。

## Requirements

- R1. 只有精确的 `/v1/messages` 推理端点才能激活自动 query/success telemetry；`/v1/messages/count_tokens` 必须单独统计且不得生成推理成功事件。
- R2. 把 beta 判定改为可测试的 model capability 规则，覆盖 Opus 5、Sonnet 5、Fable 5.1、Claude 4 family、legacy Claude 3 和未知 incoming beta 保留。
- R3. GrowthBook 六小时节流必须是账号级、跨 telemetry session 的运行时状态；session 过期/重建不得提前重发。
- R4. 自动遥测不得以固定慢速单条队列丢弃高峰请求。每次 HTTP batch 应可承载多组已完成的 query/success event，并在 session 过期前尽可能 drain；有界队列满时必须可观测而不阻塞请求。
- R5. Audit 的 last profile 必须跨小时窗口保留；只有 effective profile 真正变化才写新的 `profile_snapshot`。
- R6. Audit 增加 `status_3xx`、`status_4xx_other`、`send_failed`、pending/completed/expired-with-pending 等计数，并避免把跨小时 request/response 差异误报为指纹异常。
- R7. `redact-thinking` 等 incoming beta 继续原序保留；本任务不得根据单一最小抓包删除生产客户端实际使用的 beta。
- R8. 所有新观测仍通过严格 allowlist，不保存 token、邮箱、prompt/response、tool 内容、UUID 原值、URL、DSN 或原始 header/body。
- R9. `auto_telemetry` 默认值和现有账号开关保持不变；失败路径不得影响 `/v1/messages` 响应或流生命周期。
- R10. TLS/craftls、账号调度、限流与 billing rewrite 语义不在本次修改范围。
- R11. `tengu_api_success` 必须消费 `UsageService` 从真实上游响应解析出的 model、tokens、cost、duration、TTFT、request ID 和 stop reason；没有可靠观测的字段应省略，不得继续随机生成。

## Acceptance Criteria

- [x] AC1. count-token 请求不会创建/续期推理 telemetry pending event；对应单测覆盖精确路径分类。
- [x] AC2. 无 incoming beta 的 API 注入模式对 Opus 5、Fable 5.1 生成正确 context-management 等 beta；Claude 3 legacy 行为不回归。
- [x] AC3. 同账号 telemetry session 在六小时内多次过期重启只发送一次 GrowthBook eval。
- [x] AC4. 高峰请求能在少量 HTTP batch 中携带多组事件；session 到期时 pending 数有明确 drain/drop 语义和审计计数。
- [x] AC5. 连续小时窗口 profile 不变时只出现一次 snapshot；profile 真变化时立即出现新 snapshot。
- [x] AC6. 小时汇总覆盖 2xx/3xx/403/429/other-4xx/5xx/send failure，队列 drop 和 pending telemetry 状态可见。
- [x] AC7. 审计序列化与日志扫描确认没有敏感字段或任意外部字符串原样落盘。
- [x] AC8. 自动遥测发送失败、audit queue 满或 usage observation 缺失不影响主请求和 slot/stream 生命周期。
- [x] AC9. 相关 deterministic tests、`cargo test --lib` 和 `cargo check` 通过；repo baseline fmt 差异单独记录，不扩大无关 diff。
- [x] AC10. 线上升级说明给出预期的新审计字段和 24 小时复核指标。
- [x] AC11. 自动遥测 success event 的 token/cache/cost/duration/TTFT/request ID 来自同一次真实响应；parser 缺字段时省略而不是伪造，客户端提前断开但已解析 usage 时仍能完成观测。

## Out of Scope

- 修改 TLS/JA3/JA4/HTTP2 指纹。
- 更改自动遥测默认开关或批量修改账号配置。
- 发送新的真实账号抓包请求。
- 根据未验证的风控假设增加额外遥测流量。

## Decisions

- 本任务同时接入 `UsageService` 的真实响应观测，删除 `tengu_api_success` 中随机生成的 token、cost、duration、TTFT 和 request ID。
- 所有修复作为一个任务交付，因为 count-token 分类、usage completion、telemetry batching 和 audit correlation 共用同一请求生命周期；不拆 parent/child task。
