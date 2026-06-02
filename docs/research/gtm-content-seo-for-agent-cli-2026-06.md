# GTM / Content / SEO Research for Clawgs - June 2026

Research date: 2026-06-02 UTC / 2026-06-01 PDT.

Scope: content, SEO, GEO/AI-answer visibility, Show HN, awesome-list, package-manager, and developer-community distribution for `clawgs`. This report intentionally does not repeat the broader agent-transcript tooling landscape in `docs/research/agent-transcript-tooling-landscape-2026-06.md`.

## Executive Summary

- The highest-leverage next move is a single evidence-heavy launch page/blog post titled around **"Claude Code and Codex transcript observability from local JSONL"**, paired with a zero-config `cargo install clawgs && clawgs demo ...` path and a Show HN post. Comparable HN launches for adjacent AI CLI / Claude Code tools show 30-day star gains from roughly +36 to +832, with a practical target of **+75 to +200 stars in 30 days** if the demo and title land.
- The strongest query cluster is not generic "LLM observability"; it is the narrower long-tail around **Claude Code transcripts, hooks, observability, monitoring, analytics, and tmux/Codex sessions**. GitHub search already shows substantial repo-result competition for "claude code hooks" and "claude code monitoring", but much thinner competition for "codex session json" and "agent transcript parser".
- AI-answer/GEO visibility should be treated as a documentation packaging problem: concise definitions, comparison tables, stable schema pages, crawlable examples, clear install commands, dated source claims, and first-party links. Direct small-OSS GEO case studies were not found in this pass, so tactical claims here are marked inferred.
- Awesome-list submissions should wait until the repo has a launch artifact and enough traction to satisfy list criteria. `awesome-rust` explicitly wants >50 stars or >2,000 crate downloads; `awesome-agents` rejects brand-new/no-traction repos; `awesome-claude-code` is highly relevant but currently has an in-progress README structure.
- Disler's Claude Code hooks observability project appears to have won with a vivid demo/video plus a Claude Code hooks-specific promise, not broad SEO. Its main exploitable gaps for `clawgs` are Codex support, local stable schema, Rust CLI distribution, no-server operation, and machine-readable JSON/NDJSON rather than a dashboard-first app.

## Section 1 - Keyword & Query Landscape

Search-volume tools were not available without paid access, so the table uses primary-source proxies: HN Algolia story hits, GitHub repository search counts, and current web result observations. Counts were collected on 2026-06-02 UTC.

| Query cluster | Intent | Proxy demand | Current top / visible result | Difficulty | AI-answer coverage |
| --- | --- | --- | --- | --- | --- |
| `claude code hooks` | Hook setup, automation, monitoring | HN: 400 hits; GitHub repos: 4,901 | `hesreallyhim/awesome-claude-code` and many hook repos | High | Partial |
| `claude code observability` | Trace/monitor Claude Code behavior | HN: 57; GitHub repos: 474 | Anthropic monitoring docs, disler hooks observability, vendor tracing posts | Medium | Partial |
| `claude code monitoring` | Usage/tool/session monitoring | HN: 92; GitHub repos: 2,180 | Anthropic monitoring docs, Claude Code templates, observability guides | Medium-high | Partial |
| `claude code transcript` | Find/read local JSONL sessions | HN: 115; GitHub repos: 611 | `claude-devtools` transcript docs and `simonw/claude-code-transcripts` | Medium | Partial |
| `codex transcript` | Export/read Codex sessions | HN: 305; GitHub repos: 157 | scattered repos; no dominant schema page found | Medium-low | Weak |
| `codex session json` | Machine-readable Codex logs | HN: 166; GitHub repos: 19 | weak current repo results | Low | Weak |
| `agent transcript parser` | Parse agent logs generically | HN: 3; GitHub repos: 10 | very thin; one zero-star exact-match repo surfaced | Low | Weak |
| `tmux agent monitoring` | Track agents running in panes | HN: 7; GitHub repos: 90 | `claude_code_agent_farm`, cmux/ccmux, Reddit tmux plugin posts | Medium-low | Weak |
| `ai coding agent observability` | Broader category exploration | HN: 37; GitHub repos: 261 | vendor observability and agent framework results | Medium | Partial |

Recommended target cluster:

- Primary: **Claude Code / Codex transcript observability**
- Secondary: `Claude Code transcript JSONL parser`, `Codex session JSON parser`, `tmux agent monitoring`, `local AI coding agent status`, `agent transcript schema`, `Claude Code hooks vs transcript parsing`

