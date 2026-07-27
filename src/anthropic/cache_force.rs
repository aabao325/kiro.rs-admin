//! 缓存"强制覆盖"模式（管理面板可调）
//!
//! 跟 `cache_metering.rs`（哈希链智能模拟，跨轮命中真实前缀）并列存在、互不
//! 改动。这里提供第三种、管理员可控的行为：不管请求是否带 `cache_control`，
//! 直接按管理员设的三个比例把本次估算的 input token 总数拆成
//! `(input, cache_creation, cache_read)`。用于测试/演示——让客户端 UI 稳定
//! 看到"缓存生效"的数字，而不依赖真实的跨轮前缀命中。
//!
//! 算法与 `input_tokens` 恒 ≥ 1 的边界处理照搬自 kiro.rs 项目的
//! `src/model/cache_sim.rs`（同作者、已验证过的实现），只是包装成三档模式
//! （关闭 / 智能模拟 / 比例强制）而不是简单的开关。

use std::path::PathBuf;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

/// 面板总开关的三档模式。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CacheMode {
    /// 完全不注入缓存字段（模拟官方未使用 cache_control 时的响应）。
    Off,
    /// 现状不变：走 `cache_metering.rs` 的哈希链智能模拟。
    /// 默认档 —— 行为与引入本模块前完全一致。
    #[default]
    Auto,
    /// 不管请求有没有 cache_control，按下面三个比例强制拆分。
    Force,
}

/// 强制覆盖模式的可调设置（管理面板编辑，持久化）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CacheForceSettings {
    #[serde(default)]
    pub mode: CacheMode,
    /// cache_creation 占「可缓存基数」的比例 [0,1]
    #[serde(default = "default_creation_ratio")]
    pub creation_ratio: f64,
    /// cache_read 占「可缓存基数」的比例 [0,1]
    #[serde(default = "default_hit_ratio")]
    pub hit_ratio: f64,
    /// prompt 中算作「可缓存基数」的比例 [0,1]
    #[serde(default = "default_cacheable_ratio")]
    pub cacheable_ratio: f64,
}

fn default_creation_ratio() -> f64 {
    0.25
}
fn default_hit_ratio() -> f64 {
    0.70
}
fn default_cacheable_ratio() -> f64 {
    1.0
}

impl Default for CacheForceSettings {
    fn default() -> Self {
        Self {
            mode: CacheMode::default(),
            creation_ratio: default_creation_ratio(),
            hit_ratio: default_hit_ratio(),
            cacheable_ratio: default_cacheable_ratio(),
        }
    }
}

impl CacheForceSettings {
    /// 把三个比例 clamp 到 [0,1]，防止管理面板传入非法值。
    pub fn clamped(mut self) -> Self {
        self.creation_ratio = self.creation_ratio.clamp(0.0, 1.0);
        self.hit_ratio = self.hit_ratio.clamp(0.0, 1.0);
        self.cacheable_ratio = self.cacheable_ratio.clamp(0.0, 1.0);
        self
    }
}

/// `simulate()` 的拆分结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForcedCacheUsage {
    pub input_tokens: i32,
    pub cache_creation_input_tokens: i32,
    pub cache_read_input_tokens: i32,
}

