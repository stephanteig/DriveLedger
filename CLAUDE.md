# CLAUDE.md — DriveLedger

> Generated from BRIEF.md. Read this file in full and start working.

---

## Project

**Name:** DriveLedger
**Description:** Scans drives/folders into local indexes and shows exactly which files exist, are elsewhere/renamed, differ, or are missing compared to another drive. Exposes everything via an open MCP server so any AI agent can query it.
**Problem it solves:** Knowing with certainty whether a drive is safe to delete, even when folder structures differ, files are renamed, cameras reuse filenames, or copies were interrupted.
**Version:** 1.0.0 (MVP)

---

## Non-negotiable safety rule

**DriveLedger is read-only on every scanned path.** It must never create, modify, rename, move or delete anything on a scanned drive. The only writes allowed are index files and reports inside the user-chosen *library folder* (and app settings).

- Open scanned files with read-only flags only.
- No `fs::write`, `rename`, `remove*`, `set_permissions` or `set_times` calls that target a scan root. Add a test that scans a fixture folder and asserts that every file's mtime, size and content hash is unchanged afterwards.
- Refuse to save the library inside a path that is being scanned.

---

## Platform

| Platform | Priority | Delivery |
|----------|----------|----------|
| macOS | Primary | `.dmg` (app) + `driveledger` CLI binary |
| Windows / Linux | Later | Do not block: no macOS-only APIs in `core` |

- Development machine: MacBook Pro, Apple Silicon
- Minimum macOS: 13 Ventura

---

## Tech stack

- **Core engine: Rust** (Cargo workspace). Fast parallel walk and hashing, robust on SMB, one cross-platform binary.
- **App: Tauri v2 + React + TypeScript + Vite.** Small native bundle; calls the Rust core directly; Windows/Linux builds later for free.
- **Styling: Tailwind CSS.**
- **Storage: SQLite via `rusqlite` (bundled feature).** Indexed comparisons of 100k+ files are instant.
- **Hashing: BLAKE3.** Fast enough for full-file verification.
- **MCP: `rmcp` (official Rust MCP SDK),** with stdio and Streamable HTTP transports.
- **CLI: `clap`.**
- **Other crates:** `walkdir` + `rayon` (parallel scan), `unicode-normalization`, `serde`/`serde_json`, `schemars` (JSON Schemas for MCP tools), `anyhow`/`thiserror`, `tracing`.
- **Package manager (frontend):** npm

---

## Setup

```bash
# Toolchains
rustup update stable
node -v && npm -v
cargo install create-tauri-app tauri-cli --locked

# Workspace
cargo new --lib crates/core   --name driveledger-core
cargo new        crates/cli   --name driveledger
# Tauri app (React + TS template) in app/
npm create tauri-app@latest app -- --template react-ts --manager npm
cd app && npm install && npm install -D tailwindcss @tailwindcss/vite && cd ..

cargo build && cargo test
```

