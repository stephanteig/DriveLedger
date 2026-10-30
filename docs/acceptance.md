# Acceptance (v1.0.0)

Run on 2026-10-30 on a MacBook Pro (Apple Silicon), macOS 27, DriveLedger 1.0.0.

## 1. Fixture replay of the recorded cases

The original source folders were deleted after the manual checks, so every pattern in
`reference/README.md` is reproduced by fixture tests (`cargo test --workspace`, all green):

| Recorded pattern | Test(s) | Result |
|---|---|---|
| Case 01: `_HYSJ`/`_SVAL` prefixes vs `HYSJ`/`SVAL`, truncated `SW-FX30157.MP4` (4096-aligned), 0 missing | `case01_reserve_byhola`, `truncated_copy_is_flagged`, `reorganised_folders_are_present` | everything `present`, 1 `same_name_different_content` with `possibly_truncated`, 0 missing |
| Case 02: `.af` files newer on the target, 0 missing | `case02_af_version_drift`, `version_drift_reports_newer_side`, `bundle_copy_ranks_above_same_name_different_content` | `.af` files `same_name_different_content`, `newer_side = target`; an old copy inside `.dra` is `only_in_bundle` |
| Case 03: `:` → `_` rename, `(1)` renames, pink logos only in `.dra/MediaFiles`, GretlandReno camera-name trap, missing partner footage | `case03_hysj`, `renamed_files_are_present_elsewhere`, `match_only_inside_dra_bundle_is_only_in_bundle`, `camera_trap_same_name_and_size_is_not_backed_up` | `present_elsewhere`, `only_in_bundle`, `same_name_different_content` (not backed up), `missing` rolled up under `04_Partnere/Partnerslipp/Red-Bull` |
| NFC/NFD (`Råmateriale`, `STØY`) | `nfc_and_nfd_names_match` | `present` |
| Unmounted target (all 843 files reported missing by the prototype) | `unmounted_target_is_unavailable_never_missing` (core), `unmounted_target_exits_3_with_json_error` (CLI), `unmounted_target_is_an_error_not_missing_files` (MCP) | `TargetUnavailable` / exit code 3 / `target_unavailable`; never a missing list |
| Read-only guarantee | `scan_is_read_only`, `live_compare_is_read_only_on_both_sides`, `verify_confirms_matches_and_catches_what_sampling_misses` | size, mtime and full content of every fixture entry unchanged |

## 2. Live oracle check

Pair chosen by the user:

- Source: `/Volumes/Stephan SW T7/STUDIO-WALLIN/FredrikstadSentrum/20260911 - Kulturnatt/Foto/Raw` (51 files, 2.30 GB)
- Target: `/Volumes/HYSJ-2026_1/20260911 - Kulturnatt/Foto/Raw` (43 files, 1.95 GB)

```bash
driveledger compare "$SRC" "$TGT" --json > dl.json
python3 -I reference/prototype/compare_content_sample.py "$SRC" "$TGT" > oracle.txt
```

| | Found | Missing |
|---|---|---|
| `compare_content_sample.py` | 43 | 8 (0.35 GB) |
| DriveLedger (`present` + `present_elsewhere` + `only_in_bundle` / rest) | 43 | 8 (352.1 MB) |

The missing sets are identical path for path (`3U7A9575.CR3`, `3U7A9580.CR3`, `3U7A9583.CR3`,
`3U7A9588.CR3`, `3U7A9592.CR3`, `3U7A9595.CR3`, `3U7A9599.CR3`, `3U7A9606.CR3`), and so are the found
counts. No files were renamed, so `present_elsewhere` was 0 on both sides. DriveLedger took 1.5 s,
scanning both folders into the library.

## 3. MCP in two clients

Both clients ran headless against `driveledger mcp` (stdio) with the same prompt: call
`list_indexes`, compare the `Stephan SW T7` index against the `HYSJ-2026_1` index, and answer
with found/not-found counts and the names of the files that are not backed up.

| Client | Tools called | Answer |
|---|---|---|
| Claude Code 2.1.314 (`claude -p --mcp-config`) | `list_indexes`, `compare` | found 43, not found 8, the same 8 `.CR3` files |
| OpenAI Codex CLI 0.151.0 (`codex exec -c mcp_servers.driveledger...`) | `list_indexes`, `compare` | found 43, not found 8, the same 8 `.CR3` files |

The first Claude Code run logged `unknown format "uint64" ignored`. Non-standard numeric
`format`s are now stripped from all tool schemas, a test covers it, and the rerun was clean.

## 4. App and `.dmg`

`npm run tauri build` (in `app/`) produces:

- `target/release/bundle/macos/DriveLedger.app` (7.3 MiB)
- `target/release/bundle/dmg/DriveLedger_1.0.0_aarch64.dmg` (3.5 MiB)

The bundled app starts without errors and lists mounted volumes and saved indexes.

The build is **unsigned**. On first launch macOS Gatekeeper blocks it: open
**System Settings → Privacy & Security**, find the DriveLedger message and click **Open Anyway**
(or right-click the app in Finder → **Open**).

## Not covered

- The Compare and Search screens were built and type-checked, but not clicked through in an
  automated way. Their commands are thin wrappers over the tested core `compare`, `verify` and
  `search`.
- The live check used a small pair (51 files). A large drive pair is worth running once with
  `driveledger compare --reuse` before relying on it for deletions.