/// 按 `Force` 模式的比例设置，对总输入 token 数 `total` 做强制拆分。
///
/// `total <= 0` 时返回 `None`（调用方保持原有 usage 不变）。
///
/// `input_tokens` 恒 `>= 1`：即便 `creation_ratio + hit_ratio` 配到接近/等于
/// 1.0，本轮请求实际发生的新增输入也不该被吞成 0——0 在 Anthropic 语义下意味
/// 着"这轮没有任何新输入"，与"发生了一次真实请求"矛盾。因此可分配给
/// creation+read 的上限固定为 `total - 1`。
pub fn simulate(settings: &CacheForceSettings, total: i32) -> Option<ForcedCacheUsage> {
    if total <= 0 {
        return None;
    }

    if total == 1 {
        return Some(ForcedCacheUsage {
            input_tokens: 1,
            cache_creation_input_tokens: 0,
            cache_read_input_tokens: 0,
        });
    }

    let cacheable_cap = total - 1;
    let total_f = total as f64;
    let base = (total_f * settings.cacheable_ratio)
        .round()
        .min(cacheable_cap as f64);
    let mut creation = (base * settings.creation_ratio).round();
    let mut read = (base * settings.hit_ratio).round();

    if creation + read > base {
        let sum = creation + read;
        if sum > 0.0 {
            creation = (creation / sum * base).round();
            read = (read / sum * base).round();
        }
    }

    let mut creation = creation as i32;
    let mut read = read as i32;

    if creation > cacheable_cap {
        creation = cacheable_cap;
    }
    if creation + read > cacheable_cap {
        read = (cacheable_cap - creation).max(0);
    }

    let input_tokens = (total - creation - read).max(1);

    Some(ForcedCacheUsage {
        input_tokens,
        cache_creation_input_tokens: creation,
        cache_read_input_tokens: read,
    })
}

/// 三种模式统一解析后的最终缓存拆分结果，供 handlers/stream 直接消费。
///
/// `ephemeral_5m_input_tokens` / `ephemeral_1h_input_tokens` 是
/// `cache_creation_input_tokens` 按本次请求探测到的最大 TTL 分桶后的结果
/// （二者之和恒等于 `cache_creation_input_tokens`），对齐官方
/// `usage.cache_creation.{ephemeral_5m_input_tokens,ephemeral_1h_input_tokens}`。
#[derive(Debug, Clone, Copy, Default)]
pub struct ResolvedCacheUsage {
    pub input_tokens: i32,
    pub cache_creation_input_tokens: i32,
    pub cache_read_input_tokens: i32,
    pub ephemeral_5m_input_tokens: i32,
    pub ephemeral_1h_input_tokens: i32,
}

/// 统一入口：按当前生效模式（`force_settings.mode`）把 `total` 拆成最终三段，
/// 再按 `ttl_secs` 把 `cache_creation` 分进 5m/1h 桶。
///
/// - `Off`：全部计入 `input_tokens`，缓存字段恒为 0。
/// - `Auto`：走 `auto_usage.split_against_total`（`cache_metering.rs` 现有的
///   哈希链智能模拟结果），行为与改造前完全一致。
/// - `Force`：走 `simulate()`，`total <= 0` 时退化为全部计入 input。
///
/// `ttl_secs` 由调用方在拿到真实 total 之前、`payload` 还未被闭包吃掉时
/// 用 `cache_metering::detect_max_ttl` 探测好并传入。
pub fn resolve(
    force_settings: &CacheForceSettings,
    auto_usage: &super::cache_metering::CacheUsage,
    total: i32,
    ttl_secs: i64,
) -> ResolvedCacheUsage {
    let total = total.max(0);
    let (input_tokens, cache_creation_input_tokens, cache_read_input_tokens) =
        match force_settings.mode {
            CacheMode::Off => (total, 0, 0),
            CacheMode::Auto => auto_usage.split_against_total(total),
            CacheMode::Force => match simulate(force_settings, total) {
                Some(u) => (
                    u.input_tokens,
                    u.cache_creation_input_tokens,
                    u.cache_read_input_tokens,
                ),
                None => (total, 0, 0),
            },
        };

    let (ephemeral_5m_input_tokens, ephemeral_1h_input_tokens) =
        if cache_creation_input_tokens > 0 {
            if ttl_secs > 300 {
                (0, cache_creation_input_tokens)
            } else {
                (cache_creation_input_tokens, 0)
            }
        } else {
            (0, 0)
        };

    ResolvedCacheUsage {
        input_tokens,
        cache_creation_input_tokens,
        cache_read_input_tokens,
        ephemeral_5m_input_tokens,
        ephemeral_1h_input_tokens,
    }
}

/// 共享、可持久化的强制覆盖设置存储。
///
/// anthropic 消息处理器（读）与 admin 服务（读写）共享同一个 `Arc`。
/// 持久化到 `cache_dir/cache_force.json`。
pub struct CacheForceStore {
    inner: RwLock<CacheForceSettings>,
    path: Option<PathBuf>,
}

