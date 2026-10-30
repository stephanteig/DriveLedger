# MCP server

DriveLedger ships an MCP server in the same binary as the CLI. It follows the official
[Model Context Protocol](https://modelcontextprotocol.io) specification only, with no vendor
extensions, so any MCP client can use it.

```bash
driveledger mcp                          # stdio (default)
driveledger mcp --http 127.0.0.1:7341    # Streamable HTTP at http://127.0.0.1:7341/mcp
```

- The HTTP transport only binds to loopback addresses (`127.0.0.1`, `::1`) and refuses anything
  else. It validates the `Host` header (loopback only) and the `Origin` header (browsers on other
  sites get `403`).
- The server uses the library from `--library`, `DRIVELEDGER_LIBRARY`, the saved setting or
  `~/Documents/DriveLedger Library`, in that order.
- DriveLedger never writes to scanned drives. Every tool is `readOnlyHint: true` except `scan`,
  which writes a new index file into the library (`readOnlyHint: false`,
  `destructiveHint: false`).

Install the binary first (`cargo install --path crates/cli`, or copy `driveledger` to
`/usr/local/bin`). GUI apps such as Claude Desktop don't inherit your shell `PATH`, so use the
absolute path there (`which driveledger`).

## Client setup

### Claude Code

```bash
claude mcp add driveledger -- driveledger mcp
# or over HTTP (start `driveledger mcp --http 127.0.0.1:7341` first)
claude mcp add --transport http driveledger http://127.0.0.1:7341/mcp
```

### Claude Desktop

`~/Library/Application Support/Claude/claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "driveledger": {
      "command": "/usr/local/bin/driveledger",
      "args": ["mcp"]
    }
  }
}
```

### Cursor

`~/.cursor/mcp.json` (global) or `.cursor/mcp.json` (project):

```json
{
  "mcpServers": {
    "driveledger": {
      "command": "driveledger",
      "args": ["mcp"]
    }
  }
}
```

Over HTTP: `"driveledger": { "url": "http://127.0.0.1:7341/mcp" }`.

### OpenAI Codex CLI

`~/.codex/config.toml`:

```toml
[mcp_servers.driveledger]
command = "driveledger"
args = ["mcp"]
```

### Gemini CLI

`~/.gemini/settings.json`:

```json
{
  "mcpServers": {
    "driveledger": {
      "command": "driveledger",
      "args": ["mcp"]
    }
  }
}
```

Over HTTP: `"driveledger": { "httpUrl": "http://127.0.0.1:7341/mcp" }`.

### Generic MCP client

- **stdio:** launch `driveledger mcp` and speak newline-delimited JSON-RPC on stdin/stdout.
- **Streamable HTTP:** `POST http://127.0.0.1:7341/mcp` with
  `Accept: application/json, text/event-stream`.

```bash
curl -s -X POST http://127.0.0.1:7341/mcp \
  -H 'Content-Type: application/json' \
  -H 'Accept: application/json, text/event-stream' \
  -d '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"curl","version":"0"}}}'
```

To use another library, add `"env": { "DRIVELEDGER_LIBRARY": "/path/to/library" }` to the
JSON configs, or `env = { DRIVELEDGER_LIBRARY = "..." }` in Codex.

## Tools

Every tool has a JSON Schema for its input and output. Each one returns three things: a 1–3 line
text summary, the same data as JSON text (for clients that ignore structured content), and
`structuredContent` that matches the output schema. Large results are paged: pass `limit`
(default 200, max 5000) and the `next_cursor` from the previous page as `cursor`.

| Tool | Purpose |
|------|---------|
| `list_indexes` | Saved scans with volume, root, date, file count and size. Their `id`s are accepted by the other tools. |
| `list_volumes` | Mounted volumes, and how many indexes each has. |
| `search_files` | By path substring, glob, extension, size range or date range, across one or all indexes. |
| `folder_overview` | Folder tree with sizes and counts down to N levels (like `mapper.sh`). |
| `compare` | Source index (optionally a subfolder) against target index: summary, folder rollup and paged per-file details. |
| `find_file` | Every location of one file's content across all indexes. |
| `find_duplicates` | Groups of identical content within or across indexes. |
| `scan` | Scan a path into the library (`path`), or follow a running scan (`scan_id`). |

`verify` (full-file hashing before deletion) arrives with the verify feature.

### What the compare statuses mean

Matching is **content-based**. Two files match only when their BLAKE3 fingerprint (size plus
three 1 MiB samples) is identical, regardless of name or folder.

| Status | Meaning | Backed up? |
|--------|---------|------------|
| `present` | Same content, same name, somewhere on the target | yes |
| `present_elsewhere` | Same content under another name or path | yes |
| `only_in_bundle` | Same content, but only inside a bundle such as `.dra/MediaFiles` | content exists, but not in a proper place |
| `same_name_different_content` | A file with the same name exists but is **a different file** | **no** |
| `missing` | No identical content anywhere on the target | **no** |

`summary.content_not_found` is what is not backed up. Never treat a same-name match as a copy,
because cameras reuse file names such as `SW-FX30001.MP4` on every card. See
[matching.md](matching.md) for the full rules.

### Errors

Tool errors come back with `isError: true` and
`structuredContent.error = { kind, message }`. The important one is **`target_unavailable`**:
the drive is not mounted. Agents must report that the drive needs to be connected, and must
never conclude that files are missing.

## Example agent prompt

> Is everything in `/Volumes/HYSJ-2026_1/untitled folder2/HYSJ` backed up on `ByHola_1`?

A good agent will:

1. Call `list_volumes` and `list_indexes` to see what is mounted and already scanned.
2. Call `scan` for the source folder and for `/Volumes/ByHola_1` if there is no recent index
   (follow with `scan_id` until `state` is `done`).
3. Call `compare` with the two index IDs, then report `summary.content_not_found` and the
   largest folders in `folders` that still have `missing` or `same_name_different_content` files.
4. Suggest `driveledger verify` on the matched pairs before anything is deleted.

## Without MCP

Every CLI command takes `--json` and prints the same structures. The index files are plain
SQLite, documented in [index-format.md](index-format.md).
