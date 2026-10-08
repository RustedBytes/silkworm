use std::collections::{HashSet, VecDeque};
use std::sync::Arc;

// Sharing the bounded cache's key between membership and FIFO avoids copying
// the fingerprint. Unbounded caches have only one owner and retain Box<str>.
pub(super) enum SeenRequests {
    Bounded {
        entries: HashSet<Arc<str>>,
        order: VecDeque<Arc<str>>,
        max_entries: usize,
    },
    Unbounded(HashSet<Box<str>>),
}

impl SeenRequests {
    pub(super) fn new(max_entries: Option<usize>) -> Self {
        match max_entries {
            Some(max_entries) => Self::Bounded {
                entries: HashSet::new(),
                order: VecDeque::new(),
                max_entries,
            },
            None => Self::Unbounded(HashSet::new()),
        }
    }

    pub(super) fn contains(&self, fingerprint: &str) -> bool {
        match self {
            Self::Bounded { entries, .. } => entries.contains(fingerprint),
            Self::Unbounded(entries) => entries.contains(fingerprint),
        }
    }

    pub(super) fn insert_if_new(&mut self, fingerprint: &str) -> bool {
        match self {
            Self::Bounded {
                entries,
                order,
                max_entries,
            } => {
                if entries.contains(fingerprint) {
                    return false;
                }
                let shared: Arc<str> = Arc::from(fingerprint);
                while entries.len() >= *max_entries {
                    let Some(oldest) = order.pop_front() else {
                        break;
                    };
                    entries.remove(oldest.as_ref());
                }
                order.push_back(shared.clone());
                entries.insert(shared)
            }
            Self::Unbounded(entries) => {
                if entries.contains(fingerprint) {
                    return false;
                }
                entries.insert(fingerprint.into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SeenRequests;

    #[test]
    fn duplicate_hits_do_not_refresh_fifo_order() {
        let mut seen = SeenRequests::new(Some(2));
        assert!(seen.insert_if_new("a"));
        assert!(seen.insert_if_new("b"));
        assert!(!seen.insert_if_new("a"));
        assert!(seen.insert_if_new("c"));
        assert!(!seen.contains("a"));
        assert!(seen.contains("b"));
        assert!(seen.contains("c"));
        assert!(seen.insert_if_new("a"));
        assert!(!seen.contains("b"));
    }

    #[test]
    fn single_entry_cache_accepts_evicted_fingerprints_again() {
        let mut seen = SeenRequests::new(Some(1));
        for fingerprint in ["first", "second", "first"] {
            assert!(seen.insert_if_new(fingerprint));
            assert!(!seen.insert_if_new(fingerprint));
        }
        assert!(seen.contains("first"));
        assert!(!seen.contains("second"));
    }

    #[test]
    fn bounded_and_unbounded_keys_preserve_exact_utf8_and_empty_values() {
        for limit in [Some(5), None] {
            let mut seen = SeenRequests::new(limit);
            for fingerprint in ["", "GET https://приклад.укр/ї", "é", "e\u{301}", "雪"]
            {
                assert!(seen.insert_if_new(fingerprint));
                assert!(seen.contains(fingerprint));
                assert!(!seen.insert_if_new(fingerprint));
            }
            assert!(!seen.contains("GET https://приклад.укр/і"));
        }
    }

    #[test]
    fn unbounded_cache_retains_earlier_keys_after_growth() {
        let mut seen = SeenRequests::new(None);
        for i in 0..20_000 {
            assert!(seen.insert_if_new(&format!("GET /{i}")));
        }
        for i in [0, 1, 4096, 19_999] {
            assert!(seen.contains(&format!("GET /{i}")));
            assert!(!seen.insert_if_new(&format!("GET /{i}")));
        }
    }
}