pub type SharedCacheForceStore = Arc<CacheForceStore>;

impl CacheForceStore {
    /// 从文件加载（文件不存在或解析失败时使用默认设置：Auto 模式）。
    pub fn load(path: Option<PathBuf>) -> Arc<Self> {
        let settings = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|c| serde_json::from_str::<CacheForceSettings>(&c).ok())
            .map(|s| s.clamped())
            .unwrap_or_default();

        Arc::new(Self {
            inner: RwLock::new(settings),
            path,
        })
    }

    /// 读取当前设置快照
    pub fn snapshot(&self) -> CacheForceSettings {
        self.inner.read().clone()
    }

    /// 更新设置（clamp + 持久化），返回最终生效的设置。
    pub fn update(&self, new: CacheForceSettings) -> CacheForceSettings {
        let v = new.clamped();
        *self.inner.write() = v.clone();
        self.persist(&v);
        v
    }

    fn persist(&self, s: &CacheForceSettings) {
        let Some(path) = &self.path else {
            return;
        };
        match serde_json::to_string_pretty(s) {
            Ok(json) => {
                if let Err(e) = std::fs::write(path, json) {
                    tracing::warn!("保存 cache_force 设置失败: {}", e);
                }
            }
            Err(e) => tracing::warn!("序列化 cache_force 设置失败: {}", e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(mode: CacheMode, creation: f64, hit: f64, cacheable: f64) -> CacheForceSettings {
        CacheForceSettings {
            mode,
            creation_ratio: creation,
            hit_ratio: hit,
            cacheable_ratio: cacheable,
        }
    }

    #[test]
    fn zero_total_returns_none() {
        let s = settings(CacheMode::Force, 0.2, 0.7, 1.0);
        assert!(simulate(&s, 0).is_none());
    }

    #[test]
    fn basic_split() {
        let s = settings(CacheMode::Force, 0.2, 0.7, 1.0);
        let u = simulate(&s, 1000).unwrap();
        assert_eq!(u.cache_creation_input_tokens, 200);
        assert_eq!(u.cache_read_input_tokens, 699);
        assert_eq!(u.input_tokens, 101);
        assert_eq!(
            u.input_tokens + u.cache_creation_input_tokens + u.cache_read_input_tokens,
            1000
        );
    }

    #[test]
    fn input_tokens_never_zero_even_at_full_ratio() {
        let s = settings(CacheMode::Force, 0.5, 0.5, 1.0);
        for total in [1, 2, 10, 100, 1000, 100_000] {
            let u = simulate(&s, total).unwrap();
            assert!(u.input_tokens >= 1, "total={total} 时 input_tokens 不应小于 1");
            assert_eq!(
                u.input_tokens + u.cache_creation_input_tokens + u.cache_read_input_tokens,
                total
            );
        }
    }

    #[test]
    fn total_one_all_input_no_cache() {
        let s = settings(CacheMode::Force, 0.5, 0.5, 1.0);
        let u = simulate(&s, 1).unwrap();
        assert_eq!(u.input_tokens, 1);
        assert_eq!(u.cache_creation_input_tokens, 0);
        assert_eq!(u.cache_read_input_tokens, 0);
    }

    #[test]
    fn cacheable_ratio_caps_base() {
        let s = settings(CacheMode::Force, 0.2, 0.7, 0.5);
        let u = simulate(&s, 1000).unwrap();
        assert_eq!(u.cache_creation_input_tokens, 100);
        assert_eq!(u.cache_read_input_tokens, 350);
        assert_eq!(u.input_tokens, 550);
    }

    #[test]
    fn ratios_exceeding_one_are_scaled_to_base() {
        let s = settings(CacheMode::Force, 0.6, 0.9, 1.0);
        let u = simulate(&s, 1000).unwrap();
        assert!(u.input_tokens >= 1);
        assert!(u.cache_creation_input_tokens + u.cache_read_input_tokens <= 999);
    }

    #[test]
    fn clamped_rejects_out_of_range_ratios() {
        let s = CacheForceSettings {
            mode: CacheMode::Force,
            creation_ratio: 1.5,
            hit_ratio: -0.2,
            cacheable_ratio: 2.0,
        }
        .clamped();
        assert_eq!(s.creation_ratio, 1.0);
        assert_eq!(s.hit_ratio, 0.0);
        assert_eq!(s.cacheable_ratio, 1.0);
    }

    #[test]
    fn resolve_off_mode_all_input_zero_cache() {
        let s = settings(CacheMode::Off, 0.5, 0.5, 1.0);
        let auto = super::super::cache_metering::CacheUsage {
            cache_read: 100,
            cache_covered_est: 200,
            prompt_total_est: 400,
        };
        let r = resolve(&s, &auto, 1000, 300);
        assert_eq!(r.input_tokens, 1000);
        assert_eq!(r.cache_creation_input_tokens, 0);
        assert_eq!(r.cache_read_input_tokens, 0);
        assert_eq!(r.ephemeral_5m_input_tokens, 0);
        assert_eq!(r.ephemeral_1h_input_tokens, 0);
    }

    #[test]
    fn resolve_auto_mode_delegates_to_cache_usage() {
        let s = settings(CacheMode::Auto, 0.5, 0.5, 1.0);
        let auto = super::super::cache_metering::CacheUsage {
            cache_read: 100,
            cache_covered_est: 200,
            prompt_total_est: 400,
        };
        let (expect_input, expect_creation, expect_read) = auto.split_against_total(1000);
        let r = resolve(&s, &auto, 1000, 300);
        assert_eq!(r.input_tokens, expect_input);
        assert_eq!(r.cache_creation_input_tokens, expect_creation);
        assert_eq!(r.cache_read_input_tokens, expect_read);
        assert_eq!(
            r.ephemeral_5m_input_tokens + r.ephemeral_1h_input_tokens,
            expect_creation
        );
    }

    #[test]
    fn resolve_force_mode_uses_simulate() {
        let s = settings(CacheMode::Force, 0.2, 0.7, 1.0);
        let auto = super::super::cache_metering::CacheUsage::default();
        let r = resolve(&s, &auto, 1000, 300);
        assert_eq!(r.cache_creation_input_tokens, 200);
        assert_eq!(r.cache_read_input_tokens, 699);
        assert_eq!(r.input_tokens, 101);
        assert_eq!(r.ephemeral_5m_input_tokens, 200);
        assert_eq!(r.ephemeral_1h_input_tokens, 0);
    }

    #[test]
    fn resolve_force_mode_ttl_1h_buckets_creation_into_1h() {
        let s = settings(CacheMode::Force, 0.2, 0.7, 1.0);
        let auto = super::super::cache_metering::CacheUsage::default();
        let r = resolve(&s, &auto, 1000, 3600);
        assert_eq!(r.ephemeral_5m_input_tokens, 0);
        assert_eq!(r.ephemeral_1h_input_tokens, r.cache_creation_input_tokens);
    }

    #[test]
    fn resolve_zero_total_is_safe() {
        let s = settings(CacheMode::Force, 0.5, 0.5, 1.0);
        let auto = super::super::cache_metering::CacheUsage::default();
        let r = resolve(&s, &auto, 0, 300);
        assert_eq!(r.input_tokens, 0);
        assert_eq!(r.cache_creation_input_tokens, 0);
        assert_eq!(r.cache_read_input_tokens, 0);
    }

    #[test]
    fn store_persists_and_reloads() {
        let dir = std::env::temp_dir().join(format!("cache_force_test_{}", fastrand::u64(..)));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("cache_force.json");

        let store = CacheForceStore::load(Some(path.clone()));
        assert_eq!(store.snapshot().mode, CacheMode::Auto);

        store.update(CacheForceSettings {
            mode: CacheMode::Force,
            creation_ratio: 0.3,
            hit_ratio: 0.6,
            cacheable_ratio: 0.9,
        });

        let reloaded = CacheForceStore::load(Some(path));
        assert_eq!(reloaded.snapshot().mode, CacheMode::Force);
        assert_eq!(reloaded.snapshot().creation_ratio, 0.3);

        std::fs::remove_dir_all(&dir).ok();
    }
}
