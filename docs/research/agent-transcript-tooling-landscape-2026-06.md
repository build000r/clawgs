# AI Coding-Agent Transcript Tooling Landscape - June 2026

Research date: 2026-06-01

Scope: AI coding-agent transcript/session observability, local session logs, hook/event streams, and schema consolidation versus fragmentation. General LLM observability, MLOps, and APM are included only where they directly touch local coding-agent session state.

## Executive Summary

- Verdict: partial consolidation above the agent runtime, continued fragmentation at the local transcript/session edge. OpenTelemetry GenAI is the strongest consolidation force for LLM application traces, but it is still marked development/experimental for GenAI and does not define a complete local coding-agent transcript snapshot contract.
- Claude Code is moving fastest toward first-party local session observability. Official hooks expose lifecycle events, JSON input/output, `session_id`, `transcript_path`, subagent events, HTTP hooks, plugin hooks, and Agent SDK session state as JSONL on the filesystem. That validates the local-transcript layer as a real surface rather than a private accident.
- OpenAI Codex and other coding agents remain fragmented. Codex has a large public repo and official CLI/automation docs, but the public standardization surface is the product/API event model, not a vendor-neutral transcript schema. Cursor, Windsurf, Devin, Amp, Cline, Roo Code, Aider, OpenHands, Continue, SWE-agent, and Goose expose different session models, UI states, or repo-local artifacts.
- Observability platforms are converging on OTel-backed application tracing rather than local transcript normalization. Langfuse, Phoenix, OpenLLMetry/Traceloop, and Braintrust can capture model/tool traces, and Braintrust now has a Claude Code tracing plugin, but those tools tend to require a backend/project and optimize for LLM app traces/evals rather than portable no-server transcript snapshots.
- Strategic implication for `clawgs`: keep the thin contract-layer bet, but add bridges into the consolidating layer. The best near-term move is not to become a dashboard; it is to map `clawgs.v2` / `clawgs.emit.v2` into OTel-compatible spans/events while preserving a local, stable, vendor-neutral snapshot format.

## Section 1 - Schema & Protocol Consolidation

### OpenTelemetry GenAI Semantic Conventions

