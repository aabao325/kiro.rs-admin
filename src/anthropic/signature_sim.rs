//! 响应内 message id 与 thinking signature 的模拟。
//!
//! Kiro 上游不是 Anthropic 服务端，既不会下发官方格式的 message id，也不会
//! 下发真实的 thinking signature（对应的密钥只有 Anthropic 自己有）。本模块
//! 生成"形状像真的"的替代值：
//!
//! - `generate_message_id()`：`msg_bdrk_` + 52 位随机小写字母数字，对齐用户
//!   提供的 AWS Bedrock 真实响应样例格式（本项目代理的 Kiro 上游本身就是
//!   AWS 体系，用这种形态比直连 Anthropic 的 `msg_01...` 更贴合项目定位）。
//! - `generate_thinking_signature()`：按对真实签名做 protobuf 解码后确认的
//!   字段位置拼字节（field 6 = model 名，field 7 = 0，field 8 = 字面量
//!   "thinking"，field 11 = 随机 id，结尾固定 `18 01`），中间和结尾补随机
//!   字节到与真实样例同量级的长度，整体标准 base64 编码。
//!
//!   这里**不**试图还原签名核心的加密内容——那部分是 Anthropic 用私钥生成的
//!   认证密文，没有私钥无论怎么"逆向"都不可能产出一个能被真官方服务器验证
//!   通过的签名（这是密码学问题，不是工作量问题）。生成的签名只保证格式、
//!   长度、可 base64 解码这些外部可观察特征跟真的一致；实际上也不需要更
//!   像——上游 Kiro 从不校验 signature，本服务的 converter 把历史消息转发
//!   回 Kiro 时只读 `thinking` 文本、完全不读 `signature` 字段。

use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use uuid::Uuid;

const ID_CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
/// 对齐真实 AWS Bedrock 样例里 `msg_bdrk_` 后缀的长度。
const BEDROCK_ID_LEN: usize = 52;

const WIRE_VARINT: u8 = 0;
const WIRE_LEN: u8 = 2;

/// 生成 `msg_bdrk_` 前缀的消息 id（AWS Bedrock 真实格式：前缀 + 52 位纯小写
/// 字母数字）。
pub fn generate_message_id() -> String {
    let suffix: String = (0..BEDROCK_ID_LEN)
        .map(|_| ID_CHARSET[fastrand::usize(..ID_CHARSET.len())] as char)
        .collect();
    format!("msg_bdrk_{suffix}")
}

/// 生成一段模拟的 thinking block signature（base64）。每次调用结果不同。
pub fn generate_thinking_signature(model: &str) -> String {
    BASE64.encode(build_signature_bytes(model))
}

fn build_signature_bytes(model: &str) -> Vec<u8> {
    let mut inner = Vec::new();

    // 1. 前导的不透明二进制块（对应真实样例里紧跟在最外层之后的一段随机数据）。
    write_len_delimited(&mut inner, 1, &random_bytes(fastrand::usize(90..150)));

    // 2. field 6 = model 名
    write_len_delimited(&mut inner, 6, model.as_bytes());

    // 3. field 7 = 0（varint）
    write_tag(&mut inner, 7, WIRE_VARINT);
    write_varint(&mut inner, 0);

    // 4. field 8 = 字面量 "thinking"
    write_len_delimited(&mut inner, 8, b"thinking");

    // 5. field 11 = 随机 id（真实样例里这里是 UUID 或纯数字串，形态不固定，
    //    用 UUID v4 足够"像"）
    let id = Uuid::new_v4().to_string();
    write_len_delimited(&mut inner, 11, id.as_bytes());

    // 6. 紧跟在 id 字段后的一段固定 12 字节随机数据（两条真实样例里都有）
    write_len_delimited(&mut inner, 2, &random_bytes(12));

    // 7. 尾部不透明二进制块，把总长度补到与真实样例同量级（300~500 字节）
    let target_total = fastrand::usize(320..480);
    // outer 头部（field2 tag+len）+ inner 已有内容 + 本字段自身 tag/len 的估算开销
    let overhead_estimate = 8;
    let tail_len = target_total
        .saturating_sub(inner.len() + overhead_estimate)
        .max(60);
    write_len_delimited(&mut inner, 9, &random_bytes(tail_len));

    // 最外层：field 2 包住上面全部内容，随后跟固定的 field 3 = 1（两条真实
    // 样例结尾都是 `18 01`）。
    let mut outer = Vec::new();
    write_len_delimited(&mut outer, 2, &inner);
    write_tag(&mut outer, 3, WIRE_VARINT);
    write_varint(&mut outer, 1);
    outer
}

