//! Post-production video quality gate (S4 flagship — "no gate, no output").
//!
//! The flagship Explainer Video scenario produces a finished `mp4` by composing
//! generated scenes, narration, and captions. Before that artifact is accepted,
//! it must clear at least one **hard gate** so a broken render (empty/truncated
//! file, a still slideshow instead of an animation, or a video that does not
//! match its script) is never shipped as a "finished" product.
//!
//! This gate runs three checks:
//!   1. **Duration / integrity** — `ffprobe` confirms a real video stream with a
//!      non-trivial duration and non-zero resolution (catches a failed compose).
//!   2. **Slideshow-risk** — `ffmpeg` downsamples the video to a handful of tiny
//!      grayscale frames; the mean pixel change between consecutive frames is the
//!      *motion*. A static slideshow shows ~zero motion (high risk); an animation
//!      shows real motion (low risk). See [`slideshow_risk_from_frames`].
//!   3. **VLM match** *(optional)* — a semantic image/text verdict supplied by the
//!      caller (who runs the engine's `Vlm` capability over sampled frames). This
//!      gate stays free of a circular engine dependency by taking the verdict as
//!      an input rather than calling back into the engine.
//!
//! The gate **fails closed**: if `ffprobe`/`ffmpeg` are unavailable or a signal
//! cannot be measured, the corresponding check fails rather than passing on
//! faith — an un-certifiable video is not a finished video. Both external tools
//! are widely available (`brew install ffmpeg`); MoFA shells out to them rather
//! than re-implementing media decoding (PRD: "reuse, not self-developed").
//!
//! # Example
//! ```no_run
//! # use mofa_engine_core::quality_gate::QualityGate;
//! # async fn run() {
//! let gate = QualityGate::new();
//! let report = gate.check("final.mp4".as_ref(), Some(true)).await;
//! assert!(report.passed, "checks: {:?}", report.checks);
//! # }
//! ```

use std::path::Path;

use serde::Serialize;
use tokio::process::Command;

/// Edge length of the downsampled grayscale frames used for motion analysis.
const FRAME_EDGE: usize = 8;
/// Pixels per sampled frame (`FRAME_EDGE²`), i.e. bytes per frame in the raw
/// grayscale stream ffmpeg emits.
const FRAME_PIXELS: usize = FRAME_EDGE * FRAME_EDGE;
/// Number of frames to sample across the whole video for motion analysis.
const DEFAULT_SAMPLE_FRAMES: usize = 8;
/// Mean per-pixel change (0..1) treated as "clearly moving". Motion at or above
/// this maps to zero slideshow-risk; below it scales linearly toward full risk.
/// Deliberately small: even a real animation changes only a fraction of an 8×8
/// frame between samples, so a modest change already rules out a still slideshow.
const MOTION_FULL_SCALE: f64 = 0.06;

/// Pass/fail thresholds for the gate.
#[derive(Debug, Clone)]
pub struct QualityThresholds {
    /// Minimum acceptable video duration in seconds.
    pub min_duration_secs: f64,
    /// Maximum acceptable slideshow-risk (0 = full motion, 1 = static).
    pub max_slideshow_risk: f64,
}

impl Default for QualityThresholds {
    fn default() -> Self {
        Self {
            min_duration_secs: 1.0,
            max_slideshow_risk: 0.85,
        }
    }
}

/// One named check within a [`QualityReport`].
#[derive(Debug, Clone, Serialize)]
pub struct QualityCheck {
    /// Stable identifier (`duration`, `slideshow_risk`, `vlm_match`).
    pub name: &'static str,
    /// Whether this check passed.
    pub passed: bool,
    /// Human-readable explanation (measured value vs threshold, or why it could
    /// not be evaluated).
    pub detail: String,
}

/// The result of running the gate over one video.
#[derive(Debug, Clone, Serialize)]
pub struct QualityReport {
    /// Overall verdict: true only if every check passed.
    pub passed: bool,
    /// Measured duration in seconds, when it could be probed.
    pub duration_secs: Option<f64>,
    /// Measured slideshow-risk (0..1), when frames could be sampled.
    pub slideshow_risk: Option<f64>,
    /// Per-check breakdown, in evaluation order.
    pub checks: Vec<QualityCheck>,
}

/// Probed technical facts about a video file.
#[derive(Debug, Clone, Default, PartialEq)]
struct ProbeInfo {
    /// Container/stream duration in seconds.
    duration_secs: Option<f64>,
    /// Whether at least one video stream with a non-zero resolution is present.
    has_video: bool,
}

