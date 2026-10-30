# PLAN.md — DriveLedger implementation plan

> The order of execution for Claude Code. Each phase is **one PR** on its own branch, merged before the next phase starts.
> Read `CLAUDE.md` first (the spec and rules), then `BRIEF.md` (the why) and `reference/README.md` (the ground truth from the manual checks).

## Status
- [x] Phase 0: Repo and workspace bootstrap
- [x] Phase 1: Core index and scan
- [x] Phase 2: Fingerprinting and incremental rescan
- [x] Phase 3: Compare engine
- [x] Phase 4: CLI
- [x] Phase 5: MCP server
- [x] Phase 6: Verify and export
- [x] Phase 7: Tauri app (shell and drives/scan)
- [x] Phase 8: Tauri app (compare and search)
- [x] Phase 9: Real-world acceptance and release prep

Tick the boxes as phases merge. Keep this file updated in each PR.

---

## Phase 0: Bootstrap (direct to `main`, the only exception)
1. Run `git init` in `~/Documents/Dev/DriveLedger`. Add `.gitignore` (target/, node_modules/, dist/, *.sqlite, .env, .DS_Store) and an MIT `LICENSE`.
2. Create the Cargo workspace with `crates/core` (lib `driveledger-core`) and `crates/cli` (bin `driveledger`). Leave `app/` out for now.
3. Add `rustfmt.toml`, set clippy to deny warnings in CI, and add a minimal `README.md`.
4. Add GitHub Actions CI on macOS-latest: `cargo fmt --check`, `cargo clippy -- -D warnings` and `cargo test`. A remote isn't required yet; the user creates it later.
5. Make the initial commit to `main`. **Every later change goes through a branch and a PR.** If there is no GitHub remote yet, still use branches, then merge locally with `--no-ff` and list each "PR" summary in the merge commit. Ask the user once whether they want to create the GitHub repo now (`gh repo create`).

**Done:** `cargo build && cargo test` is green.

## Phase 1: Core index and scan (`feature/core-scan-index`)
1. `index.rs`: the SQLite schema, versioned with a `meta(key, value)` table that includes `schema_version`.
   - `scan(id, volume_name, volume_uuid, root_path, started_at, finished_at, tool_version, file_count, total_bytes)`
   - `file(id, rel_path, rel_path_nfc, name_nfc, ext, size, mtime, ctime, in_bundle, fingerprint, fingerprint_kind, full_hash)`
   - `error(rel_path, kind, message)`
   - Indexes on `fingerprint`, `(name_nfc)`, `(size)` and `(rel_path_nfc)`.
2. `volume.rs`: a `VolumeInfo` trait. The macOS implementation uses `diskutil info -plist <path>` to get the name and UUID, and falls back to the statfs mount point. It also needs `is_mounted_and_readable(root)`.
3. `scan.rs`: a parallel walk (`walkdir` and `rayon`) that applies the configurable skip list from `CLAUDE.md`, detects bundles (`.dra`, `.app`, `.fcpbundle`, `.RDC`, `.photoslibrary`, `.band`, `.logicx`) and sets `in_bundle`. It records errors rather than failing, and reports progress through a callback.
4. Use **read-only file access only.** Add the guard test `scan_is_read_only`: hash and stat every fixture file before and after a scan, and assert they are identical.
5. Name the library output `<library>/<volume>__<root-slug>__<YYYYMMDD-HHMMSS>.dlidx.sqlite`, and refuse a library folder inside the scan root.

**Tests:** a fixture builder in `crates/core/tests/common/` that generates trees in a temp dir (never commit large binaries; generate deterministic pseudo-random bytes).

