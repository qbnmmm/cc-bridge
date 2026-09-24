# 前端 API 与数据契约

## 单一 API client

所有管理端 fetch 封装集中在 `web/src/api.ts`：

- `request<T>` 统一添加 JSON content type 和 Bearer 管理密码。
- 非 2xx 尝试读取 `{"error": ...}` 并抛出 `Error`。
- `api` 对象定义 accounts、tokens、dashboard 和 OAuth endpoints。
- response/request interface 与后端 model/handler 字段保持 snake_case。

页面组件不得直接复制 fetch/header/error 逻辑。新增 endpoint 时先在 `api.ts` 增加 typed method，再在页面调用。

## 认证

`web/src/router.ts` 把管理员密码保存在内存 `authToken` 和 localStorage key `claude-code-gateway_auth`。启动时通过 `GET /admin/dashboard` 验证一次；失败清除保存值。路由用 `meta.requiresAuth` 和 `beforeEach` 保护 dashboard。

后端 `admin_auth` 同时支持 `x-api-key` 与 Bearer，但前端固定使用 Bearer。不要在页面或 URL 中传递密码。

## Account 字段

权威链路：

```text
src/model/account.rs
  -> src/store/account_store.rs
  -> src/handler/router.rs
  -> web/src/api.ts::Account
  -> Accounts.vue
```

重要契约：

- `auth_type` 是 `setup_token | oauth`；切换模式时前端必须提供对应 secret，后端 `normalize_account_auth` 会清空另一模式字段。
- `expires_at` response 是毫秒时间戳；输入可发送 RFC3339 或毫秒，`Accounts.vue::normalizeExpiresAtInput` 当前转为 ISO string。
- nullable identity 字段用 `null` 清除，而不是空字符串保留旧值。
- `priority` 数值越小越优先；`concurrency` 是每账号流式生命周期上限。
- `usage_data` 给 UI 的 utilization 是百分比 `0..100`，不要再乘 100。
- `rate_limit_reset_at` 是账号级限制；Sonnet overlay 在 `usage_data.seven_day_sonnet` 与 `sonnet_rate_limited_until`。

## Token 和分页

accounts 默认 page size 12，tokens 默认 20；后端都 clamp 到 1..100。列表响应统一为 `PagedResult<T>`。token 的 allowed/blocked accounts 当前是逗号分隔 ID 字符串，后端 model helper 负责解析；前端仅编辑字符串和辅助选择。

## Scenario: 编辑现有账号的提示词工作目录

### 1. Scope / Trigger

- 仅用于编辑现有账号；新建账号继续由 `identity.rs` 生成默认路径。
- 这是 UI → API → JSON 存储 → prompt rewriter 的跨层字段。

### 2. Signatures

- `PUT /admin/accounts/:id`
- Request 顶层可选字段：`prompt_working_dir: string`
- 存储目标：`accounts.canonical_prompt_env.working_dir`（无 schema 变更）

### 3. Contracts

- Request 不接受前端整体覆盖 `canonical_prompt_env`。
- 字段缺失时不修改 prompt env；字段存在时 trim 后只写 `working_dir`，保留其他和未知 JSON 字段。
- Response 仍通过 `Account.canonical_prompt_env.working_dir` 返回最终值。

### 4. Validation & Error Matrix

- 非 string、空值、非 `/` 开头、含空白/控制字符、超过 1024 字符 -> `400 Bad Request`。
- `canonical_prompt_env` 不是 JSON object -> `400 Bad Request`。
- 合法更新后 store 必须失效 schedulable-account cache。

### 5. Good/Base/Bad Cases

- Good: `/Users/dev/project` 会同时作为 working directory，并为 prompt 内 home 路径提供 `/Users/dev/` 前缀。
- Base: `/workspace/project` 只替换 working-directory 文本，不改写其他 `/Users/<name>/` 文本。
- Bad: `relative/path` 或 `/Users/dev/my project` 必须在写库前拒绝。

### 6. Tests Required

