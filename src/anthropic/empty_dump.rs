//! 空回现场取样：把触发「HTTP 200 但零产出」的上游请求体落盘。
//!
//! 背景（2026-07-27 实测，24h / 102249 次流式请求 / 32 次空回）：
//! - 上游全程沉默，最后吐一个不含内容的 chunk 就关流（`first_token_ms` 恒等于
//!   `duration_ms`）；耗时 1.7s–191s 不固定，所以不是超时触发。
//! - **换凭据无效**：六组客户端重试，每组都被路由到不同账号，每次仍然空回。
//!   同一段对话内容换三个账号打过去三次全空 → 触发条件在请求内容里，
//!   不在账号状态上。自动换号重试因此不是可行的修法。
//! - token 量不构成单调关系（30–60K 档 0.059%，>60K 反而降回 0.018%），
//!   所以「限制请求大小」也规避不了。
//!
//! 剩下唯一能定位的路径就是拿到请求体本身做对比。trace 只存元数据，看不到内容。
//!
//! 默认关闭，零开销；置 `KIRO_DUMP_EMPTY=1` 才落盘，且只在判定为空回时写。
//! 正常请求一个字节都不写。

use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;

/// 开关环境变量。接受 `1` / `true` / `yes`（大小写不敏感）。
const ENV_ENABLE: &str = "KIRO_DUMP_EMPTY";
/// 落盘目录环境变量，默认 `dump`（相对进程工作目录）。
const ENV_DIR: &str = "KIRO_DUMP_EMPTY_DIR";
/// 单个请求体落盘上限（字节）。超过则截断——诊断只需要结构与首尾形状，
/// 完整的 50MB 请求体既没必要也会撑爆磁盘。
const BODY_MAX: usize = 2 * 1024 * 1024;

/// 进程内只解析一次开关，避免每次请求都读环境变量。
fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var(ENV_ENABLE)
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                v == "1" || v == "true" || v == "yes"
            })
            .unwrap_or(false)
    })
}

fn dump_dir() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        std::env::var(ENV_DIR)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("dump"))
    })
}

/// 是否已启用取样。调用方可用它跳过 body 克隆等准备工作。
pub fn is_enabled() -> bool {
    enabled()
}

/// 落盘一次空回现场。
///
/// `trace_id` 用于和 trace 记录对齐；`reason` 是失败分类
/// （`upstream_empty_response` / `upstream_stream_error`）。
///
/// 失败仅 warn，绝不影响请求本身——这是诊断设施，不能反过来拖累线上。
pub fn record(trace_id: &str, reason: &str, model: &str, request_body: &str) {
    if !enabled() {
        return;
    }
    let dir = dump_dir();
    if let Err(e) = std::fs::create_dir_all(dir) {
        tracing::warn!("创建空回取样目录 {} 失败: {}", dir.display(), e);
        return;
    }

    // 文件名只用 trace_id（uuid v4，本身是安全字符集），不拼接外部字符串。
    let path = dir.join(format!("{}.json", sanitize(trace_id)));
    let (body, truncated) = truncate(request_body);
    let envelope = serde_json::json!({
        "traceId": trace_id,
        "reason": reason,
        "model": model,
        "ts": chrono::Utc::now().to_rfc3339(),
        "bodyTruncated": truncated,
        "bodyBytes": request_body.len(),
        // 原样保存字符串而不是解析后的 JSON：要对比的正是发出去的字节，
        // 重新序列化会改变字段顺序、丢失原始形态。
        "requestBody": body,
    });

    let write = || -> std::io::Result<()> {
        let mut f = std::fs::File::create(&path)?;
        f.write_all(serde_json::to_string_pretty(&envelope)?.as_bytes())?;
        f.write_all(b"\n")
    };
    match write() {
        Ok(()) => tracing::warn!(
            trace_id,
            reason,
            "已落盘空回现场: {}（诊断用，可在排查完成后关闭 {}）",
            path.display(),
            ENV_ENABLE
        ),
        Err(e) => tracing::warn!("落盘空回现场 {} 失败: {}", path.display(), e),
    }
}

/// 只保留文件名安全字符，防止 trace_id 异常时穿越目录。
fn sanitize(id: &str) -> String {
    id.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(64)
        .collect()
}

/// 按字符边界截断到 [`BODY_MAX`]，返回 (内容, 是否被截断)。
fn truncate(body: &str) -> (&str, bool) {
    if body.len() <= BODY_MAX {
        return (body, false);
    }
    let mut end = BODY_MAX;
    while end > 0 && !body.is_char_boundary(end) {
        end -= 1;
    }
    (&body[..end], true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_strips_path_traversal() {
        assert_eq!(sanitize("../../etc/passwd"), "etcpasswd");
        assert_eq!(
            sanitize("9f8e7d6c-1234-4abc-89de-000000000001"),
            "9f8e7d6c-1234-4abc-89de-000000000001"
        );
        assert_eq!(sanitize(""), "");
        // 超长 id 被裁到 64 字符
        assert_eq!(sanitize(&"a".repeat(200)).len(), 64);
    }

    #[test]
    fn truncate_respects_limit_and_char_boundary() {
        let short = "hello";
        assert_eq!(truncate(short), ("hello", false));

        // 多字节字符跨越上限时不能切碎
        let long = "中".repeat(BODY_MAX);
        let (out, truncated) = truncate(&long);
        assert!(truncated);
        assert!(out.len() <= BODY_MAX);
        assert!(out.chars().all(|c| c == '中'), "不应产生半个字符");
    }

    /// 未设开关时必须完全不落盘——诊断设施默认零开销。
    #[test]
    fn disabled_by_default_writes_nothing() {
        // 该测试进程未设 KIRO_DUMP_EMPTY，enabled() 应为 false。
        // 注意 OnceLock 只初始化一次，故本测试不修改环境变量。
        assert!(!is_enabled());
        record("trace-should-not-exist", "r", "m", "{}");
        assert!(
            !PathBuf::from("dump/trace-should-not-exist.json").exists(),
            "关闭时不应产生任何文件"
        );
    }
}
