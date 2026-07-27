# Fork 自有改动清单

本文档记录本仓库（`aabao325/kiro.rs-admin`）相对上游 `ZyphrZero/kiro.rs` 的**自有改动**，
用于每次合并上游前后核对、防止被上游覆盖丢失。

- 上次合并：upstream v0.7.2（`ebf8e1c`），合并提交 `b76a5ba`，分支 `merge-upstream-0.7.2`
- 原分叉点：`ad257d2` — upstream v0.7.1
- fork 自有提交：`582d316` 新增缓存 → `fbce8bc` 优化响应顺序 → `9a98f03` 优化提示词
- CI：`.github/workflows/check.yaml`（非 master 分支触发 `cargo check` + `test`，
  clippy 仅参考不阻断——仓库存量 lint 175 条全在既有代码里）

改动分五块，重要性由高到低：

| # | 模块 | 核心文件 | 必须保留 |
|---|------|---------|---------|
| 1 | 缓存三档模式（Off/Auto/Force） | `src/anthropic/cache_force.rs` | ✅ 长期功能 |
| 2 | 身份兜底提示词 | `src/anthropic/converter.rs` | ✅ 长期功能 |
| 3 | Token 上报口径改造 | `handlers.rs` / `stream.rs` / `websearch_loop.rs` | ✅ 与上游分歧，已选 fork 版 |
| 4 | 响应体字段对齐官方 | `handlers.rs` / `stream.rs` / `signature_sim.rs` | ✅ 长期功能 |
| 5 | ~~模型名透传~~ | — | ❌ 已弃用，采上游实现 |

---

## 1. 缓存三档模式（`cache_force`）

新增文件 `src/anthropic/cache_force.rs`（443 行），提供管理面板可调的缓存上报模式，
与上游原有的 `cache_metering.rs`（哈希链智能模拟）**并列存在、互不改动**。

三档模式（`CacheMode`）：

- **`Off`** — 完全不注入缓存字段，`cache_creation`/`cache_read` 恒为 0，
  模拟官方未使用 `cache_control` 时的响应形态。
- **`Auto`**（默认，行为等同改造前）— 走 `cache_metering.rs` 的哈希链前缀命中。
- **`Force`** — 无视请求是否带 `cache_control`，按管理员设定的三个比例
  （`cacheable_ratio` / `creation_ratio` / `hit_ratio`，均 clamp 到 `[0,1]`）
  强制拆分本次估算的 input token 总量。

关键实现约束（合并时不能破坏）：

- `input_tokens` 恒 `>= 1`：可分配给 `creation + read` 的上限固定为 `total - 1`。
  Anthropic 语义下 `input_tokens == 0` 意味着"这轮无任何新输入"，与"发生了一次真实请求"矛盾。
- `Force` / `Off` 模式下跳过 `compute_cache_usage` 的哈希链查写，省掉 O(会话长度) 的同步开销。
- 统一出口是 `cache_force::resolve(settings, auto_usage, total, ttl_secs) -> ResolvedCacheUsage`，
  流式与非流式两条路径都只调它，保证口径一致。

TTL 分桶：`ResolvedCacheUsage` 额外给出 `ephemeral_5m_input_tokens` /
`ephemeral_1h_input_tokens`（二者之和恒等于 `cache_creation_input_tokens`），
由 `cache_metering::detect_max_ttl` 探测请求里出现过的最大 TTL 决定分到哪个桶。
为此把上游的 `detect_max_ttl` 从私有函数提升为 `pub(crate)`。

配套改动：

- `src/main.rs` — 启动时 `CacheForceStore::load(cache_dir/cache_force.json)`，持久化设置。
- `src/anthropic/middleware.rs` — `AppState.cache_force: Option<SharedCacheForceStore>` + `with_cache_force()`。
- `src/anthropic/router.rs` — `create_router` 增加第 9 个参数 `cache_force`。
- `src/admin/middleware.rs` — `AdminState.cache_force`（与 anthropic 路由共享同一 store）。
- `src/admin/handlers.rs` — `GET/PUT /api/admin/cache-force`。
- `src/admin/router.rs` — 挂载上述路由。
- admin-ui：`api/cache-force.ts`、`hooks/use-cache-force.ts`、`components/cache-force-card.tsx`（186 行面板）、
  `components/topbar-tools.tsx` 接入入口、`types/api.ts` 类型。

## 2. 身份兜底提示词（`IDENTITY_FALLBACK_POLICY`）

`src/anthropic/converter.rs` 新增常量 `IDENTITY_FALLBACK_POLICY`：给"裸请求"
（客户端没带 `system`，或带的 `system` 里没有任何身份设定）一个稳定的默认口径——
被问及身份/名称/模型版本/开发方时回答 Claude / Anthropic，且不猜测具体版本号。

