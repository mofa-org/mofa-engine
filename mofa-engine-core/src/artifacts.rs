//! Retention and cleanup for engine-generated artifacts.
//!
//! Backends that produce files (TTS audio, and later image/video output) write
//! them with a `mofa_` name prefix into a shared directory. Left alone these
//! accumulate, so the engine runs a periodic sweep that deletes artifacts older
//! than a configured retention. Only files the engine created (matched by
//! prefix) are ever removed, so pointing the sweeper at a shared temp dir cannot
//! touch unrelated files.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Name prefix stamped on every engine-generated artifact.
pub const ARTIFACT_PREFIX: &str = "mofa_";

/// Deletes stale engine artifacts from a directory.
#[derive(Debug, Clone)]
pub struct ArtifactSweeper {
    dir: PathBuf,
    retention: Duration,
}

impl ArtifactSweeper {
    /// Create a sweeper for `dir` (defaulting to the system temp dir) that
    /// removes engine artifacts older than `retention`.
    pub fn new(dir: Option<PathBuf>, retention: Duration) -> Self {
        Self {
            dir: dir.unwrap_or_else(std::env::temp_dir),
            retention,
        }
    }

    /// The retention window; artifacts at least this old are removed.
    pub fn retention(&self) -> Duration {
        self.retention
    }

    /// Delete engine artifacts older than the retention. Returns how many files
    /// were removed. Non-engine files and unreadable entries are left untouched.
    pub fn sweep(&self) -> usize {
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
}
