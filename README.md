# DriveLedger

DriveLedger scans drives and folders into local indexes and tells you, with certainty, which files
from one drive exist on another. Matching is by **content** (BLAKE3 fingerprints), so it works
when folders were reorganised, files were renamed, cameras reused file names such as
`SW-FX30001.MP4`, or a copy was interrupted.

Every file in the source gets one status:

| Status | Meaning |
|---|---|
| `present` | identical content on the target, same name |
| `present_elsewhere` | identical content under another name or path |
| `only_in_bundle` | identical content, but only inside a package such as `.dra/MediaFiles` |
| `same_name_different_content` | same name, different file (newer/older version, truncated copy). **Not backed up.** |
| `missing` | no identical content anywhere on the target. **Not backed up.** |

Everything is available from a CLI, a macOS app and an open MCP server, so any AI agent can ask
"is everything in X backed up on Y?".

## Safety

- **Read-only on every scanned path.** Files are opened read-only, and DriveLedger never creates,
  modifies, renames, moves or deletes anything on a scanned drive. A test scans, compares and
  verifies fixture trees and asserts that every file's size, mtime and content are unchanged.
- The only writes are index files in your library folder, app settings, and exports you
  explicitly save. Exports are refused inside a scanned folder.
- A drive that is not mounted is reported as **unavailable**, never as "everything missing".
- `verify` hashes every byte of each matched pair before you delete anything.

## Install

**App:** build the `.dmg` (below) and drag DriveLedger to Applications. The build is unsigned: on
first launch open **System Settings → Privacy & Security → Open Anyway**.

**CLI:** `cargo install --path crates/cli`, which installs `driveledger` into `~/.cargo/bin`.

Requires macOS 13+. The core has no macOS-only APIs, so Windows and Linux builds can follow.

## CLI

```bash
driveledger scan "/Volumes/HYSJ-2026_1/untitled folder2/HYSJ"
driveledger compare "/Volumes/HYSJ-2026_1/untitled folder2/HYSJ" /Volumes/ByHola_1
driveledger compare <source-id> <target-id> --status missing --json
driveledger verify <source-id> <target-id> [--subpath P]   # full hash before deleting
driveledger export <id> --tsv [--out list.tsv]             # like filliste.sh
driveledger overview <id> --tsv                            # like mapper.sh
driveledger list | volumes | search <text> [--glob 'SW-FX3*.MP4']
driveledger config --set-library ~/Documents/DriveLedger\ Library
```

`compare` accepts index files, index IDs (from `list`) or live folders; live folders are scanned
into the library first (`--reuse` reuses a scan younger than 24 h). Every command supports
`--json`. Exit codes: 0 ok, 1 error, 2 usage, 3 target not mounted, 4 verify found differences.

The library defaults to `~/Documents/DriveLedger Library` and can be changed with `--library`,
`DRIVELEDGER_LIBRARY`, `driveledger config --set-library` or the app's Settings.

## MCP

```bash
claude mcp add driveledger -- driveledger mcp      # Claude Code, stdio
driveledger mcp --http 127.0.0.1:7341              # Streamable HTTP, loopback only
```

Nine tools: `list_indexes`, `list_volumes`, `search_files`, `folder_overview`, `compare`,
`find_file`, `find_duplicates`, `scan`, `verify`. All have JSON Schemas and paging. Setup for
Claude Desktop, Cursor, Codex CLI, Gemini CLI and generic clients is in [docs/mcp.md](docs/mcp.md).

## App

- **Drives:** mounted volumes with their last scan, saved indexes, Scan / scan a subfolder.
- **Scans:** live progress (files, bytes hashed, errors, current path), cancel.
- **Compare:** summary cards per status, folder rollup, a virtualised file table filterable by
  status, extension and path, row details with every target location, Reveal in Finder, Verify
  selected and Export report.
- **Search:** across all indexes, with every copy of a file's content.
- **Settings:** library folder, skip list and bundle extensions.

## Docs

- [docs/matching.md](docs/matching.md): how classification works and why
- [docs/index-format.md](docs/index-format.md): the SQLite index schema, for tools and agents
- [docs/mcp.md](docs/mcp.md): MCP setup and tool reference
- [docs/acceptance.md](docs/acceptance.md): v1.0.0 acceptance run

## Build

```bash
cargo build && cargo test            # core, CLI, MCP
cd app && npm install
npm run tauri dev                    # app in development
npm run tauri build                  # .app and .dmg in target/release/bundle
```

**Troubleshooting (macOS):** if linking fails with `tapi error: malformed file ... unknown
architecture`, your Command Line Tools linker is older than the default SDK. Either update the
Command Line Tools, or point at an older SDK:
`export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk`.

## License

MIT
