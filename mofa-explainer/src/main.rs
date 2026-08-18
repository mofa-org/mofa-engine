//! mofa-explainer — a real app: a topic in, a finished explainer video out.
//!
//!   mofa-explainer "How neural networks learn" --seconds 30 --out explainer.mp4
//!
//! This is the S4 flagship as a tool a person actually runs. It drives the MoFA
//! engine through the whole pipeline — `Chat` (script) → `ImageGen` (scene
//! visuals) → `Tts` (narration) — then composes a **real, playable `.mp4`** with
//! FFmpeg (reused, not re-implemented, per PRD scope) and runs the engine's
//! **hard quality gate** so a broken render is never presented as finished.
//!
//! Local-first: on a laptop with Ollama it produces a narrated, subtitled video
//! fully offline at `$0`.
//!
//! ## Graceful degradation (and its honest limits)
//!
//! Narration uses the engine's built-in zero-config voice, so it always works
//! offline. The other two modalities have no zero-config offline path — real image
//! generation and speech recognition both need a model — so the app degrades
//! honestly rather than pretending:
//!
//!   - **No image backend** (no local SD / cloud key): instead of a blank frame,
//!     the app renders real per-scene **title cards** with FFmpeg (`drawtext`) —
//!     genuine, varying visuals, just not AI-generated imagery. Configure an image
//!     backend (or a cloud key) and the same pipeline uses real scene images.
//!   - **No ASR**: subtitle *timing* is derived from the script (proportional to
//!     length) rather than from `Asr` word timestamps. The words are exact; only
//!     the timing is approximate until an ASR backend is configured.

use std::path::{Path, PathBuf};

use clap::Parser;
use mofa_engine_core::quality_gate::QualityGate;
use mofa_engine_core::{Engine, EngineConfig};
use mofa_kernel::{Capability, InferenceRequest, Message, Prefer};

/// Topic → narrated, subtitled explainer video (.mp4), via the MoFA engine.
#[derive(Parser, Debug)]
#[command(name = "mofa-explainer", version, about)]
struct Cli {
    /// The explainer topic, e.g. "How neural networks learn".
    topic: String,

    /// Where to write the finished video.
    #[arg(short, long, default_value = "explainer.mp4")]
    out: PathBuf,

    /// Target length in seconds (guides the script; the real length follows the
    /// narration audio).
    #[arg(long, default_value_t = 30)]
    seconds: u32,

    /// Number of scenes to plan.
    #[arg(long, default_value_t = 4)]
    scenes: usize,

    /// Locality preference (`auto` allows cloud fallback; `local` stays offline).
    #[arg(long, default_value = "local", value_parser = ["auto", "local", "cloud"])]
    prefer: String,

    /// Narration voice (macOS `say` voice on the default backend, e.g. "Samantha").
    #[arg(long)]
    voice: Option<String>,

    /// Chat model override (default: routed local-first).
    #[arg(long)]
    model: Option<String>,

    /// Keep the intermediate work directory (script, audio, scenes, subtitles).
    #[arg(long)]
    keep_work: bool,

    /// Use a specific engine config instead of the auto-provisioned offline one.
    #[arg(short, long)]
    config: Option<PathBuf>,
}

