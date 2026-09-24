# 更新 Claude Code 环境指纹版本

## Goal

将 cc-bridge 的 Claude Code 客户端身份基线从 `2.1.81` 对齐到经过静态分析和最小脱敏抓包验证的 `2.1.258`，确保版本、构建元数据、Stainless headers、User-Agent、beta、请求 ID 和遥测环境不会形成明显的跨版本矛盾。

本任务以协议兼容和降低误报为目标，不承诺规避 Anthropic 风控或保证账号不会被限制。

## Background

- 工作目录为 `/Users/qiubingnan/RustProject/cc-bridge`，分支为 `ccb`。
- 当前 `src/model/identity.rs`、`src/service/rewriter.rs`、`src/service/oauth.rs`、`src/service/telemetry.rs` 和测试仍包含 `2.1.81`。
- 用户已确认目标版本为 `2.1.258`，且本地不能登录 Claude.ai，只能使用现有中转站 API。
- 抓包未使用本地 Claude.ai 登录或 OAuth 提取；真实中转站只执行了 Sonnet、Haiku 各一次最小 `OK` 请求，总计 350 input tokens、48 output tokens，无工具、无并发、无遥测和无额外流量。
- 官方 2.1.258 darwin arm64 二进制确认：
  - version/version base：`2.1.258`
  - build time：`2026-09-01T21:54:40Z`
  - Stainless package：`0.112.1`
  - Stainless OS：`MacOS`
  - runtime：`node` / `v26.3.0`
- 官方 `--print` 请求使用 `claude-cli/2.1.258 (external, sdk-cli)`；交互式入口静态控制流默认 `cli`。cc-bridge 当前强制改成 `cli`，会丢失真实 entrypoint。
- 2.1.258 Haiku 请求仍包含 `claude-code-20250219`；当前 cc-bridge 对所有 Haiku 省略该 beta 的规则已过期。
- 2.1.258 请求按 body/模型包含 `thinking-token-count-2026-05-13`、`cache-diagnosis-2026-04-07`、`mid-conversation-system-2026-04-07` 和 `effort-2025-11-24`；当前模型名-only 计算不足。
- 第三方中转站请求可缺少 `x-client-request-id`，但官方 first-party host 生成请求会带该 header；cc-bridge 的 ClaudeCode 分支当前不会补齐。
- 2.1.258 抓包没有默认发送 `redact-thinking-2026-02-12`；当前 cc-bridge 无条件/广泛强制该 beta 缺少当前证据。
- 当前只获得协商出的 TLS cipher/ALPN，没有完整 ClientHello、JA3/JA4 或直接 Anthropic HTTP/2 证据，因此不能据此修改 craftls。

## Requirements

- R1. 在 canonical identity 所属模块集中定义 `2.1.258` release metadata，消除生产路径中的独立旧版本常量。
- R2. 新账号使用 2.1.258 version、version base、build time、Stainless package 和已验证 runtime/profile 值。
- R3. 已有账号采用读取时内存规范化：运行时使用统一 release metadata，保留 platform、arch、terminal、package managers、device ID 和 process ranges 等非 release 身份字段；账号后续编辑时自然写回，不做启动期批量 JSON migration。
- R4. Claude Code 请求改写只能替换 UA 中的版本部分，必须保留 incoming entrypoint/suffix，例如 `sdk-cli`、`cli` 或 Agent SDK 附加信息。
- R5. `X-Stainless-OS`、package version 和 runtime version 与 2.1.258 profile 一致；端点各自使用抓包证明的 header 集，不盲目共用 messages headers。
- R6. Beta 计算必须读取模型和 request body 特征，覆盖 thinking、effort、mid-conversation、cache diagnosis、Haiku 顺序和 `[1m]`；保留未知 incoming beta 且避免重复。
- R7. 转发到 first-party Anthropic 前确保 `X-Claude-Code-Session-Id` 透传且 `x-client-request-id` 存在；第三方 relay 输入是否包含 request ID 不作为前提。
- R8. 不再默认强制 `redact-thinking`，除非 current 2.1.258 条件或 incoming header 明确要求。
- R9. Setup Token 与 OAuth 上游账号继续保留所需 `oauth-2025-04-20` 语义；其精确顺序没有 live OAuth 证据时不做未经验证的大幅重排。
- R10. 更新 full env/telemetry release fields，并使用 `CanonicalEnvData.is_running_with_bun`，不再在序列化时硬编码 `false`。
- R11. TLS/craftls 本次默认不修改；只有新的直接、脱敏传输证据证明具体差异时才扩大范围。
- R12. 不在仓库或 `/tmp` 留下 token、账号标识、原始抓包或临时 CA/private key。
- R13. 增加可选的线上 fingerprint audit：启用后写入 `<LOG_DIR>/fingerprint-audit.jsonl`，只保存小时级汇总和即时 anomaly，不保存原始请求/响应。
- R14. Audit 必须使用严格 allowlist：允许版本、平台、模型、entrypoint、beta、thinking/effort、状态码和 presence/counter；禁止 token、邮箱、prompt、response、tool 内容、UUID 原值、relay URL、DSN 和代理凭证。
- R15. Audit 写入采用有界非阻塞队列，队列满或文件失败只能增加 dropped/error counter 并告警，不能阻塞或失败主请求与流式响应。
- R16. Audit 文件独立轮转，目标为单文件 10 MiB、最多 6 个历史文件，Unix 文件权限 `0600`；默认关闭，通过 `FINGERPRINT_AUDIT_ENABLED=true` 显式开启。
- R17. Audit 至少输出 profile/session/hourly/anomaly 事件，并检测 version、Stainless、UA entrypoint、beta、thinking 与 first-party request ID 的一致性。