The SEO opportunity is in owning a narrow "local transcript contract" definition page before the category vocabulary settles. Generic "LLM observability" is already owned by platforms and standards; `clawgs` should not fight there except through comparison/export sections.

## Section 2 - GEO / AI-Search Citation Mechanics

Direct, documented small-OSS cases where a new CLI achieved measurable ChatGPT/Perplexity citation after launch were **[NOT FOUND]**. The actionable mechanics below are therefore a mix of platform documentation and inferred retrieval behavior.

Verified platform mechanics:

- OpenAI's publisher FAQ says pages must be crawlable for ChatGPT search visibility and notes ChatGPT referral URLs include `utm_source=chatgpt.com`, which gives sites a way to measure inbound ChatGPT search traffic. Source: [OpenAI publisher FAQ](https://help.openai.com/en/articles/12627856-publishers-and-developers-faq).
- Google documents that `nosnippet`, `data-nosnippet`, `max-snippet`, and `noindex` controls affect AI Overviews / AI Mode presentation and direct input eligibility. Sources: [Google AI features and your website](https://developers.google.com/search/docs/appearance/ai-overviews), [Google robots meta tags](https://developers.google.com/search/docs/crawling-indexing/robots-meta-tag).
- Perplexity is publicly positioned as an answer engine with citations; publisher/distribution reporting confirms cited-source links are central to the product, but direct ranking factors for OSS repos were not found. Source: [Axios on Perplexity Enterprise Pro](https://www.axios.com/2024/04/23/perplexity-ai-enterprise-search-answer-engine).

Recommended `clawgs` changes for AI answer inclusion:

1. Add a docs page with a direct answer in the first paragraph: "`clawgs` is a local Rust CLI that parses Claude Code and Codex JSONL transcripts into a stable JSON schema and emits live NDJSON status from tmux sessions."
2. Add query-shaped H2s: "Where are Claude Code transcripts stored?", "How do I parse Codex session JSON?", "How do I monitor Claude Code and Codex in tmux?", "What is the difference between hooks and transcript parsing?"
3. Add comparison tables against raw JSONL, disler hooks observability, Claude Code monitoring docs, Langfuse/OTel/Braintrust, and transcript viewers.
4. Make schema pages crawlable and self-contained with examples. AI answer engines need a quotable definition plus the actual JSON shape.
5. Add `utm_source=chatgpt.com` / `perplexity` analytics handling if the project later gets a docs site; for GitHub-only, use GitHub traffic referrers as a rough proxy.

## Section 3 - Show HN Launch Benchmarks

HN scores/comments are from the Hacker News Firebase API. Star counts are from GitHub API current repo data and the GitHub stargazers endpoint with `starred_at`, accessed 2026-06-02 UTC.

| Project | HN post date | HN URL | HN score/comments | Stars before launch | Stars after roughly 30 days | 30-day delta | Notes |
| --- | --- | --- | --- | ---: | ---: | ---: | --- |
| Plandex | 2024-04-03 | [HN 39918500](https://news.ycombinator.com/item?id=39918500) | 304 / 111 | [NOT FETCHED] | [NOT FETCHED] | [NOT FOUND] | Strong framing: terminal AI coding engine, concrete pain, demo link. Current GitHub stars: 15,434. |
| Ell | 2024-08-02 | [HN 41138085](https://news.ycombinator.com/item?id=41138085) | 214 / 84 | 3 | 364 | +361 | Small CLI, Unix/pipes positioning, direct GitHub link. |
| Mysti | 2025-12-23 | [HN 46365105](https://news.ycombinator.com/item?id=46365105) | 216 / 178 | 2 | 834 | +832 | Agent-composition hook into Claude/Codex/Gemini zeitgeist. |
| Ensue Skill | 2025-12-29 | [HN 46426624](https://news.ycombinator.com/item?id=46426624) | 202 / 226 | 1 | 391 | +390 | Title names a felt Claude Code pain: "forgetting everything." |
| Rudel | 2026-03-12 | [HN 47350416](https://news.ycombinator.com/item?id=47350416) | 144 / 86 | 4 | 248 | +244 | Very close category: Claude Code/Codex session analytics plus concrete dataset stats. |
| TUI-use | 2026-04-08 | [HN 47692661](https://news.ycombinator.com/item?id=47692661) | 52 / 37 | 2 | 230 | +228 | Agent + terminal/TUI control. Useful lower-score benchmark. |
| Nori CLI | 2026-01-14 | [HN 46616562](https://news.ycombinator.com/item?id=46616562) | 37 / 9 | 2 | 100 | +98 | Claude Code CLI wrapper, below-front-page-ish but still reached first 100 stars. |
| Term-CLI | 2026-03-04 | [HN 47242297](https://news.ycombinator.com/item?id=47242297) | 9 / 2 | 29 | 65 | +36 | Good failure/lower-bound case: specific but low HN traction. |

Pattern analysis:

- Titles that name a painful state win: "forgetting everything", "session analytics", "control interactive terminal programs", "AI coding engine for complex tasks".
- HN responds to concrete demos and first-hand build narratives. Abstract "observability" is weaker unless paired with "what did my agent do?" or "why did it get stuck?"
- Agent/Codex/Claude specificity is currently a stronger hook than generic "Rust CLI".
- Even low-to-mid HN scores can move a small repo meaningfully if it starts near zero.

Recommended Show HN framing:

- Title: `Show HN: Clawgs - local Claude Code and Codex transcript snapshots for tmux agents`
- First paragraph: "I run multiple Claude Code and Codex sessions in tmux and wanted a stable machine-readable answer to: which agent is active, waiting, stuck, or ready for me? `clawgs` is a Rust CLI that parses local JSONL transcripts into a versioned JSON schema and can emit live NDJSON status without a hosted backend."
- Demo path: show `cargo install clawgs`, `clawgs demo extract --tool codex --pretty`, `clawgs demo emit --pretty`, then one screenshot/GIF of tmux status integration.

## Section 4 - Awesome-List & Package Manager Discovery

| List / directory | URL | Relevance | Last activity / size | Submission criteria | Expected lift | Action |
| --- | --- | --- | --- | --- | --- | --- |
| Awesome Claude Code | <https://github.com/hesreallyhim/awesome-claude-code> | Very high | 45,411 stars; pushed 2026-04-27 | README currently says structure is being reworked; no formal criteria found | [INFERRED] High referral/credibility if accepted | Submit after launch page and demo GIF exist. |
| Awesome Claude Code variant | <https://github.com/jqueryscript/awesome-claude-code> | High | 401 stars; pushed 2026-05-28 | No formal criteria found in root listing | [INFERRED] Medium | Submit as "Tooling" / "Hooks" adjacent entry after public launch. |
| Awesome Agents | <https://github.com/kyrolabs/awesome-agents> | Medium-high | 2,372 stars; pushed 2026-05-31 | Open source, high quality, traction, maintained, clear added value; rejects brand-new/no-traction repos | [INFERRED] Medium | Wait for >50-100 stars or launch proof; then submit under software development / observability. |
| Awesome Rust | <https://github.com/rust-unofficial/awesome-rust> | Medium | 57,665 stars; pushed 2026-05-31 | Explicitly accepts projects with >50 GitHub stars or >2,000 crates.io downloads; alphabetical template required | [INFERRED] Medium-high after threshold | Submit once threshold is met; `clawgs` already has crates.io path. |
| Awesome CLI apps in a CSV | <https://github.com/toolleeo/awesome-cli-apps-in-a-csv> | Medium | 2,512 stars; pushed 2026-04-25 | Add row to `data/apps.csv` or open issue/email; requires homepage or git URL | [INFERRED] Low-medium | Easy early submission after README category copy is final. |

Homebrew vs crates.io:

- Homebrew is better for macOS CLI conversion after discovery because it is a default install habit for many developers and exposes install analytics. Homebrew documents install-on-request analytics and formula event APIs. Sources: [Homebrew Analytics](https://docs.brew.sh/Analytics), [Homebrew formula API](https://formulae.brew.sh/docs/api/).
- `homebrew/core` is not available immediately for `clawgs`: Homebrew's self-submitted notability guidance is much higher than the normal threshold, and new formulae need stable tagged releases and maintainer confidence. Source: [Homebrew Acceptable Formulae](https://docs.brew.sh/Acceptable-Formulae).
- crates.io remains necessary credibility for a Rust CLI but weak as top-of-funnel discovery. Cargo docs describe `cargo search`, and Rust docs describe crates.io as the central registry, but the observed adoption funnel for comparable tools starts with HN/GitHub/community first. Sources: [Cargo search](https://doc.rust-lang.org/beta/cargo/commands/cargo-search.html), [Cargo dependencies guide](https://doc.rust-lang.org/cargo/guide/dependencies.html).

Recommendation: create a tap or documented `cargo install` first; pursue `homebrew/core` after public traction, tags, CI, and ideally non-author demand.

## Section 5 - Dev-Influencer & Community Channel Map

| Channel / account / community | Platform | Audience | Relevance | Evidence | Suggested ask / content |
| --- | --- | --- | --- | --- | --- |
| Hacker News Show HN | HN | Developer generalists | Very high | Comparable AI/CLI launches above produced +36 to +832 stars in 30 days | Launch with concrete pain, demo GIF, schema example, and no marketing copy. |
| r/ClaudeCode / r/ClaudeAI | Reddit | Claude Code practitioners | High | Search results show repeated transcript, hooks, observability, tmux status, and plugin posts | Post after HN with "I built a local transcript parser for Claude Code + Codex" and ask for workflows/log edge cases. |
| r/tmux | Reddit | tmux power users | Medium-high | Multiple recent AI-agent tmux status posts surfaced in search | Post only the tmux status angle, not "AI observability" broadly. |
| Rust users forum / r/rust | Forum/Reddit | Rust developers | Medium | Rust CLI credibility and `cargo install` path; less direct agent pain | Post only after first public traction and with crate ergonomics/perf notes. |
| Claude Code / Anthropic community | Discord/forums | Claude Code builders | High | Official hooks and monitoring docs validate the surface | Ask for transcript formats and hook edge cases; avoid broad self-promo. |
| IndyDevDan / Disler orbit | YouTube/X/GitHub | Claude Code hooks and agent workflow audience | High but relationship-dependent | Disler repo search footprint includes video/podcast pages linked to GitHub project | Ask for feedback only after a tight comparison demo exists. |
| AI-coding X/Twitter | X | Fast-moving AI coding users | Medium | [INFERRED] Common distribution channel; direct measurable evidence not gathered | One demo clip, one benchmark table, link to HN/GitHub. |

Priority sequence: HN first, then r/ClaudeCode/r/ClaudeAI, then awesome-list PRs, then Rust/tmux communities, then direct outreach to specific Claude Code workflow creators with a demo link.

## Section 6 - Competitive Content Teardown: disler/hooks

Verified footprint:

- GitHub repo: <https://github.com/disler/claude-code-hooks-multi-agent-observability>. GitHub API showed 1,443 stars and 377 forks on 2026-06-02 UTC.
- README leads with "Real-time monitoring and visualization for Claude Code agents" and a quick-start dashboard flow. It is app/dashboard/hook-first, with a `.claude` folder copy workflow.
- Search results show a video/podcast footprint around "I Can SEE EVERYTHING: Claude Code Hooks for Multi Agent Observability" and secondary indexed documentation pages.

Stargazer timing from GitHub stargazers API:

| Month | Stars gained |
| --- | ---: |
| 2025-07 | 329 |
| 2025-08 | 92 |
| 2025-09 | 56 |
| 2025-10 | 188 |
| 2025-11 | 124 |
| 2025-12 | 64 |
| 2026-01 | 66 |
| 2026-02 | 272 |
| 2026-03 | 115 |
| 2026-04 | 85 |
| 2026-05 | 47 |
| 2026-06 through Jun 2 | 5 |

Largest verified star days: 2025-07-14 (+82), 2025-07-15 (+56), 2026-02-09 (+48), 2026-02-10 (+41), 2025-07-16 (+35). The exact off-GitHub source of each spike is **[NOT FOUND]**, but the visible content footprint supports the inference that video/demo distribution drove much of the early adoption.

Exploitable gaps for `clawgs`:

- disler is Claude Code hook/dashboard-first; `clawgs` can be **transcript/schema-first** and **Claude + Codex**.
- disler requires copying `.claude` hooks and running a server/client; `clawgs` can show a no-server CLI demo from a clean machine.
- disler is optimized for visualization; `clawgs` can own machine-readable contracts for downstream tools.
- disler is not positioned around tmux status or local JSON/NDJSON protocol stability.

## Prioritized 30-Day GTM Sequence

1. Publish one canonical launch doc: `docs/claude-codex-transcript-observability.md` or a small docs site page.
   - Leverage: gives Google/AI engines a crawlable definition page and gives HN/Reddit something better than README-only.
   - Effort: 0.5-1 day.
   - Metric: page exists, indexed by GitHub/search, includes five query-shaped H2s and schema examples.

2. Add a 30-60 second terminal/tmux demo GIF and make it first-viewport in README.
   - Leverage: HN and Reddit need immediate proof.
   - Effort: 0.5 day.
   - Metric: README shows `cargo install` plus `demo extract`, `demo emit`, and a live/tmux status example.

3. Launch on Show HN.
   - Leverage: strongest verified star-growth channel in this research.
   - Effort: 0.5 day after doc/demo.
   - Metric: target >50 HN points and +75 to +200 GitHub stars in 30 days; stretch +250.

4. Post a technical follow-up to r/ClaudeCode and r/ClaudeAI.
   - Leverage: tight audience and direct intent around transcripts/hooks/observability.
   - Effort: 1-2 hours.
   - Metric: 10+ comments with real workflow examples or edge cases; +25 stars/referrals.

5. Submit to `awesome-claude-code` and `awesome-cli-apps-in-a-csv`.
   - Leverage: durable category backlinks and AI-citation surface.
   - Effort: 1 hour each.
   - Metric: PR opened and accepted or clear maintainer feedback.

6. After crossing 50 stars or 2,000 crate downloads, submit to `awesome-rust`.
   - Leverage: stronger Rust credibility once criteria are met.
   - Effort: 1 hour.
   - Metric: accepted listing under command-line/development tooling.

7. Prepare Homebrew path, but do not lead with it.
   - Leverage: conversion, not discovery.
   - Effort: 0.5-1 day for tap; core later.
   - Metric: tap formula works; `homebrew/core` deferred until notability/non-author demand.

## Evidence Map and Uncertainty Flags

Verified:

- HN scores/comment counts for comparable posts via HN Firebase API.
- 30-day star deltas for Ell, Mysti, Ensue Skill, Rudel, TUI-use, Nori CLI, and Term-CLI via GitHub stargazers API.
- Current GitHub star counts and repo metadata via GitHub API.
- Awesome-list repository stars/activity via GitHub API.
- `awesome-rust` and `awesome-agents` contribution criteria via their `CONTRIBUTING.md`.
- Homebrew formula acceptance constraints and analytics existence via official Homebrew docs.
- OpenAI/Google publisher/crawling controls via official docs.

Inferred:

- Query demand where no public keyword-volume data was available.
- Expected traffic/star lift from awesome-list inclusion.
- AI-answer inclusion tactics beyond platform crawl/control mechanics.
- Disler's exact off-GitHub distribution sources for star spikes.

Not found:

- Public keyword volume for the exact `clawgs` long-tail queries.
- Direct case study of a small OSS CLI achieving measured ChatGPT/Perplexity citation immediately after launch.
- Precise 30-day star deltas for Plandex without a larger GitHub pagination pull.
- Formal contribution criteria for `hesreallyhim/awesome-claude-code`.

## Oracle / Deep Research Execution Notes

- Prompt file: `/tmp/clawgs-gtm-content-seo-deep-research-2026-06-02.md`.
- Prompt sizing command: `oracle --dry-run summary -p "$(cat /tmp/clawgs-gtm-content-seo-deep-research-2026-06-02.md)"`.
- Route guard command: `node /Users/b/repos/opensource/skills/deep-research-prompt/assets/scripts/check-oracle-tab-local-route.mjs`.
- Oracle launch command: `oracle --engine browser --remote-chrome 127.0.0.1:9222 --browser-model-strategy ignore --pre-submit-hook "node /Users/b/repos/opensource/skills/deep-research-prompt/assets/scripts/toggle-deep-research.mjs" --timeout 30m --slug "clawgs gtm seo" -p "$(cat /tmp/clawgs-gtm-content-seo-deep-research-2026-06-02.md)"`.
- Oracle session status at 2026-06-01 17:43 PDT: `clawgs-gtm-seo` running in browser mode.
- First watcher command after the required 30-minute delay: `DEEP_RESEARCH_OUTPUT=/tmp/clawgs-gtm-content-seo-deep-research-result.md node /Users/b/repos/opensource/skills/deep-research-prompt/assets/scripts/await-deep-research.mjs --no-initial-delay`.
- Oracle result: the browser session completed, but the stored answer was only `Pro thinking` and no Deep Research dossier was captured.
- Watcher result: failed with exit 7 because multiple unrelated `chatgpt.com` tabs were present on the DevTools port and none exposed an unambiguous `clawgs` target.
- Completion basis: source-backed fallback research using GitHub API, GitHub stargazers API, HN Firebase/Algolia APIs, official awesome-list contribution files, official Homebrew/Cargo/OpenAI/Google docs, and current web search observations. The Oracle limitation is explicit, and no uncited Oracle claims are used in this report.
