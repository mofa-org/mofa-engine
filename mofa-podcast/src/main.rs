//! mofa-podcast — a real app: an article in, a narrated podcast episode out.
//!
//! This is not a capability demo; it is a small tool a person would actually run:
//!
//!   mofa-podcast https://example.com/some-article --out episode.mp3
//!
//! It ingests an article (an http(s) URL, a local file, or stdin), asks the MoFA
//! engine to rewrite it into a spoken-style script (`Chat`), then synthesizes a
//! **real, playable `.mp3`** (`Tts`) — all local-first, so on a laptop with Ollama
//! it runs fully offline at `$0`.
//!
//! ## Why this app exists (driving the framework)
//!
//! Building a real app surfaces what the engine is missing. Two gaps this app hit,
//! and how it works around them for now (both are engine TODOs, not app quirks):
//!
//!   1. **No zero-config local TTS.** The engine's `local_tts` backend shells out
//!      to a command that must already be installed (Crane/Kokoro). So on macOS we
//!      auto-provision a `say`-based adapter (see [`provision_offline_config`]) to
//!      give the app a working voice with nothing to download. The engine should
//!      ship a built-in local TTS so this isn't necessary.
//!   2. **`local_tts` ignores voice/speed/format.** The adapter only substitutes
//!      `{text}`/`{text_file}`/`{output}`, so a request's `params.voice` can't reach
//!      the command. We use a fixed voice until the engine passes these through.

use std::path::{Path, PathBuf};

use clap::Parser;
use mofa_engine_core::{Engine, EngineConfig};
use mofa_kernel::{Capability, InferenceRequest, Message, Prefer};

/// Article → narrated podcast episode (.mp3), via the MoFA engine.
#[derive(Parser, Debug)]
#[command(name = "mofa-podcast", version, about)]
struct Cli {
    /// Source article: an `http(s)://` URL, a local file path, or `-` for stdin.
    source: String,

    /// Where to write the episode. The extension should be `.mp3`.
    #[arg(short, long, default_value = "episode.mp3")]
    out: PathBuf,

    /// Locality preference. `local` (default) keeps everything on-device and free;
    /// `auto` allows a cloud fallback; `cloud` forces cloud.
    #[arg(long, default_value = "local", value_parser = ["auto", "local", "cloud"])]
    prefer: String,

    /// Override the chat model (e.g. `ollama/llama3.2`). Default: routed local-first.
    #[arg(long)]
    model: Option<String>,

    /// Target script length, in words.
    #[arg(long, default_value_t = 180)]
    max_words: usize,

    /// Also write the generated script next to the audio (as `<out>.md`).
    #[arg(long)]
    keep_script: bool,

    /// Use a specific engine config instead of the auto-provisioned offline one.
    #[arg(short, long)]
    config: Option<PathBuf>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    // ==========================================================================
    // 1. Ingest the article
    // ==========================================================================
    // The source can be a URL (fetched + de-HTML'd), a file, or stdin. We keep
    // the extractor deliberately simple (tag strip + entity decode); the rewrite
    // step tolerates messy input, and better readability extraction is a future
    // improvement rather than a blocker.
    println!("→ reading article from {}", describe_source(&cli.source));
    let article = load_source(&cli.source).await?;
    let article = article.trim();
    if article.is_empty() {
        return Err("the source produced no text".into());
    }
    println!("  {} chars\n", article.len());

    // ==========================================================================
    // 2. Boot the engine
    // ==========================================================================
    // Without an explicit --config we provision an offline config: local Ollama for
    // chat + a macOS `say` adapter for TTS, so the app works with nothing to set up.
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
    // 3. Rewrite the article into a spoken-style script (Chat)
    // ==========================================================================
    // `hint_next="tts"` warms the synthesis model while the script is generated, so
    // stage 4 starts hot — the cross-capability warmup a token proxy can't do.
    println!(
        "→ writing script ({} words, prefer={})…",
        cli.max_words, cli.prefer
    );
    let script_req = InferenceRequest {
        capability: Some(Capability::Chat),
        model: cli.model.clone(),
        messages: vec![
            Message {
                role: "system".into(),
                content: format!(
                    "You are a podcast host. Rewrite the article the user sends into \
                     a natural, colloquial monologue of about {} words. Ignore website \
                     navigation, ads, and boilerplate; keep only the substance. Output \
                     only what the host says aloud — no headings, no stage directions.",
                    cli.max_words
                ),
                ..Default::default()
            },
            Message {
                role: "user".into(),
                content: article.to_string(),
                ..Default::default()
            },
        ],
        prefer,
        hint_next: Some("tts".into()),
        ..Default::default()
    };
    let script_resp = engine
        .invoke(script_req)
        .await
        .map_err(|e| stage_error("script (chat)", e))?;
    let script = script_resp.text.unwrap_or_default();
    let script = script.trim();
    if script.is_empty() {
        return Err("the model returned an empty script".into());
    }
    println!(
        "  {} in {}ms · {}\n",
        script_resp.provider,
        script_resp.duration_ms,
        cost_label(script_resp.cost_usd),
    );
    println!(
        "── script ─────────────────────────────────────────────\n{script}\n───────────────────────────────────────────────────────\n"
    );

