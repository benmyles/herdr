# Crush Integration Plan

Branch: `feat/crush-integration` · Status: implemented · Updated: 2026-08-18

> Scope superseded after approval: Crush extended in the fork at
> `/Users/ben/src/ben-crush` (branch `feat/session-start-hook`) with a
> `SessionStart` hook event and `<config dir>/hooks/*.json` fragments, so the
> integration is a full session-identity integration (screen manifest for
> state, SessionStart reporting for native restore) rather than the plan's
> original detection-only scope. Herdr work follows the qwen template
> (`a4d52ab6`) plus the grok fragment-style install for the hook config.

## Goal

Add Crush (charm.land CLI agent, binary `crush`) as a first-class detected agent in Herdr, following the screen-manifest pattern used for Claude Code and Codex.

## Scope decision

| Layer | In scope? | Why |
| --- | --- | --- |
| Process-name identification for `crush` | yes | mandatory for detection |
| Screen manifest (`idle` / `working` / `blocked`) | yes | primary deliverable |
| `herdr integration install crush` (hook/session identity) | **no** | Crush's hook system only supports `PreToolUse`. There is no session-start, turn-end, or exit event that could report lifecycle state or session identity, so any reporter would be self-contradictory: it could set `working` but never `idle`/`blocked`, and Herdr would still need the screen manifest as authority. Mirrors Codex/Claude, which also rely on screen manifests despite having hooks. |
| Native session restore (`crush --session <id>` / `--continue`) | **no** | Resume flags exist (verified via `crush --help`), but there is no channel to report the live session id back to Herdr. Revisit only if Crush gains session lifecycle hooks. |
| Docs role label | "none" | same as Maki, Kiro, Amp: detection only, no install integration |

## Verified facts

- Binary: `crush` (interactive mode is plain `crush`).
- Resume mechanics exist (`--session`, `--continue`) but no identity reporting.
- Crush config/hook docs: only `PreToolUse` hooks exist (crush-config and crush-hooks skills).
- Herdr manifest engine is at `MANIFEST_ENGINE_VERSION = 3` (`src/detect/manifest_update.rs:15`), so all region types (including `osc_title` / `osc_progress` / `top_non_empty_lines`) are available.
- The `maki` commit (`3b8aeee1`, screen-manifest agent, integration role "none") and the `qwen` commit (`a4d52ab6`) are the exact templates for the footprint below.

## Phase 1: Evidence capture for the manifest (prerequisite, no coding)

Per AGENTS.md, manifest rules must be evidence-based. Use the `herdr-throwaway-repro` skill to create a disposable named session, then:

1. Drive a real `crush` pane into each target state (prompt idle, running a task, permission prompt / question prompt blocked).
2. Capture detection source per state:
   - `herdr agent read <pane> --source detection --format text`
   - `herdr agent read <pane> --source detection --format ansi` (spinner/styling evidence)
3. Note whether Crush emits OSC terminal title (0/2) or OSC 9;4 progress at all; capture titles per state if it does. Also capture whether blocked prompts have distinct bottom-region chrome (e.g. `[y/n]` lines, `Esc to cancel`) worth gating on `bottom_non_empty_lines(n)`.
4. Decide invariant controls vs alternatives; encode as explicit AND/OR gates; never match whole-pane incidental text.
5. Write rules to `src/detect/manifests/crush.toml`, copy to `~/.config/herdr/agent-detection/crush.toml`, run `herdr server reload-agent-manifests`, iterate with `herdr agent explain <pane> --json`, then remove the override.

Open items the evidence phase must answer:
- Deterministic `working` marker (spinner glyph via OSC title/progress wins over raw content if available).
- Whether Crush alternate screens (permission forms) are reliably bottom-anchored.
- Windows executable name for Crush (if `crush` differs there, mirror the Cursor `cfg!(windows)` pattern in `interactive_agent_executable`).

## Phase 2: Code and catalog changes

Template: `git show 3b8aeee1` and current master conventions. File-by-file:

### Rust

1. **`src/detect/manifests/crush.toml`** (new)
   - `id = "crush"`, dated dot version, `min_engine_version = 3`, rules from Phase 1.