/// Video canvas. 720p is a good default for an explainer and cheap to render.
const WIDTH: u32 = 1280;
const HEIGHT: u32 = 720;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    // FFmpeg is the composition engine (reused, not built). Fail early and clearly
    // if it (or ffprobe) is missing — that's the one hard external dependency.
    ensure_tool("ffmpeg")?;
    ensure_tool("ffprobe")?;

    // A work directory for all intermediates, beside the output unless kept.
    let work = make_work_dir(&cli.out)?;
    println!("→ topic: {}\n  work dir: {}\n", cli.topic, work.display());

    // ==========================================================================
    // Boot the engine (offline config unless --config is given)
    // ==========================================================================
    let config_path = match &cli.config {
        Some(p) => Some(p.clone()),
        None => provision_offline_config()?,
    };
    let engine = Engine::try_new(EngineConfig::load(config_path.as_deref()))
        .await
        .map_err(|e| format!("engine init failed: {e}"))?;
    engine.refresh_resources().await;

    let prefer = match cli.prefer.as_str() {
        "local" => Prefer::Local,
        "cloud" => Prefer::Cloud,
        _ => Prefer::Auto,
    };

    // ==========================================================================
    // 1. Script (Chat) — a scene-by-scene narration, warming ImageGen next.
    // ==========================================================================
    println!(
        "→ writing a {}-scene script (~{}s)…",
        cli.scenes, cli.seconds
    );
    let script = write_script(&engine, &cli, prefer).await?;
    let scenes = split_scenes(&script, cli.scenes);
    println!("  {} scene(s) planned\n", scenes.len());

    // ==========================================================================
    // 2. Narration (TTS) — the full script becomes the soundtrack; its real
    //    duration drives everything downstream (scene timing, subtitles).
    // ==========================================================================
    println!("→ synthesizing narration…");
    let audio = synthesize_narration(&engine, &cli, prefer, &script, &work).await?;
    let duration = ffprobe_duration(&audio).await?;
    println!("  narration: {:.1}s → {}\n", duration, audio.display());

    // ==========================================================================
    // 3. Scene visuals (ImageGen) — try the engine; fall back to title cards.
    // ==========================================================================
    println!("→ generating scene visuals…");
    let images = generate_scenes(&engine, prefer, &scenes, &work).await;
    if images.is_empty() {
        println!("  no image backend or system font — using a solid background\n");
    } else {
        println!("  {} scene visual(s)\n", images.len());
    }

    // ==========================================================================
    // 4. Subtitles — timed from the script (see module note on ASR).
    // ==========================================================================
    let srt = work.join("subtitles.srt");
    write_srt(&script, duration, &srt)?;

    // ==========================================================================
    // 5. Compose (FFmpeg) — scenes + narration + subtitles → final.mp4.
    // ==========================================================================
    println!("→ composing video (ffmpeg)…");
    compose(&images, &audio, &srt, duration, &cli.out).await?;
    println!("  composed → {}\n", cli.out.display());

    // ==========================================================================
    // 6. Quality gate — the hard gate: no gate, no output.
    // ==========================================================================
    println!("== quality gate ==");
    let report = QualityGate::new().check(&cli.out, None).await;
    for c in &report.checks {
        println!(
            "  [{}] {}: {}",
            if c.passed { "PASS" } else { "FAIL" },
            c.name,
            c.detail
        );
    }

    if !cli.keep_work {
        let _ = std::fs::remove_dir_all(&work);
    }

    if report.passed {
        println!("\n🎬 explainer ready → {}", cli.out.display());
        Ok(())
    } else {
        // A failing gate is a real outcome, not a crash: the artifact exists but is
        // not certified. Surface it as an error so scripts/CI can react.
        Err(format!(
            "quality gate REJECTED {} — see checks above (no gate, no output)",
            cli.out.display()
        ))
    }
}

// ==============================================================================
// Pipeline stages
// ==============================================================================

/// Ask the engine for a scene-by-scene narration script.
async fn write_script(engine: &Engine, cli: &Cli, prefer: Prefer) -> Result<String, String> {
    let req = InferenceRequest {
        capability: Some(Capability::Chat),
        model: cli.model.clone(),
        messages: vec![
            Message {
                role: "system".into(),
                content: format!(
                    "You write concise explainer-video narration. For the user's \
                     topic, write exactly {} short scenes for a ~{}s video. Separate \
                     scenes with a blank line. Each scene is one or two spoken \
                     sentences — no scene labels, no stage directions, just the \
                     narration.",
                    cli.scenes, cli.seconds
                ),
                ..Default::default()
            },
            Message {
                role: "user".into(),
                content: cli.topic.clone(),
                ..Default::default()
            },
        ],
        prefer,
        // Warm the image model while the script is written (cross-capability warmup).
        hint_next: Some("image_gen".into()),
        ..Default::default()
    };
    let resp = engine
        .invoke(req)
        .await
        .map_err(|e| stage_error("script (chat)", e))?;
    let script = resp.text.unwrap_or_default().trim().to_string();
    if script.is_empty() {
        return Err("the model returned an empty script".into());
    }
    println!(
        "  {} in {}ms · {}",
        resp.provider,
        resp.duration_ms,
        cost_label(resp.cost_usd)
    );
    Ok(script)
}