**关键设计：不覆盖用户自己的身份设定。** 新增 `client_declares_identity()`
检测客户端 `system` 是否已自带身份/人格/角色设定，命中则该策略整段不注入，
用户的设定直接生效。检测覆盖：

- 英文：`you are` / `you're` / `your name is` / `act as` / `roleplay as` /
  `pretend to be` / `persona` / `identity` / `impersonate`（共 10 个标记，小写后匹配）
- 中文：`你是` / `你叫` / `你的名字` / `你的身份` / `扮演` / `角色扮演` / `人格` /
  `身份是` / `自称`（共 9 个，原串匹配）

判定刻意做得**宽松**（宁可多让路、少覆盖）：身份是用户显式表达的意图，
误覆盖（用户要猫娘、模型坚持自称 Claude）比误让路（用户没设身份、模型自称什么都行）
后果严重得多。

同时重写了 `build_history` 的系统消息组装逻辑，这是与上游冲突的主要位置：

- **上游**：只有客户端带了 `system` 或启用了 thinking，才注入
  `user + assistant("I will follow these instructions.")` 系统消息对。
- **本 fork**：统一在一处组装，顺序为 `thinking_prefix`（若需要）→ 客户端 `system`
  → `SYSTEM_CHUNKED_POLICY`（仅当客户端有 system）→ `IDENTITY_FALLBACK_POLICY`
  （仅当客户端未自带身份设定）。上游那个 `else if thinking_prefix` 分支已被此逻辑覆盖，
  合并时应删除，否则会重复注入。

注入内容**不计入**上报给客户端的 token（token 估算跑在注入前的原始 payload 上）。

单测：`test_client_declares_identity_detects_user_persona`、
`test_client_declares_identity_ignores_plain_instructions`、
`test_build_history_injects_identity_fallback_without_system`、
`test_build_history_skips_identity_fallback_when_client_sets_persona`。

## 3. Token 上报口径改造（与上游实质分歧，已决定保留 fork 版）

### 3.1 input_tokens 来源

改造前后的差别只有一行，但影响所有上报数字：

```rust
// 上游（含 v0.7.2，至今未变）：contextUsageEvent 反推值优先
fn resolve_usage_input_tokens(fallback: i32, context_total: Option<i32>) -> i32 {
    context_total.unwrap_or(fallback)
}
// 其中 context_total = (context_usage_percentage × get_context_window_size(model)) / 100

// 本 fork：固定用客户端估算值
fn resolve_usage_input_tokens(fallback: i32) -> i32 { fallback }
```

改动理由（写在代码注释里）：`百分比 × 上下文窗口` 这个值会把 Kiro 上游自身的隐藏
agent 上下文、以及本项目注入的 `SYSTEM_CHUNKED_POLICY` / `IDENTITY_LOCK_POLICY` /
工具描述后缀一并计入；窗口越大（1M 模型）百分比的量化误差放大越严重——
实测一个几乎不含内容的请求被放大成 6000+ token。

同一改动落在三处：`handlers.rs` 非流式路径、`stream.rs` 的
`resolved_cache_usage_full()`、`websearch_loop.rs` 的多轮搜索循环
（后者改为每轮用 `token::count_all_tokens` 对当前 payload 重新估算，
以便工具结果追加带来的增长仍然可见）。

`Event::ContextUsage` 本身仍然处理，但只用于判断 `>= 100%` 时设
`stop_reason = model_context_window_exceeded`，不再用于算 token。

### 3.2 thinking_tokens 统计

`stream.rs` 新增 `StreamContext.thinking_output_tokens`（`output_tokens` 的子集），
在 5 处 thinking 文本出口累加，用于上报 `usage.output_tokens_details.thinking_tokens`。
非流式路径在 `handlers.rs` 里单独估算（优先原生 `reasoningContent`，
否则回退 `<thinking>` 文本提取）。

### 3.3 两套口径的取舍（2026-07-27 已决策）

| | 上游（v0.7.1→v0.7.2 未变） | 本 fork（已选） |
|---|---|---|
| input_tokens 来源 | contextUsage 百分比 × 窗口，拿不到才回落估算 | 固定客户端估算 |
| 是否含注入内容 | 含（Kiro 隐藏 agent 上下文 + 注入策略 + 工具描述） | 不含 |
| 1M 窗口模型表现 | 量化误差放大，实测空请求 → 6000+ token | 稳定 |
| 与 cache 分摊自洽性 | total 与 cache 覆盖量两套口径混用 | 同一套本地估算 |

上游那套更接近"上游真实消耗了多少"，fork 这套更接近"客户端以为自己发了多少"。
选 fork 版是因为这些数字要给客户端看——`tokens长度测试.md` 里 16 token 的短请求
必须显示 16。**若将来改为监控真实上游占用，需要换回上游口径。**

## 4. 响应体字段对齐官方样例

依据 `tokens长度测试.md` 里抓到的真实 Bedrock 响应样例，把响应体调成同形状。

