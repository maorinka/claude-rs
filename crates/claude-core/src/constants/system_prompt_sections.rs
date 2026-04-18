//! System prompt section caching helper.
//!
//! Port of `src/constants/systemPromptSections.ts`. TS memoises each
//! section by name so recomputation between turns doesn't bust the
//! Claude prompt cache. The Rust port keeps the same API shape backed
//! by a process-wide HashMap.
//!
//! The `cache_break: true` variant is honoured the same way: each
//! resolve call recomputes and stores, so subsequent consumers of the
//! same section see the fresh value within the turn but the cache
//! layer below this one will detect the prefix change.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock, RwLock};

type ComputeFn = Arc<
    dyn Fn() -> Pin<Box<dyn Future<Output = Option<String>> + Send + Sync>>
        + Send
        + Sync,
>;

pub struct SystemPromptSection {
    pub name: String,
    pub compute: ComputeFn,
    pub cache_break: bool,
}

impl SystemPromptSection {
    /// Memoised section — computed once, cached until `clear_sections` fires.
    pub fn new<F, Fut>(name: impl Into<String>, f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<String>> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            compute: Arc::new(move || Box::pin(f())),
            cache_break: false,
        }
    }

    /// Volatile section — recomputed every turn. Busts the prompt cache
    /// when the value changes. Reason must be provided so reviewers can
    /// see why cache-breaking is acceptable.
    pub fn dangerous_uncached<F, Fut>(
        name: impl Into<String>,
        _reason: &str,
        f: F,
    ) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<String>> + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            compute: Arc::new(move || Box::pin(f())),
            cache_break: true,
        }
    }
}

// ── Process-wide cache ─────────────────────────────────────────────────────

static CACHE: OnceLock<Arc<RwLock<HashMap<String, Option<String>>>>> = OnceLock::new();

fn cache() -> Arc<RwLock<HashMap<String, Option<String>>>> {
    CACHE
        .get_or_init(|| Arc::new(RwLock::new(HashMap::new())))
        .clone()
}

/// Resolve every section, returning their final strings in order.
pub async fn resolve_sections(sections: &[SystemPromptSection]) -> Vec<Option<String>> {
    let mut out = Vec::with_capacity(sections.len());
    for s in sections {
        if !s.cache_break {
            if let Ok(guard) = cache().read() {
                if let Some(v) = guard.get(&s.name) {
                    out.push(v.clone());
                    continue;
                }
            }
        }
        let v = (s.compute)().await;
        if let Ok(mut guard) = cache().write() {
            guard.insert(s.name.clone(), v.clone());
        }
        out.push(v);
    }
    out
}

/// Clear all cached section values. Mirrors TS `clearSystemPromptSections`.
pub fn clear_sections() {
    if let Ok(mut guard) = cache().write() {
        guard.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Mutex;

    /// Cache is process-wide; serialise tests that mutate it.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    #[tokio::test]
    async fn memoises_non_cache_break_section() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_sections();
        static CALLS: AtomicU32 = AtomicU32::new(0);
        CALLS.store(0, Ordering::SeqCst);
        let s = SystemPromptSection::new("memoised", || async {
            CALLS.fetch_add(1, Ordering::SeqCst);
            Some("value".to_string())
        });
        let a = resolve_sections(std::slice::from_ref(&s)).await;
        let b = resolve_sections(std::slice::from_ref(&s)).await;
        assert_eq!(a, vec![Some("value".to_string())]);
        assert_eq!(b, vec![Some("value".to_string())]);
        assert_eq!(CALLS.load(Ordering::SeqCst), 1, "should have been called once");
        clear_sections();
    }

    #[tokio::test]
    async fn cache_break_recomputes() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_sections();
        static CALLS: AtomicU32 = AtomicU32::new(0);
        CALLS.store(0, Ordering::SeqCst);
        let s = SystemPromptSection::dangerous_uncached(
            "volatile",
            "test recomputation",
            || async {
                let n = CALLS.fetch_add(1, Ordering::SeqCst);
                Some(format!("iter {}", n))
            },
        );
        let a = resolve_sections(std::slice::from_ref(&s)).await;
        let b = resolve_sections(std::slice::from_ref(&s)).await;
        assert_ne!(a, b);
        assert_eq!(CALLS.load(Ordering::SeqCst), 2);
        clear_sections();
    }

    #[tokio::test]
    async fn clear_sections_drops_cache() {
        let _g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        clear_sections();
        static CALLS: AtomicU32 = AtomicU32::new(0);
        CALLS.store(0, Ordering::SeqCst);
        let s = SystemPromptSection::new("resettable", || async {
            CALLS.fetch_add(1, Ordering::SeqCst);
            Some("x".to_string())
        });
        let _ = resolve_sections(std::slice::from_ref(&s)).await;
        clear_sections();
        let _ = resolve_sections(std::slice::from_ref(&s)).await;
        assert_eq!(CALLS.load(Ordering::SeqCst), 2);
        clear_sections();
    }
}
