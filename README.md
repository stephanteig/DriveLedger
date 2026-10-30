# DriveLedger

Scans drives and folders into local indexes and tells you, with certainty, which files
from one drive exist on another: present, present elsewhere/renamed, different version,
or missing. Matching is content-based (BLAKE3 fingerprints), so reorganised folders,
renamed files and reused camera filenames are handled correctly.

Everything is exposed through an open MCP server so any MCP-capable agent can query it.

> **Safety:** DriveLedger is read-only on every scanned path. It only writes index files
> to a library folder you choose.

## Status

Work in progress (v1.0.0 MVP). See `PLAN.md` for the implementation plan.

## CLI

```bash
driveledger scan "/Volumes/HYSJ-2026_1/untitled folder2/HYSJ"
driveledger compare "/Volumes/HYSJ-2026_1/untitled folder2/HYSJ" /Volumes/ByHola_1
driveledger compare <source-id> <target-id> --status missing --json
driveledger list | volumes | search <text> [--glob 'SW-FX3*.MP4'] | overview <id>
driveledger config --set-library ~/Documents/DriveLedger\ Library
```

`compare` accepts index files, index IDs (from `list`) or live folders; live folders are scanned
into the library first (`--reuse` reuses a scan younger than 24 h). Every command supports
`--json`. Exit codes: 0 ok, 1 error, 2 usage, 3 target not mounted.

The library defaults to `~/Documents/DriveLedger Library` and can be changed with `--library`,
`DRIVELEDGER_LIBRARY` or `driveledger config --set-library`.

## MCP

```bash
claude mcp add driveledger -- driveledger mcp      # Claude Code, stdio
driveledger mcp --http 127.0.0.1:7341              # Streamable HTTP, loopback only
```

Setup for Claude Desktop, Cursor, Codex CLI, Gemini CLI and generic clients is in
[docs/mcp.md](docs/mcp.md).

## Build

```bash
cargo build
cargo test
```

**Troubleshooting (macOS):** if linking fails with `tapi error: malformed file ... unknown
architecture`, your Command Line Tools linker is older than the default SDK. Either update the
Command Line Tools, or point at an older SDK:
`export SDKROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX26.5.sdk`.

## License

MIT
