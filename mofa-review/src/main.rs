//! mofa-review — a real app: a git diff in, an annotated review report out.
//!
//!   mofa-review                      # review the working-tree diff, locally
//!   mofa-review --staged             # review what's staged
//!   mofa-review --range HEAD~3..HEAD # review a commit range
//!   git diff | mofa-review -         # review a piped diff
//!
//! It feeds the diff to the MoFA engine as a high-effort reasoning request and
//! **streams the review as it is written** — the model's thought chain
//! (`Reasoning` chunks) shown separately from the final annotated report (`Text`
//! chunks) — then saves the report as Markdown.
//!
//! The whole thing runs **on-device** by default (`prefer=local`): your unreleased
//! code never leaves the machine, and a `--max-cost` ceiling keeps any cloud
//! fallback cheap. That privacy + cost story is the point — a plain cloud code
//! reviewer can offer neither.

use std::io::Write;
use std::path::PathBuf;

use clap::Parser;
use mofa_engine_core::{Engine, EngineConfig};
use mofa_kernel::{
    Capability, InferenceRequest, Message, Prefer, Reasoning, ReasoningEffort, StreamChunk,
};

/// Review a git diff / PR locally, producing an annotated report.
#[derive(Parser, Debug)]
#[command(name = "mofa-review", version, about)]
struct Cli {
    /// Diff source: a file path, or `-` for stdin. Omit to use git (see flags).
    source: Option<String>,

    /// Review the staged diff (`git diff --staged`).
    #[arg(long, conflicts_with_all = ["range", "source"])]
    staged: bool,

    /// Review a commit range or revision, e.g. `HEAD~3..HEAD` (`git diff <range>`).
    #[arg(long, conflicts_with_all = ["staged", "source"])]
    range: Option<String>,

    /// Run git in this repository directory (default: current directory).
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Reasoning effort → routes to a cheaper or stronger tier.
    #[arg(long, default_value = "medium", value_parser = ["low", "medium", "high"])]
    effort: String,

    /// Locality preference. `local` (default) keeps the code on-device.
    #[arg(long, default_value = "local", value_parser = ["auto", "local", "cloud"])]
    prefer: String,

    /// Per-review spend ceiling in USD (a pricier cloud model is priced out).
    #[arg(long, default_value_t = 0.05)]
    max_cost: f64,

    /// Chat/reasoning model override (default: routed local-first).
    #[arg(long)]
    model: Option<String>,

    /// Where to write the report.
    #[arg(short, long, default_value = "review_report.md")]
    out: PathBuf,

    /// Use a specific engine config instead of the auto-provisioned offline one.
    #[arg(short, long)]
    config: Option<PathBuf>,
}

/// Large diffs blow past a small local model's context, so we cap what we send and
/// note the truncation in the prompt.
const MAX_DIFF_CHARS: usize = 12_000;

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
    // 1. Get the diff (git, a file, or stdin)
    // ==========================================================================
    let (diff, origin) = load_diff(&cli).await?;
    if diff.trim().is_empty() {
        return Err("no changes to review — stage something, pass --range, or pipe a diff".into());
    }
    let (diff, truncated) = clamp(&diff, MAX_DIFF_CHARS);
    println!(
        "→ reviewing {origin} ({} chars{})\n",
        diff.len(),
        if truncated { ", truncated" } else { "" }
    );

    // ==========================================================================
    // 2. Boot the engine (offline config unless --config is given)
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
    let effort = match cli.effort.as_str() {
        "low" => ReasoningEffort::Low,
        "high" => ReasoningEffort::High,
        _ => ReasoningEffort::Medium,
    };

    // ==========================================================================
    // 3. Stream the review (thought chain vs. report)
    // ==========================================================================
    let request = InferenceRequest {
        capability: Some(Capability::Chat),
        model: cli.model.clone(),
        messages: vec![
            Message {
                role: "system".into(),
                content: "You are a senior code reviewer. Review the unified diff the \
                          user sends. Think step by step, then output a concise Markdown \
                          report: a one-line summary, then bulleted findings each tagged \
                          severity (blocker / major / minor / nit) with file:line and a \
                          suggested fix. Be specific; skip praise."
                    .into(),
                ..Default::default()
            },
            Message {
                role: "user".into(),
                content: format!("Review this diff:\n\n```diff\n{diff}\n```"),
                ..Default::default()
            },
        ],
        reasoning: Some(Reasoning {
            effort,
            include: true,
        }),
        prefer,
        // Free/local models estimate to $0 and always fit under the ceiling.
        max_cost_usd: Some(cli.max_cost),
        ..Default::default()
    };

    println!("== review (streaming) ==\n");
    let mut report = String::new();
    let mut meta: Option<(String, String)> = None; // (provider, model)
    let mut in_thoughts = false;
    let mut in_report = false;

    let mut rx = engine.invoke_stream(request);
    while let Some(chunk) = rx.recv().await {
        match chunk {
            StreamChunk::Started {
                provider,
                model_used,
                ..
            } => {
                println!("[routed to {provider}/{model_used}]\n");
                meta = Some((provider, model_used));
            }
            StreamChunk::Reasoning { delta } => {
                if !in_thoughts {
                    print!("🧠 thinking: ");
                    in_thoughts = true;
                }
                print!("{delta}");
                let _ = std::io::stdout().flush();
            }
            StreamChunk::Text { delta } => {
                if !in_report {
                    println!("\n\n📝 report:\n");
                    in_report = true;
                }
                print!("{delta}");
                let _ = std::io::stdout().flush();
                report.push_str(&delta);
            }
            StreamChunk::Completed {
                tokens_used,
                cost_usd,
                ..
            } => {
                let (provider, model) = meta.clone().unwrap_or_default();
                let locality = if provider == "system-tts" || cost_usd.unwrap_or(0.0) == 0.0 {
                    "LOCAL"
                } else {
                    "cloud"
                };
                let cost = cost_usd
                    .map(|c| format!("${c:.6}"))
                    .unwrap_or_else(|| "$0.00 (local/free)".into());
                println!(
                    "\n\n[done — {provider}/{model} · {locality} · tokens {:?} · {cost}]",
                    tokens_used
                );
                write_report(&cli.out, origin.as_str(), &report, &provider, &model)?;
                println!("report saved → {}", cli.out.display());
            }
            StreamChunk::Error(info) => {
                eprintln!("\nerror [{:?}]: {}", info.code, info.message);
                for a in &info.failed_chain {
                    eprintln!("  tried {}/{}: {}", a.provider, a.model, a.reason);
                }
                return Err("review failed".into());
            }
            _ => {}
        }
    }
    Ok(())
}