/// Synthesize the narration audio for the whole script.
async fn synthesize_narration(
    engine: &Engine,
    cli: &Cli,
    prefer: Prefer,
    script: &str,
    work: &Path,
) -> Result<PathBuf, String> {
    let req = InferenceRequest {
        capability: Some(Capability::Tts),
        messages: vec![Message {
            role: "user".into(),
            content: script.to_string(),
            ..Default::default()
        }],
        prefer,
        params: serde_json::json!({
            "format": "mp3",
            "voice": cli.voice.clone().unwrap_or_default(),
        }),
        ..Default::default()
    };
    let resp = engine
        .invoke(req)
        .await
        .map_err(|e| stage_error("narration (tts)", e))?;
    let produced = resp.file.ok_or("the TTS backend returned no audio file")?;
    // Move the engine artifact into our work dir under a stable name.
    let dest = work.join("narration.mp3");
    std::fs::rename(&produced, &dest)
        .or_else(|_| std::fs::copy(&produced, &dest).map(|_| ()))
        .map_err(|e| format!("placing narration: {e}"))?;
    println!(
        "  {} in {}ms · {}",
        resp.provider,
        resp.duration_ms,
        cost_label(resp.cost_usd)
    );
    Ok(dest)
}

/// Try to generate one image per scene through the engine. Returns whatever
/// succeeded (possibly empty) — a missing image backend is expected offline and
/// is not fatal (the composition falls back to title cards).
async fn generate_scenes(
    engine: &Engine,
    prefer: Prefer,
    scenes: &[String],
    work: &Path,
) -> Vec<PathBuf> {
    let mut images = Vec::new();
    for (i, scene) in scenes.iter().enumerate() {
        let req = InferenceRequest {
            capability: Some(Capability::ImageGen),
            messages: vec![Message {
                role: "user".into(),
                content: format!("A clean, minimal illustration for: {scene}"),
                ..Default::default()
            }],
            prefer,
            params: serde_json::json!({ "size": format!("{WIDTH}x{HEIGHT}") }),
            ..Default::default()
        };
        match engine.invoke(req).await {
            Ok(resp) => {
                if let Some(file) = resp.file {
                    let dest = work.join(format!("scene_{i:03}.png"));
                    if std::fs::rename(&file, &dest)
                        .or_else(|_| std::fs::copy(&file, &dest).map(|_| ()))
                        .is_ok()
                    {
                        images.push(dest);
                    }
                }
            }
            Err(_) => {
                // First failure means no usable image backend; stop trying rather
                // than repeat the same routing failure for every scene.
                break;
            }
        }
    }

    // No AI image backend (the offline default): render real per-scene title cards
    // with FFmpeg so the video still has genuine, varying visuals rather than a
    // blank background. Needs a system font; if none is found we return empty and
    // the composition falls back to a solid background.
    if images.is_empty() {
        images = render_title_cards(scenes, work).await;
    }
    images
}

/// A short palette of dark backgrounds cycled across scenes for visual variety
/// (variety also satisfies the quality gate's "slideshow risk" motion check).
const CARD_PALETTE: [&str; 5] = ["0x0f172a", "0x1e293b", "0x172554", "0x1e1b4b", "0x0f766e"];

/// Render one title card per scene: the scene's opening line, centered white text
/// over a colored background, written to a PNG via FFmpeg's `drawtext`. Uses
/// `textfile=` (not inline text) so scene punctuation needs no filtergraph
/// escaping. Best-effort per scene; a card that fails to render is skipped.
async fn render_title_cards(scenes: &[String], work: &Path) -> Vec<PathBuf> {
    let Some(font) = find_system_font() else {
        return Vec::new();
    };

    let mut cards = Vec::new();
    for (i, scene) in scenes.iter().enumerate() {
        let label = wrap_label(scene, 24, 5);
        let card_txt = work.join(format!("card_{i:03}.txt"));
        if std::fs::write(&card_txt, &label).is_err() {
            continue;
        }
        let out = work.join(format!("scene_{i:03}.png"));
        let color = CARD_PALETTE[i % CARD_PALETTE.len()];
        let vf = format!(
            "drawtext=fontfile={font}:textfile={txt}:fontcolor=white:fontsize=54:\
             x=(w-text_w)/2:y=(h-text_h)/2:line_spacing=16",
            font = font.display(),
            txt = card_txt.display(),
        );
        let args = vec![
            "-y".into(),
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("color=c={color}:s={WIDTH}x{HEIGHT}"),
            "-vf".into(),
            vf,
            "-frames:v".into(),
            "1".into(),
            out.to_string_lossy().into(),
        ];
        if run_cmd("ffmpeg", &args).await.is_ok() {
            cards.push(out);
        }
        let _ = std::fs::remove_file(&card_txt);
    }
    cards
}

