//! 事件基础定义
//!
//! 定义事件类型枚举、trait 和统一事件结构

use crate::kiro::parser::error::{ParseError, ParseResult};
use crate::kiro::parser::frame::Frame;

/// 事件类型枚举
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventType {
    /// 助手响应事件
    AssistantResponse,
    /// 工具使用事件
    ToolUse,
    /// 计费事件
    Metering,
    /// 上下文使用率事件
    ContextUsage,
    /// 推理内容事件
    ReasoningContent,
    /// 未知事件类型
    Unknown,
}

impl EventType {
    /// 从事件类型字符串解析
    pub fn from_str(s: &str) -> Self {
        match s {
            "assistantResponseEvent" => Self::AssistantResponse,
            "toolUseEvent" => Self::ToolUse,
            "meteringEvent" => Self::Metering,
            "contextUsageEvent" => Self::ContextUsage,
            "reasoningContentEvent" => Self::ReasoningContent,
            _ => Self::Unknown,
        }
    }

    /// 转换为事件类型字符串
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AssistantResponse => "assistantResponseEvent",
            Self::ToolUse => "toolUseEvent",
            Self::Metering => "meteringEvent",
            Self::ContextUsage => "contextUsageEvent",
            Self::ReasoningContent => "reasoningContentEvent",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// 未知帧 payload 的日志片段上限（字节）。只为定位新事件类型的形状，
/// 不需要全文；过长会把日志刷爆。
const UNKNOWN_PAYLOAD_SNIPPET_MAX: usize = 512;

/// 按字符边界截断 payload，避免切碎 UTF-8。
fn truncate_snippet(payload: &str) -> String {
    let trimmed = payload.trim();
    if trimmed.len() <= UNKNOWN_PAYLOAD_SNIPPET_MAX {
        return trimmed.to_string();
    }
    let mut end = UNKNOWN_PAYLOAD_SNIPPET_MAX;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…(truncated)", &trimmed[..end])
}

/// 事件 payload trait
///
/// 所有具体事件类型都需要实现此 trait
pub trait EventPayload: Sized {
    /// 从帧解析事件负载
    fn from_frame(frame: &Frame) -> ParseResult<Self>;
}

/// 统一事件枚举
///
/// 封装所有可能的事件类型
#[derive(Debug, Clone)]
pub enum Event {
    /// 助手响应
    AssistantResponse(super::AssistantResponseEvent),
    /// 工具使用
    ToolUse(super::ToolUseEvent),
    /// 计费
    Metering(super::MeteringEvent),
    /// 上下文使用率
    ContextUsage(super::ContextUsageEvent),
    /// 推理内容
    ReasoningContent(super::ReasoningContentEvent),
    /// 未知事件：保留 `:event-type` 名与 payload 片段。
    ///
    /// 这两个字段是排查「流跑完了但什么都没发生」的唯一线索——上游一旦引入新
    /// 事件类型，没有类型名就只能看到「有个不认识的帧」，无从下手加解析。
    Unknown {
        /// 帧头 `:event-type` 的原值
        event_type: String,
        /// payload 文本片段（已截断，仅用于日志）
        payload_snippet: String,
    },
    /// 服务端错误
    Error {
        /// 错误代码
        error_code: String,
        /// 错误消息
        error_message: String,
    },
    /// 服务端异常
    Exception {
        /// 异常类型
        exception_type: String,
        /// 异常消息
        message: String,
    },
}

impl Event {
    /// 从帧解析事件
    pub fn from_frame(frame: Frame) -> ParseResult<Self> {
        let message_type = frame.message_type().unwrap_or("event");

        match message_type {
            "event" => Self::parse_event(frame),
            "error" => Self::parse_error(frame),
            "exception" => Self::parse_exception(frame),
            other => Err(ParseError::InvalidMessageType(other.to_string())),
        }
    }

