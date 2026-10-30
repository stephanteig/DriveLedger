# Index format

Every scan produces one SQLite file in the library folder:

```
<library>/<volume>__<root-slug>__<YYYYMMDD-HHMMSS>.dlidx.sqlite
```

The file is self-contained and can be read by any SQLite client, so agents without MCP can
query it directly. Open it read-only (`sqlite3 -readonly`, or `mode=ro` in a URI).

The current schema version is **1**.

## `meta`

| key              | value                                      |
|------------------|--------------------------------------------|
| `schema_version` | Integer as text. Refuse versions you don't know. |
| `created_by`     | `driveledger <version>`                    |

## `scan` (exactly one row, `id = 1`)

| column         | type    | notes |
|----------------|---------|-------|
| `volume_name`  | TEXT    | e.g. `ByHola_1` |
| `volume_uuid`  | TEXT    | From `diskutil` on macOS. NULL for SMB shares and when unknown. |
| `fs_type`      | TEXT    | e.g. `apfs`, `exfat`, `smbfs` |
| `mount_point`  | TEXT    | e.g. `/Volumes/ByHola_1` |
| `root_path`    | TEXT    | Absolute path that was scanned |
| `started_at`   | TEXT    | RFC 3339, UTC |
| `finished_at`  | TEXT    | RFC 3339, UTC. A finished index always has it. |
| `tool_version` | TEXT    | DriveLedger version |
| `file_count`   | INTEGER | Rows in `file` |
| `total_bytes`  | INTEGER | Sum of `file.size` |
| `error_count`  | INTEGER | Rows in `error` |

## `file`

| column             | type    | notes |
|--------------------|---------|-------|
| `rel_path`         | TEXT    | Path relative to `root_path`, `/`-separated, exactly as on disk (may be NFD) |
| `rel_path_nfc`     | TEXT    | NFC-normalised `rel_path`. Use this for lookups. |
| `name_nfc`         | TEXT    | NFC-normalised file name |
| `ext`              | TEXT    | Lower-case extension without the dot, or `''` |
| `size`             | INTEGER | Bytes |
| `mtime`            | INTEGER | Modification time, **nanoseconds** since the Unix epoch |
| `ctime`            | INTEGER | Creation (birth) time, nanoseconds since the Unix epoch. NULL where the file system does not provide it. |
| `in_bundle`        | INTEGER | 1 if a parent folder is a macOS package (`.dra`, `.app`, `.fcpbundle`, `.RDC`, `.photoslibrary`, `.band`, `.logicx`) |
| `fingerprint`      | TEXT    | Hex BLAKE3 content fingerprint. NULL if the file could not be read (see `error`). |
| `fingerprint_kind` | TEXT    | `sample3` or `full` |
| `full_hash`        | TEXT    | Hex BLAKE3 of the whole file, filled in by `driveledger verify` |

Indexes exist on `fingerprint`, `name_nfc`, `size` and `rel_path_nfc`.

### Fingerprints

- `sample3`: BLAKE3 over `size (u64 little-endian) || first 1 MiB || middle 1 MiB || last 1 MiB`.
  The middle sample starts at `size / 2 - 512 KiB`. Files of 3 MiB or less are hashed whole
  (still prefixed by the size).
- `full`: BLAKE3 over the whole file. When present, `fingerprint` still holds the `sample3`
  value so indexes remain comparable; `full_hash` holds the full hash.

## `error`

Paths that could not be indexed. Unreadable files are always recorded here, never silently
skipped.

| column     | type | notes |
|------------|------|-------|
| `rel_path` | TEXT | Relative path (empty for the root itself) |
| `kind`     | TEXT | `walk`, `metadata`, `read`, `skipped_symlink`, `skipped_special`, `non_utf8_name` |
| `message`  | TEXT | OS error text |

## Skipped names

Not indexed and not recorded as errors: `.DS_Store`, `._*`, `Icon\r`, `.Spotlight-V100`,
`.fseventsd`, `.Trashes`, `.TemporaryItems`, `.DocumentRevisions-V100`. The list is
configurable.

## Example queries

```sql
-- Where does this content live?
SELECT rel_path, size FROM file WHERE fingerprint = ?;

-- Biggest extensions
SELECT ext, COUNT(*), SUM(size) / 1e9 AS gb FROM file GROUP BY ext ORDER BY gb DESC;
```
