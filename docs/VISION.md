# Vision

This document captures the strategic thesis behind `clawgs`: what it is for,
what it is not for, and why it exists despite adjacent tools that already solve
other parts of the problem.

## Mission

Make the live state of Claude Code and Codex sessions machine-readable through
stable contracts, so downstream tools never have to parse raw JSONL themselves.

## Vision

`clawgs` should become the canonical normalization layer between AI coding agent
sessions and anything that wants to know what those sessions are doing right now.
Not a dashboard. Not an analytics platform. A small, correct bridge that turns
private session logs and live tmux panes into a contract other tools can depend
on without coupling to the log format of any single agent.

## Values

### 1. Contract stability over feature breadth

The `clawgs.v2` extract schema and the `clawgs.emit.v2` protocol are the
product. New features are only worth adding if they do not break downstream
consumers. This has caused us to say "no" to fields that would be useful but
unstable.

### 2. Zero-config legibility

A stranger cloning the repo should be able to run `demo extract` and `demo emit`
and understand the output without credentials, private logs, or a running tmux
server. If the demo path stops working from a clean checkout, the project is
broken.

### 3. Honest surface area

`clawgs` is a parser, a protocol, and a tmux bridge. It is not pretending to be
an observability platform, a transcript database, or a hosted service. Scope
creep toward any of those categories should be resisted.

## The Wedge

The defensible wedge is not "AI agent observability" in general.

The wedge is:

> Developers running Claude Code or Codex in tmux who need a stable,
> machine-readable view of session state without writing bespoke jq/rg glue or
> adopting a full observability platform.

That sits between three established buckets:

- **Full observability platforms** (Langfuse, OpenLLMetry, Dify) — powerful but
  heavy, designed for hosted multi-user tracing, not local agent pane polling
- **Hook-based monitoring** (disler/claude-code-hooks-multi-agent-observability)
  — event-driven, Python, focused on real-time dashboards and hook integration;
  1,332 stars but carries its own stability issues (JSON parse crashes blocking
  Bash, no license until recently)
- **Transcript viewers** (codex-transcripts, ai-sessions, codex-transcript-viewer)
  — render logs to HTML for human reading; no stable machine-readable contract,
  no live protocol

`clawgs` exists for the case where all three buckets are close but none is a
clean fit: you want a stable JSON contract from local sessions, you want it live
over NDJSON, and you do not want to deploy a server or adopt a Python event
pipeline.

## Who It Is For

- Solo developers running multiple Claude Code or Codex sessions in tmux who
  want programmatic visibility into what each session is doing
- Tool authors building status bars, dashboards, or notification hooks who need
  a stable upstream contract rather than parsing raw logs
- Anyone who wants a replayable demo of the extract and emit protocol without
  needing live agent credentials

## Who It Is Not For

- Teams needing multi-user, hosted observability with auth and retention
  (use Langfuse)
- Users who want a visual dashboard out of the box (use the hooks observability
  project or build one on top of `clawgs emit`)
- People who just want to read transcripts as HTML (use codex-transcripts or
  ai-sessions)
- Organizations that need OpenTelemetry-native tracing with spans and traces
  (use OpenLLMetry/Traceloop)

## Competitive Fit

| Category | Examples | What they do well | Why they do not replace clawgs |
|----------|----------|-------------------|-------------------------------|
| Full observability platforms | Langfuse (24k stars), OpenLLMetry (7k), Dify (136k) | Hosted tracing, evaluation, prompt management, team collaboration | Heavyweight; require server deployment; not designed for local tmux pane polling or single-developer CLI workflows |
| Hook-based Claude Code monitoring | disler/claude-code-hooks-multi-agent-observability (1.3k stars) | Real-time event streaming via Claude Code hooks; Python ecosystem | No stable extract schema; Python-only; known crash-on-parse bugs that block Bash; no Codex support; no offline demo path |
| Transcript viewers | codex-transcripts, codex-transcript-viewer, ai-sessions | Human-readable HTML rendering of session logs | No machine-readable contract; no live protocol; no tmux bridge; display-only |
| Tmux orchestration | claude-tmux-orchestration, splitmind | Multi-agent session spawning and coordination | Session management, not session state extraction; different problem |
| Agent replay/safety | dreadnode/agent-lens (92 stars) | Replay and interpretability research tooling | Safety/research focus; not a normalizer for live status emission |

## Market Map

Axes:
- `X`: Platform weight (thin CLI tool ... heavy hosted platform)
- `Y`: Live status focus (archival/display ... real-time protocol)

```text
10 |  .    .    .    .    .    .    .    .    .    .
 9 |  .    .    .    .    .    .    .    .    .    .
 8 |  CG   .    .    .    .    .    .    .    .    .
 7 |  .    .    HK   .    .    .    .    .    .    .
 6 |  .    .    .    .    .    .    .    .    .    .
 5 |  .    .    .    .    .    .    .    .   OLM   .
 4 |  .    .    .    .    .    .    .    .    .    .
 3 |  .    .    .    .    .    .    .    .    .   LF
 2 |  .   TV    .    .    .    .    .    .    .    .
 1 |  .    .    .    .    .    .    .    .    .   DY
   + --------------------------------------------------
       1    2    3    4    5    6    7    8    9   10
```

