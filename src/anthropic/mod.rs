//! Anthropic API 兼容服务模块
//!
//! 提供与 Anthropic Claude API 兼容的 HTTP 服务端点。
//!
//! # 支持的端点
//!
//! ## 标准端点 (/v1)
//! - `GET /v1/models` - 获取可用模型列表
//! - `POST /v1/messages` - 创建消息（对话）
//! - `POST /v1/messages/count_tokens` - 计算 token 数量
//!
//! ## Claude Code 兼容端点 (/cc/v1)
//! - `POST /cc/v1/messages` - 创建消息（流式响应会等待 contextUsageEvent 后再发送 message_start，确保 input_tokens 准确）
//! - `POST /cc/v1/messages/count_tokens` - 计算 token 数量（与 /v1 相同）
//!
//! # 使用示例
//! ```rust,ignore
//! use kiro_rs::anthropic;
//!
//! let app = anthropic::create_router("your-api-key");
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await?;
//! axum::serve(listener, app).await?;
//! ```

mod converter;
mod handlers;
mod middleware;
mod openai;
mod responses;
pub mod cache_force;
pub mod cache_metering;
mod router;
pub mod signature_sim;
pub mod stream;
pub mod types;
mod websearch;
mod websearch_loop;

// `create_router_with_provider` 是公开扩展点（允许外部以自定义 provider 构造路由），
// 项目内默认走 `create_router_with_shared_key`，因此本身不会触发该函数。
#[allow(unused_imports)]
pub use router::create_router_with_provider;
pub use router::create_router;

#[cfg(test)]
mod response_field_guard {
    //! 响应字段纯净性守卫。
    //!
    //! 上游把 `meteringEvent` 的 credit 计费元数据透传进 Anthropic / OpenAI 响应的
    //! `usage` 对象（`credit_usage` / `credit_unit` / `credit_unit_plural`）。这些
    //! 不是 Anthropic 官方字段，本 fork 刻意不对外输出：credit 只在管理端内部用于
    //! 统计（trace / 用量面板 / 客户端 Key 累计）。
    //!
    //! 这条测试把该约定固化住 —— 后续从上游合并功能时，若这些字段被重新带回
    //! 响应构造路径，CI 会直接失败，而不是等到客户端发现响应里多了非标准字段。
    //!
    //! 刻意做成源码级扫描而非行为断言：响应体在多处 `json!` 字面量里拼装
    //! （handlers / stream / openai / responses / websearch），逐个构造运行时
    //! 场景成本过高，而这些字段名本身足够特异，扫描不会误报。

    /// 禁止出现在对外响应构造中的字段名。
    ///
    /// 只列**响应字段名**，不列内部变量名：`credits` 作为累计变量在
    /// `websearch_loop.rs` 里合法使用，不在禁止范围内。
    const FORBIDDEN_RESPONSE_FIELDS: &[&str] = &[
        "credit_usage",
        "credit_unit",
        "credit_unit_plural",
        "\"cost\"",
    ];

    /// 参与扫描的源文件（对外响应体的全部构造点）
    const RESPONSE_SOURCES: &[(&str, &str)] = &[
        ("handlers.rs", include_str!("handlers.rs")),
        ("stream.rs", include_str!("stream.rs")),
        ("openai.rs", include_str!("openai.rs")),
        ("responses.rs", include_str!("responses.rs")),
        ("websearch.rs", include_str!("websearch.rs")),
        ("websearch_loop.rs", include_str!("websearch_loop.rs")),
    ];

    #[test]
    fn response_paths_never_emit_credit_or_cost_fields() {
        let mut violations = Vec::new();

        for (name, source) in RESPONSE_SOURCES {
            for (lineno, line) in source.lines().enumerate() {
                for field in FORBIDDEN_RESPONSE_FIELDS {
                    if line.contains(field) {
                        violations.push(format!(
                            "{}:{}: 出现禁止的响应字段 {} -> {}",
                            name,
                            lineno + 1,
                            field,
                            line.trim()
                        ));
                    }
                }
            }
        }

        assert!(
            violations.is_empty(),
            "对外响应体不得包含 credit / cost 等非 Anthropic 官方字段。\n\
             credit 计费数据只允许留在管理端内部统计里。\n\
             若确需放宽，请连同本测试一起评审。\n违规位置：\n{}",
            violations.join("\n")
        );
    }
}
