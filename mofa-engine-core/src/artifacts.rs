//! Retention and cleanup for engine-generated artifacts.
//!
//! Backends that produce files (TTS audio, and later image/video output) write
//! them with a `mofa_` name prefix into one artifact directory — by default a
//! mofa-owned subdirectory of the system temp dir ([`default_artifact_dir`]),
//! never the shared temp dir itself. Left alone these accumulate, so the engine
//! runs a periodic sweep that deletes artifacts older than a configured
//! retention. Only files the engine created (matched by prefix) are ever
//! removed, and the mofa-owned default keeps even those deletions away from
//! other tenants of a world-writable shared temp dir.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Name prefix stamped on every engine-generated artifact.
pub(crate) const ARTIFACT_PREFIX: &str = "mofa_";

/// Directory the engine writes artifacts into when none is configured: a
/// mofa-OWNED subdirectory of the system temp dir — never the shared temp
/// dir itself. The [`ArtifactSweeper`] deletes by name prefix, and sweeping
/// a world-writable shared directory would happily remove another tenant's
/// (or another app's) `mofa_*` files. Writers and the sweeper resolve
/// through this one function so they always agree on the location.
pub(crate) fn default_artifact_dir() -> PathBuf {
    std::env::temp_dir().join("mofa_artifacts")
}

/// Resolve a configured artifact directory, falling back to
/// [`default_artifact_dir`] when unset (or blank).
pub(crate) fn resolve_artifact_dir(configured: Option<String>) -> PathBuf {
    configured
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(default_artifact_dir)
}

/// Resolve the artifact directory and create it on a best-effort basis, so a
/// writer into a not-yet-existing default subdirectory does not fail. A
/// creation failure is logged, not fatal — the write itself will surface the
/// error with more context if the directory truly cannot be used.
pub(crate) fn ensure_artifact_dir(configured: Option<String>) -> PathBuf {
    let dir = resolve_artifact_dir(configured);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            dir = %dir.display(),
            error = %e,
            "failed to create artifact directory"
        );
    }
    dir
}

/// Deletes stale engine artifacts from a directory.
#[derive(Debug, Clone)]
pub(crate) struct ArtifactSweeper {
    dir: PathBuf,
    retention: Duration,
}

impl ArtifactSweeper {
    /// Create a sweeper for `dir` (defaulting to [`default_artifact_dir`], a
    /// mofa-owned subdirectory of the system temp) that removes engine
    /// artifacts older than `retention`.
    pub(crate) fn new(dir: Option<PathBuf>, retention: Duration) -> Self {
        Self {
            dir: dir.unwrap_or_else(default_artifact_dir),
            retention,
        }
    }

    /// The retention window; artifacts at least this old are removed.
    pub(crate) fn retention(&self) -> Duration {
        self.retention
    }

    /// Delete engine artifacts older than the retention. Returns how many files
    /// were removed. Non-engine files and unreadable entries are left untouched.
    pub(crate) fn sweep(&self) -> usize {
        let now = SystemTime::now();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return 0;
        };
        let mut removed = 0;
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with(ARTIFACT_PREFIX) {
                continue;
            }
            let is_stale = entry
                .metadata()
                .and_then(|m| m.modified())
                .map(|modified| {
                    now.duration_since(modified)
                        .map(|age| age >= self.retention)
                        .unwrap_or(false)
                })
                .unwrap_or(false);
            if is_stale && std::fs::remove_file(entry.path()).is_ok() {
                removed += 1;
            }
        }
        removed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sweeps_only_stale_engine_files() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path().to_path_buf();

        let artifact = dir_path.join("mofa_tts_abc.wav");
        let unrelated = dir_path.join("important.txt");
        std::fs::write(&artifact, b"audio").unwrap();
        std::fs::write(&unrelated, b"keep me").unwrap();

        // Retention 0 → every engine artifact is stale and removed; the
        // non-engine file is never eligible.
        let sweeper = ArtifactSweeper::new(Some(dir_path.clone()), Duration::ZERO);
        assert_eq!(sweeper.sweep(), 1);
        assert!(!artifact.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn keeps_fresh_artifacts() {
        let dir = tempfile::tempdir().unwrap();
        let artifact = dir.path().join("mofa_tts_fresh.wav");
        std::fs::write(&artifact, b"audio").unwrap();

        // A one-hour retention keeps a just-written file.
        let sweeper =
            ArtifactSweeper::new(Some(dir.path().to_path_buf()), Duration::from_secs(3600));
        assert_eq!(sweeper.sweep(), 0);
        assert!(artifact.exists());
    }

    #[test]
    fn missing_directory_is_a_noop() {
        let sweeper =
            ArtifactSweeper::new(Some(PathBuf::from("/nonexistent/mofa/dir")), Duration::ZERO);
        assert_eq!(sweeper.sweep(), 0);
    }

    #[test]
    fn default_artifact_dir_is_a_mofa_owned_subdirectory() {
        // #4 review: the default must never be the shared system temp dir
        // itself — the sweeper deletes by name prefix, and sweeping a
        // world-writable shared directory would delete any other tenant's
        // `mofa_*` files.
        let dir = resolve_artifact_dir(None);
        assert_eq!(dir, std::env::temp_dir().join("mofa_artifacts"));
        assert_ne!(dir, std::env::temp_dir());
        // A configured dir is honored verbatim.
        assert_eq!(
            resolve_artifact_dir(Some("/tmp/custom-mofa".into())),
            PathBuf::from("/tmp/custom-mofa")
        );
    }

    #[test]
    fn sweeper_defaults_to_the_mofa_owned_subdirectory() {
        let sweeper = ArtifactSweeper::new(None, Duration::ZERO);
        assert_eq!(sweeper.dir, std::env::temp_dir().join("mofa_artifacts"));
    }
}