| Label | Project | Read |
|-------|---------|------|
| `CG` | clawgs | Thin CLI, strong live protocol focus |
| `HK` | disler/hooks-observability | Medium weight (Python + hooks), live event focus |
| `TV` | codex-transcripts et al. | Thin, archival/display only |
| `OLM` | traceloop/openllmetry | Heavy SDK, moderate live focus (OTel spans) |
| `LF` | langfuse/langfuse | Heavy hosted platform, archival + eval focus |
| `DY` | langgenius/dify | Heaviest platform, workflow/orchestration focus |

## Evidence From Comparable Repos

Data pulled 2026-04-03 via `gh`.

### 1. Stability and reliability pain in hook-based monitoring

- disler/hooks-observability: "Bug: `send_event.py` exits with code 1 on JSON
  parse error, blocking all Bash operations"
- disler/hooks-observability: "Crashes started appearing once this tool was added"
- disler/hooks-observability: "Please repackage this project as a Claude Code Plugin"

Takeaway: users want monitoring but are getting burned by fragile Python glue
that sits in the critical path of their shell. A Rust binary with a stable
protocol that does not block Bash is a real differentiator.

### 2. Drift detection and session-boundary awareness emerging as a need

- langfuse/langfuse: "RFC: Session-boundary behavioral drift monitoring —
  tracking context compression effects across long-running agents" (3 reactions)
- disler/hooks-observability: "Feature: Behavioral trend analysis — detecting
  agent drift across sessions"

Takeaway: the market is starting to ask for session-aware intelligence, not just
raw event logging. `clawgs` already has shipped session-boundary awareness
through its extract snapshot model, tmux pane reconciliation, and additive
`clawgs.emit.v2` `session_deltas` facts for started/changed/exited/removed
sessions.

### 3. Heavyweight platform fatigue at the small end

- Langfuse has 599 open issues, many about self-hosted deployment complexity,
  Docker security vulnerabilities, Redis regressions, and SSO configuration
- OpenLLMetry has 450 open issues

Takeaway: solo developers and small teams do not want to run a platform to see
what their agents are doing. The gap for a zero-dependency CLI tool is real.

### 4. Codex transcript tooling is fragmented and display-only

- Multiple small repos (5-11 stars each) converting Codex JSONL to HTML
- No stable schema, no live protocol, no Claude support in any of them
- yoavf/ai-sessions stopped shipping updates in Dec 2025

Takeaway: there is interest in making transcripts useful, but nobody has built a
contract layer. They are all rendering to HTML for humans. `clawgs` is the only
tool offering a machine-readable contract that covers both Claude and Codex.

## Build on the Backs of Giants

| Tool | What it does well | How clawgs should use it |
|------|-------------------|--------------------------|
| tmux | Session multiplexing, pane capture, hook events | Already integrated via `tmux-emit` and `tmux-notify` — keep this tight |
| Claude Code hooks system | Event-driven lifecycle callbacks | `clawgs` can be wired as a hook consumer rather than competing with the hooks system; the emit protocol is the stable downstream contract |
| jq / NDJSON ecosystem | Universal JSON line processing | The emit protocol is already NDJSON; any jq pipeline can consume it directly |
| Langfuse / OpenLLMetry | Hosted tracing and evaluation | If users need platform-grade observability, `clawgs` output can be a data source for these platforms rather than replacing them |

## Strategic Non-Goals

`clawgs` should not become:

- **A dashboard or UI** — the protocol is the product; dashboards are for
  downstream consumers to build
- **A hosted service or multi-user platform** — Langfuse owns that space with
  24k stars and platform-level investment
- **A general LLM observability SDK** — OpenLLMetry and Traceloop own the
  OTel-native tracing space
- **A Python event pipeline** — the hooks-observability project already occupies
  that niche, with all its tradeoffs
- **A transcript viewer** — rendering to HTML is solved by multiple small tools;
  the contract layer underneath is the unsolved problem

## Product Test

New work should pass a simple filter:

- Does this make the extract schema or emit protocol more correct, more stable,
  or more useful to downstream consumers?
- Or does it drag the project toward becoming a dashboard, a platform, or a
  general-purpose agent framework?

If the answer looks more like Langfuse or the hooks-observability project than
`clawgs`, the burden of proof should be high.

## Directional Bets

Based on what the landscape research surfaced:

### 1. Become the contract layer other tools compose with

Nobody in the landscape offers a stable, versioned JSON schema for agent session
state. The transcript viewers render to HTML. The hooks project streams raw
events. Langfuse has its own proprietary trace model. `clawgs.v2` and
`clawgs.emit.v2` are the only versioned, documented contracts in this space.
Double down on this: make the schema strict, publish JSON Schema files, and make
it trivial for downstream tools to validate against.

### 2. Ship a Claude Code hook integration

The most-reacted issue on the hooks-observability project is "Please repackage
this project as a Claude Code Plugin." Users want monitoring wired into the hook
lifecycle. `clawgs` can ship a thin hook that feeds session events into
`clawgs emit --stdio`, giving users the stability of a Rust binary instead of
a Python script in the critical path.

### 3. Lean into multi-session reconciliation

The drift-detection RFCs in both Langfuse and the hooks project signal that
users want to understand how sessions evolve over time, not just see snapshots.
`clawgs` already does per-pane reconciliation in `tmux-emit` and now exposes
compact `session_deltas` in `clawgs.emit.v2` so downstream tools can see
started, changed, exited, removed, and transcript-ambiguous sessions without
parsing raw pane text. The next frontier is richer drift analysis over those
facts, not proving the basic boundary contract exists.