/// Find a usable TrueType font for `drawtext`, checking the common macOS and Linux
/// locations. Returns `None` if none is present (title cards are then skipped).
fn find_system_font() -> Option<PathBuf> {
    const CANDIDATES: [&str; 6] = [
        "/System/Library/Fonts/Helvetica.ttc",
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/Library/Fonts/Arial.ttf",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/usr/share/fonts/dejavu/DejaVuSans.ttf",
    ];
    CANDIDATES.iter().map(PathBuf::from).find(|p| p.is_file())
}

/// Reduce a scene to a short, centered caption: take its opening sentence and word
/// wrap it to at most `max_lines` lines of about `width` characters, appending an
/// ellipsis when truncated.
fn wrap_label(scene: &str, width: usize, max_lines: usize) -> String {
    // The opening sentence is the heading; the full text rides in the subtitles.
    let head = scene.split(['.', '!', '?']).next().unwrap_or(scene).trim();

    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    for word in head.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            lines.push(std::mem::take(&mut line));
            if lines.len() == max_lines {
                break;
            }
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if lines.len() < max_lines && !line.is_empty() {
        lines.push(line);
    }
    let truncated = lines.len() == max_lines;
    let mut label = lines.join("\n");
    if truncated {
        label.push('…');
    }
    if label.is_empty() {
        label.push_str("• • •");
    }
    label
}

// ==============================================================================
// Subtitles (timed from the script)
// ==============================================================================

/// Max characters per subtitle cue, so long sentences become several readable
/// cues instead of one wall of text.
const MAX_CUE_CHARS: usize = 84;

/// Write an SRT whose cues cover the whole narration, each cue allotted time
/// proportional to its length. Sentences are the base unit, further split at word
/// boundaries when they exceed [`MAX_CUE_CHARS`]. Word-accurate timing would come
/// from `Asr`; this length-proportional timing is the offline approximation.
fn write_srt(script: &str, duration: f64, path: &Path) -> Result<(), String> {
    let cues: Vec<String> = split_sentences(script)
        .iter()
        .flat_map(|s| chunk_cue(s, MAX_CUE_CHARS))
        .collect();
    if cues.is_empty() {
        return Err("no subtitle text".into());
    }
    let total_chars: usize = cues.iter().map(|c| c.len().max(1)).sum();

    let mut srt = String::new();
    let mut t = 0.0f64;
    for (i, cue) in cues.iter().enumerate() {
        let share = cue.len().max(1) as f64 / total_chars as f64;
        let start = t;
        let end = (t + share * duration).min(duration);
        t = end;
        srt.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            format_ts(start),
            format_ts(end),
            cue
        ));
    }
    std::fs::write(path, srt).map_err(|e| format!("writing subtitles: {e}"))
}

/// Break one cue into word-bounded pieces no longer than `width` characters. A
/// single word longer than `width` is kept whole (never split mid-word).
fn chunk_cue(cue: &str, width: usize) -> Vec<String> {
    let mut chunks = Vec::new();
    let mut line = String::new();
    for word in cue.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            chunks.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        chunks.push(line);
    }
    chunks
}

/// Split prose into sentence-ish cues on `.`/`!`/`?` boundaries.
fn split_sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in text.chars() {
        if ch == '\n' {
            cur.push(' ');
            continue;
        }
        cur.push(ch);
        if matches!(ch, '.' | '!' | '?') {
            let s = cur.trim().to_string();
            if !s.is_empty() {
                out.push(s);
            }
            cur.clear();
        }
    }
    let tail = cur.trim().to_string();
    if !tail.is_empty() {
        out.push(tail);
    }
    out
}

/// Format seconds as an SRT timestamp `HH:MM:SS,mmm`.
fn format_ts(secs: f64) -> String {
    let ms = (secs * 1000.0).round() as u64;
    let (h, rem) = (ms / 3_600_000, ms % 3_600_000);
    let (m, rem) = (rem / 60_000, rem % 60_000);
    let (s, milli) = (rem / 1000, rem % 1000);
    format!("{h:02}:{m:02}:{s:02},{milli:03}")
}

// ==============================================================================
// Composition (FFmpeg)
// ==============================================================================