### 4.1 字段顺序与新增字段

`Cargo.toml` 给 `serde_json` 开 `preserve_order`，使 `json!` 字面量顺序即序列化顺序。
非流式响应体与流式 `message_start` / `message_delta` 统一为：

```
model, id, type, role, content, stop_reason, stop_sequence, stop_details, usage
```

`usage` 内新增：`cache_creation.{ephemeral_5m_input_tokens, ephemeral_1h_input_tokens}`、
`output_tokens_details.thinking_tokens`、`service_tier: "standard"`、
`inference_geo: "not_available"`。

`SseStateManager::generate_final_events` 现为 **8 参**，是两侧新增参数的合成结果：
fork 加的 `ephemeral_5m` / `ephemeral_1h` / `thinking_tokens`，加上游 v0.7.2 加的
`metering: Option<&MeteringEvent>`。`usage` JSON 以 fork 的官方字段顺序为基底，
再叠上游的 `credit_usage` / `credit_unit` / `credit_unit_plural` 透传
（仅在收到过 meteringEvent 时追加）。**下次合并这里仍是首要冲突点。**

### 4.2 context_management 字段按 beta header 开关

官方仅在请求带 `anthropic-beta: context-management-2025-06-27` 时才返回
`context_management` 字段（未开启时该 key 完全不出现，而非给 null）。
新增 `context_management_requested(&HeaderMap)`，对所有同名 header 值按逗号切分后精确匹配。

为此 `post_messages` / `post_messages_cc` 的签名多了 `headers: HeaderMap` 参数；
`openai.rs` / `responses.rs` 里的内部合成调用传 `HeaderMap::new()`。

### 4.3 message id 与 thinking signature 模拟

新增 `src/anthropic/signature_sim.rs`（212 行），取代上游的固定占位字符串
`THINKING_SIGNATURE_PLACEHOLDER`（该常量已被本 fork 删除）：

- `generate_message_id()` → `msg_bdrk_` + 52 位随机小写字母数字（对齐 Bedrock 真实格式）。
- `generate_thinking_signature(model)` → 按真实签名的 protobuf 字段位置拼字节
  （field 6 = 模型名，field 7 = 0，field 8 = `"thinking"`，field 11 = 随机 id，结尾 `18 01`），
  补随机字节到同量级长度后 base64 编码。

仅保证格式/长度/可 base64 解码这些外部可观察特征一致；不试图还原加密内容
（没有 Anthropic 私钥，密码学上不可能）。上游 Kiro 从不校验 signature，
converter 回传历史消息时也只读 `thinking` 文本。

## 5. 模型名透传（已弃用 fork 版，采上游实现）

fork 曾把 `map_model` 的白名单拦截改为"未登记模型原样透传给上游"，
并补了 `claude-opus-5` / `claude-haiku-5` 显式归一。合并 v0.7.2 时已整份弃用。

上游 v0.7.2 的 `c72dc52` 做了同一件事但更完善：`normalize_claude_model` 支持
`-latest` 后缀、8 位日期后缀、`claude-sonnet-5-2 → claude-sonnet-5.2` 版本号规范化、
旧式 `claude-3-5-sonnet-20241022 → claude-sonnet-3.5`，另有 ID 长度/控制字符校验
（`MAX_MODEL_ID_LEN = 256`）与配置驱动的自定义模型表（`src/model/custom_models.rs`）。

**已采上游实现。** 一个待实测确认项：`get_context_window_size` 对
`claude-opus-5` 的窗口判定——上游走 `map_model` 归一后应命中 1M 分支，
但没有对应单测，建议实际发一次 opus-5 请求核对上报的 input_tokens 量级。

---

## 下次合并上游的检查清单

1. `git merge upstream/master`，预期冲突集中在
   `converter.rs` / `handlers.rs` / `stream.rs` / `router.rs` / `main.rs` /
   `topbar-tools.tsx`。
2. `converter.rs` 冲突若过于交错，取上游整份后重贴两处 fork 改动
   （`IDENTITY_FALLBACK_POLICY` + `client_declares_identity` 常量函数、
   `build_history` 系统消息组装），比逐块解冲突可靠。
3. `generate_final_events` 签名核对：fork 的 3 个参数 + 上游新增的都要在。
4. 确认 `resolve_usage_input_tokens` 仍是单参版本（不吃 `context_input_tokens`）。
5. 确认 `router.rs` 三个 `create_router*` 函数都把 `cache_force` 传到底。
6. 推分支等 `.github/workflows/check.yaml` 跑绿（`cargo check` + `test` 是硬门槛，
   clippy 仅参考）。
7. `Cargo.toml` 若改了依赖 feature，记得 `Cargo.lock` 也要同步
   （`--locked` 会拒绝不一致，如 `serde_json` 的 `preserve_order` → `indexmap`）。

