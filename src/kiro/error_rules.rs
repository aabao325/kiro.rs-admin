//! 自定义错误规则：按响应体关键词匹配，命中后自动处置凭据
//!
//! 上游会不打招呼地改报错文案、下架模型。内置的判定函数（`is_monthly_request_limit`
//! / `is_account_throttled` / ...）都是硬编码短语，改一次就要改代码重新发版。
//! 这里提供一层管理员可配的规则表，无需改代码即可对新出现的错误文案做出反应。
//!
//! 典型场景（用户实测）：模型被下架后上游返回
//! `400 {"message":"Invalid model ID. Please select a different model to continue.",
//! "reason":"INVALID_MODEL_ID"}`。原链路在 400 分支直接终止，既不计失败也不禁用，
//! 于是每次请求都打到同一个坏账号上。配一条关键词 `Invalid model ID` +
//! 动作 `disable` 即可让它自动退出候选。
//!
//! # 与内置判定的关系
//!
//! 规则在 provider 里**先于所有状态码分支**求值（必须先于 400 分支，否则
//! 400 类错误永远走不到规则）。命中即按规则动作处置并结束本轮；未命中则完全
//! 按既有链路走，行为不变。默认规则表为空，即默认不改变任何现有行为。

use std::path::PathBuf;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// 关键词组合方式
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MatchMode {
    /// 任一关键词命中即算命中（默认）
    #[default]
    Any,
    /// 所有关键词都必须出现
    All,
}

/// 命中后的处置动作
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RuleAction {
    /// 禁用该凭据并故障转移到下一个可用凭据（默认）
    #[default]
    Disable,
    /// 让该凭据进入临时冷却，到期自动恢复，期间故障转移
    Cooldown,
    /// 只累加失败计数，靠既有的连续失败阈值间接禁用
    CountFailure,
    /// 立即终止本次请求，不重试、不切换、不计失败
    Abort,
}

impl RuleAction {
    /// 用于日志与 trace 的稳定名称
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Cooldown => "cooldown",
            Self::CountFailure => "countFailure",
            Self::Abort => "abort",
        }
    }
}

/// 单条自定义错误规则
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ErrorRule {
    /// 规则名，用于日志、trace 与面板显示「因哪条规则被禁用」
    pub name: String,

    /// 是否启用。关闭后该条完全不参与匹配，便于临时停用而不删除配置。
    #[serde(default = "default_true")]
    pub enabled: bool,

    /// 要匹配的关键词。空列表视为永不命中（避免误配成匹配一切）。
    #[serde(default)]
    pub keywords: Vec<String>,

    /// 关键词组合方式
    #[serde(default)]
    pub match_mode: MatchMode,

    /// 关键词是否区分大小写。默认不区分 —— 上游文案的大小写并不稳定。
    #[serde(default)]
    pub case_sensitive: bool,

    /// 限定生效的 HTTP 状态码。空列表表示不限状态码。
    ///
    /// 建议尽量填上：仅凭关键词匹配容易被正常响应内容里偶然出现的同名短语误触，
    /// 叠加状态码约束能显著降低误判面。
    #[serde(default)]
    pub status_codes: Vec<u16>,

    /// 命中后的动作
    #[serde(default)]
    pub action: RuleAction,

    /// `action = cooldown` 时的冷却秒数。其它动作忽略此字段。
    #[serde(default = "default_cooldown_secs")]
    pub cooldown_secs: u64,

    /// 被本规则禁用的凭据是否参与自愈。
    ///
    /// 默认 `false`：规则针对的多是「不可能自行恢复」的状态（模型下架、账号封禁），
    /// 让它参与自愈只是反复无效重试。确实可能自愈的场景再打开。
    #[serde(default)]
    pub self_healable: bool,

    /// 执行 `disable` 前至少要保留的可用凭据数。
    ///
    /// `0`（默认）= 无防护，命中即禁用。设为 `N > 0` 时，若禁用后可用凭据会少于
    /// `N`，则降级为只计失败并输出告警 —— 用于避免「模型下架」这类错误把整个
    /// 凭据池逐个禁干净（该错误的根因是模型不可用，而非账号坏了）。
    #[serde(default)]
    pub min_available: u32,
}

fn default_true() -> bool {
    true
}

fn default_cooldown_secs() -> u64 {
    30 * 60
}