// ==============================================================================
// Diff acquisition
// ==============================================================================

/// Resolve the diff and a human-readable description of where it came from.
async fn load_diff(cli: &Cli) -> Result<(String, String), String> {
    // Explicit source: stdin or a file.
    if let Some(src) = &cli.source {
        if src == "-" {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .map_err(|e| format!("reading stdin: {e}"))?;
            return Ok((buf, "piped diff".into()));
        }
        let text = std::fs::read_to_string(src).map_err(|e| format!("reading {src}: {e}"))?;
        return Ok((text, format!("file {src}")));
    }

    // Otherwise, ask git.
    if cli.staged {
        return Ok((
            git(cli, &["diff", "--staged"]).await?,
            "staged changes".into(),
        ));
    }
    if let Some(range) = &cli.range {
        return Ok((git(cli, &["diff", range]).await?, format!("range {range}")));
    }

    // Default: the working-tree diff; if it's empty, fall back to the last commit so
    // `mofa-review` on a clean tree still reviews *something* useful.
    let working = git(cli, &["diff"]).await?;
    if !working.trim().is_empty() {
        return Ok((working, "working-tree changes".into()));
    }
    let last = git(cli, &["diff", "HEAD~1", "HEAD"]).await?;
    Ok((last, "last commit (HEAD~1..HEAD)".into()))
}

/// Run `git` in the target repo, returning stdout. A non-zero exit (e.g. not a
/// repo) becomes a clear error.
async fn git(cli: &Cli, args: &[&str]) -> Result<String, String> {
    let out = tokio::process::Command::new("git")
        .arg("-C")
        .arg(&cli.repo)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("failed to run git (is it installed?): {e}"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

/// Truncate to `max` chars on a line boundary, reporting whether it was cut.
fn clamp(text: &str, max: usize) -> (String, bool) {
    if text.len() <= max {
        return (text.to_string(), false);
    }
    let cut = text[..max].rfind('\n').unwrap_or(max);
    (
        format!("{}\n… (diff truncated for review)", &text[..cut]),
        true,
    )
}

// ==============================================================================
// Report output
// ==============================================================================

/// Write the streamed report to Markdown with a small metadata header.
fn write_report(
    out: &PathBuf,
    origin: &str,
    report: &str,
    provider: &str,
    model: &str,
) -> Result<(), String> {
    let doc = format!(
        "# Code review\n\n\
         - **Source:** {origin}\n\
         - **Reviewer:** {provider}/{model} (via MoFA, local-first)\n\n\
         ---\n\n{}\n",
        report.trim()
    );
    std::fs::write(out, doc).map_err(|e| format!("writing {}: {e}", out.display()))
}

// ==============================================================================
// Offline config provisioning
// ==============================================================================

/// Write a minimal offline engine config: local Ollama for chat/reasoning. No TTS
/// or other backends are needed for review.
fn provision_offline_config() -> Result<Option<PathBuf>, String> {
    let dir = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("mofa-review");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;

    let config = dir.join("config.toml");
    let toml = r#"# Auto-generated by mofa-review for offline, on-device review.
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