2. **`src/detect/mod.rs`**
   - Add `Agent::Crush` variant.
   - `ALL: [Self; 22]` → `[Self; 23]`; `SCREEN_MANIFEST_AGENTS: [Self; 20]` → `[Self; 21]`.
   - `agent_label` → `"crush"`; `interactive_agent_executable` → `"crush"`.
   - `lookup_agent` / `parse_agent_label` / `identify_agent`: add `"crush"`.
   - Tests: `identify_agent`, `parse_agent_label`, `agent_label`, `interactive_agent_executable` table, and any exhaustive `Agent::ALL` assertions.
3. **`src/detect/manifest.rs`** — add `("crush", include_str!("manifests/crush.toml"))` to `BUNDLED_MANIFESTS` (alphabetical position).
4. **`src/config/sound.rs`** — add `crush: AgentSoundSetting` to `AgentSoundOverrides`, the `Agent::Crush` match arm, and the `Default` impl (maki template).
5. **`src/config/model.rs`** — extend the `cjk_ime_agents` doc comment with `crush` (config surface consistency, maki template).
6. **`src/config/sidebar.rs`** — add `Agent::Crush` to the exhaustive `accepts_every_canonical_agent_override_key` test list.

### Remote manifest catalog (website)

7. **`website/agent-detection/crush.toml`** (new) — byte-identical to the bundled manifest.
8. **`website/agent-detection/index.toml`** — add `[[agents]] id = "crush"` entry (alphabetical).

### `docs/next` (include ja and zh-cn where the table/list exists)

9. **`CHANGELOG.md`** — one `Added` bullet in the Unreleased section ("Added crush detection with idle, working, and blocked screen states."). Applied to `docs/next/CHANGELOG.md` only (not root, per docs rules).
10. **`README.md`** — add a crush row to the agent detection table (maki precedent: root-README staging only, does not touch `README.zh-CN.md`; will double-check the zh-CN table for the same row during implementation).
11. **`agents.mdx`** (en, ja, zh-cn) — add table row `| Crush | screen manifest | none |`.
12. **`configuration.mdx`** (en, ja, zh-cn) — append `crush` to the accepted `cjk_ime_agents` names.
13. **`cli-reference.mdx`** and **`agent-automation.mdx`** (en, ja, zh-cn) — append `crush` to the supported `--kind` lists (current master keeps these lists complete; convention from `3f809476`).
14. **`website/src/data/config-reference.json`** — add `ui.sound.agents.crush` enum entry (default `"default"`, values `default/on/off`) and refresh the `cjk_ime_agents` description to include `crush`.

### API schema

No changes. `docs/next/api/herdr-api.schema.json` only carries `IntegrationTarget`, which is untouched because there is no install integration; schema tests regenerate/compare from Rust types and stay green.

## Phase 3: Validation

- `just check` (fmt + nextest + maintenance scripts: manifest check, config reference check, docs translation parity, vendored-tree checks).
- `scripts/agent_detection_manifest_check.py --require-website` (bundled vs website catalog parity).
- Targeted: `cargo nextest run detect` plus the sidebar/sound config tests.
- Live loop against the throwaway session (Phase 1) as the behavior check; no large agent-specific fixture suites, per AGENTS.md.
- Performance: detection rules are evaluated only for crush panes in the existing per-pane detection path; no new per-render work, so no bench-render-scale delta expected. Will still run the hot-path architecture test via `just check`.

## Commit plan

One commit, conventional style, no closing keywords, staged on `feat/crush-integration`:

```
feat: add crush agent detection

Adds crush to process-name identification, a screen manifest for idle,
working, and blocked states, per-agent sound overrides, the website
manifest catalog, and staged next-release docs.
```

Proposal goes to Can for alignment before committing; PR opened afterward per maintainer workflow.

## Deferred (future work, needs Crush-side changes)

- Install integration + session identity restore: blocked on Crush session/turn lifecycle hooks (currently `PreToolUse` only).
- Lifecycle-authority reporting: only if Crush adds `SessionStart`-style hooks; screen manifest remains authority meanwhile.

## Risks

- Evidence phase may find no reliable `idle`/`working` marker (e.g. pure spinner glyphs without OSC title). Fallback: fewer working rules, rely on PTY-activity heuristics plus blocked rules; document any fallback as `default_known_agent_idle_fallback`.
- If Crush's blocked prompts resemble the generic shell, blocked gating MUST stay strict per the "Blocked state" doc; under-matching is preferred to false-blocking.