## Phase 2: Fingerprinting and incremental rescan (`feature/fingerprint`)
1. `fingerprint.rs`: compute `sample3` as BLAKE3 over `size_le_bytes || first 1 MiB || middle 1 MiB || last 1 MiB`. Hash files up to 3 MiB in full. `full` is a streaming BLAKE3 over the whole file.
2. Limit hashing concurrency per volume (default 4, or 2 for network volumes; detect SMB via the statfs fs type).
3. **Incremental rescan:** given a previous index of the same volume UUID and root, reuse the fingerprint when `(rel_path, size, mtime)` matches.
4. Test that two files with the same name and size but different content get different fingerprints (the camera trap).

## Phase 3: Compare engine (`feature/compare`)
1. `compare.rs`: `compare(source: &Index, target: &Index, opts) -> CompareResult`.
2. Accept an optional `source_subpath` filter. `target` can be either a saved index or a live path. For a live path, check `is_mounted_and_readable` first and return `TargetUnavailable` if it fails.
3. Classify each file following the rules in `CLAUDE.md`, in this precedence:
   1. `present`
   2. `present_elsewhere`
   3. `only_in_bundle`
   4. `same_name_different_content`, with `newer_side` and the `possibly_truncated` hint
   5. `missing`
4. Return a `CompareSummary` with counts and bytes per status, plus a per-folder rollup (depth configurable, default 3) and paged details.
5. **Fixture tests, one per real case** (see `reference/README.md`):
   - Reorganised layout (`_HYSJ/x` against `HYSJ/2026/x`) gives `present`.
   - Renamed (`a (1).pdf`, `:` changed to `_`) gives `present_elsewhere`.
   - Camera trap: same name and size, different content, gives `same_name_different_content`, and the file counts as not backed up.
   - Truncated copy: the source is a 4096-aligned prefix of the target, which gives `possibly_truncated`.
   - Version drift: a `.af` that is newer on the target gives `newer_side = target`.
   - NFC/NFD names (`Råmateriale`, `STØY`) match.
   - A match only inside `x.dra/MediaFiles/...` gives `only_in_bundle`.
   - An unmounted target gives `TargetUnavailable`, never a list of missing files.
6. A benchmark test: comparing 100k against 100k synthetic rows takes under 2 s on an M-series Mac (in-memory SQLite).

## Phase 4: CLI (`feature/cli`)
1. Add these `clap` subcommands:
   - `scan <path> [--library DIR]`
   - `compare <src> <tgt> [--subpath P] [--depth N] [--status S...] [--json]`
   - `list [--json]`
   - `volumes [--json]`
   - `search <query> [--ext] [--min-size] [--json]`
   - `overview <idx> [--depth N] [--json]`
2. `<src>` and `<tgt>` can each be an index file, an index ID or a live path. For a live path, scan into the library first, or reuse a fresh index with `--reuse`.
3. Default library: `~/Documents/DriveLedger Library` (create on first use). It can be overridden with `--library` or `DRIVELEDGER_LIBRARY`, and the setting is stored in the platform config directory.
4. Human output is a coloured summary table plus a per-folder rollup, like the manual reports. Every command supports `--json`.
5. Add snapshot tests with `insta` for the human and JSON output.

## Phase 5: MCP server (`feature/mcp`)
1. `crates/cli/src/mcp.rs` uses `rmcp`. `driveledger mcp` runs over stdio; `driveledger mcp --http 127.0.0.1:7341` runs Streamable HTTP and must refuse non-loopback binds.
2. Implement the tools from `CLAUDE.md` as thin wrappers over core, with `schemars` input and output schemas, `readOnlyHint: true` (`scan` is the only tool that writes, and only to the library), and `limit`/`cursor` paging. Each returns structured content plus a 1–3 line text summary.
3. Tool descriptions must say plainly that matching is content-based and that `missing` means "no identical content anywhere on the target", so agents don't over-trust name matches.
4. Write `docs/mcp.md` with config snippets for Claude Code (`claude mcp add driveledger -- driveledger mcp`), Claude Desktop, Cursor, Codex CLI, Gemini CLI and a generic HTTP client. Add the example agent prompt: "Is everything in X backed up on Y?"
5. Write an integration test that starts the stdio server, sends `initialize`, `tools/list` and `tools/call compare`, and asserts that the JSON matches the CLI's `--json` output.