fn random_bytes(len: usize) -> Vec<u8> {
    (0..len).map(|_| fastrand::u8(..)).collect()
}

fn write_tag(buf: &mut Vec<u8>, field_number: u32, wire_type: u8) {
    let tag = (field_number << 3) | (wire_type as u32);
    write_varint(buf, tag as u64);
}

fn write_varint(buf: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        buf.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn write_len_delimited(buf: &mut Vec<u8>, field_number: u32, data: &[u8]) {
    write_tag(buf, field_number, WIRE_LEN);
    write_varint(buf, data.len() as u64);
    buf.extend_from_slice(data);
}

/// 从 protobuf varint 里读出值和消费的字节数，供测试解析用。
fn read_varint(data: &[u8]) -> Option<(u64, usize)> {
    let mut value: u64 = 0;
    let mut shift = 0u32;
    for (i, &b) in data.iter().enumerate() {
        value |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            return Some((value, i + 1));
        }
        shift += 7;
        if shift >= 64 {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_id_format() {
        for _ in 0..20 {
            let id = generate_message_id();
            assert!(id.starts_with("msg_bdrk_"));
            let suffix = &id["msg_bdrk_".len()..];
            assert_eq!(suffix.len(), BEDROCK_ID_LEN);
            assert!(
                suffix.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit()),
                "id 后缀必须是纯小写字母数字: {suffix}"
            );
        }
    }

    #[test]
    fn message_id_is_random_each_call() {
        let a = generate_message_id();
        let b = generate_message_id();
        assert_ne!(a, b);
    }

    #[test]
    fn signature_is_valid_base64_and_reasonable_length() {
        for _ in 0..20 {
            let sig = generate_thinking_signature("claude-opus-4-8");
            let raw = BASE64.decode(&sig).expect("必须是合法 base64");
            // 对齐两条真实样例（376 / 490 字节）的量级。
            assert!(
                raw.len() >= 300 && raw.len() <= 600,
                "签名解码后长度 {} 不在预期区间",
                raw.len()
            );
        }
    }

    #[test]
    fn signature_is_random_each_call() {
        let a = generate_thinking_signature("claude-opus-4-8");
        let b = generate_thinking_signature("claude-opus-4-8");
        assert_ne!(a, b);
    }

    #[test]
    fn signature_ends_with_field3_eq_1() {
        let sig = generate_thinking_signature("claude-opus-4-8");
        let raw = BASE64.decode(&sig).unwrap();
        // 最后两字节固定是 field3(varint)=1 → tag 0x18, value 0x01
        assert_eq!(&raw[raw.len() - 2..], &[0x18, 0x01]);
    }

    #[test]
    fn signature_embeds_model_and_thinking_marker() {
        let sig = generate_thinking_signature("claude-opus-4-8");
        let raw = BASE64.decode(&sig).unwrap();

        // 手动解析：field2(outer) -> 里面顺序找 field6(model) 和 field8("thinking")
        let (_tag, consumed) = read_varint(&raw).unwrap();
        let mut pos = consumed;
        let (inner_len, consumed) = read_varint(&raw[pos..]).unwrap();
        pos += consumed;
        let inner = &raw[pos..pos + inner_len as usize];

        let model_needle = b"claude-opus-4-8";
        assert!(
            inner
                .windows(model_needle.len())
                .any(|w| w == model_needle),
            "签名里必须能找到 model 名明文"
        );
        assert!(
            inner.windows(b"thinking".len()).any(|w| w == b"thinking"),
            "签名里必须能找到 \"thinking\" 标记明文"
        );
    }
}
