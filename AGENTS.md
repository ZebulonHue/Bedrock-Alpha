# AGENTS.md — Project Bedrock

Guidance for AI coding assistants (and humans) working on this repository.

## What this project is

Project Bedrock is **a professional desktop tool for artists that happens to
understand Minecraft worlds — it is NOT a Minecraft clone.** The product is
the extraction pipeline (Minecraft → Blender). The renderer exists only to
help artists choose what to export. Never sacrifice export accuracy for
visual effects.

## The golden rule

**Every completed milestone must leave the application in a working,
compilable state. Never begin the next feature until the previous one is
fully functional and verified.**

Before declaring any task done, run and pass:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Crate boundaries (hard rules)

| Crate | Responsibility | Must NOT |
| --- | --- | --- |
| `bedrock-app` | Entry point, window, app-state wiring | contain business logic |
| `bedrock-ui` | egui panels, dock layout, theme, log view | parse worlds, touch GPU |
| `bedrock-render` | WGPU viewport rendering | export, parse |
| `bedrock-parser` | Read Java/Bedrock saves, NBT, chunks | UI, GPU, export |
| `bedrock-export` | Export files from parsed data | UI, rendering |
| `bedrock-blender` | Blender import conventions & tooling | UI, parsing |
| `bedrock-cache` | Thumbnail/chunk/texture caches | UI, GPU |
| `bedrock-settings` | Persistent preferences | anything else |

- The renderer consumes parsed data; it is never responsible for export.
- The export system is never responsible for rendering.
- UI never contains business logic; systems communicate through plain data.

## Conventions

- Small modules, single responsibility. Avoid giant files.
- Document every public item (`///`).
- Shared dependencies are pinned once in the root `[workspace.dependencies]`;
  member crates use `{ workspace = true }`. Do not add a new dependency
  without justification in the PR/description.
- The egui ecosystem crates (`egui`, `eframe`, `egui_wgpu`, `egui_dock`) must
  always be upgraded together. After any bump, run `cargo tree -d` and
  confirm only one egui version exists.
- Match the style of the surrounding code. No drive-by refactors, no
  unsolicited renames or reformatting.
- Unit tests live next to the code (`#[cfg(test)]`). Do not unit-test code
  that requires a GPU or a window; keep logic headless so it stays testable.

## Commands

- Run the app: `cargo run --release -p bedrock-app`
- Tests: `cargo test --workspace`
- Lint: `cargo clippy --workspace --all-targets -- -D warnings`
- Format: `cargo fmt --all`
- Regenerate icons: `python tools/generate_icon.py`

## Roadmap

See `docs/PRD.md`. Phases: 1 Foundation → 2 World Detection → 3 Parser →
4 Renderer → 5 Export → 6 Blender Pipeline → 7 Polish.
