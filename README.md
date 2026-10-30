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
