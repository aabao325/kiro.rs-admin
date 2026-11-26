//! `/v1/models` 的动态模型目录
//!
//! 以前 `/v1/models` 返回的是写死的静态清单，和凭据真实可用的模型脱节：上游
//! 上新 / 下架模型后，客户端拿到的列表就不再准确。这里把「当前所有启用凭据的
//! `ListAvailableModels` 并集」缓存起来，供 `/v1/models` 读取。
//!
//! # 刷新策略
//!
//! - 缓存有效期内直接返回，不产生任何上游请求；
//! - 过期后在**后台**刷新，本次请求仍返回旧快照（没有快照则回退静态清单），
//!   避免 `/v1/models` 被上游延迟拖住；
//! - 同一时刻只允许一个刷新任务在跑；
//! - 刷新失败时保留旧快照，并至少隔 [`RETRY_BACKOFF`] 才重试，避免上游故障
//!   期间每个请求都触发一轮聚合。

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::RwLock;

use crate::kiro::model::available_models::UpstreamModel;
use crate::kiro::provider::KiroProvider;

/// 聚合结果的有效期。凭据增删/续订不会主动失效，最多等这么久。
const TTL: Duration = Duration::from_secs(300);

/// 刷新失败后的重试间隔。
const RETRY_BACKOFF: Duration = Duration::from_secs(60);

#[derive(Default)]
struct CatalogState {
    /// 上一次成功聚合的结果
    models: Vec<UpstreamModel>,
    /// 上一次**成功**聚合的时间
    fetched_at: Option<Instant>,
    /// 上一次发起刷新的时间（无论成败），用于失败退避
    last_attempt: Option<Instant>,
    /// 是否有刷新任务在跑
    refreshing: bool,
}

impl CatalogState {
    /// 当前快照是否已经过期，需要（再次）刷新。
    fn needs_refresh(&self, now: Instant) -> bool {
        if self.refreshing {
            return false;
        }
        // 刚失败过：等退避窗口过去再试
        if let Some(last) = self.last_attempt {
            if now.duration_since(last) < RETRY_BACKOFF {
                return false;
            }
        }
        match self.fetched_at {
            Some(at) => now.duration_since(at) >= TTL,
            None => true,
        }
    }
}

/// 线程安全的模型目录。内部是 `Arc<RwLock<_>>`，克隆成本极低。
#[derive(Clone, Default)]
pub struct ModelsCatalog {
    inner: Arc<RwLock<CatalogState>>,
}

impl ModelsCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// 取当前快照，过期时顺带触发一次后台刷新。
    ///
    /// 返回可能为空（首次启动且刷新尚未完成），调用方需自行回退静态清单。
    pub fn snapshot_or_refresh(&self, provider: &Arc<KiroProvider>) -> Vec<UpstreamModel> {
        let now = Instant::now();
        let (cached, needs_refresh) = {
            let st = self.inner.read();
            (st.models.clone(), st.needs_refresh(now))
        };

        if needs_refresh {
            self.spawn_refresh(provider.clone());
        }

        cached
    }

    /// 丢弃缓存时间戳，下一次读取会触发刷新。
    pub fn invalidate(&self) {
        let mut st = self.inner.write();
        st.fetched_at = None;
        st.last_attempt = None;
    }

    fn spawn_refresh(&self, provider: Arc<KiroProvider>) {
        {
            let mut st = self.inner.write();
            if st.refreshing {
                return;
            }
            st.refreshing = true;
            st.last_attempt = Some(Instant::now());
        }

        let inner = self.inner.clone();
        tokio::spawn(async move {
            let models = provider.fetch_available_models_union().await;

            let mut st = inner.write();
            st.refreshing = false;
            if !models.is_empty() {
                // 上游下发的 maxInputTokens 才是该账号真实的上下文窗口，
                // 顺手覆盖给转换层的窗口表，静态表只作兜底。
                super::converter::set_upstream_context_windows(&models);
                st.models = models;
                st.fetched_at = Some(Instant::now());
            }
            // 空结果视为本轮失败：保留旧快照，靠 RETRY_BACKOFF 控制重试节奏。
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn upstream(id: &str) -> UpstreamModel {
        UpstreamModel {
            model_id: id.to_string(),
            model_name: None,
            description: None,
            token_limits: None,
        }
    }

    #[test]
    fn empty_catalog_needs_refresh() {
        let st = CatalogState::default();
        assert!(st.needs_refresh(Instant::now()));
    }

    #[test]
    fn fresh_catalog_does_not_refresh() {
        let st = CatalogState {
            models: vec![upstream("claude-opus-5-5")],
            fetched_at: Some(Instant::now()),
            last_attempt: Some(Instant::now()),
            refreshing: false,
        };
        assert!(!st.needs_refresh(Instant::now()));
    }

    #[test]
    fn stale_catalog_needs_refresh() {
        let st = CatalogState {
            models: vec![upstream("claude-opus-5-5")],
            fetched_at: Some(Instant::now() - TTL - Duration::from_secs(1)),
            last_attempt: Some(Instant::now() - TTL - Duration::from_secs(1)),
            refreshing: false,
        };
        assert!(st.needs_refresh(Instant::now()));
    }

    #[test]
    fn failed_refresh_backs_off() {
        let st = CatalogState {
            models: vec![],
            fetched_at: None,
            last_attempt: Some(Instant::now()),
            refreshing: false,
        };
        assert!(!st.needs_refresh(Instant::now()), "刚失败过应退避");
        assert!(
            st.needs_refresh(Instant::now() + RETRY_BACKOFF + Duration::from_secs(1)),
            "退避结束后应重试"
        );
    }

    #[test]
    fn refreshing_blocks_duplicate_refresh() {
        let st = CatalogState {
            models: vec![],
            fetched_at: None,
            last_attempt: None,
            refreshing: true,
        };
        assert!(!st.needs_refresh(Instant::now()));
    }

    #[test]
    fn invalidate_forces_next_read_to_refresh() {
        let catalog = ModelsCatalog::new();
        {
            let mut st = catalog.inner.write();
            st.models = vec![upstream("claude-opus-5-5")];
            st.fetched_at = Some(Instant::now());
        }
        {
            let st = catalog.inner.read();
            assert!(!st.needs_refresh(Instant::now()));
        }
        catalog.invalidate();
        let st = catalog.inner.read();
        assert!(st.needs_refresh(Instant::now()));
    }
}
