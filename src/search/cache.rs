//! LRU caching layer for performance optimization.
//!
//! Provides caching for:
//! - Query results (avoid re-searching)
//! - Skill embeddings (avoid re-computing)
//! - Session fingerprints (dedup suggestions)
//! - Negative route results (avoid re-evaluating no-match decisions)
//!
//! Cache sizes are configurable and default to reasonable limits that
//! balance memory usage with hit rates for typical workloads.

use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use lru::LruCache;

use crate::search::hybrid::HybridResult;

/// Default cache size for query results (number of queries)
const DEFAULT_QUERY_CACHE_SIZE: usize = 128;

/// Default cache size for embeddings (number of skills)
const DEFAULT_EMBEDDING_CACHE_SIZE: usize = 1024;

/// Default cache size for fingerprints (number of sessions)
const DEFAULT_FINGERPRINT_CACHE_SIZE: usize = 256;

/// Default cache size for negative route results
const DEFAULT_NEGATIVE_ROUTE_CACHE_SIZE: usize = 512;

/// Default TTL for negative route cache entries (300 seconds = 5 minutes)
const DEFAULT_NEGATIVE_ROUTE_TTL_SECS: u64 = 300;

/// Query result entry with metadata for cache management.
#[derive(Debug, Clone)]
pub struct CachedQueryResult {
    /// The hybrid search results
    pub results: Vec<HybridResult>,
    /// When this entry was cached
    pub cached_at: std::time::Instant,
    /// Number of times this entry was hit
    pub hit_count: u64,
}

/// Embedding cache entry.
#[derive(Debug, Clone)]
pub struct CachedEmbedding {
    /// The embedding vector
    pub embedding: Vec<f32>,
    /// Content hash used to generate this embedding
    pub content_hash: String,
}

/// Session fingerprint for deduplication.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct SessionFingerprint {
    /// Hash of significant session content
    pub content_hash: String,
    /// Key topics/keywords from the session
    pub keywords: Vec<String>,
}

/// Entry for negative route cache (route decision = no_match).
///
/// Caches the decision that a given task+cwd combination has no matching
/// route, avoiding redundant route evaluation.
#[derive(Debug, Clone)]
pub struct NegativeRouteEntry {
    /// The cache key hash
    pub key_hash: u64,
    /// When this entry was created (for TTL enforcement)
    pub created_at: std::time::Instant,
    /// Invalidation generation at creation time — entry is invalid if the
    /// current generation differs.
    pub generation: u64,
}

/// Cache key for negative route results.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct NegativeRouteKey {
    /// Normalized task text
    pub task_text: String,
    /// CWD fingerprint (filesystem path or hash)
    pub cwd_fingerprint: String,
}

impl NegativeRouteKey {
    /// Compute a 64-bit hash for use as the LRU cache key.
    pub fn hash(&self) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.task_text.hash(&mut hasher);
        self.cwd_fingerprint.hash(&mut hasher);
        hasher.finish()
    }
}

/// Thread-safe LRU caching layer for search operations.
///
/// Caches are protected by mutexes for concurrent access.
/// All cache operations are non-blocking (try-lock pattern).
pub struct CacheLayer {
    /// Query result cache (query hash -> results)
    query_cache: Mutex<LruCache<u64, CachedQueryResult>>,
    /// Embedding cache (skill ID -> embedding)
    embedding_cache: Mutex<LruCache<String, CachedEmbedding>>,
    /// Session fingerprint cache (session ID -> fingerprint)
    fingerprint_cache: Mutex<LruCache<String, SessionFingerprint>>,
    /// Negative route cache (route key hash -> entry)
    negative_route_cache: Mutex<LruCache<u64, NegativeRouteEntry>>,
    /// Cache statistics
    stats: Mutex<CacheStats>,
    /// Invalidation generation — bumped on provider sync or archive mutation.
    /// Negative route entries cached under a different generation are stale.
    invalidation_generation: AtomicU64,
    /// TTL for negative route cache entries in seconds.
    negative_route_ttl_secs: u64,
}