- Origin / owner: OpenTelemetry semantic-conventions project.
- Scope: attributes, events, metrics, exceptions, model spans, and agent spans for generative AI systems.
- Current adoption evidence: The OpenTelemetry GenAI page lists events, metrics, model spans, and agent spans, but marks the GenAI conventions as `Development` and documents an opt-in transition plan for latest experimental conventions rather than a stable default. Source: [OpenTelemetry GenAI semantic conventions](https://opentelemetry.io/docs/specs/semconv/gen-ai/), accessed 2026-06-01.
- Adoption signal: `open-telemetry/semantic-conventions` had 588 GitHub stars, 361 forks, and a push on 2026-06-01T23:30:35Z via GitHub API, accessed 2026-06-01.
- Trajectory: growing, but not settled for local coding-agent transcripts.
- Relevance to `clawgs`: high as an export/interoperability target. OTel can represent spans/events for model calls, tools, and agent operations, but it is not itself a compact transcript-normalization schema for Claude/Codex JSONL sessions.
- Gap: no canonical representation found for local session boundaries, terminal panes, transcript file identity, pending user action, per-session deltas, or a stable no-server snapshot.

### Model Context Protocol (MCP)

- Origin / owner: Model Context Protocol project, originally associated with Anthropic and now broadly supported across clients.
- Scope: connecting AI applications to external tools, data sources, workflows, prompts, and apps.
- Current adoption evidence: MCP docs describe it as an open-source standard for connecting AI applications to external systems and claim broad support across Claude, ChatGPT, VS Code, Cursor, and others. Source: [MCP introduction](https://modelcontextprotocol.io/docs/getting-started/intro), accessed 2026-06-01.
- Adoption signal: `modelcontextprotocol/modelcontextprotocol` had 8,289 GitHub stars, 1,562 forks, and a push on 2026-06-01T22:03:24Z via GitHub API, accessed 2026-06-01.
- Trajectory: growing quickly as a tool/context protocol.
- Relevance to `clawgs`: medium. MCP tools show up in Claude Code hook events and can be observed as tool calls, but MCP standardizes tool connectivity, not local coding-agent transcript storage.
- Gap: MCP does not replace transcript normalization; it can feed the event stream that a normalizer captures.

### Claude Code Hooks, Plugins, and Agent SDK

- Origin / owner: Anthropic / Claude Code.
- Scope: lifecycle hook events, JSON handler inputs/outputs, command/HTTP/prompt/agent hooks, plugin hooks, and Agent SDK session execution.
- Current adoption evidence: Claude Code hooks fire for session, turn, tool, subagent, task, file/worktree, compaction, and session-end lifecycle points. Official docs state that command hooks receive JSON on stdin and HTTP hooks receive the same JSON in the POST body. Hook common fields include session metadata, and event examples include `transcript_path`. Sources: [Claude Code hooks reference](https://code.claude.com/docs/en/hooks), accessed 2026-06-01.
- First-party transcript evidence: Subagent hooks include the main `transcript_path`, a nested `agent_transcript_path`, and `last_assistant_message`; the Agent SDK comparison says local SDK session state is JSONL on the filesystem, while Managed Agents use Anthropic-hosted event logs. Sources: [Claude Code hooks reference](https://code.claude.com/docs/en/hooks), [Claude Code Agent SDK overview](https://code.claude.com/docs/en/agent-sdk/overview), accessed 2026-06-01.
- Trajectory: growing and directly relevant.
- Relevance to `clawgs`: very high. Anthropic is exposing local session observability surfaces, but they are Claude-specific. This strengthens the need for a normalization layer across Claude, Codex, and other agents.
- Gap: no vendor-neutral cross-agent schema; the hook model is a product API and includes product-specific event names and paths.

### OpenAI Responses API and Codex

- Origin / owner: OpenAI.
- Scope: Responses API typed streaming events, tool calls, Codex CLI/app/automation surfaces.
- Current adoption evidence: OpenAI docs describe the Responses API as using typed semantic streaming events and list examples such as response created/in progress/completed, output item added/done, content part added/done, text deltas, function-call argument deltas, file-search events, and code-interpreter events. Source: [OpenAI streaming responses guide](https://developers.openai.com/api/docs/guides/streaming-responses), accessed 2026-06-01.
- Codex evidence: The OpenAI Codex repo describes Codex CLI as a local coding agent; official docs list Codex CLI, SDK, app server, MCP server, GitHub Action, and a cookbook for an agent improvement loop with traces/evals. Sources: [openai/codex](https://github.com/openai/codex), [Codex CLI docs](https://developers.openai.com/codex/cli), accessed 2026-06-01.
- Adoption signal: `openai/codex` had 87,655 GitHub stars, 12,854 forks, and a push on 2026-06-01T23:30:22Z via GitHub API, accessed 2026-06-01.
- Trajectory: growing, with strong OpenAI first-party momentum.
- Relevance to `clawgs`: high for Codex support, medium for standardization. Responses API events are a typed runtime/API event stream, but they do not standardize the local Codex CLI session JSONL transcript as a cross-vendor format.
- Gap: local transcript/session files remain product-specific.

### Langfuse Data Model and OTel Ingestion

- Origin / owner: Langfuse.
- Scope: LLM application traces, sessions, observations, prompt management, evals, and OTel ingestion.
- Current adoption evidence: Langfuse docs define traces that capture prompts, responses, token usage, latency, tools, and retrieval steps; docs also state Langfuse is open source and self-hostable. The OTel integration page says Langfuse can receive traces on `/api/public/otel`, maps evolving OTel GenAI attributes into the Langfuse data model, and recommends SDKs for Python/JS. Sources: [Langfuse observability overview](https://langfuse.com/docs/observability/overview), [Langfuse OpenTelemetry integration](https://langfuse.com/integrations/native/opentelemetry), accessed 2026-06-01.
- Adoption signal: `langfuse/langfuse` had 28,324 GitHub stars, 2,920 forks, and a push on 2026-06-01T18:43:26Z via GitHub API, accessed 2026-06-01.
- Trajectory: growing as an LLM application observability backend.
- Relevance to `clawgs`: medium-high as an export target; low as a direct substitute for a no-server local transcript normalizer.
- Gap: Langfuse assumes instrumentation/export into a backend/project, not direct normalization of private Claude/Codex JSONL into a portable local contract.

## Section 2 - Agent Runtime Landscape

| Agent | Owner / community | Structured log or event surface | Open/proprietary | Session-state machine-readability | Adoption signal as of 2026-06-01 | Direct compatibility implication for `clawgs` |
| --- | --- | --- | --- | --- | --- | --- |
| Claude Code | Anthropic | Official hooks with JSON event payloads, transcript paths, subagent paths, Agent SDK JSONL session state | Proprietary product with documented hooks/SDK | High for Claude-specific local sessions | Official docs expose rich lifecycle hooks and filesystem JSONL session state | Must remain first-class; likely best source for hook-fed live events |
| OpenAI Codex CLI | OpenAI | Local CLI repo, Responses API typed events, Codex automation docs; local transcript schema remains product-specific | Open-source repo / proprietary service surfaces | Medium-high for Codex-specific sessions | `openai/codex`: 87,655 stars; latest release visible on GitHub page as 0.136.0 on Jun 1, 2026 | Must remain first-class; map Codex session files and API event shapes separately |
| Cursor | Anysphere | Background Agent and API docs; public local transcript schema not found in this pass | Proprietary | Medium for product state, low for portable local logs | Official docs found for Background Agent/API, but no public transcript contract verified | Treat as future adapter; likely needs reverse engineering or official export/API |
| Windsurf Cascade | Cognition/Windsurf | Cascade tool calls, todos, checkpoints, MCP, hooks, simultaneous cascades in official docs | Proprietary | Medium for product UI/events, low for portable local logs | Official docs describe tool calling, checkpoints, worktrees, hooks, and sharing conversations | Candidate adapter if hooks expose usable event JSON; avoid assuming stable log files |
| Devin | Cognition | Web/CLI/API with session insights, shell, IDE, browser, and feedback logs | Proprietary | Medium for hosted/workspace sessions, low for local no-server logs | Official docs describe CLI, API, Session Insights, shell output, IDE, browser | Competitive for full platform workflows, not a direct local transcript normalizer |
| Aider | Aider-AI community/company | Git-based CLI workflows; structured transcript standard not verified | Open source | Medium-low for machine-readable session state | `Aider-AI/aider`: 45,644 stars, 4,525 forks, pushed 2026-05-22 | Candidate adapter; likely different CLI/log model than Claude/Codex |
| Cline | Cline | VS Code extension agent; internal state/log format not verified | Open source extension | Medium-low for portable logs | `cline/cline`: 62,609 stars, 6,583 forks, pushed 2026-06-01 | Important adoption target, but likely VS Code-extension-specific |
| Roo Code | Roo Code community/company | VS Code extension agent; internal state/log format not verified | Open source extension | Medium-low for portable logs | `RooCodeInc/Roo-Code`: 24,181 stars, 3,284 forks, pushed 2026-05-15 | Similar adapter class to Cline |
| Continue.dev | Continue | IDE assistant/agent framework; logs/schema not verified | Open source | Medium-low for portable session logs | `continuedev/continue`: 33,482 stars, 4,594 forks, pushed 2026-06-01 | Integration target if it exposes chat/session state through extension APIs |
| OpenHands | OpenHands | Agent platform with own runtime, cloud/local surfaces; transcript contract not verified | Source-available/open-source mix per repo | Medium inside its platform, low for cross-vendor local logs | `OpenHands/OpenHands`: 75,599 stars, 9,584 forks, pushed 2026-06-01 | More competitor/platform than simple adapter; may need platform-specific exporter |
| SWE-agent | SWE-agent | Research/benchmark-oriented software agent; structured traces likely internal but not verified here | Open source | Medium for benchmark runs, low for local dev transcript standard | `SWE-agent/SWE-agent`: 19,387 stars, 2,113 forks, pushed 2026-06-01 | Useful benchmark/harness adapter, not central local CLI wedge |
| Goose | aaif-goose | On-machine extensible AI agent; structured transcript standard not verified | Open source | Medium-low | `aaif-goose/goose`: 46,175 stars, pushed 2026-06-01 | Candidate local adapter if logs are accessible |
| Amp | Sourcegraph | Terminal coding agent; public Open CLI listing says local CLI via npm, official logs not verified | Proprietary/service-backed CLI | Low-medium | Open CLI lists `@sourcegraph/amp` and docs/site links | Track, but do not prioritize until public session export/log evidence exists |

## Section 3 - Hook and Observability Ecosystem Health

### Claude Code hook ecosystem

- The official hook surface is now broad enough to be considered first-party observability infrastructure. It covers session start/end, prompts, assistant display, tool calls, permission events, subagents, task lifecycle, file/worktree changes, compaction, and MCP elicitation. Source: [Claude Code hooks reference](https://code.claude.com/docs/en/hooks), accessed 2026-06-01.
- Hooks are still product-specific and handler-oriented. They are a way to run commands/HTTP endpoints or inject decisions/context, not a standalone neutral transcript schema.
- `disler/claude-code-hooks-multi-agent-observability` remains a useful proof that developers want Claude Code monitoring: 1,442 stars, 377 forks, last pushed 2026-02-08T23:59:13Z, license not detected by GitHub API, accessed 2026-06-01. It is not a dominant standard and appears tied to Claude Code hook mechanics.
- The historical fragility class remains relevant because hooks execute scripts and parse JSON. Official Claude docs now document JSON input/output, exit codes, HTTP hooks, and hook debugging, which helps, but each third-party hook pipeline can still fail independently.

### Braintrust Claude Code plugin

- Braintrust has a direct Claude Code integration. Docs instruct users to add the `braintrustdata/braintrust-claude-plugin` marketplace, install `trace-claude-code`, enable `TRACE_TO_BRAINTRUST`, and then trace Claude Code sessions as hierarchical spans with session root, turns, and tool calls. Source: [Braintrust Claude Code integration](https://www.braintrust.dev/docs/integrations/developer-tools/claude-code), accessed 2026-06-01.
- This is the clearest competitive signal against `clawgs`: a funded observability/evals vendor is tracing Claude Code directly.
- Threat boundary: it sends traces to Braintrust and is Claude Code-specific. It does not appear to define a no-server, vendor-neutral local snapshot format.

### OpenTelemetry-backed LLM observability

- Langfuse, OpenLLMetry/Traceloop, and Phoenix are active and OTel-aligned. Langfuse can ingest OTel traces and maps evolving OTel GenAI attributes; Traceloop emits standard OTLP HTTP; Phoenix is built on OpenTelemetry/OpenInference and accepts traces over OTLP. Sources: [Langfuse OTel integration](https://langfuse.com/integrations/native/opentelemetry), [Traceloop OTel collector integration](https://docs.traceloop.com/docs/openllmetry/integrations/otel-collector), [Phoenix overview](https://arize.com/docs/phoenix), accessed 2026-06-01.
- These tools capture model calls, retrieval, tool use, and custom logic inside LLM applications. That overlaps with agent traces but not with local transcript discovery, terminal/tmux session state, or product-specific JSONL normalization.

## Section 4 - Platform Consolidation Signals

- Strong consolidation: OpenTelemetry as the lingua franca for LLM application telemetry. Langfuse, OpenLLMetry, Traceloop, Phoenix, and OTel GenAI all point toward OTLP-compatible traces.
- Weak consolidation: local coding-agent transcript/session state. Claude Code, Codex CLI, Cursor, Windsurf, Devin, Cline/Roo, Aider, OpenHands, and Goose all expose different workflows and runtime surfaces.
- Direct platform threat: Braintrust's Claude Code plugin. It traces Claude Code sessions as spans and tool calls. This squeezes a Claude-only observability product, but it does not remove the cross-agent local contract wedge.
- Local-first signal: Phoenix is open-source and can run locally; Langfuse is self-hostable; Traceloop routes to any OTel collector. However, these are still observability backends/instrumentation stacks, not tiny transcript normalizers.
- 6-12 month absorption assessment: medium risk for Claude Code-only tracing, low-to-medium risk for a vendor-neutral no-server normalization layer. The more platforms standardize on OTel export, the more valuable a local transcript-to-OTel bridge becomes.

## Section 5 - Distribution and Discovery

Evidence-backed ranking for this category:

1. GitHub repository discovery and social proof. Star counts, recent pushes, issues, releases, and README quality are the clearest public adoption signals for Cline, OpenHands, Codex, Aider, Continue, Roo Code, Langfuse, Phoenix, OpenLLMetry, and similar tools.
2. Official docs and extension/plugin marketplaces. Claude Code plugins, VS Code/Open VSX extensions, Cursor/Windsurf docs, and Codex docs are where users learn first-party extension points.
3. Package-manager install paths. npm, Homebrew, curl installers, and CLI docs matter for conversion after discovery. Codex documents curl, npm, Homebrew cask, and GitHub release installs. Source: [openai/codex README](https://github.com/openai/codex), accessed 2026-06-01.
4. Community channels. Hacker News, Reddit, Discord, X, and YouTube can drive awareness, but claims from those channels should be treated as qualitative unless backed by primary repo/docs data.
5. crates.io. For a Rust CLI, crates.io is credible installation infrastructure but weak category discovery on its own. A `cargo install` path is necessary for Rust users, but the adoption funnel likely starts in GitHub/docs/social/agent marketplaces, not crates.io search.

Implication: keep crates.io polished, but market/distribute `clawgs` through GitHub README examples, schema docs, Claude Code/Codex-specific guides, and OTel/Langfuse/Braintrust/Phoenix interoperability examples.

## Section 6 - Direct Competitive Threats

| Tool/project | Origin | What it does | Threat level | Evidence | Reasoning for `clawgs` |
| --- | --- | --- | --- | --- | --- |
| Braintrust Claude Code plugin | Braintrust | Traces Claude Code sessions to Braintrust as hierarchical spans with session root, turns, and tool calls | Medium | Official Braintrust docs, accessed 2026-06-01 | Strong Claude-specific substitute for hosted tracing; not a no-server cross-agent snapshot schema |
| Claude Code hooks/plugins/Agent SDK | Anthropic | First-party lifecycle hooks, plugin hooks, filesystem JSONL session state in Agent SDK | Medium | Official Claude Code docs, accessed 2026-06-01 | Makes Claude observability easier and may reduce need for reverse engineering; still product-specific and creates adapter opportunity |
| OpenTelemetry GenAI | OpenTelemetry | Standardizes GenAI traces/events/metrics/spans | Medium | Official OTel docs mark GenAI development, accessed 2026-06-01 | Strong export target; not direct local transcript schema |
| Langfuse | Langfuse | OTel-backed LLM app tracing, sessions, observations, evals, self-hosting | Medium-low | Official docs + GitHub API, accessed 2026-06-01 | Absorbs users who want a platform/backend; complements local normalizer if `clawgs` exports traces |
| Arize Phoenix | Arize | Open-source local/cloud AI tracing/evals built on OTel/OpenInference | Medium-low | Official Phoenix docs + GitHub API, accessed 2026-06-01 | Good local observability option, but still app instrumentation rather than transcript parsing |
| OpenLLMetry / Traceloop | Traceloop | OTel instrumentations for LLM apps | Low-medium | Official docs + GitHub API, accessed 2026-06-01 | Consolidates trace transport; no evidence of coding-agent transcript normalization |
| disler Claude hook observability | Community | Real-time Claude Code hook monitoring/dashboard | Low-medium | GitHub API, accessed 2026-06-01 | Validates demand; limited to Claude hooks and not a broad standard |
| Codex/OpenAI first-party automation | OpenAI | Codex CLI, SDK, app server, MCP server, typed API events | Medium | Official Codex docs + GitHub API, accessed 2026-06-01 | Could expose better first-party export later; today remains product-specific |
| Small transcript viewers/exporters | Community | Convert Claude/Codex transcripts to HTML/Markdown or dashboards | Low | GitHub search found low-star `cc-session-viewer`, `codex-transcripts`, and related repos, accessed 2026-06-01 | Mostly presentation/export, not stable machine contracts |

## Strategic Implications

- Keep `clawgs` narrow, but add OTel export. The market is consolidating around OTel for trace transport, so `clawgs` should become a local transcript/session source that can emit OTel-compatible spans/events without requiring a backend.
- Treat Claude Code hooks as an official input path, not a competitor to avoid. Add a documented hook configuration that feeds `clawgs.emit.v2` or a future append-only event stream, while continuing to support transcript discovery.
- Expand adapters selectively by evidence of accessible session state. Highest-priority next adapters: Cline/Roo Code or Continue if their extension state/logs are accessible; Cursor/Windsurf only when an official API/export or stable local artifact is found.
- Do not become a hosted observability product. Langfuse, Braintrust, Phoenix, and Traceloop already compete there. `clawgs` should be the small local bridge that can feed those systems.
- Publish the schema story clearly. The differentiator is "stable, versioned, local, vendor-neutral snapshots and live NDJSON status," not "another dashboard."

## Uncertainty Flags

- [LOW CONFIDENCE] Cursor local transcript/session storage format. Official docs were found for Background Agents/API, but this pass did not verify a stable local log contract.
- [LOW CONFIDENCE] Windsurf Cascade hook payloads. Official Cascade docs mention hooks and tool calling, but this pass did not inspect a complete hook schema.
- [LOW CONFIDENCE] Amp session/log export. Public Open CLI metadata confirms a local CLI install path, but official Sourcegraph/Amp docs were not fully validated in this pass.
- [INFERRED] crates.io is weak as discovery for AI coding-agent tools. This follows from observed discovery surfaces and common CLI adoption patterns, but no quantitative crates.io funnel data was found.
- [NOT FOUND] A dominant vendor-neutral schema specifically for local coding-agent transcript/session snapshots.
- [NOT FOUND] Evidence that Langfuse, Phoenix, OpenLLMetry, or Traceloop directly ingest Claude Code/Codex local JSONL transcripts without a custom adapter, excluding Braintrust's Claude Code-specific plugin.
- [NOT FOUND] A well-resourced first-party product that directly substitutes for a no-server, cross-agent transcript normalization layer.

## Source Ledger

Official docs and standards:

- OpenTelemetry GenAI semantic conventions: <https://opentelemetry.io/docs/specs/semconv/gen-ai/>. Accessed 2026-06-01.
- Model Context Protocol introduction: <https://modelcontextprotocol.io/docs/getting-started/intro>. Accessed 2026-06-01.
- Claude Code hooks reference: <https://code.claude.com/docs/en/hooks>. Accessed 2026-06-01.
- Claude Code Agent SDK overview: <https://code.claude.com/docs/en/agent-sdk/overview>. Accessed 2026-06-01.
- OpenAI Responses API streaming guide: <https://developers.openai.com/api/docs/guides/streaming-responses>. Accessed 2026-06-01.
- OpenAI Codex CLI docs: <https://developers.openai.com/codex/cli>. Accessed 2026-06-01.
- OpenAI Codex GitHub repo: <https://github.com/openai/codex>. Accessed 2026-06-01.
- Langfuse observability overview: <https://langfuse.com/docs/observability/overview>. Accessed 2026-06-01.
- Langfuse OpenTelemetry integration: <https://langfuse.com/integrations/native/opentelemetry>. Accessed 2026-06-01.
- Langfuse data model: <https://langfuse.com/docs/observability/data-model>. Accessed 2026-06-01.
- Traceloop OpenLLMetry OTel collector integration: <https://docs.traceloop.com/docs/openllmetry/integrations/otel-collector>. Accessed 2026-06-01.
- Arize Phoenix overview: <https://arize.com/docs/phoenix>. Accessed 2026-06-01.
- Braintrust Claude Code integration: <https://www.braintrust.dev/docs/integrations/developer-tools/claude-code>. Accessed 2026-06-01.
- Windsurf Cascade docs: <https://docs.windsurf.com/windsurf/cascade/cascade>. Accessed 2026-06-01.
- Devin docs: <https://docs.devin.ai/get-started/devin-intro>. Accessed 2026-06-01.

GitHub API snapshots, accessed 2026-06-01:

- `openai/codex`: 87,655 stars; 12,854 forks; pushed 2026-06-01T23:30:22Z; Apache-2.0.
- `Aider-AI/aider`: 45,644 stars; 4,525 forks; pushed 2026-05-22T14:02:20Z; Apache-2.0.
- `cline/cline`: 62,609 stars; 6,583 forks; pushed 2026-06-01T23:22:06Z; Apache-2.0.
- `RooCodeInc/Roo-Code`: 24,181 stars; 3,284 forks; pushed 2026-05-15T18:08:47Z; Apache-2.0.
- `continuedev/continue`: 33,482 stars; 4,594 forks; pushed 2026-06-01T21:46:42Z; Apache-2.0.
- `OpenHands/OpenHands`: 75,599 stars; 9,584 forks; pushed 2026-06-01T22:33:31Z; license `NOASSERTION`.
- `SWE-agent/SWE-agent`: 19,387 stars; 2,113 forks; pushed 2026-06-01T22:29:37Z; MIT.
- `aaif-goose/goose`: 46,175 stars; pushed 2026-06-01T23:29:14Z.
- `langfuse/langfuse`: 28,324 stars; 2,920 forks; pushed 2026-06-01T18:43:26Z; license `NOASSERTION`.
- `traceloop/openllmetry`: 7,162 stars; 983 forks; pushed 2026-05-31T08:10:30Z; Apache-2.0.
- `Arize-ai/phoenix`: 9,949 stars; 905 forks; pushed 2026-06-01T23:31:01Z; license `NOASSERTION`.
- `modelcontextprotocol/modelcontextprotocol`: 8,289 stars; 1,562 forks; pushed 2026-06-01T22:03:24Z; license `NOASSERTION`.
- `open-telemetry/semantic-conventions`: 588 stars; 361 forks; pushed 2026-06-01T23:30:35Z; Apache-2.0.
- `Helicone/helicone`: 5,766 stars; 592 forks; pushed 2026-05-18T23:17:57Z; Apache-2.0.
- `disler/claude-code-hooks-multi-agent-observability`: 1,442 stars; 377 forks; pushed 2026-02-08T23:59:13Z; no license detected by GitHub API.

Research execution notes:

- Deep Research prompt written to `/tmp/clawgs-agent-transcript-market-deep-research-2026-06-01.md`.
- Oracle route guard command: `node /Users/b/repos/opensource/skills/deep-research-prompt/assets/scripts/check-oracle-tab-local-route.mjs`.
- Route guard result: safe; Oracle exposes target-selection or pre-submit hook support.
- Oracle command launched: `oracle --engine browser --remote-chrome 127.0.0.1:9222 --browser-model-strategy ignore --pre-submit-hook "node /Users/b/repos/opensource/skills/deep-research-prompt/assets/scripts/toggle-deep-research.mjs" --timeout 30m --slug "clawgs transcript market" -p "$(cat /tmp/clawgs-agent-transcript-market-deep-research-2026-06-01.md)"`.
- Oracle session slug: `clawgs-transcript-market`.
- Oracle result: the browser session completed, but the captured answer was only `Pro thinking`, not a Deep Research dossier.
- Watcher output target attempted: `/tmp/clawgs-agent-transcript-market-deep-research-result.md`.
- Watcher result: after the required 30-minute delay, a `--no-initial-delay` capture attempt failed with exit 8 because no `chatgpt.com` tab on `127.0.0.1:9222` matched the original target conversation `6a1e15bf-5f3c-83e8-8463-8e4a0feece38`.
- Report basis: direct current-source research from official docs, standards pages, and GitHub API snapshots, not a completed Oracle/Deep Research dossier.