/// Validates a finished video against technical + semantic quality checks by
/// shelling out to `ffprobe` and `ffmpeg`.
pub struct QualityGate {
    /// `ffprobe` program (resolved on `PATH`).
    ffprobe: String,
    /// `ffmpeg` program (resolved on `PATH`).
    ffmpeg: String,
    /// Frames sampled across the video for motion analysis.
    sample_frames: usize,
    /// Pass/fail thresholds.
    thresholds: QualityThresholds,
}

impl Default for QualityGate {
    fn default() -> Self {
        Self::new()
    }
}

impl QualityGate {
    /// A gate with default tools (`ffprobe`/`ffmpeg`) and thresholds.
    pub fn new() -> Self {
        Self {
            ffprobe: "ffprobe".into(),
            ffmpeg: "ffmpeg".into(),
            sample_frames: DEFAULT_SAMPLE_FRAMES,
            thresholds: QualityThresholds::default(),
        }
    }

    /// Override the pass/fail thresholds.
    pub fn with_thresholds(mut self, thresholds: QualityThresholds) -> Self {
        self.thresholds = thresholds;
        self
    }

    /// Run the gate over `video`, optionally folding in a caller-supplied VLM
    /// image/text match verdict (`vlm_ok`). Never panics or errors: a tool or
    /// signal that cannot be evaluated becomes a *failed* check (fail-closed), so
    /// the returned [`QualityReport`] always reflects a safe verdict.
    pub async fn check(&self, video: &Path, vlm_ok: Option<bool>) -> QualityReport {
        let probe = self.probe(video).await.unwrap_or_default();
        // Only bother sampling frames if the file looks like a real video.
        let slideshow_risk = if probe.has_video {
            self.sample_slideshow_risk(video, probe.duration_secs).await
        } else {
            None
        };
        self.evaluate(&probe, slideshow_risk, vlm_ok)
    }

    /// Combine measured signals into a [`QualityReport`]. Pure: no I/O, so the
    /// verdict logic is unit-testable independent of `ffprobe`/`ffmpeg`.
    fn evaluate(
        &self,
        probe: &ProbeInfo,
        slideshow_risk: Option<f64>,
        vlm_ok: Option<bool>,
    ) -> QualityReport {
        let mut checks = Vec::new();

        let dur_pass = probe.has_video
            && probe
                .duration_secs
                .is_some_and(|d| d >= self.thresholds.min_duration_secs);
        checks.push(QualityCheck {
            name: "duration",
            passed: dur_pass,
            detail: match (probe.has_video, probe.duration_secs) {
                (false, _) => "no decodable video stream (ffprobe failed or empty file)".into(),
                (true, Some(d)) => {
                    format!("{d:.2}s (min {:.2}s)", self.thresholds.min_duration_secs)
                }
                (true, None) => "video stream present but duration unknown".into(),
            },
        });

        let slide_pass = slideshow_risk.is_some_and(|r| r <= self.thresholds.max_slideshow_risk);
        checks.push(QualityCheck {
            name: "slideshow_risk",
            passed: slide_pass,
            detail: match slideshow_risk {
                Some(r) => format!(
                    "risk {r:.2} (max {:.2}); {}",
                    self.thresholds.max_slideshow_risk,
                    if r <= self.thresholds.max_slideshow_risk {
                        "sufficient motion"
                    } else {
                        "looks like a static slideshow"
                    }
                ),
                None => "could not sample frames (ffmpeg unavailable or too few frames)".into(),
            },
        });

        // VLM check only exists when the caller supplied a verdict.
        if let Some(ok) = vlm_ok {
            checks.push(QualityCheck {
                name: "vlm_match",
                passed: ok,
                detail: if ok {
                    "VLM judged frames consistent with the script".into()
                } else {
                    "VLM judged frames inconsistent with the script".into()
                },
            });
        }

        QualityReport {
            passed: checks.iter().all(|c| c.passed),
            duration_secs: probe.duration_secs,
            slideshow_risk,
            checks,
        }
    }

    /// Probe duration + stream presence via a single `ffprobe` JSON call.
    async fn probe(&self, video: &Path) -> Option<ProbeInfo> {
        let output = Command::new(&self.ffprobe)
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration:stream=codec_type,width,height",
                "-of",
                "json",
            ])
            .arg(video)
            .kill_on_drop(true)
            .output()
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }
        QualityGate::parse_ffprobe_json(&String::from_utf8_lossy(&output.stdout))
    }

    /// Sample `sample_frames` tiny grayscale frames across the video and reduce
    /// them to a slideshow-risk score. Returns `None` when frames cannot be
    /// produced (e.g. `ffmpeg` missing) or fewer than two are available.
    async fn sample_slideshow_risk(&self, video: &Path, duration: Option<f64>) -> Option<f64> {
        // Sample evenly across the whole clip: fps = frames / duration. Fall back
        // to 1 fps when duration is unknown.
        let fps = match duration {
            Some(d) if d > 0.0 => (self.sample_frames as f64 / d).max(0.01),
            _ => 1.0,
        };
        let vf = format!("fps={fps:.4},scale={FRAME_EDGE}:{FRAME_EDGE},format=gray");
        let output = Command::new(&self.ffmpeg)
            .args(["-v", "error", "-i"])
            .arg(video)
            .args(["-vf", &vf, "-f", "rawvideo", "-pix_fmt", "gray", "-"])
            .kill_on_drop(true)
            .output()
            .await
            .ok()?;
        if !output.status.success() {
            return None;
        }
        QualityGate::slideshow_risk_from_frames(&output.stdout, FRAME_PIXELS)
    }
}