/// Cache statistics for monitoring and tuning.
#[derive(Debug, Clone, Default)]
pub struct CacheStats {
    /// Total query cache hits
    pub query_hits: u64,
    /// Total query cache misses
    pub query_misses: u64,
    /// Total embedding cache hits
    pub embedding_hits: u64,
    /// Total embedding cache misses
    pub embedding_misses: u64,
    /// Total fingerprint cache hits
    pub fingerprint_hits: u64,
    /// Total fingerprint cache misses
    pub fingerprint_misses: u64,
    /// Total negative route cache hits
    pub negative_route_hits: u64,
    /// Total negative route cache misses
    pub negative_route_misses: u64,
    /// Current invalidation generation
    pub invalidation_generation: u64,
}

impl CacheStats {
    /// Calculate query cache hit rate.
    #[must_use]
    pub fn query_hit_rate(&self) -> f64 {
        let total = self.query_hits + self.query_misses;
        if total == 0 {
            0.0
        } else {
            self.query_hits as f64 / total as f64
        }
    }

    /// Calculate embedding cache hit rate.
    #[must_use]
    pub fn embedding_hit_rate(&self) -> f64 {
        let total = self.embedding_hits + self.embedding_misses;
        if total == 0 {
            0.0
        } else {
            self.embedding_hits as f64 / total as f64
        }
    }

    /// Calculate fingerprint cache hit rate.
    #[must_use]
    pub fn fingerprint_hit_rate(&self) -> f64 {
        let total = self.fingerprint_hits + self.fingerprint_misses;
        if total == 0 {
            0.0
        } else {
            self.fingerprint_hits as f64 / total as f64
        }
    }

    /// Calculate negative route cache hit rate.
    #[must_use]
    pub fn negative_route_hit_rate(&self) -> f64 {
        let total = self.negative_route_hits + self.negative_route_misses;
        if total == 0 {
            0.0
        } else {
            self.negative_route_hits as f64 / total as f64
        }
    }
}

impl Default for CacheLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl CacheLayer {
    /// Create a new cache layer with default sizes.
    #[must_use]
    pub fn new() -> Self {
        Self::with_sizes(
            DEFAULT_QUERY_CACHE_SIZE,
            DEFAULT_EMBEDDING_CACHE_SIZE,
            DEFAULT_FINGERPRINT_CACHE_SIZE,
            DEFAULT_NEGATIVE_ROUTE_CACHE_SIZE,
        )
    }