impl ErrorRule {
    /// 判断本规则是否命中给定响应。
    ///
    /// `status` 为 `None` 表示无 HTTP 状态可用（如网络层错误），此时带状态码
    /// 约束的规则一律不命中 —— 无法确认约束成立时不应放宽判定。
    pub fn matches(&self, status: Option<u16>, body: &str) -> bool {
        if !self.enabled || self.keywords.is_empty() {
            return false;
        }

        if !self.status_codes.is_empty() {
            match status {
                Some(code) if self.status_codes.contains(&code) => {}
                _ => return false,
            }
        }

        // 大小写不敏感时统一降为小写再比。只在需要时才分配。
        let haystack_lower;
        let haystack = if self.case_sensitive {
            body
        } else {
            haystack_lower = body.to_lowercase();
            &haystack_lower
        };

        let mut hit = |kw: &String| {
            let kw = kw.trim();
            if kw.is_empty() {
                return false;
            }
            if self.case_sensitive {
                haystack.contains(kw)
            } else {
                haystack.contains(&kw.to_lowercase())
            }
        };

        match self.match_mode {
            MatchMode::Any => self.keywords.iter().any(&mut hit),
            MatchMode::All => self.keywords.iter().all(&mut hit),
        }
    }
}

/// 规则表的运行时容器。
///
/// 与 [`crate::anthropic::cache_force`] 同构：`RwLock` 持有当前值，写入时同步
/// 落盘 `config.json`。读路径在每次请求失败时都会走到，因此用读锁而非 Mutex。
pub struct ErrorRuleStore {
    rules: RwLock<Vec<ErrorRule>>,
    config_path: Option<PathBuf>,
}

impl ErrorRuleStore {
    pub fn new(rules: Vec<ErrorRule>, config_path: Option<PathBuf>) -> Self {
        Self {
            rules: RwLock::new(rules),
            config_path,
        }
    }

    /// 当前规则表快照（Admin API 读取）
    pub fn snapshot(&self) -> Vec<ErrorRule> {
        self.rules.read().clone()
    }

    /// 是否存在任何启用的规则。
    ///
    /// 供 provider 在读取响应体之前做廉价短路：规则表为空（默认状态）时
    /// 完全不引入额外开销。
    pub fn has_enabled(&self) -> bool {
        self.rules.read().iter().any(|r| r.enabled)
    }

    /// 找出第一条命中的规则。按配置顺序求值，因此顺序即优先级。
    pub fn first_match(&self, status: Option<u16>, body: &str) -> Option<ErrorRule> {
        self.rules
            .read()
            .iter()
            .find(|rule| rule.matches(status, body))
            .cloned()
    }

    /// 整表替换并持久化（Admin API 写入）。返回生效后的规则表。
    pub fn replace(&self, rules: Vec<ErrorRule>) -> anyhow::Result<Vec<ErrorRule>> {
        let sanitized: Vec<ErrorRule> = rules.into_iter().map(sanitize_rule).collect();

        {
            let mut guard = self.rules.write();
            *guard = sanitized.clone();
        }

        if let Err(e) = self.persist(&sanitized) {
            tracing::warn!("自定义错误规则持久化失败（仅当前进程生效）: {}", e);
        }

        Ok(sanitized)
    }

    fn persist(&self, rules: &[ErrorRule]) -> anyhow::Result<()> {
        use anyhow::Context;

        let Some(path) = self.config_path.as_deref() else {
            return Ok(());
        };

        let mut config = crate::model::config::Config::load(path)
            .with_context(|| format!("重新加载配置失败: {}", path.display()))?;
        config.error_rules = rules.to_vec();
        config
            .save()
            .with_context(|| format!("写入配置失败: {}", path.display()))?;
        Ok(())
    }
}