Add crate dependencies with `cargo add` (pick current stable versions, don't pin from memory).

---

## File structure

```
DriveLedger/
├── Cargo.toml                 # workspace: crates/core, crates/cli, app/src-tauri
├── crates/
│   ├── core/                  # driveledger-core: ALL logic lives here
│   │   └── src/
│   │       ├── scan.rs        # walk, filters, metadata, fingerprint
│   │       ├── fingerprint.rs # sample hash + full hash
│   │       ├── index.rs       # SQLite schema, read/write, migrations
│   │       ├── compare.rs     # matching + classification
│   │       ├── volume.rs      # mount detection, volume name/UUID
│   │       ├── export.rs      # TSV / JSON
│   │       └── lib.rs
│   └── cli/                   # `driveledger` binary: scan, compare, verify, export, list, mcp
│       └── src/
│           ├── main.rs
│           └── mcp.rs         # MCP server (stdio + HTTP), thin wrapper over core
├── app/                       # Tauri v2 + React UI
│   ├── src/                   # React (TS)
│   └── src-tauri/             # Tauri commands → driveledger-core
├── docs/
│   ├── index-format.md        # SQLite schema, documented for third-party agents
│   ├── mcp.md                 # tools, schemas, setup snippets for multiple agents
│   └── matching.md            # how classification works
├── tests/fixtures/            # generated fixture trees (see Definition of Done)
├── LICENSE                    # MIT
└── README.md
```

---

## Core behaviour (implement exactly)

### Scan
- Input: a root path plus a library folder. Output: `<library>/<volume-name>__<root-slug>__<timestamp>.dlidx.sqlite`.
- Skip `.DS_Store`, `._*`, `Icon\r`, `.Spotlight-V100`, `.fseventsd`, `.Trashes`, `.TemporaryItems` and `.DocumentRevisions-V100`. Keep the skip list configurable.
- For each file, store: relative path, NFC-normalised name and path, extension, size, mtime, ctime and `fingerprint`.
- Treat macOS packages (`.dra`, `.app`, `.fcpbundle`, `.RDC`, etc.) as normal folders when scanning, but flag files inside them with `in_bundle=1`.
- **Fingerprint:** BLAKE3 over `size || first 1 MiB || middle 1 MiB || last 1 MiB`. For files up to 3 MiB, hash the whole file. Store `fingerprint_kind = "sample3"`.
- **Full hash:** filled in on demand by `verify`. Store `full_hash` and set `fingerprint_kind = "full"`.
- **Incremental rescan:** if the path, size and mtime are unchanged from the previous index of the same volume and root, reuse the fingerprint.
- Store scan metadata: volume name, volume UUID (via `diskutil info -plist` on macOS, behind a trait), root, start and end time, file count, total bytes, tool version and errors (unreadable paths). An unreadable file is a recorded error. It is never silently skipped.
- Show progress through a callback/channel for the UI and CLI: files seen, bytes hashed, current path.

### Compare (source → target)
- **Before comparing:** if the target is a live path, verify that it is mounted and readable. If it isn't, return a `TargetUnavailable` error. Never return a "missing" result for an absent target.
- Match each source file against the **entire** target index, regardless of folder structure. Classify each file as one of:
  1. `present`: same fingerprint, and same NFC name.
  2. `present_elsewhere`: same fingerprint, but a different name and/or path. Report every location.
  3. `only_in_bundle`: the only matches are inside bundles (`in_bundle=1`).
  4. `same_name_different_content`: a file with the same name exists, but the fingerprint differs. Include both mtimes and sizes, plus a `newer_side` field. If the source is smaller and its size is a multiple of 4096, add the hint `possibly_truncated`.
  5. `missing`: no fingerprint match anywhere.
- **Camera filename trap:** never treat matching name and size as "present" without a fingerprint match.
- **Output:** a per-file list, plus a summary grouped by folder (count and bytes), with the same grouping we used manually (two to three levels deep).

### Verify
- Compute a full BLAKE3 hash for chosen files or folders on both sides and confirm the match. Used before deleting.

---

## MCP server: open for ALL agents

Agent-agnostic by design:

- Follow the official MCP spec only. No vendor-specific extensions, and nothing that assumes Claude.
- **Transports:** `driveledger mcp` (stdio, the default) and `driveledger mcp --http 127.0.0.1:7341` (Streamable HTTP, bound to localhost only).
- Every tool has a JSON Schema for input and output (via `schemars`), returns structured JSON plus a short text summary, and pages any large result (`limit` / `cursor`).
- All tools are **read-only**. Mark them with `readOnlyHint: true`.
- `docs/mcp.md` must include copy-paste config for Claude Code, Claude Desktop, Cursor, OpenAI Codex CLI, Gemini CLI and a generic MCP client.
- Agents without MCP get the same operations through the CLI: every CLI command supports `--json`. They can also read the documented SQLite schema directly.

**Tools (v1):**
| Tool | Purpose |
|------|---------|
| `list_indexes` | Saved scans with volume, root, date, file count and size |
| `list_volumes` | Mounted volumes, and whether each has an index |
| `search_files` | By name or path substring, glob, extension, size range or date range, across one or all indexes |
| `folder_overview` | Folder tree with sizes and counts down to N levels (like `mapper.sh`) |
| `compare` | Source index (or subfolder) against target index. Returns the summary plus paged details |
| `find_file` | Every location of one file's fingerprint across all indexes |
| `find_duplicates` | Groups of identical fingerprints within or across indexes |
| `scan` | Start a scan of a path into the library and return its progress or ID. This writes only to the library |
| `verify` | Full-hash verify of specific paths on both sides |

---

## Features: MVP (in this order)

1. **Core scan + index:** the `driveledger scan <path> --library <dir>` CLI, with progress output.
2. **Core compare:** `driveledger compare <source.dlidx> <target.dlidx> [--subpath X] [--json]`, giving the summary plus details.
3. **MCP server:** stdio and HTTP, all tools above, plus `docs/mcp.md`.
4. **Export:** `driveledger export <idx> --tsv|--json`. The TSV columns match `~/filliste.sh` (`size_mb, modified, path`).
5. **Verify:** `driveledger verify ...`
6. **Tauri app**, with these screens:
   - **Drives:** mounted volumes plus saved indexes, and a "Scan" button.
   - **Scan progress:** live counters.
   - **Compare:** pick source and target (and an optional subfolder), then show summary cards by status, a folder table and a file table filterable by status. Each row has a "Reveal in Finder" action. This only opens Finder.
   - **Search:** across all indexes.
   - **Settings:** library folder, skip list.

## Not in scope for v1 (keep the architecture open)

- Integrity check of video containers (truncated MP4/MOV/R3D). Leave a `checks` module hook in core.
- Copying missing files. This would be the first write feature and needs a separate design.
- Scheduled scans, duplicate cleanup and Windows/Linux packaging.

---

## UI & Design

- **Theme:** dark. Background `#0B0D12`, surfaces `#12151C` and `#1A1E27`, border `#262B36`.
- **Style:** technical and data-dense, Evervault-inspired. Tables first, no decorative fluff.
- **Accents:** cyan `#22D3EE` (primary and "present"), violet `#A78BFA` ("present elsewhere"), amber `#FBBF24` ("different version"), magenta `#F472B6` ("missing").
- **Fonts:** Space Grotesk for the UI and JetBrains Mono for paths, sizes and hashes. Bundle both locally in `app/src/assets/fonts`.
- **Avoid:** grey corporate minimalism, generic purple gradients, Inter/Roboto/Arial.
- Show sizes in human-readable units (GB/MB) and dates as `YYYY-MM-DD`.
- It should look like a paid pro tool.

---

## Data & Storage

- One SQLite file per scan in the user-chosen library folder. Settings go in the platform config directory.
- Version the schema with a `meta` table, and document it in `docs/index-format.md`.
- Indexes, columns and NFC normalisation must be designed for fast fingerprint and name lookups.
- No secrets and no network access, apart from the MCP HTTP transport bound to `127.0.0.1`.

---

## Workflow rules (absolute)

- **All code changes via Pull Request.** Never commit directly to `main`. The only exception is the initial commit of a brand-new repo.
- **Branch names:** `feature/description` or `fix/description`.
- **Commit messages:** short and descriptive, in English.
- **Before creating a file,** check whether it already exists.
- **On errors,** try to fix them yourself (at most 2 attempts), then report clearly with the error message.
- **No hardcoded secrets.** Use `.env` and `.gitignore` if any secret ever becomes necessary. None are needed now.
- Logic goes in `driveledger-core`. The CLI, MCP and Tauri layers stay thin.

---

## First steps for Claude Code

Do this before writing any code:

1. Read this whole file, then `PLAN.md` (the phase-by-phase execution order, one PR per phase), `reference/README.md` (ground truth from the manual checks) and `BRIEF.md` for background.
2. Confirm the tech stack and explain in brief why it fits.
3. List every setup command you will run.
4. Create the workspace and file structure above.
5. Ask ONE clarifying question if something is unclear. No more than one.

Then start with MVP feature 1.

---

## Definition of Done

v1 is done when:

- [ ] **Fixture tests in `tests/` cover every case:** reorganised folders, a renamed file, two different files with the same camera name and size, a truncated copy, an older and newer version of the same file, NFC/NFD names (æøå), a file inside a `.dra` bundle, and an unmounted target (returns `TargetUnavailable`).
- [ ] The read-only test passes: scanning and comparing leave the fixture tree byte-for-byte and mtime-identical.
- [ ] **Real-world acceptance:** the original source folders from 2026-10-30 were deleted afterwards, so:
  - The fixture tests reproduce every pattern recorded in `reference/README.md`: case 01 (truncated `SW-FX30157.MP4`), case 02 (9 newer `.af` versions on the target), case 03 (renamed, bundle-only, the camera-name trap, 396 missing) and the unmounted-target incident.
  - On a real drive pair the user picks, DriveLedger's present/missing split is identical to `reference/prototype/compare_content_sample.py`.
- [ ] The same results come back through MCP in Claude Code and in at least one other MCP client.
- [ ] The app starts without errors on macOS, and builds as `.dmg`.
- [ ] `README.md`, `docs/mcp.md`, `docs/index-format.md` and `docs/matching.md` exist.
- [ ] The code is readable, with comments where the matching logic isn't obvious.

---

## Technical context

- Development machine: MacBook Pro (Apple Silicon).
- The drives are external SSDs/HDDs (APFS and exFAT) and SMB network shares, for example `/Volumes/100.119.237.113/...`. Expect slow I/O and high latency, and parallelise carefully: limit hashing concurrency per volume.
- Typical content is camera video: Sony FX3 `SW-FX3xxxx.MP4` with `M01.XML` sidecars, RED `.R3D` in `.RDC` folders, DJI `.MP4`/`.LRF`, Canon `.MP4`, Affinity `.af`, and DaVinci `.dra`/`.drt`. Files are often 0.3–12 GB, so never read whole files during a normal scan.
- The existing scripts `~/filliste.sh` and `~/mapper.sh` define the export formats users already rely on.
- Benchmark: clean, readable, maintainable code that a senior developer would review. Prefer clarity over clever one-liners.

---

*Generated from BRIEF.md — Stephan Teig*