    /// Create a new cache layer with custom sizes.
    #[must_use]
    pub fn with_sizes(
        query_size: usize,
        embedding_size: usize,
        fingerprint_size: usize,
        negative_route_size: usize,
    ) -> Self {
        Self {
            query_cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(query_size).unwrap_or(NonZeroUsize::new(1).unwrap()),
            )),
            embedding_cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(embedding_size).unwrap_or(NonZeroUsize::new(1).unwrap()),
            )),
            fingerprint_cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(fingerprint_size).unwrap_or(NonZeroUsize::new(1).unwrap()),
            )),
            negative_route_cache: Mutex::new(LruCache::new(
                NonZeroUsize::new(negative_route_size).unwrap_or(NonZeroUsize::new(1).unwrap()),
            )),
            stats: Mutex::new(CacheStats::default()),
            invalidation_generation: AtomicU64::new(0),
            negative_route_ttl_secs: DEFAULT_NEGATIVE_ROUTE_TTL_SECS,
        }
    }

    /// Compute a hash key for a query string.
    fn query_hash(query: &str, limit: usize) -> u64 {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        query.hash(&mut hasher);
        limit.hash(&mut hasher);
        hasher.finish()
    }

    /// Get cached query results.
    ///
    /// Returns None if not cached or cache is locked.
    pub fn get_query(&self, query: &str, limit: usize) -> Option<Vec<HybridResult>> {
        let key = Self::query_hash(query, limit);
        let mut cache = self.query_cache.try_lock().ok()?;
        let mut stats = self.stats.try_lock().ok()?;

        if let Some(entry) = cache.get_mut(&key) {
            entry.hit_count += 1;
            stats.query_hits += 1;
            Some(entry.results.clone())
        } else {
            stats.query_misses += 1;
            None
        }
    }

    /// Cache query results.
    ///
    /// Silently fails if cache is locked.
    pub fn put_query(&self, query: &str, limit: usize, results: Vec<HybridResult>) {
        let key = Self::query_hash(query, limit);
        if let Ok(mut cache) = self.query_cache.try_lock() {
            cache.put(
                key,
                CachedQueryResult {
                    results,
                    cached_at: std::time::Instant::now(),
                    hit_count: 0,
                },
            );
        }
    }

    /// Get cached embedding for a skill.
    ///
    /// Returns None if not cached, hash mismatch, or cache is locked.
    pub fn get_embedding(&self, skill_id: &str, content_hash: &str) -> Option<Vec<f32>> {
        let mut cache = self.embedding_cache.try_lock().ok()?;
        let mut stats = self.stats.try_lock().ok()?;

        if let Some(entry) = cache.get(skill_id) {
            if entry.content_hash == content_hash {
                stats.embedding_hits += 1;
                return Some(entry.embedding.clone());
            }
        }
        stats.embedding_misses += 1;
        None
    }

    /// Cache embedding for a skill.
    ///
    /// Silently fails if cache is locked.
    pub fn put_embedding(&self, skill_id: &str, content_hash: &str, embedding: Vec<f32>) {
        if let Ok(mut cache) = self.embedding_cache.try_lock() {
            cache.put(
                skill_id.to_string(),
                CachedEmbedding {
                    embedding,
                    content_hash: content_hash.to_string(),
                },
            );
        }
    }

    /// Get cached fingerprint for a session.
    ///
    /// Returns None if not cached or cache is locked.
    pub fn get_fingerprint(&self, session_id: &str) -> Option<SessionFingerprint> {
        let mut cache = self.fingerprint_cache.try_lock().ok()?;
        let mut stats = self.stats.try_lock().ok()?;

        if let Some(fp) = cache.get(session_id) {
            stats.fingerprint_hits += 1;
            Some(fp.clone())
        } else {
            stats.fingerprint_misses += 1;
            None
        }
    }

    /// Cache fingerprint for a session.
    ///
    /// Silently fails if cache is locked.
    pub fn put_fingerprint(&self, session_id: &str, fingerprint: SessionFingerprint) {
        if let Ok(mut cache) = self.fingerprint_cache.try_lock() {
            cache.put(session_id.to_string(), fingerprint);
        }
    }

    // =========================================================================
    // Negative route cache
    // =========================================================================

    /// Check if a route decision is cached as `no_match`.
    ///
    /// Returns `true` if the cache has a valid (non-expired, same generation)
    /// entry for this key, meaning the route is known to be a no-match.
    pub fn get_negative_route(&self, key: &NegativeRouteKey) -> bool {
        let cache_key = key.hash();
        let mut cache = match self.negative_route_cache.try_lock() {
            Ok(c) => c,
            Err(_) => return false,
        };
        let mut stats = match self.stats.try_lock() {
            Ok(c) => c,
            Err(_) => return false,
        };

        if let Some(entry) = cache.get_mut(&cache_key) {
            // Check generation (invalidated by sync/archive mutation)
            let current_gen = self.invalidation_generation.load(Ordering::Relaxed);
            if entry.generation != current_gen {
                stats.negative_route_misses += 1;
                return false;
            }

            // Check TTL using full Duration precision so TTL=0 expires immediately
            // instead of waiting for the next whole-second boundary.
            if entry.created_at.elapsed()
                >= std::time::Duration::from_secs(self.negative_route_ttl_secs)
            {
                stats.negative_route_misses += 1;
                return false;
            }

            stats.negative_route_hits += 1;
            true
        } else {
            stats.negative_route_misses += 1;
            false
        }
    }

    /// Cache a route decision as `no_match`.
    pub fn put_negative_route(&self, key: &NegativeRouteKey) {
        let cache_key = key.hash();
        let generation = self.invalidation_generation.load(Ordering::Relaxed);

        if let Ok(mut cache) = self.negative_route_cache.try_lock() {
            cache.put(
                cache_key,
                NegativeRouteEntry {
                    key_hash: cache_key,
                    created_at: std::time::Instant::now(),
                    generation,
                },
            );
        }
    }

    /// Invalidate all negative route cache entries.
    ///
    /// Call this after any provider sync or archive mutation to ensure
    /// stale route decisions are re-evaluated.
    pub fn invalidate_negative_routes(&self) {
        self.invalidation_generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Get the current invalidation generation.
    pub fn invalidation_generation(&self) -> u64 {
        self.invalidation_generation.load(Ordering::Relaxed)
    }

    /// Set a custom TTL for negative route entries (in seconds).
    pub fn set_negative_route_ttl(&mut self, ttl_secs: u64) {
        self.negative_route_ttl_secs = ttl_secs;
    }

    /// Get current cache statistics.
    pub fn stats(&self) -> CacheStats {
        let r#gen = self.invalidation_generation.load(Ordering::Relaxed);
        let mut s = self.stats.try_lock().map(|s| s.clone()).unwrap_or_default();
        s.invalidation_generation = r#gen;
        s
    }

    /// Clear all caches.
    pub fn clear(&self) {
        if let Ok(mut cache) = self.query_cache.try_lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.embedding_cache.try_lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.fingerprint_cache.try_lock() {
            cache.clear();
        }
        if let Ok(mut cache) = self.negative_route_cache.try_lock() {
            cache.clear();
        }
        if let Ok(mut stats) = self.stats.try_lock() {
            *stats = CacheStats::default();
        }
    }

    /// Get the current number of entries in each cache.
    pub fn sizes(&self) -> (usize, usize, usize, usize) {
        let query = self.query_cache.try_lock().map(|c| c.len()).unwrap_or(0);
        let embedding = self
            .embedding_cache
            .try_lock()
            .map(|c| c.len())
            .unwrap_or(0);
        let fingerprint = self
            .fingerprint_cache
            .try_lock()
            .map(|c| c.len())
            .unwrap_or(0);
        let negative_route = self
            .negative_route_cache
            .try_lock()
            .map(|c| c.len())
            .unwrap_or(0);
        (query, embedding, fingerprint, negative_route)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_cache_basic() {
        let cache = CacheLayer::new();

        // Miss on empty cache
        assert!(cache.get_query("test query", 10).is_none());

        // Put and get
        let results = vec![HybridResult {
            skill_id: "skill-1".to_string(),
            score: 0.95,
            bm25_rank: Some(1),
            semantic_rank: Some(1),
            bm25_score: Some(0.9),
            semantic_score: Some(0.85),
        }];
        cache.put_query("test query", 10, results.clone());

        let cached = cache.get_query("test query", 10).unwrap();
        assert_eq!(cached.len(), 1);
        assert_eq!(cached[0].skill_id, "skill-1");
    }

    #[test]
    fn test_query_cache_different_limits() {
        let cache = CacheLayer::new();

        let results_10 = vec![HybridResult {
            skill_id: "skill-1".to_string(),
            score: 0.9,
            bm25_rank: Some(1),
            semantic_rank: Some(1),
            bm25_score: Some(0.85),
            semantic_score: Some(0.8),
        }];
        let results_20 = vec![
            HybridResult {
                skill_id: "skill-1".to_string(),
                score: 0.9,
                bm25_rank: Some(1),
                semantic_rank: Some(1),
                bm25_score: Some(0.85),
                semantic_score: Some(0.8),
            },
            HybridResult {
                skill_id: "skill-2".to_string(),
                score: 0.8,
                bm25_rank: Some(2),
                semantic_rank: Some(2),
                bm25_score: Some(0.75),
                semantic_score: Some(0.7),
            },
        ];

        cache.put_query("test", 10, results_10);
        cache.put_query("test", 20, results_20);

        // Different limits should return different results
        assert_eq!(cache.get_query("test", 10).unwrap().len(), 1);
        assert_eq!(cache.get_query("test", 20).unwrap().len(), 2);
    }

    #[test]
    fn test_embedding_cache_basic() {
        let cache = CacheLayer::new();

        // Miss on empty cache
        assert!(cache.get_embedding("skill-1", "hash1").is_none());

        // Put and get
        let embedding = vec![0.1, 0.2, 0.3];
        cache.put_embedding("skill-1", "hash1", embedding.clone());

        let cached = cache.get_embedding("skill-1", "hash1").unwrap();
        assert_eq!(cached, embedding);
    }

    #[test]
    fn test_embedding_cache_hash_invalidation() {
        let cache = CacheLayer::new();

        let embedding = vec![0.1, 0.2, 0.3];
        cache.put_embedding("skill-1", "hash1", embedding);

        // Different hash should miss
        assert!(cache.get_embedding("skill-1", "hash2").is_none());
    }

    #[test]
    fn test_fingerprint_cache_basic() {
        let cache = CacheLayer::new();

        // Miss on empty cache
        assert!(cache.get_fingerprint("session-1").is_none());

        // Put and get
        let fp = SessionFingerprint {
            content_hash: "abc123".to_string(),
            keywords: vec!["rust".to_string(), "async".to_string()],
        };
        cache.put_fingerprint("session-1", fp.clone());

        let cached = cache.get_fingerprint("session-1").unwrap();
        assert_eq!(cached.content_hash, "abc123");
        assert_eq!(cached.keywords.len(), 2);
    }

    #[test]
    fn test_cache_stats() {
        let cache = CacheLayer::new();

        // Generate some hits and misses
        cache.get_query("miss1", 10);
        cache.get_query("miss2", 10);
        cache.put_query("hit", 10, vec![]);
        cache.get_query("hit", 10);
        cache.get_query("hit", 10);

        let stats = cache.stats();
        assert_eq!(stats.query_misses, 2);
        assert_eq!(stats.query_hits, 2);
        assert!((stats.query_hit_rate() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_cache_clear() {
        let cache = CacheLayer::new();

        cache.put_query("test", 10, vec![]);
        cache.put_embedding("skill-1", "hash", vec![0.1]);
        cache.put_fingerprint(
            "session-1",
            SessionFingerprint {
                content_hash: "x".to_string(),
                keywords: vec![],
            },
        );

        let key = NegativeRouteKey {
            task_text: "test".to_string(),
            cwd_fingerprint: "/test".to_string(),
        };
        cache.put_negative_route(&key);

        let (q, e, f, n) = cache.sizes();
        assert_eq!((q, e, f, n), (1, 1, 1, 1));

        cache.clear();

        let (q, e, f, n) = cache.sizes();
        assert_eq!((q, e, f, n), (0, 0, 0, 0));
    }

    #[test]
    fn test_cache_lru_eviction() {
        // Small cache to test eviction
        let cache = CacheLayer::with_sizes(2, 2, 2, 2);

        // Fill cache
        cache.put_query("q1", 10, vec![]);
        cache.put_query("q2", 10, vec![]);

        // Both should be present
        assert!(cache.get_query("q1", 10).is_some());
        assert!(cache.get_query("q2", 10).is_some());

        // Add third entry, should evict q1 (least recently used after we accessed q2)
        cache.put_query("q3", 10, vec![]);

        // q2 was accessed more recently, q1 should be evicted
        assert!(cache.get_query("q1", 10).is_none());
        assert!(cache.get_query("q2", 10).is_some());
        assert!(cache.get_query("q3", 10).is_some());
    }

    #[test]
    fn test_default_cache_sizes() {
        let cache = CacheLayer::default();
        // Just verify it creates without panicking
        assert!(cache.get_query("test", 10).is_none());
    }

    // =========================================================================
    // Negative route cache tests
    // =========================================================================

    #[test]
    fn test_negative_route_basic() {
        let cache = CacheLayer::new();

        let key = NegativeRouteKey {
            task_text: "build the project".to_string(),
            cwd_fingerprint: "/home/user/project".to_string(),
        };

        // Miss on empty cache
        assert!(!cache.get_negative_route(&key));

        // Put and get
        cache.put_negative_route(&key);
        assert!(cache.get_negative_route(&key));
    }

    #[test]
    fn test_negative_route_different_key() {
        let cache = CacheLayer::new();

        let key1 = NegativeRouteKey {
            task_text: "build the project".to_string(),
            cwd_fingerprint: "/home/user/project".to_string(),
        };
        let key2 = NegativeRouteKey {
            task_text: "build the project".to_string(),
            cwd_fingerprint: "/home/user/other".to_string(),
        };

        cache.put_negative_route(&key1);
        assert!(cache.get_negative_route(&key1));
        assert!(!cache.get_negative_route(&key2));
    }

    #[test]
    fn test_negative_route_invalidation() {
        let cache = CacheLayer::new();

        let key = NegativeRouteKey {
            task_text: "test task".to_string(),
            cwd_fingerprint: "/test".to_string(),
        };

        cache.put_negative_route(&key);
        assert!(cache.get_negative_route(&key));

        // Invalidate all negative routes
        cache.invalidate_negative_routes();
        assert!(!cache.get_negative_route(&key));
    }

    #[test]
    fn test_negative_route_ttl() {
        let mut cache = CacheLayer::new();
        cache.set_negative_route_ttl(0); // Immediate expiry

        let key = NegativeRouteKey {
            task_text: "test task".to_string(),
            cwd_fingerprint: "/test".to_string(),
        };

        cache.put_negative_route(&key);
        // Should miss because TTL is 0 (immediate expiry)
        // Note: this might be flaky, but with TTL=0 the entry expires immediately
        std::thread::sleep(std::time::Duration::from_millis(10));
        assert!(!cache.get_negative_route(&key));
    }

    #[test]
    fn test_negative_route_stats() {
        let cache = CacheLayer::new();

        let key = NegativeRouteKey {
            task_text: "task".to_string(),
            cwd_fingerprint: "/cwd".to_string(),
        };

        // Miss
        cache.get_negative_route(&key);
        let stats = cache.stats();
        assert_eq!(stats.negative_route_misses, 1);
        assert_eq!(stats.negative_route_hits, 0);

        // Hit
        cache.put_negative_route(&key);
        cache.get_negative_route(&key);
        let stats = cache.stats();
        assert_eq!(stats.negative_route_hits, 1);
        assert_eq!(stats.negative_route_misses, 1);
    }

    #[test]
    fn test_negative_route_invalidation_generation() {
        let cache = CacheLayer::new();
        let gen0 = cache.invalidation_generation();

        cache.invalidate_negative_routes();
        let gen1 = cache.invalidation_generation();
        assert_eq!(gen1, gen0 + 1);

        // Entry cached at gen0 should be stale after bump
        let key = NegativeRouteKey {
            task_text: "task".to_string(),
            cwd_fingerprint: "/cwd".to_string(),
        };
        cache.put_negative_route(&key);
        assert!(cache.get_negative_route(&key));

        cache.invalidate_negative_routes();
        assert!(!cache.get_negative_route(&key));
    }
}
