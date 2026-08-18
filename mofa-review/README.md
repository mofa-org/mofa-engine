# mofa-review

Review a git diff / PR **on your own machine** and get an annotated Markdown
report — driven by the MoFA engine. Local-first by default: your unreleased code
never leaves the laptop, and a `--max-cost` ceiling keeps any cloud fallback cheap.

```
mofa-review                          # review the working-tree diff
mofa-review --staged                 # review staged changes
mofa-review --range HEAD~3..HEAD     # review a commit range
git diff | mofa-review -             # review a piped diff
mofa-review ./some.patch             # review a diff file
```

## 5-minute run (offline)

```bash
ollama pull qwen2.5:0.5b        # any chat/reasoning model (bigger = better reviews)
cargo run -p mofa-review -- --out review_report.md
```

The diff is sent as a reasoning request and the review **streams as it's written**:
with a reasoning model (e.g. a distilled R1) the thought chain (`Reasoning` chunks)
prints separately from the final report (`Text` chunks); other models just stream
the report. The result is saved to `review_report.md`.

## Options

- `--effort low|medium|high` — routes to a cheaper or stronger reasoning tier.
- `--prefer local|auto|cloud` — `local` (default) keeps code on-device.
- `--max-cost <usd>` — a pricier cloud model is priced out (default `0.05`).
- `--model <id>` / `--config <path>` — override routing or the engine config.

## Why local matters

A cloud code reviewer can't offer this: the diff of your unreleased code stays on
the machine (`prefer=local`), the run is free, and the routing/cost is visible per
review. That privacy + cost story is the point of running review through MoFA.