## Phase 6: Verify and export (`feature/verify-export`)
1. `driveledger verify <src> <tgt> [--subpath P]` computes full BLAKE3 on both sides for the matched pairs, then reports `verified`, `mismatch` and `unreadable`. It stores `full_hash` in the indexes; that write goes to the library only.
2. `driveledger export <idx> --tsv|--json [--out FILE]`. The TSV columns must be identical to `reference/scripts/filliste.sh`, and `overview --tsv` must match `mapper.sh`.
3. Add the `verify` MCP tool.

## Phase 7: Tauri app, shell and drives/scan (`feature/app-shell`)
1. `npm create tauri-app@latest app -- --template react-ts --manager npm`. Add `app/src-tauri` to the workspace, set up Tailwind v4 and bundle the Space Grotesk and JetBrains Mono fonts locally.
2. Set the design tokens from `CLAUDE.md` as CSS variables, with the dark theme as the only theme in v1.
3. Tauri commands call `driveledger-core` directly, with no shelling out to the CLI.
4. **Drives screen:** mounted volumes, each shown with its last index date, plus saved indexes. Each card has a "Scan" button, and there's a folder picker for scanning a subfolder.
5. **Scan screen:** live progress (files, bytes hashed, current path, errors) and a cancel button.
6. **Settings:** the library folder, editable through a picker, and the skip list.

## Phase 8: Tauri app, compare and search (`feature/app-compare`)
1. **Compare screen:**
   - Pick a source (index or folder) and a target, with an optional subpath.
   - Summary cards per status, using the status colours from `CLAUDE.md`, showing counts and GB.
   - A folder rollup table (sortable by GB).
   - A virtualised file table (100k rows) that filters by status and extension, and searches by path.
   - Row details show every target location, both mtimes and sizes, and the hints.
   - "Reveal in Finder", which only opens Finder, and "Export report", which writes TSV/JSON to a location the user picks.
   - A "Verify selected" action runs Phase 6 verify on the chosen rows.
2. **Search screen:** a query across all indexes, with every location of each file shown.
3. If the target is unavailable, show a clear "Connect <volume name>" state, never an empty "missing" list.

## Phase 9: Real-world acceptance and release prep (`feature/acceptance`)
The original source folders were deleted, so acceptance has two parts:
1. **Fixture replay:** all Phase 3 fixture tests pass, and their outcomes match the patterns recorded in `reference/README.md` (cases 01–03 and the unmounted incident).
2. **Live oracle check:** with the user's drives connected, run DriveLedger and `reference/prototype/compare_content_sample.py` on at least one real pair the user picks (for example a folder on `HYSJ-2026_1` or `Reserve` against `ByHola_1`). The present/missing sets must be identical. Document the run in `docs/acceptance.md`.
3. MCP check: run the same comparison through MCP in Claude Code and in at least one other MCP client (Cursor or Codex CLI). Record the results.
4. Build the `.dmg` with `npm run tauri build`. It is unsigned for now; document Gatekeeper's "Open anyway".
5. Finish `README.md` (what it is, the safety guarantee, install, CLI, MCP, app screenshots), `docs/index-format.md` and `docs/matching.md`.
6. Tag `v1.0.0`.

---

## Rules recap (full list in `CLAUDE.md`)
- Read-only on scanned paths, always. The read-only guard test must never be removed or weakened.
- One PR per phase, on a `feature/...` or `fix/...` branch, with English commit messages.
- Logic in `driveledger-core`; the CLI, MCP and app are thin layers.
- If something fails, try at most twice, then report the exact error.
- Stop and ask the user only when a decision is genuinely theirs to make (for example creating the GitHub repo, or which drives to use for the Phase 9 live run).
