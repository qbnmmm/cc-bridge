# 提交方案（已获用户授权）

## 1. feat: 增加请求性能监控与慢响应诊断

包含后端生命周期采集、SQLite/PG 性能存储与查询、管理台及 README/spec。以下 tracked 文件的改动全部来自本次任务；token_store.rs、tlsfp.rs 只包含 rustfmt 格式调整。

- `.env.example`
- `.trellis/spec/gateway/backend/performance-monitoring.md`
- `.trellis/spec/web/frontend/api-contracts.md`
- `README.md`
- `src/config.rs`
- `src/handler/mod.rs`
- `src/handler/performance.rs`
- `src/handler/router.rs`
- `src/main.rs`
- `src/model/mod.rs`
- `src/model/performance.rs`
- `src/service/gateway.rs`
- `src/service/mod.rs`
- `src/service/performance.rs`
- `src/service/usage.rs`
- `src/store/db.rs`
- `src/store/mod.rs`
- `src/store/performance_store.rs`
- `src/store/token_store.rs`
- `src/store/usage_store.rs`
- `src/tlsfp/tlsfp.rs`
- `web/src/api.ts`
- `web/src/components/Dashboard.vue`
- `web/src/components/performance/Performance.vue`
- `web/src/components/performance/PerformanceDetail.vue`
- `web/src/components/performance/PerformanceFilters.vue`
- `web/src/components/performance/PerformanceOverview.vue`
- `web/src/components/performance/PerformanceRequests.vue`
- `web/src/composables/usePerformance.ts`
- `web/src/lib/performance.ts`
- `web/src/router.ts`

## 2. chore(task): 记录性能监控方案与验收

仅包含本次新建 `.trellis/tasks/09-24-request-performance-monitoring/` 内的 PRD、design、implement、research、validation、commit-plan、task.json 和 create 自动生成的 context manifests。

## 保留的既有工作

不纳入本次提交：既有未跟踪 `.agents/`、`.claude/`、`.codex/`、Trellis runtime/配置/其他任务/工作区记录、`AGENTS.md`、`fingerprint-audit.jsonl`，以及原本已存在但未跟踪的 spec 文件。

`.trellis/spec/gateway/backend/index.md` 原本未跟踪，本次只追加性能规范索引行；保留在工作区，不将整份既有索引默认为本次新文件提交。新建 performance-monitoring.md 和已跟踪的 frontend/api-contracts.md 包含本次实际契约。

用户已授权提交并推送 GitHub、通过 CI 构建镜像。逐路径提交功能和任务文档，并新增发布版本提交 `chore(release): 发布 v1.8.3-qbn.11`；归档本任务后推送 origin/ccb。既有未跟踪 workspace 日志仅本地追加，避免把旧会话记录混入此次发布。