- Handler 单测：合法路径、缺失字段、未知 JSON 保留与非法矩阵。
- SQLite 往返：update 后重新读取 `working_dir` 并确认未知字段仍在。
- Rewriter 单测：home/non-home、Unicode、`$` 字面量和 `<system-reminder>` 作用域。
- 前端严格构建：`npm run build`。

### 7. Wrong vs Correct

Wrong：前端发送整个 `canonical_prompt_env`，容易丢失后端未知字段。

Correct：前端只发 `prompt_working_dir`，handler 就地更新已存在 JSON object 的 `working_dir`。

## OAuth flow

生成 auth URL 返回 `auth_url/session_id`，exchange 返回秒单位 `expires_at`；`Accounts.vue::applyOAuthResult` 转成毫秒再填表。OAuth session 只在后端内存保留 30 分钟且 exchange 为 take-once，失败后可能需要重新生成链接。不要把 session 当持久资源。

## 跨层修改规则

新增/改名/删除 API 字段时，必须同一变更中检查 Rust serde、SQL mapping、handler request/response、`api.ts` interface 和模板消费点。新增 SPA route 时还必须在 `src/handler/router.rs` 注册直接访问路径，否则刷新会进入 gateway fallback。

## Scenario: 账号模型级周用量展示

### 1. Scope / Trigger

- Accounts 卡片消费 `usage_data.limits` 并展示 Sonnet、Fable 或未来模型的 scoped weekly quota。

### 2. Signatures

- `UsageData.limits?: ScopedUsageLimit[] | null`。
- `ScopedUsageLimit` 字段：`type`、`scope.model.model_group`、`scope.model.display_name`、`utilization`、`resets_at`、可选 `status`。
- 兼容字段：`seven_day_sonnet?`、`seven_day_fable?`。

### 3. Contracts

- 只展示 `type=weekly_scoped` 且 group/utilization/reset 有效的项目。
- dynamic row 优先；缺少同 group dynamic row 时才使用 legacy mirror。
- 缺失/null 表示“不适用或无数据”，不得显示 0%。
- claim 名称优先使用 scoped `display_name`；只有 `seven_day_overage_included` 且无 metadata 时显示中性名称“7 天套餐内模型额度”。

### 4. Validation & Error Matrix

- 未知 group -> 使用由 group 生成的稳定可读名称并展示。
- utilization 非数字或 reset 缺失 -> 忽略该项。
- 重复 group -> 后项覆盖前项。
- unknown representative claim -> 原样回退，不隐藏协议变化。

### 5. Good/Base/Bad Cases

- Good：上游返回 Fable 42%、Sonnet 18%，页面显示两条真实进度条。
- Base：只有旧 `seven_day_sonnet`，页面显示一条 Sonnet。
- Bad：固定渲染“7 天 Sonnet 0%”，把无数据误报为零用量。

### 6. Tests Required

- `vue-tsc -b`/`npm run build` 严格类型通过。
- 后端 fixture 覆盖 dynamic/legacy/null，页面 helper 与 `UsageData` 契约保持一致。

### 7. Wrong vs Correct

Wrong：在模板为每个模型硬编码永久行，并用 `?? 0` 填充缺失数据。

Correct：先规范化实际返回的 scoped limits，再用 `v-for` 渲染存在的模型行。

## 请求性能页面

`/performance` 同时在 Vue 与后端 SPA 路由注册，API 定义在 api.ts。usePerformance 管理 active 5 秒/overview 60 秒轮询；页面隐藏/卸载暂停并 abort，旧响应不能覆盖新筛选结果，失败保留已有数据与更新时间。性能组件的 null 必须显示 `—`，不能当零；active 明确当前实例、与历史时间范围无关。首内容含思考/工具输入，首正文仅 text，完整时延分位数只计算完整成功且非空样本。请求取消只称“取消 / 响应释放”，不能指认责任方。后端字段、保留/鉴权和质量验证见 gateway/backend/performance-monitoring.md。
