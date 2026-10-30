# BRIEF.md — DriveLedger

**Date:** 2026-10-30
**Status:** Planning complete, ready for Claude Code

---

## 1. Idea & Problem

**Idea:** A tool that scans entire drives and folders into a saved index, compares indexes against each other, and shows exactly what is and isn't backed up. An MCP server exposes the indexes so any AI agent (Claude or others) can search and compare them.

**Problem:** Checking whether a drive is safe to delete takes hours of manual work and guesswork. On 30 Oct 2026 we did this by hand for the ByHola drives and ran into every hard case:

- **Different folder structures:** the same material lived under `_HYSJ/...` on one drive and `HYSJ/2026/...` on another.
- **Reused camera filenames:** Sony FX3 reuses names like `SW-FX30001.MP4` on every shoot. Matching on name, or even name plus size, isn't enough, so the actual content has to be compared.
- **Renamed files:** for example `... (1).pdf`, or `:` changed to `_` in the OdaRose files.
- **Interrupted copies:** `SW-FX30157.MP4` in the Reserve backup was truncated (missing moov atom).
- **Version drift:** `.af` (Affinity) files existed in older and newer versions on different drives.
- **Unicode:** æ/ø/å filenames can be encoded differently (NFC/NFD) on different file systems.
- **Unmounted drives:** a drive that isn't connected made every file look "missing". The tool must detect this rather than report false results.
- **Junk folders:** media files inside DaVinci `.dra/MediaFiles` bundles count as copies, but aren't a proper home for the file.

**Users:** Stephan for now. It will be published open source on GitHub later.

**Inspiration:** today's manual workflow and the scripts `~/filliste.sh` (file list TSV) and `~/mapper.sh` (folder overview TSV).

---

## 2. Features

**MVP (v1), in priority order:**
1. **Scan:** walk a drive or folder and save an index with path, size, dates and a fingerprint (hash of three 1 MB samples).
2. **Compare in the app:** source index against target index. Each file is classified as:
   - present
   - present elsewhere or under another name
   - same name but different content (newer or older)
   - missing
3. **MCP server:** open and standards-based, so any MCP-capable agent can use it (Claude Code, Claude Desktop, Cursor, Codex, Gemini CLI, custom agents).
4. **Deep verify:** an optional full-file hash for specific files or folders before you delete them.
5. **CLI:** the same engine from the terminal, with JSON output.
6. **Export:** TSV and JSON, readable by people, Excel and LLMs.

**v2 / later:**
- Integrity check of video files (truncated MP4/MOV/R3D).
- Copying missing files to a target with verification afterwards. This would be the first feature that writes to drives, so it needs to be designed separately.
- Duplicate cleanup suggestions.
- Scheduled rescans.
- Windows and Linux builds.

**Will NOT do:**
- Change, move or delete files on scanned drives. The tool is **read-only** on everything it scans.
- Write anything other than indexes and reports, and only to a library folder the user chooses.
- Act as a backup tool in v1.

---

## 3. Platform & Distribution

- **macOS** (Apple Silicon) first. Distributed as `.dmg` plus a CLI binary.
- The stack is cross-platform ready, so Windows and Linux can be added later without a rewrite.
- **Open source** on GitHub later (suggested license: MIT).
- **Internet:** never. Everything runs locally. The MCP server runs locally (stdio, or HTTP on localhost).

---

## 4. UI & Design

- **Style:** technical and dark, with dense data tables, Evervault-inspired.
- **Colours:**
  - Background: near-black `#0B0D12`
  - Accents: cyan `#22D3EE` and violet `#A78BFA`
  - Magenta `#F472B6` for "missing"
  - Amber for "different version"
- **Fonts:** Space Grotesk for the UI, JetBrains Mono for paths and numbers.
- **Avoid:** grey corporate minimalism, generic purple gradients, Inter/Roboto/Arial.

---

## 5. Data & Storage

- **Format:** one SQLite file per scan, stored in a library folder chosen by the user. Export to TSV/JSON.
- **Stored:** path, NFC-normalised name, size, mtime/ctime, fingerprint, optional full hash, scan metadata (volume name, volume UUID, root path, time, tool version).
- **Sensitive data:** only file paths and metadata, no file contents. No API keys are needed.

---

## 6. Technical Decisions

**Stack:** a Rust core engine, a Tauri v2 + React + TypeScript app, and an MCP server in the same binary.

**Why:**
- **Rust:** fast parallel scanning of 100k+ files, robust on network drives (SMB), and one binary for every platform.
- **Tauri over Electron:** a small app (about 10 MB, where Electron is about 150 MB), it uses the Rust engine directly, and builds for Windows and Linux.
- **SQLite:** comparing 30k against 30k files is instant with indexes. Plain text would be slow on large drives.
- **One engine:** `driveledger-core` is used by the app, the CLI and the MCP server, so all three always give the same answers.

**MCP must be open for all agents:**
- It follows the official MCP specification, with no Claude-specific extensions.
- Both transports are supported: **stdio** (for local agents) and **Streamable HTTP** on localhost (for agents that can't start processes).
- Every tool has a JSON Schema for input and output, and returns structured JSON.
- The index format (SQLite schema) is documented in `docs/index-format.md`, so agents without MCP can read the files directly or use `driveledger ... --json`.

**Fingerprint strategy:**
- Default: BLAKE3 over size + the first, middle and last 1 MB. This is the approach that worked today; it takes seconds even over a network.
- Deep verify: a full BLAKE3 hash, run on demand.

---

## 7. Scope

**v1 is done when:** DriveLedger can redo today's checks on the ByHola drives, and both the app and Claude (via MCP) produce the same results we got by hand:
- Reserve `ByHola` against `ByHola_1`: 78 matches, 1 truncated file with a different size (SW-FX30157), 0 missing.
- HYSJ-2026_1 `ByHola` against `ByHola_1`: 834 matches, 9 older `.af` versions, 0 missing.
- HYSJ-2026_1 `HYSJ` against `ByHola_1`: 7 found under another name, 396 missing (about 100 GB, mainly `04_Partnere`).

> **Note (same day):** the source folders were deleted after the checks. The results are kept in `reference/ground-truth/` and are checked using fixture tests, plus the Python prototype as a test oracle on a new real drive pair (see `PLAN.md`, Phase 9).

**Timeline:** personal project with no deadline, but the code should be good enough to publish.
