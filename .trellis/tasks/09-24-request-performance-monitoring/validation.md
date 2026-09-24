# 验收记录

日期：2026-09-24；工作区 `/Users/qiubingnan/RustProject/cc-bridge`，分支 `ccb`，基线 `b2a0c21`。

## 自动验证

| 检查 | 结果 |
| --- | --- |
| vue-tsc + Vite production build | 通过 |
| cargo test --lib --quiet | 230 passed / 0 failed / 1 ignored |
| 隔离 PostgreSQL 15 专项（显式 --ignored） | 1 passed；与 SQLite 共用数据断言 |
| cargo check --locked | 通过 |
| cargo fmt --check / git diff --check | 通过 |

新增测试覆盖生命周期、心跳/思考/文本/工具口径、内容解析复用、五类响应编码原字节透传、HTTP200 流错误、缺少终止、取消、队列/数据库故障、禁用/过期、HTTP frame/size_hint/trailer 保持、SQL 去重/精确分位数/过滤/清理和输入边界。现有遥测/并发槽/超时/计费测试通过。

最初在沙箱内运行时，4 项既有 loopback 测试因端口权限失败；经工具批准在沙箱外完整重跑通过。迁移版本提升后同步更新原有 schema 断言。PostgreSQL 实测发现并修复本功能中的 Any NULL 类型丢失和问号占位符兼容问题；不是仅静态核对 SQL。

## HTTP 与浏览器验收

仅使用临时 SQLite、测试 Token、空账号库和构造的性能记录，无生产数据、无真实模型调用。

- 未认证访问 overview/active/requests/dimensions 全部 401；本地推理鉴权失败有独立记录。
- 非法 sort/page/时间范围被拒绝；32 分钟记录进入正确直方图档，按耗时排序居首，UUID 详情查询成功。
- 桌面登录/导航/直接刷新、模型筛选、慢请求表格与详情通过。
- 持续未完成的本地上传在 active 列表实时显示增长的 age/idle；真正 30 分钟 SSE 心跳场景使用确定性测试验证，未等待真实 30 分钟。
- 模拟 overview 请求失败后，保留最近指标并显示错误与更新时间。
- 手机 390px 宽度：表格内部可横向滚动，document.scrollWidth=390；页面无浏览器执行错误。
- 所有临时服务和独立浏览器会话已停止。

预览（测试数据）：`/Users/qiubingnan/.codex/visualizations/2026/09/24/01a0d123-769b-7af3-b597-d08039702ca7/performance-preview.png`。

## 适用边界

实时列表限当前实例；历史保留 30 个新加坡自然日。无线上部署或真实上游性能测量，本次交付不能证明今天慢响应的实际原因。生成时序为网关消费观测，受压缩和客户端背压影响；进程崩溃/队列丢弃可能造成缺口。
