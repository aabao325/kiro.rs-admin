//! 上游元数据事件
//!
//! Kiro 在 `metadataEvent.tokenUsage` 中返回本次模型调用的**服务端真实**
//! token 用量——也就是上游据以计费的那份数字。四个字段是单次调用的最终快照，
//! 不是增量事件；调用方应在同一条流内保留最后一份快照。
//!
//! 注意与 `metering.rs` 的区别：`meteringEvent` 只下发 credit 总数，不含
//! token 明细；token 明细在**本事件**里。两者是不同的事件类型。

use serde::Deserialize;

use crate::kiro::parser::error::ParseResult;
use crate::kiro::parser::frame::Frame;

use super::base::EventPayload;

/// 单次 Kiro 模型调用的精确 token 用量。
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    /// 未命中缓存、也未写入缓存的输入 token。
    #[serde(default)]
    pub uncached_input_tokens: i32,
    /// 模型输出 token。
    #[serde(default)]
    pub output_tokens: i32,
    /// 从服务端 prompt cache 读取的输入 token。
    #[serde(default)]
    pub cache_read_input_tokens: i32,
    /// 本次写入服务端 prompt cache 的输入 token。
    #[serde(default)]
    pub cache_write_input_tokens: i32,
}

impl TokenUsage {
    /// 清理不可信上游值，确保所有计数非负。
    pub fn sanitized(self) -> Self {
        Self {
            uncached_input_tokens: self.uncached_input_tokens.max(0),
            output_tokens: self.output_tokens.max(0),
            cache_read_input_tokens: self.cache_read_input_tokens.max(0),
            cache_write_input_tokens: self.cache_write_input_tokens.max(0),
        }
    }

    /// 从服务端真值的输入三段中扣除中转层内置提示词。
    ///
    /// 上游不提供内置前缀落在哪个缓存桶的分段归属；身份历史是稳定的首段前缀，
    /// 因此按 cache read → cache write → uncached 的顺序做归属估算。总输入扣减是确定的，
    /// 三桶归属是保守估算。最多扣到总输入仍剩 1，避免 tokenizer 估算略高时误吞
    /// 真实用户输入。output_tokens 不受影响。
    pub fn subtract_injected_input(self, injected_tokens: i32) -> Self {
        let mut usage = self.sanitized();
        let max_deduct = usage.total_input_tokens().saturating_sub(1).max(0);
        let mut remaining = injected_tokens.max(0).min(max_deduct);

        let from_read = remaining.min(usage.cache_read_input_tokens);
        usage.cache_read_input_tokens -= from_read;
        remaining -= from_read;

        let from_write = remaining.min(usage.cache_write_input_tokens);
        usage.cache_write_input_tokens -= from_write;
        remaining -= from_write;

        let from_uncached = remaining.min(usage.uncached_input_tokens);
        usage.uncached_input_tokens -= from_uncached;

        usage
    }

    /// 合并多次真实 provider 调用的用量。
    ///
    /// websearch 循环、工具多轮会在一次客户端请求内打上游多次，每次都下发
    /// 自己的 tokenUsage 快照；对客户端要报告的是它们的总和。
    pub fn saturating_add(self, other: Self) -> Self {
        let left = self.sanitized();
        let right = other.sanitized();
        Self {
            uncached_input_tokens: left
                .uncached_input_tokens
                .saturating_add(right.uncached_input_tokens),
            output_tokens: left.output_tokens.saturating_add(right.output_tokens),
            cache_read_input_tokens: left
                .cache_read_input_tokens
                .saturating_add(right.cache_read_input_tokens),
            cache_write_input_tokens: left
                .cache_write_input_tokens
                .saturating_add(right.cache_write_input_tokens),
        }
    }

    /// Anthropic 口径的总输入 token（三段之和）。
    pub fn total_input_tokens(self) -> i32 {
        let usage = self.sanitized();
        usage
            .uncached_input_tokens
            .saturating_add(usage.cache_write_input_tokens)
            .saturating_add(usage.cache_read_input_tokens)
    }

    /// 是否为「全零」快照。
    ///
    /// 上游偶尔会下发一个字段全 0 的 tokenUsage。把它当真值会让本次请求的
    /// usage 变成 0，既不可能也会让记账凭空少一笔，因此视同缺失、回退估算。
    pub fn is_empty(self) -> bool {
        let u = self.sanitized();
        u.uncached_input_tokens == 0
            && u.output_tokens == 0
            && u.cache_read_input_tokens == 0
            && u.cache_write_input_tokens == 0
    }
}

/// `metadataEvent` payload。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataEvent {
    /// 有些 metadataEvent 只携带 stopReason，因此 tokenUsage 必须保持可选。
    #[serde(default)]
    pub token_usage: Option<TokenUsage>,
}

impl EventPayload for MetadataEvent {
    fn from_frame(frame: &Frame) -> ParseResult<Self> {
        frame.payload_as_json()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_official_token_usage_shape() {
        let event: MetadataEvent = serde_json::from_str(
            r#"{
                "tokenUsage": {
                    "uncachedInputTokens": 101,
                    "outputTokens": 23,
                    "cacheReadInputTokens": 300,
                    "cacheWriteInputTokens": 40
                },
                "stopReason": "end_turn"
            }"#,
        )
        .unwrap();

        let usage = event.token_usage.unwrap();
        assert_eq!(usage.uncached_input_tokens, 101);
        assert_eq!(usage.output_tokens, 23);
        assert_eq!(usage.cache_read_input_tokens, 300);
        assert_eq!(usage.cache_write_input_tokens, 40);
        assert_eq!(usage.total_input_tokens(), 441);
    }