/// 清理管理面板传入的规则：裁剪空白、去掉空关键词、约束数值范围。
fn sanitize_rule(mut rule: ErrorRule) -> ErrorRule {
    rule.name = rule.name.trim().to_string();
    if rule.name.is_empty() {
        rule.name = "未命名规则".to_string();
    }
    rule.keywords = rule
        .keywords
        .into_iter()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
        .collect();
    rule.status_codes.sort_unstable();
    rule.status_codes.dedup();
    // 冷却上限 24 小时，与账号级风控冷却保持同一量级约束
    rule.cooldown_secs = rule.cooldown_secs.clamp(1, 86_400);
    rule
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(keywords: &[&str]) -> ErrorRule {
        ErrorRule {
            name: "test".into(),
            enabled: true,
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            match_mode: MatchMode::Any,
            case_sensitive: false,
            status_codes: vec![],
            action: RuleAction::Disable,
            cooldown_secs: 60,
            self_healable: false,
            min_available: 0,
        }
    }

    /// 用户实测的模型下架报文必须被最朴素的一条规则命中。
    #[test]
    fn matches_real_invalid_model_id_body() {
        let body = r#"{"message":"Invalid model ID. Please select a different model to continue.","reason":"INVALID_MODEL_ID"}"#;
        assert!(rule(&["Invalid model ID"]).matches(Some(400), body));
        assert!(rule(&["INVALID_MODEL_ID"]).matches(Some(400), body));
    }

    /// 默认大小写不敏感：上游文案大小写不稳定，不该因此漏判。
    #[test]
    fn case_insensitive_by_default() {
        let body = r#"{"reason":"INVALID_MODEL_ID"}"#;
        assert!(rule(&["invalid_model_id"]).matches(None, body));

        let mut sensitive = rule(&["invalid_model_id"]);
        sensitive.case_sensitive = true;
        assert!(
            !sensitive.matches(None, body),
            "开启大小写敏感后不应命中"
        );
    }

    /// 状态码约束用于压低误判面。
    #[test]
    fn status_codes_narrow_the_match() {
        let mut r = rule(&["Invalid model ID"]);
        r.status_codes = vec![400];
        let body = "Invalid model ID";

        assert!(r.matches(Some(400), body));
        assert!(!r.matches(Some(500), body), "状态码不符不应命中");
        assert!(
            !r.matches(None, body),
            "无状态码可用时，带状态码约束的规则不应命中"
        );
    }

    /// `all` 模式要求全部关键词出现，用于表达高特异组合
    /// （如账号封禁需同时含 suspended 与 locked your account）。
    #[test]
    fn all_mode_requires_every_keyword() {
        let mut r = rule(&["suspended", "locked your account"]);
        r.match_mode = MatchMode::All;

        assert!(r.matches(None, "your account is suspended and we locked your account"));
        assert!(
            !r.matches(None, "your account is suspended"),
            "缺一个关键词不应命中"
        );
    }

    /// 空关键词表绝不能匹配一切 —— 那会让一条误配的规则禁掉所有凭据。
    #[test]
    fn empty_keywords_never_match() {
        assert!(!rule(&[]).matches(Some(400), "anything at all"));
        // 全是空白的关键词等价于空表
        let mut blank = rule(&["   "]);
        assert!(!blank.matches(Some(400), "anything"));
        blank.match_mode = MatchMode::All;
        assert!(!blank.matches(Some(400), "anything"));
    }

    /// 停用的规则不参与匹配。
    #[test]
    fn disabled_rule_never_matches() {
        let mut r = rule(&["Invalid model ID"]);
        r.enabled = false;
        assert!(!r.matches(Some(400), "Invalid model ID"));
    }

    /// 规则顺序即优先级，`first_match` 取最靠前的一条。
    #[test]
    fn first_match_follows_config_order() {
        let mut first = rule(&["Invalid model ID"]);
        first.name = "模型下架".into();
        let mut second = rule(&["Invalid"]);
        second.name = "宽泛兜底".into();

        let store = ErrorRuleStore::new(vec![first, second], None);
        let hit = store.first_match(Some(400), "Invalid model ID").unwrap();
        assert_eq!(hit.name, "模型下架");
    }

    /// 默认规则表为空 → 不改变任何现有行为。
    #[test]
    fn empty_store_has_no_enabled_rules() {
        let store = ErrorRuleStore::new(vec![], None);
        assert!(!store.has_enabled());
        assert!(store.first_match(Some(400), "Invalid model ID").is_none());
    }

    /// 写入时清理：裁空白、丢空关键词、状态码去重排序、冷却值收敛到合法区间。
    #[test]
    fn replace_sanitizes_input() {
        let store = ErrorRuleStore::new(vec![], None);
        let dirty = ErrorRule {
            name: "  ".into(),
            enabled: true,
            keywords: vec!["  Invalid model ID  ".into(), "   ".into()],
            match_mode: MatchMode::Any,
            case_sensitive: false,
            status_codes: vec![400, 400, 402],
            action: RuleAction::Cooldown,
            cooldown_secs: 999_999,
            self_healable: false,
            min_available: 0,
        };

        let saved = store.replace(vec![dirty]).unwrap();
        let r = &saved[0];
        assert_eq!(r.name, "未命名规则", "空名字应有兜底");
        assert_eq!(r.keywords, vec!["Invalid model ID".to_string()]);
        assert_eq!(r.status_codes, vec![400, 402], "状态码应去重并排序");
        assert_eq!(r.cooldown_secs, 86_400, "冷却值应收敛到上限");
        // 清理后仍应正常命中
        assert!(r.matches(Some(400), r#"{"reason":"invalid model id"}"#));
    }
}