## Acceptance Criteria

- [x] AC1. `src/` 中不再残留非测试意图的 `2.1.81` 和 `0.70.0` 客户端常量。
- [x] AC2. 新账号和旧账号的有效运行时 release metadata 均为 2.1.258，非 release 身份字段保持不变。
- [x] AC3. messages、token test、usage 和 telemetry 的 UA/version/Stainless 字段来自统一 profile。
- [x] AC4. `sdk-cli` 等 incoming UA suffix 在版本规范化后仍保留。
- [x] AC5. Sonnet/Haiku、thinking/effort、first-party request ID 和 `[1m]` 的 beta/header 测试与 2.1.258 证据一致。
- [x] AC6. 未知 incoming beta 保留且无重复；`redact-thinking` 不再无条件添加；OAuth beta 语义保持。
- [x] AC7. full env 使用验证过的 version/build/runtime/Bun 字段，metrics `service.version` 与 UA 一致。
- [x] AC8. `cargo test --lib`（199 项）和 `cargo check` 通过；本任务修改文件 rustfmt 通过，repo-wide `cargo fmt --check` 仅剩未改动的 `token_store.rs` / `tlsfp.rs` 基线格式差异。
- [x] AC9. 最终 diff 不包含抓包脚本、原始流量、token、relay URL、账号标识或无证据的 TLS 修改。
- [x] AC10. 交付说明明确剩余风险和证据限制，不作账号安全保证。
- [x] AC11. 开启 audit 后生成合法 NDJSON，包含 schema version、时间、账号数字 ID、脱敏 profile/counters/mismatches，不包含任何禁写字段。
- [x] AC12. Audit 异常记录能覆盖旧 version/Stainless、UA suffix 被覆盖、beta/思考状态不一致和缺少 first-party request ID。
- [x] AC13. Audit 关闭时不创建专用文件；开启时轮转和权限符合设计，队列/IO 故障不影响请求结果。

## Out of Scope

- 本地 Claude.ai 登录、OAuth 凭证提取或使用真实账号直接连接 Anthropic 做抓包。
- 绕过或对抗 Anthropic 的账号安全、滥用检测或服务条款执行。
- 保证账号不被封禁或恢复已被限制的账号。
- 无直接传输证据支持的 TLS/JA3/JA4/HTTP2 fingerprint 修改。
- 下载并验证非 darwin arm64 的 2.1.258 官方二进制。

## Decisions

- 已有账号采用读取时内存规范化，后续编辑时自然写回；不立即批量修改 SQLite/PostgreSQL 的 `canonical_env` JSON。
- 现有账号的 `auto_telemetry=true` 保持不变；本任务不自动切换任何账号的遥测开关，创建账号的默认值继续为 false。
- 将脱敏线上 fingerprint audit 纳入本任务；功能默认关闭，目标部署显式开启后按小时汇总并即时记录 anomaly。
