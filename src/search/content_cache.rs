use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Cache key for archive-backed load results.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LoadCacheKey {
    /// Canonical skill identifier.
    pub skill_id: String,
    /// Request scope such as `level:complete|deps:auto` or `section:checklist|deps:off`.
    pub cache_scope: String,
}

/// Cached load payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedLoad {
    /// Serialized `LoadResult` JSON payload.
    pub result_json: String,
}

/// Archive-backed content cache for resolved load results.
/// Stores serialized load results to avoid repeated parse/resolve/disclose work.
pub struct ContentCache {
    cache_dir: PathBuf,
}

impl ContentCache {
    /// Create a new content cache with the given cache directory.
    pub fn new(cache_dir: PathBuf) -> Self {
        Self { cache_dir }
    }

    /// Get a cached load payload for the given key and content hash.
    /// Returns None if not found or hash mismatch.
    pub fn get_load(&self, key: &LoadCacheKey, content_hash: &str) -> Option<CachedLoad> {
        let path = self.cache_path(key, content_hash);
        let contents = fs::read_to_string(&path).ok()?;
        serde_json::from_str(&contents).ok()
    }

    /// Store a load payload in cache.
    pub fn put_load(
        &self,
        key: &LoadCacheKey,
        content_hash: &str,
        payload: &CachedLoad,
    ) -> std::io::Result<()> {
        fs::create_dir_all(&self.cache_dir)?;

        let path = self.cache_path(key, content_hash);
        let json = serde_json::to_string(payload)
            .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
        fs::write(path, json)?;

        Ok(())
    }

    /// Invalidate all cache entries for a skill.
    pub fn invalidate(&self, skill_id: &str) -> std::io::Result<()> {
        if !self.cache_dir.exists() {
            return Ok(());
        }

        let prefix = format!("{}__", sanitize_component(skill_id));
        for entry in fs::read_dir(&self.cache_dir)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(&prefix))
            {
                let _ = fs::remove_file(entry.path());
            }
        }
        Ok(())
    }

    /// Invalidate all cache entries.
    pub fn invalidate_all(&self) -> std::io::Result<()> {
        if self.cache_dir.exists() {
            fs::remove_dir_all(&self.cache_dir)?;
        }
        fs::create_dir_all(&self.cache_dir)?;
        Ok(())
    }

    fn cache_path(&self, key: &LoadCacheKey, content_hash: &str) -> PathBuf {
        self.cache_dir.join(format!(
            "{}__{}__{}.json",
            sanitize_component(&key.skill_id),
            sanitize_component(&key.cache_scope),
            content_hash
        ))
    }
}

fn sanitize_component(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache_key(skill_id: &str, scope: &str) -> LoadCacheKey {
        LoadCacheKey {
            skill_id: skill_id.to_string(),
            cache_scope: scope.to_string(),
        }
    }

    #[test]
    fn test_cache_put_get() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = ContentCache::new(tmp.path().to_path_buf());
        let key = cache_key("claude/skill-1", "section:overview|deps:auto");

        let payload = CachedLoad {
            result_json: r#"{"skill_id":"claude/skill-1"}"#.to_string(),
        };

        cache.put_load(&key, "hash123", &payload).unwrap();

        let loaded = cache.get_load(&key, "hash123").unwrap();
        assert_eq!(loaded.result_json, payload.result_json);
    }

    #[test]
    fn test_cache_hash_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = ContentCache::new(tmp.path().to_path_buf());
        let key = cache_key("claude/skill-1", "section:overview|deps:auto");

        let payload = CachedLoad {
            result_json: r#"{"skill_id":"claude/skill-1"}"#.to_string(),
        };

        cache.put_load(&key, "hash123", &payload).unwrap();

        assert!(cache.get_load(&key, "wrong_hash").is_none());
    }

    #[test]
    fn test_invalidate_skill_prefix_only() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = ContentCache::new(tmp.path().to_path_buf());
        let key_a = cache_key("claude/skill-1", "section:overview|deps:auto");
        let key_b = cache_key("codex/skill-2", "section:overview|deps:auto");

        let payload = CachedLoad {
            result_json: r#"{"skill_id":"claude/skill-1"}"#.to_string(),
        };

        cache.put_load(&key_a, "hash123", &payload).unwrap();
        cache.put_load(&key_b, "hash456", &payload).unwrap();

        cache.invalidate("claude/skill-1").unwrap();
        assert!(cache.get_load(&key_a, "hash123").is_none());
        assert!(cache.get_load(&key_b, "hash456").is_some());
    }
}