    if cli.keep_script {
        let md = cli.out.with_extension("md");
        std::fs::write(&md, script)
            .map_err(|e| format!("could not write {}: {e}", md.display()))?;
        println!("  script saved → {}", md.display());
    }

    // ==========================================================================
    // 4. Synthesize narration (TTS) → the real artifact
    // ==========================================================================
    println!("→ synthesizing narration…");
    let tts_req = InferenceRequest {
        capability: Some(Capability::Tts),
        messages: vec![Message {
            role: "user".into(),
            content: script.to_string(),
            ..Default::default()
        }],
        prefer,
        params: serde_json::json!({ "format": "mp3" }),
        ..Default::default()
    };
    let tts_resp = engine
        .invoke(tts_req)
        .await
        .map_err(|e| stage_error("narration (tts)", e))?;
    let produced = tts_resp
        .file
        .ok_or("the TTS backend returned no audio file")?;

    // The engine writes into its artifact directory; move it to the requested path
    // so the user gets `episode.mp3` where they asked for it.
    std::fs::rename(&produced, &cli.out)
        .or_else(|_| std::fs::copy(&produced, &cli.out).map(|_| ()))
        .map_err(|e| format!("could not place audio at {}: {e}", cli.out.display()))?;

    println!(
        "  {} in {}ms · {}\n",
        tts_resp.provider,
        tts_resp.duration_ms,
        cost_label(tts_resp.cost_usd),
    );
    println!("🎧 episode ready → {}", cli.out.display());
    Ok(())
}

// ==============================================================================
// Source ingestion
// ==============================================================================

/// A friendly one-word description of where the article is coming from.
fn describe_source(source: &str) -> String {
    if source == "-" {
        "stdin".into()
    } else if is_url(source) {
        source.to_string()
    } else {
        format!("file {source}")
    }
}

fn is_url(s: &str) -> bool {
    s.starts_with("http://") || s.starts_with("https://")
}

/// Load the article text from a URL, a file, or stdin. HTML sources are reduced
/// to plain text; anything else is used verbatim.
async fn load_source(source: &str) -> Result<String, String> {
    if source == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .map_err(|e| format!("reading stdin: {e}"))?;
        return Ok(buf);
    }

    if is_url(source) {
        let body = reqwest::get(source)
            .await
            .map_err(|e| format!("fetching {source}: {e}"))?
            .text()
            .await
            .map_err(|e| format!("reading {source}: {e}"))?;
        return Ok(html_to_text(&body));
    }

    // A local file: strip HTML only when it looks like markup.
    let raw = std::fs::read_to_string(source).map_err(|e| format!("reading {source}: {e}"))?;
    let looks_html =
        source.ends_with(".html") || source.ends_with(".htm") || raw.trim_start().starts_with('<');
    Ok(if looks_html { html_to_text(&raw) } else { raw })
}

/// A pragmatic HTML → text reduction: drop `<script>`/`<style>` bodies, remove the
/// remaining tags, decode the handful of entities that matter for prose, and
/// collapse whitespace. Not a full readability engine — good enough to feed the
/// rewrite, and flagged as a future improvement.
fn html_to_text(html: &str) -> String {
    let without_blocks = strip_blocks(html, "script");
    let without_blocks = strip_blocks(&without_blocks, "style");

    // Remove tags, turning block-level closers into newlines so paragraphs survive.
    let mut out = String::with_capacity(without_blocks.len());
    let mut in_tag = false;
    for ch in without_blocks.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(ch),
            _ => {}
        }
    }

    let decoded = out
        .replace("&nbsp;", " ")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&rsquo;", "'")
        .replace("&ldquo;", "\"")
        .replace("&rdquo;", "\"");

    // Collapse runs of whitespace into single spaces.
    decoded.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Remove `<tag>…</tag>` blocks (used for `<script>`/`<style>`), case-insensitively.