/// Compose the final mp4. With scene images we build a slideshow timed to the
/// audio; without them a solid background carries the narration. Subtitles are
/// burned in best-effort — if the build lacks libass we retry without them rather
/// than fail the render.
async fn compose(
    images: &[PathBuf],
    audio: &Path,
    srt: &Path,
    duration: f64,
    out: &Path,
) -> Result<(), String> {
    // The subtitles filter, escaped for FFmpeg's filtergraph (the path is quoted so
    // spaces are safe; we keep cues centered near the bottom).
    let sub_filter = format!(
        "subtitles=filename='{}':force_style='Alignment=2,MarginV=40,FontSize=22'",
        srt.to_string_lossy().replace('\'', "\\'")
    );

    // Two input strategies, one output recipe.
    let base_args: Vec<String> = if images.is_empty() {
        // Solid background sized to the audio length.
        vec![
            "-f".into(),
            "lavfi".into(),
            "-i".into(),
            format!("color=c=0x0f172a:s={WIDTH}x{HEIGHT}:r=25:d={duration:.3}"),
            "-i".into(),
            audio.to_string_lossy().into(),
        ]
    } else {
        // A concat slideshow: each image shown for an equal slice of the audio.
        let list = out.with_extension("concat.txt");
        write_concat_list(images, duration, &list)?;
        vec![
            "-f".into(),
            "concat".into(),
            "-safe".into(),
            "0".into(),
            "-i".into(),
            list.to_string_lossy().into(),
            "-i".into(),
            audio.to_string_lossy().into(),
        ]
    };

    // Scale/pad images to the canvas (a no-op for the solid background), then subs.
    let vf_with_subs = format!(
        "scale={WIDTH}:{HEIGHT}:force_original_aspect_ratio=decrease,\
         pad={WIDTH}:{HEIGHT}:(ow-iw)/2:(oh-ih)/2,format=yuv420p,{sub_filter}"
    );
    let vf_plain = format!(
        "scale={WIDTH}:{HEIGHT}:force_original_aspect_ratio=decrease,pad={WIDTH}:{HEIGHT}:(ow-iw)/2:(oh-ih)/2,format=yuv420p"
    );

    // First attempt: with burned-in subtitles.
    let mut args = base_args.clone();
    push_output_args(&mut args, &vf_with_subs, out);
    let mut result = run_cmd("ffmpeg", &args).await.map(|_| ());

    // Retry without subtitles (e.g. an ffmpeg built without libass).
    if result.is_err() {
        eprintln!("  note: subtitle burn-in failed; rendering without subtitles");
        let mut args = base_args;
        push_output_args(&mut args, &vf_plain, out);
        result = run_cmd("ffmpeg", &args).await.map(|_| ());
    }

    // Remove the transient concat list (only written for the slideshow path).
    if !images.is_empty() {
        let _ = std::fs::remove_file(out.with_extension("concat.txt"));
    }
    result
}

/// Append the shared encoder/output arguments to an FFmpeg invocation.
fn push_output_args(args: &mut Vec<String>, vf: &str, out: &Path) {
    args.extend([
        "-vf".into(),
        vf.into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-c:a".into(),
        "aac".into(),
        "-shortest".into(),
        "-movflags".into(),
        "+faststart".into(),
        "-y".into(),
        out.to_string_lossy().into(),
    ]);
}

/// Write a concat-demuxer list that shows each image for an equal share of the
/// total duration. The last file is repeated because the concat demuxer ignores
/// the final `duration` directive.
///
/// Paths are absolute: the concat demuxer resolves each `file` entry relative to
/// the **list file's own directory**, so a cwd-relative path would be misresolved
/// (e.g. doubled) when the list lives in a subdirectory.
fn write_concat_list(images: &[PathBuf], duration: f64, path: &Path) -> Result<(), String> {
    let per = duration / images.len() as f64;
    let abs = |p: &PathBuf| std::fs::canonicalize(p).unwrap_or_else(|_| p.clone());
    let mut list = String::new();
    for img in images {
        list.push_str(&format!(
            "file '{}'\nduration {:.3}\n",
            abs(img).display(),
            per
        ));
    }
    if let Some(last) = images.last() {
        list.push_str(&format!("file '{}'\n", abs(last).display()));
    }
    std::fs::write(path, list).map_err(|e| format!("writing concat list: {e}"))
}

// ==============================================================================
// External-tool helpers
// ==============================================================================

