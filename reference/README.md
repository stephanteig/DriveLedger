# Reference material: the manual checks from 2026-10-30

DriveLedger automates a manual workflow done with Claude Code on 2026-10-30. This folder holds everything from that session.

## scripts/
- `filliste.sh`: full file list to TSV (`size_mb, modified, path`). DriveLedger's TSV export must match this format.
- `mapper.sh`: folder overview to TSV (`size, N files, path`, default depth 4). The `folder_overview` tool must match this.

The originals are still in `~/` because the user runs them from there.

## prototype/
Python prototypes of the matching logic. Usage: `python3 -I <script> <source> <target>`.
- `compare_name_size.py`: matches by NFC filename and size anywhere on the target. The first version. It is **fooled by reused camera filenames**.
- `compare_content_sample.py`: matches by size plus a SHA-1 of 3 × 1 MiB samples (first, middle and last) anywhere on the target, so it catches renamed and moved files. This is the logic DriveLedger's `sample3` fingerprint is based on (DriveLedger uses BLAKE3 instead of SHA-1).

**Use `compare_content_sample.py` as a test oracle:** on any real source/target pair, DriveLedger's present/missing split must agree with it.

## ground-truth/
The source folders were **deleted by the user after the checks**, so these results cannot be re-run live. Treat them as recorded expected outcomes, and use them to design fixture tests.

### Case 01: `Reserve/StephanT7-BACKUP_7August2026/ByHola` → `ByHola_1` (no file saved; recorded here)
- 79 files (about 82 GB of R3D, MP4 and LRF footage). The folders were prefixed `_HYSJ`, `_LUSH`, `_STØY` and `_SVAL`, while the target used `HYSJ`, `LUSH`, etc.
- 78 matched by name and size.
- 1 same name, different size: `_SVAL/2027/05_PROJEKTER/.../SW-FX30157.MP4`. The source is 1,783,848,960 bytes, a multiple of 4096, and **truncated** (ffprobe: "moov atom not found"). The target is 2,416,201,903 bytes and plays (38.4 s).
- 0 missing.

### Case 02: `HYSJ-2026_1/untitled folder2/ByHola` → `ByHola_1`
File: `02_..._name-size.txt`
- 843 files (about 70 GB). 834 matched by name and size, and 0 were missing.
- 9 Affinity `.af` files had the same name but a different size. **The target was newer in all 9** (version drift). Some also had copies inside DaVinci `.dra/MediaFiles` bundles.

### Case 03: `HYSJ-2026_1/untitled folder2/HYSJ` → `ByHola_1` (the target layout was `HYSJ/2026/...`)
Files: `03_..._name-size.txt`, `03_..._content-sample.txt` and `03_..._missing-list.txt`
- 1,156 files, about 157 GB.
- **Name and size only:** 759 matched, 13 had the same name but a different size, and 384 were missing.
- **Content sample:** 760 found and **396 missing (about 100.18 GB)**. Of the 760 found, 7 were under a different name or path:
  - 1 OdaRose `.mov` whose name had `:` changed to `_`.
  - 4 SpareBank 1 logo files differing only by `(1)` in the name.
  - 2 pink logo PNGs found **only** inside `ByHola/Projects/.../*.dra/MediaFiles/...`. These should be classified `only_in_bundle`.
- **Camera filename trap:** GretlandReno `SW-FX3000x.MP4` and `M01.XML` files shared names, and in one case also the size (`SW-FX30001M01.XML`), with different shoots on the target. Only the content sample showed they were different files.
- The missing files were mostly `04_Partnere/Partnerslipp` (Red-Bull 36.7 GB, Studio-Wallin 24.5 GB, ...) and `04_Partnere/Partnervideo` (GretlandReno 19.7 GB, Viedu 11.8 GB).

### Incident: unmounted target
In one run `ByHola_1` was not mounted, and the prototype reported **all 843 files as missing**. DriveLedger must detect this and return `TargetUnavailable`.
