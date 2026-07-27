# Viral Product Score No-Ragrets Plan

Date: 2026-07-04

Source bead: `clawgs-viral-product-score-flywheel-njl`

`/viral-product-score` and `/dueling-idea-wizards` are not exposed in this
agent lane, so this is the repo-grounded fallback deliverable: a compact score
read, the strongest product bets, and executable Beads to push each score
toward perfect.

## Product Read

`clawgs` turns Claude Code and Codex transcript logs into stable
`clawgs.v2` snapshots and emits the live `clawgs.emit.v2` NDJSON status
protocol. The public wedge is already clear: zero-config demos, sanitized
fixtures, schema references, a tmux bridge, and a small Rust binary instead of a
hosted observability platform.

The strongest viral path is not a generic "agent observability" story. It is a
developer-to-developer proof loop: a maintainer can run one command, see a clean
snapshot/protocol exchange, paste a compact contract receipt into an issue or
README, and downstream tool authors can copy the same integration shape without
touching private transcripts.

## Scorecard

| Dimension | Current | Target | Why |
|---|---:|---:|---|
| First-use wow | 7 | 10 | `demo extract` and `demo emit` are strong, but the repo needs a screenshotable proof path that shows why stable agent-state contracts beat raw JSONL. |
| Share loop | 4 | 9 | The project has schemas and examples, but lacks a tiny shareable artifact that proves a session snapshot or emit update without leaking private logs. |
| Activation friction | 6 | 9 | Source install and clean-install proof are documented. The README still says crates.io `0.2.0` has a portability gap until `0.3.0` is published, so the adoption path needs one obvious verified lane. |
| Downstream composability | 7 | 10 | The schemas are solid, but a copyable jq/status-bar/minimal consumer example would make the contract feel immediately adoptable. |
| Trust and scope clarity | 8 | 10 | The vision doc is disciplined. The growth path should keep repeating that `clawgs` is a contract layer, not a dashboard, transcript database, or hosted platform. |

## Dueling Ideas Result

1. Build a full dashboard around `clawgs emit`.
2. Add broad transcript search and analytics.
3. Package a shareable contract proof plus downstream starter path.

Winner: shareable contract proof plus downstream starter path.

A dashboard or analytics layer could attract attention, but both would blur the
repo's stated non-goals. The highest-leverage move is to make the existing
contract surface impossible to miss: prove the zero-config demo, show a compact
receipt, and give downstream consumers a minimal integration that depends on
the schema instead of raw logs.

## Execution Beads

The follow-up beads created from this plan should deliver:

- A first-wow demo proof that runs from a clean checkout or installed binary and
  captures `demo extract`, `demo emit`, and one real schema validation cue.
- A privacy-safe contract receipt format that can be pasted into READMEs,
  issues, or launch posts without private transcript content.
- A minimal downstream consumer starter, such as a jq recipe, status-line
  example, or tiny fixture-backed adapter, that proves another tool can adopt
  `clawgs.v2` or `clawgs.emit.v2` without scraping raw logs.
- A launch packet that is gated on the proof, receipt, and consumer starter, and
  keeps the message focused on contract stability and local operation.

## Retro Check

This plan avoids the biggest likely regret: chasing a dashboard or observability
platform shape when the repo's durable advantage is the contract layer. It also
avoids premature claims about distribution by making the launch packet depend on
a verified install lane and checked-in proof artifacts.