    /// 只有 stopReason 的 metadataEvent 不等于「用量为 0」，必须是 None，
    /// 否则调用方会把它当真值、把本次请求的 usage 记成 0。
    #[test]
    fn metadata_without_token_usage_is_not_treated_as_zero_truth() {
        let event: MetadataEvent = serde_json::from_str(r#"{"stopReason":"end_turn"}"#).unwrap();
        assert!(event.token_usage.is_none());
    }

    #[test]
    fn token_usage_with_missing_fields_defaults_only_missing_fields_to_zero() {
        let event: MetadataEvent =
            serde_json::from_str(r#"{"tokenUsage":{"outputTokens":9}}"#).unwrap();

        assert_eq!(
            event.token_usage,
            Some(TokenUsage {
                uncached_input_tokens: 0,
                output_tokens: 9,
                cache_read_input_tokens: 0,
                cache_write_input_tokens: 0,
            })
        );
    }

    #[test]
    fn sanitizes_negative_values() {
        let usage = TokenUsage {
            uncached_input_tokens: -1,
            output_tokens: -2,
            cache_read_input_tokens: -3,
            cache_write_input_tokens: -4,
        }
        .sanitized();

        assert_eq!(usage, TokenUsage::default());
    }

    #[test]
    fn adds_multiple_provider_calls_without_overflowing() {
        let first = TokenUsage {
            uncached_input_tokens: i32::MAX,
            output_tokens: 3,
            cache_read_input_tokens: 20,
            cache_write_input_tokens: 4,
        };
        let second = TokenUsage {
            uncached_input_tokens: 7,
            output_tokens: 5,
            cache_read_input_tokens: 11,
            cache_write_input_tokens: 2,
        };

        assert_eq!(
            first.saturating_add(second),
            TokenUsage {
                uncached_input_tokens: i32::MAX,
                output_tokens: 8,
                cache_read_input_tokens: 31,
                cache_write_input_tokens: 6,
            }
        );
    }

    #[test]
    fn subtracts_injected_tokens_across_input_buckets_only() {
        let usage = TokenUsage {
            uncached_input_tokens: 100,
            output_tokens: 23,
            cache_read_input_tokens: 300,
            cache_write_input_tokens: 40,
        };

        assert_eq!(
            usage.subtract_injected_input(125),
            TokenUsage {
                uncached_input_tokens: 100,
                output_tokens: 23,
                cache_read_input_tokens: 175,
                cache_write_input_tokens: 40,
            }
        );
    }

    #[test]
    fn injected_token_subtraction_keeps_one_real_input_token() {
        let usage = TokenUsage {
            uncached_input_tokens: 2,
            output_tokens: 7,
            cache_read_input_tokens: 3,
            cache_write_input_tokens: 4,
        };
        let adjusted = usage.subtract_injected_input(i32::MAX);

        assert_eq!(adjusted.total_input_tokens(), 1);
        assert_eq!(adjusted.output_tokens, 7);
        assert_eq!(adjusted.uncached_input_tokens, 1);
        assert_eq!(adjusted.cache_write_input_tokens, 0);
        assert_eq!(adjusted.cache_read_input_tokens, 0);
    }

    #[test]
    fn non_positive_injected_tokens_do_nothing() {
        let usage = TokenUsage {
            uncached_input_tokens: 5,
            output_tokens: 2,
            cache_read_input_tokens: 3,
            cache_write_input_tokens: 1,
        };
        assert_eq!(usage.subtract_injected_input(0), usage);
        assert_eq!(usage.subtract_injected_input(-10), usage);
    }

    /// 全零快照视同缺失：否则本次请求的 usage 会变成 0，记账凭空少一笔。
    #[test]
    fn all_zero_snapshot_is_treated_as_empty() {
        assert!(TokenUsage::default().is_empty());
        assert!(
            !TokenUsage {
                output_tokens: 1,
                ..Default::default()
            }
            .is_empty()
        );
    }
    /// 守住边界：`TokenUsage` 只承载 token 明细，不得混入 credit / cost。
    ///
    /// 上游 0.7.2 起把 meteringEvent 的 credit 透传进了 Anthropic/OpenAI 响应的
    /// usage 对象；本 fork 刻意不跟进——对外响应只输出 Anthropic 官方字段。
    /// credit 仅用于管理端内部统计，不进 API 响应体。这里用结构体自身的字段
    /// 集合固化该边界，避免后续移植上游时被顺手带回来。
    #[test]
    fn token_usage_carries_no_credit_fields() {
        let json = serde_json::to_string(&serde_json::json!({
            "uncachedInputTokens": 1,
            "outputTokens": 2,
            "cacheReadInputTokens": 3,
            "cacheWriteInputTokens": 4,
        }))
        .unwrap();
        // 反序列化再序列化，确认字段集合没有扩张出 credit 类字段
        let parsed: TokenUsage = serde_json::from_str(&json).unwrap();
        let out = serde_json::to_value(serde_json::json!({
            "uncached": parsed.uncached_input_tokens,
            "out": parsed.output_tokens,
        }))
        .unwrap();
        let text = out.to_string().to_ascii_lowercase();
        assert!(!text.contains("credit"), "usage 口径不得含 credit");
        assert!(!text.contains("cost"), "usage 口径不得含 cost");
    }
}
