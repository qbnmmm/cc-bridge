# Claude Code 2.1.258 静态特征初查

## 基准

- 官方客户端二进制：`/Users/qiubingnan/.cac/versions/2.1.258/claude`
- `--version`：`2.1.258 (Claude Code)`
- 二进制内嵌元数据：
  - `VERSION=2.1.258`
  - `BUILD_TIME=2026-09-01T21:54:40Z`
  - `GIT_SHA=b3cd543a1f6fcdf4d8fabc0f5e5538d2ee7f38e1`
  - `DD_SOURCEMAP_GROUP=darwin`

## 已确认的请求相关差异

1. API User-Agent 生成规则仍为：
   - 默认 CLI 入口：`claude-cli/2.1.258 (external, cli)`
   - `CLAUDE_CODE_ENTRYPOINT`、Agent SDK client app 和 workload 可动态追加到括号部分。
2. Anthropic JS SDK/Stainless package version 已是 `0.112.1`。
   - cc-bridge 当前在 `src/service/rewriter.rs` 和 `src/service/oauth.rs` 固定为 `0.70.0`，不能只升级 Claude Code 版本字符串而保留该值。
3. 二进制仍包含以下稳定 header/endpoint 线索：
   - `/v1/messages?beta=true`
   - `X-Stainless-Lang`
   - `X-Stainless-Package-Version`
   - `X-Stainless-OS`
   - `X-Stainless-Arch`
   - `X-Stainless-Runtime`
   - `X-Stainless-Runtime-Version`
   - `/api/oauth/usage`
   - `/api/claude_code/metrics`
4. Beta catalog 相比 cc-bridge 当前实现已经扩展，静态二进制中至少包含：
   - `claude-code-20250219`
   - `oauth-2025-04-20`
   - `interleaved-thinking-2025-05-14`
   - `context-1m-2025-08-07`
   - `context-management-2025-06-27`
   - `prompt-caching-scope-2026-01-05`
   - `redact-thinking-2026-02-12`
   - `task-budgets-2026-03-13`
   - `prompt-caching-evict-2026-05-12`
   - `thinking-token-count-2026-05-13`
   - `per-turn-control-2026-07-01`
   - `mid-conversation-tool-changes-2026-07-01`
5. 仅凭静态字符串不能断定每次请求实际发送哪些 beta。它们受模型、认证方式、入口、功能开关、请求 body 和会话状态影响。

## cc-bridge 当前明显不一致点

- `src/model/identity.rs`：`version/version_base=2.1.81`，`build_time=2026-03-20T21:26:18Z`。
- `src/service/rewriter.rs`：默认版本 `2.1.81`，Stainless package `0.70.0`，beta 选择规则基于较旧客户端。
- `src/service/oauth.rs`：token test 和 usage 请求仍声明 `2.1.81`；token test Stainless package 为 `0.70.0`。
- `src/service/telemetry.rs`：fallback UA、测试 identity 和 `service.version` 继承旧值。
- `src/service/account.rs`：UA 识别测试仍固定 `2.1.81`，这些测试可改为新版本或改写为与具体版本无关的断言。

## 为什么仍建议抓包

静态分析可以可靠确认版本、构建时间、SDK 版本和候选字段，但不能确认运行时最终 wire image。最小抓包需要回答：

1. 标准 Sonnet 中转站请求实际 header 集、wire casing、header 顺序和 beta 顺序。
2. Haiku 请求是否仍省略 `claude-code-20250219`，以及其他模型条件差异。
3. `X-Claude-Code-Session-Id`、`x-client-request-id`、`x-stainless-helper-method` 等动态字段在实际请求中的存在条件。
4. 使用现有中转站 API 时，官方 2.1.258 发给 relay 的实际 header/body 结构。
5. `/api/oauth/usage`、event/metrics 等 Claude.ai OAuth 辅助路径无法由本地登录抓取，需要从二进制控制流和 cc-bridge rewrite 输出核对，并明确标为非 live evidence。
6. 事件 `env` 对象在 2.1.258 中的完整字段集合，尤其新增/删除字段及空值省略规则。
7. 传输层特征：TLS ClientHello 的 JA3/JA4、cipher/extension 顺序、ALPN，以及 HTTP/2 SETTINGS、伪 header 与普通 header 顺序。当前 cc-bridge 使用 `NODEJS_FINGERPRINT`，而 2.1.258 是 Bun 打包的原生 Mach-O，不能仅凭名称假定两者仍相同。

## 抓包数据最小化与脱敏

- 只执行低成本、无工具调用的固定提示词，例如要求返回 `OK`。
- 首选通过现有中转站 API 发两次请求：默认 Sonnet 一次、Haiku 一次；不执行 Claude.ai 登录，不运行 1M、fast mode 或远程控制。
- 应用层用 MITM 记录最终 HTTP 请求；传输层另用本地握手/packet 记录分析 ClientHello 和 HTTP/2 参数，避免把 MITM 自己的上游 TLS 当成 Claude Code 的 TLS。
- 保存结构，不保存秘密：删除 `Authorization`、Cookie、token、邮箱、account/org UUID、device/session/request ID、提示词正文和响应正文。
- 原始抓包只落在 `/tmp`，生成脱敏摘要后删除原始文件。
- 不把抓包作为“规避风控”的依据，只用于验证客户端兼容性和消除自相矛盾的协议声明。

## 工具状态

- 本机有 `/opt/homebrew/bin/mitmdump`。
- `/Users/qiubingnan/.local/bin/claude-capture` 是一个指向仓库 `scripts/telemetry-capture.sh` 的断链；当前仓库没有该脚本，因此不能直接复用旧入口。

## Relay-only capture limitation

The local environment has relay credentials in the active CAC settings (`ANTHROPIC_BASE_URL` and `ANTHROPIC_AUTH_TOKEN`) but no usable local Claude.ai login. Therefore live capture must target the existing relay. This is sufficient for the official client-to-relay application profile and client-side TLS handshake, but it is not direct evidence for Claude.ai OAuth usage/metrics/event endpoints or server-dependent HTTP/2 behavior against `api.anthropic.com`.