    /// 解析事件类型消息
    fn parse_event(frame: Frame) -> ParseResult<Self> {
        let event_type_str = frame.event_type().unwrap_or("unknown");
        let event_type = EventType::from_str(event_type_str);

        match event_type {
            EventType::AssistantResponse => {
                let payload = super::AssistantResponseEvent::from_frame(&frame)?;
                Ok(Self::AssistantResponse(payload))
            }
            EventType::ToolUse => {
                let payload = super::ToolUseEvent::from_frame(&frame)?;
                Ok(Self::ToolUse(payload))
            }
            EventType::Metering => {
                let payload = super::MeteringEvent::from_frame(&frame)?;
                Ok(Self::Metering(payload))
            }
            EventType::ContextUsage => {
                let payload = super::ContextUsageEvent::from_frame(&frame)?;
                Ok(Self::ContextUsage(payload))
            }
            EventType::ReasoningContent => {
                let payload = super::ReasoningContentEvent::from_frame(&frame)?;
                Ok(Self::ReasoningContent(payload))
            }
            EventType::Unknown => Ok(Self::Unknown {
                event_type: event_type_str.to_string(),
                payload_snippet: truncate_snippet(&frame.payload_as_str()),
            }),
        }
    }

    /// 解析错误类型消息
    fn parse_error(frame: Frame) -> ParseResult<Self> {
        let error_code = frame
            .headers
            .error_code()
            .unwrap_or("UnknownError")
            .to_string();
        let error_message = frame.payload_as_str();

        Ok(Self::Error {
            error_code,
            error_message,
        })
    }

    /// 解析异常类型消息
    fn parse_exception(frame: Frame) -> ParseResult<Self> {
        let exception_type = frame
            .headers
            .exception_type()
            .unwrap_or("UnknownException")
            .to_string();
        let message = frame.payload_as_str();

        Ok(Self::Exception {
            exception_type,
            message,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_event_type_from_str() {
        assert_eq!(
            EventType::from_str("assistantResponseEvent"),
            EventType::AssistantResponse
        );
        assert_eq!(EventType::from_str("toolUseEvent"), EventType::ToolUse);
        assert_eq!(EventType::from_str("meteringEvent"), EventType::Metering);
        assert_eq!(
            EventType::from_str("contextUsageEvent"),
            EventType::ContextUsage
        );
        assert_eq!(
            EventType::from_str("reasoningContentEvent"),
            EventType::ReasoningContent
        );
        assert_eq!(EventType::from_str("unknown_type"), EventType::Unknown);
    }

    #[test]
    fn test_event_type_as_str() {
        assert_eq!(
            EventType::AssistantResponse.as_str(),
            "assistantResponseEvent"
        );
        assert_eq!(EventType::ToolUse.as_str(), "toolUseEvent");
    }

    #[test]
    fn truncate_snippet_respects_limit_and_trims() {
        assert_eq!(truncate_snippet("  hi  "), "hi");
        let long = "x".repeat(UNKNOWN_PAYLOAD_SNIPPET_MAX + 50);
        let out = truncate_snippet(&long);
        assert!(out.ends_with("…(truncated)"));
        assert!(out.len() <= UNKNOWN_PAYLOAD_SNIPPET_MAX + 20);
    }

    /// 未知帧必须带出 `:event-type` 名与 payload 片段——这是排查上游新增
    /// 事件类型的唯一线索。
    #[test]
    fn unknown_event_carries_type_name_and_payload() {
        use crate::kiro::parser::header::HeaderValue;

        let mut headers = crate::kiro::parser::header::Headers::new();
        headers.insert(
            ":message-type".to_string(),
            HeaderValue::String("event".to_string()),
        );
        headers.insert(
            ":event-type".to_string(),
            HeaderValue::String("brandNewEvent".to_string()),
        );
        let frame = Frame {
            headers,
            payload: br#"{"foo":"bar"}"#.to_vec(),
        };

        match Event::from_frame(frame).unwrap() {
            Event::Unknown {
                event_type,
                payload_snippet,
            } => {
                assert_eq!(event_type, "brandNewEvent");
                assert_eq!(payload_snippet, r#"{"foo":"bar"}"#);
            }
            other => panic!("应解析为 Unknown，实际: {:?}", other),
        }
    }
}