/// Pure parsing/scoring helpers, grouped as private associated functions so the
/// signal extraction shares the `QualityGate` namespace and stays unit-testable
/// without invoking `ffprobe`/`ffmpeg`.
impl QualityGate {
    /// Parse the `ffprobe -of json` payload into a [`ProbeInfo`].
    fn parse_ffprobe_json(json: &str) -> Option<ProbeInfo> {
        let root: serde_json::Value = serde_json::from_str(json).ok()?;

        let duration_secs = root
            .get("format")
            .and_then(|f| f.get("duration"))
            .and_then(|d| d.as_str())
            .and_then(|s| s.trim().parse::<f64>().ok())
            .filter(|d| d.is_finite() && *d > 0.0);

        let has_video = root
            .get("streams")
            .and_then(|s| s.as_array())
            .is_some_and(|streams| {
                streams.iter().any(|st| {
                    st.get("codec_type").and_then(|c| c.as_str()) == Some("video")
                        && st
                            .get("width")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0)
                            > 0
                        && st
                            .get("height")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(0)
                            > 0
                })
            });

        Some(ProbeInfo {
            duration_secs,
            has_video,
        })
    }

    /// Reduce a raw grayscale frame stream (frames of `frame_pixels` bytes, back to
    /// back) to a slideshow-risk score in `0..=1`.
    ///
    /// The *motion* between two consecutive frames is the mean absolute per-pixel
    /// difference, normalized to `0..1`. Averaged over all consecutive pairs and
    /// compared against [`MOTION_FULL_SCALE`], it yields `risk = 1 - motion_ratio`:
    /// identical frames (a still slideshow) score risk `1.0`; frames that change by
    /// [`MOTION_FULL_SCALE`] or more score risk `0.0`. Returns `None` when fewer than
    /// two whole frames are present (motion is undefined).
    fn slideshow_risk_from_frames(raw: &[u8], frame_pixels: usize) -> Option<f64> {
        if frame_pixels == 0 {
            return None;
        }
        let frames: Vec<&[u8]> = raw.chunks_exact(frame_pixels).collect();
        if frames.len() < 2 {
            return None;
        }

        let mut total_motion = 0.0;
        for pair in frames.windows(2) {
            let diff: u64 = pair[0]
                .iter()
                .zip(pair[1].iter())
                .map(|(a, b)| a.abs_diff(*b) as u64)
                .sum();
            // Mean absolute difference for this pair, normalized to 0..1.
            total_motion += diff as f64 / (frame_pixels as f64 * 255.0);
        }
        let motion = total_motion / (frames.len() - 1) as f64;
        let motion_ratio = (motion / MOTION_FULL_SCALE).clamp(0.0, 1.0);
        Some(1.0 - motion_ratio)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ffprobe_json_extracts_duration_and_video_stream() {
        let json = r#"{
            "streams": [
                {"codec_type": "audio"},
                {"codec_type": "video", "width": 1024, "height": 576}
            ],
            "format": {"duration": "12.34"}
        }"#;
        let info = QualityGate::parse_ffprobe_json(json).unwrap();
        assert_eq!(info.duration_secs, Some(12.34));
        assert!(info.has_video);
    }

    #[test]
    fn parse_ffprobe_json_rejects_zero_resolution_and_missing_duration() {
        // A "video" stream with no resolution is not a real render; N/A duration
        // must not parse to a bogus number.
        let json = r#"{
            "streams": [{"codec_type": "video", "width": 0, "height": 0}],
            "format": {"duration": "N/A"}
        }"#;
        let info = QualityGate::parse_ffprobe_json(json).unwrap();
        assert_eq!(info.duration_secs, None);
        assert!(!info.has_video);
    }

    #[test]
    fn identical_frames_are_full_slideshow_risk() {
        // Two identical (non-uniform) frames: zero motion → maximum risk.
        let mut frame = vec![10u8; FRAME_PIXELS];
        frame[0] = 200; // some internal structure, still identical across frames
        let mut raw = frame.clone();
        raw.extend_from_slice(&frame);
        assert_eq!(
            QualityGate::slideshow_risk_from_frames(&raw, FRAME_PIXELS),
            Some(1.0)
        );
    }

    #[test]
    fn strongly_changing_frames_are_low_risk() {
        // Black frame then white frame: maximal motion → zero risk.
        let mut raw = vec![0u8; FRAME_PIXELS];
        raw.extend(std::iter::repeat_n(255u8, FRAME_PIXELS));
        assert_eq!(
            QualityGate::slideshow_risk_from_frames(&raw, FRAME_PIXELS),
            Some(0.0)
        );
    }

    #[test]
    fn slideshow_risk_needs_two_frames() {
        assert_eq!(
            QualityGate::slideshow_risk_from_frames(&[1, 2, 3], 64),
            None
        );
        assert_eq!(QualityGate::slideshow_risk_from_frames(&[], 64), None);
        assert_eq!(
            QualityGate::slideshow_risk_from_frames(&[0u8; 128], 0),
            None
        );
    }

    #[test]
    fn small_motion_scales_between_extremes() {
        // A change well below MOTION_FULL_SCALE lands near (but below) full risk,
        // confirming the score is monotone rather than binary.
        let base = vec![100u8; FRAME_PIXELS];
        let mut moved = base.clone();
        moved[0] = 130; // one pixel changes by 30/255 over 64 pixels → tiny motion
        let mut raw = base.clone();
        raw.extend_from_slice(&moved);
        let risk = QualityGate::slideshow_risk_from_frames(&raw, FRAME_PIXELS).unwrap();
        assert!(risk > 0.9 && risk < 1.0, "risk was {risk}");
    }

    /// End-to-end against real `ffmpeg`/`ffprobe`. Ignored by default (the tools
    /// are not present on every CI runner); run explicitly with:
    /// `cargo test -p mofa-engine-core --lib quality_gate -- --ignored`.
    /// It generates its own fixtures, so it needs no checked-in media.
    #[tokio::test]
    #[ignore = "requires ffmpeg/ffprobe on PATH"]
    async fn real_gate_distinguishes_animation_slideshow_and_broken() {
        let dir = std::env::temp_dir().join(format!("mofa_qg_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let animation = dir.join("animation.mp4");
        let slideshow = dir.join("slideshow.mp4");
        let broken = dir.join("broken.mp4");

        let make = |args: &[&str]| {
            std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-y"])
                .args(args)
                .status()
                .unwrap()
                .success()
        };
        assert!(make(&[
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x240:rate=24:duration=4",
            "-pix_fmt",
            "yuv420p",
            animation.to_str().unwrap(),
        ]));
        assert!(make(&[
            "-f",
            "lavfi",
            "-i",
            "color=c=blue:size=320x240:rate=24:duration=4",
            "-pix_fmt",
            "yuv420p",
            slideshow.to_str().unwrap(),
        ]));
        std::fs::write(&broken, b"not a video").unwrap();

        let gate = QualityGate::new();

        // Animation: real motion → low slideshow-risk → passes the gate.
        let anim = gate.check(&animation, Some(true)).await;
        assert!(anim.passed, "animation report: {anim:?}");
        assert!(anim.slideshow_risk.unwrap() <= gate.thresholds.max_slideshow_risk);

        // Slideshow: a held still → high slideshow-risk → fails the slideshow check.
        let slide = gate.check(&slideshow, Some(true)).await;
        assert!(!slide.passed, "slideshow report: {slide:?}");
        assert!(slide.slideshow_risk.unwrap() > gate.thresholds.max_slideshow_risk);

        // Broken file: no decodable video stream → fails closed.
        let bad = gate.check(&broken, None).await;
        assert!(!bad.passed, "broken report: {bad:?}");
        assert!(!bad.checks[0].passed);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn evaluate_passes_only_when_all_checks_pass() {
        let gate = QualityGate::new();
        let good = ProbeInfo {
            duration_secs: Some(10.0),
            has_video: true,
        };

        // All three checks green.
        let report = gate.evaluate(&good, Some(0.2), Some(true));
        assert!(report.passed);
        assert_eq!(report.checks.len(), 3);

        // A failed VLM verdict fails the whole gate.
        let report = gate.evaluate(&good, Some(0.2), Some(false));
        assert!(!report.passed);

        // Excessive slideshow-risk fails the gate.
        let report = gate.evaluate(&good, Some(0.95), Some(true));
        assert!(!report.passed);

        // Fail-closed: an un-probeable video (no signals) does not pass, and the
        // optional VLM check is simply absent.
        let report = gate.evaluate(&ProbeInfo::default(), None, None);
        assert!(!report.passed);
        assert_eq!(report.checks.len(), 2);
        assert!(report.checks.iter().all(|c| !c.passed));
    }
}