fn strip_blocks(input: &str, tag: &str) -> String {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let lower = input.to_ascii_lowercase();
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0usize;
    while let Some(rel) = lower[cursor..].find(&open) {
        let start = cursor + rel;
        out.push_str(&input[cursor..start]);
        match lower[start..].find(&close) {
            Some(end_rel) => cursor = start + end_rel + close.len(),
            None => {
                cursor = input.len();
                break;
            }
        }
    }
    out.push_str(&input[cursor..]);
    out
}

// ==============================================================================
// Offline TTS provisioning (macOS `say` → mp3)
// ==============================================================================

/// Write a self-contained engine config that makes the app work offline with
/// nothing to install: local Ollama for chat, and a macOS `say`-based `local_tts`
/// adapter for a real voice. Returns the config path, or `None` on non-macOS (the
/// engine then falls back to auto-detection and TTS may be unavailable).
///
/// This lives in the app, not the engine, on purpose: it is the workaround for the
/// "no zero-config local TTS" gap, and keeping it here makes that gap visible.
fn provision_offline_config() -> Result<Option<PathBuf>, String> {
    if !cfg!(target_os = "macos") {
        eprintln!(
            "note: offline TTS auto-setup is macOS-only. Configure a `local_tts` \
             backend (see config.example.toml) or pass --config; chat will still run."
        );
        return Ok(None);
    }

    let dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("mofa-podcast");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;

    // The TTS wrapper: `say` produces AIFF, which we transcode to mp3 with lame
    // (falling back to ffmpeg). Both are common on a dev Mac; if neither is present
    // the wrapper exits non-zero and the engine surfaces a clean TTS error.
    let wrapper = dir.join("say-tts.sh");
    std::fs::write(&wrapper, SAY_TTS_WRAPPER)
        .map_err(|e| format!("writing {}: {e}", wrapper.display()))?;
    make_executable(&wrapper)?;

    // The config: Ollama (auto-discovers pulled chat models) + the say adapter.
    let config = dir.join("config.toml");
    let toml = format!(
        r#"# Auto-generated by mofa-podcast for offline, zero-setup runs.
[[providers]]
name = "ollama"
kind = "ollama"
base_url = "http://127.0.0.1:11434"
priority = 1
cost_tier = "free"

[[providers]]
name = "say-tts"
kind = "local_tts"
base_url = ""
command = "{wrapper}"
args = ["--text-file", "{{text_file}}", "--output", "{{output}}"]
output_format = "mp3"
priority = 1
enabled = true

  [[providers.models]]
  name = "say"
  capability = "tts"
  memory_mb = 64
"#,
        wrapper = wrapper.display(),
    );
    std::fs::write(&config, toml).map_err(|e| format!("writing {}: {e}", config.display()))?;
    Ok(Some(config))
}

/// The `say`→mp3 wrapper script, written verbatim to the cache dir. It reads the
/// engine's `{text_file}` and writes mp3 to `{output}`.
const SAY_TTS_WRAPPER: &str = r#"#!/usr/bin/env bash
# say-tts.sh — a zero-download local TTS for mofa-podcast.
# Usage: say-tts.sh --text-file <path> --output <path.mp3>
set -euo pipefail

text_file=""
output=""
while [ "$#" -gt 0 ]; do
  case "$1" in
    --text-file) text_file="$2"; shift 2 ;;
    --output)    output="$2";    shift 2 ;;
    *)           shift ;;
  esac
done

if [ -z "$text_file" ] || [ -z "$output" ]; then
  echo "say-tts: missing --text-file or --output" >&2
  exit 2
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
aiff="$work/speech.aiff"

say -f "$text_file" -o "$aiff"

if command -v lame >/dev/null 2>&1; then
  lame --quiet "$aiff" "$output"
elif command -v ffmpeg >/dev/null 2>&1; then
  ffmpeg -y -loglevel error -i "$aiff" "$output"
else
  echo "say-tts: need 'lame' or 'ffmpeg' to make mp3 (brew install lame)" >&2
  exit 3
fi
"#;

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .map_err(|e| format!("stat {}: {e}", path.display()))?
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).map_err(|e| format!("chmod {}: {e}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

// ==============================================================================
// Small formatting helpers
// ==============================================================================

/// Render a cost as a dollar figure, or the local/free case.
fn cost_label(cost: Option<f64>) -> String {
    match cost {
        Some(c) if c > 0.0 => format!("${c:.6}"),
        _ => "$0.00 (local/free)".into(),
    }
}

/// Turn an engine error into a message that includes any failover chain, so a
/// stage failure explains what was tried (e.g. "confidential, no local model").
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