/// Probe a media file's duration in seconds via ffprobe.
async fn ffprobe_duration(path: &Path) -> Result<f64, String> {
    let out = run_cmd(
        "ffprobe",
        &[
            "-v".into(),
            "error".into(),
            "-show_entries".into(),
            "format=duration".into(),
            "-of".into(),
            "default=noprint_wrappers=1:nokey=1".into(),
            path.to_string_lossy().into(),
        ],
    )
    .await?;
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .map_err(|_| "ffprobe returned no duration".into())
}

/// Run an external command, returning its output on success or a message that
/// includes stderr on failure.
async fn run_cmd(program: &str, args: &[String]) -> Result<std::process::Output, String> {
    let out = tokio::process::Command::new(program)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("failed to run {program}: {e}"))?;
    if out.status.success() {
        Ok(out)
    } else {
        Err(format!(
            "{program} exited with {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Fail early with an install hint if a required external tool is missing.
fn ensure_tool(tool: &str) -> Result<(), String> {
    let found = std::env::var_os("PATH")
        .map(|paths| {
            std::env::split_paths(&paths).any(|dir| {
                let p = dir.join(tool);
                p.is_file()
            })
        })
        .unwrap_or(false);
    if found {
        Ok(())
    } else {
        Err(format!(
            "'{tool}' not found on PATH — install FFmpeg (e.g. `brew install ffmpeg`)"
        ))
    }
}

// ==============================================================================
// Small helpers
// ==============================================================================

/// Split the script into `n` scenes on blank lines, falling back to sentence
/// grouping when the model didn't paragraph-separate them.
fn split_scenes(script: &str, n: usize) -> Vec<String> {
    let paras: Vec<String> = script
        .split("\n\n")
        .map(|p| p.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|p| !p.trim().is_empty())
        .collect();
    if paras.len() >= 2 {
        return paras;
    }
    // One block: distribute sentences across the requested scene count.
    let sentences = split_sentences(script);
    if sentences.len() <= 1 || n <= 1 {
        return vec![script.split_whitespace().collect::<Vec<_>>().join(" ")];
    }
    let per = sentences.len().div_ceil(n);
    sentences.chunks(per).map(|chunk| chunk.join(" ")).collect()
}

/// A work directory beside the output (e.g. `explainer.work/`).
fn make_work_dir(out: &Path) -> Result<PathBuf, String> {
    let stem = out.file_stem().map(|s| s.to_owned()).unwrap_or_default();
    let dir = out
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{}.work", stem.to_string_lossy()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating work dir: {e}"))?;
    Ok(dir)
}

/// Render a cost as a dollar figure, or the local/free case.
fn cost_label(cost: Option<f64>) -> String {
    match cost {
        Some(c) if c > 0.0 => format!("${c:.6}"),
        _ => "$0.00 (local/free)".into(),
    }
}

/// Turn an engine error into a message that includes any failover chain.
fn stage_error(stage: &str, e: mofa_kernel::EngineError) -> String {
    let info = e.info();
    let mut msg = format!("{stage} failed [{:?}]: {}", info.code, info.message);
    for a in &info.failed_chain {
        msg.push_str(&format!(
            "\n    tried {}/{}: {}",
            a.provider, a.model, a.reason
        ));
    }
    msg
}

// ==============================================================================
// Offline config provisioning (shared shape with mofa-podcast)
// ==============================================================================

/// Provision a minimal offline engine config: local Ollama for chat. Narration
/// needs no setup — the engine auto-registers the OS-native voice (macOS `say` /
/// Linux `espeak`) when a config declares no TTS backend, so this config declares
/// none. Scene imagery still has no zero-config local path (real image generation
/// needs a model), so visuals fall back to title cards unless a cloud key / local
/// SD is configured.
fn provision_offline_config() -> Result<Option<PathBuf>, String> {
    let dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("mofa-explainer");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;

    let config = dir.join("config.toml");
    let toml = r#"# Auto-generated by mofa-explainer for offline, zero-setup runs.
# Chat is local Ollama; narration is the engine's built-in system voice (no config).
[[providers]]
name = "ollama"
kind = "ollama"
base_url = "http://127.0.0.1:11434"
priority = 1
cost_tier = "free"
"#;
    std::fs::write(&config, toml).map_err(|e| format!("writing {}: {e}", config.display()))?;
    Ok(Some(config))
}
