# How matching works

DriveLedger answers one question per source file: **does identical content exist anywhere on the
target?** Folder structure is irrelevant, and file names are only used to describe a match or to
explain a miss.

## Fingerprints

Each file gets a `sample3` fingerprint: BLAKE3 over

```
size (u64 little-endian) || first 1 MiB || middle 1 MiB || last 1 MiB
```

The middle sample starts at `size / 2 - 512 KiB`. Files of 3 MiB or less are hashed in full. The
sample positions are the same as in `reference/prototype/compare_content_sample.py`, which was used
for the manual checks.

Including the size means a truncated copy never matches the original. Sampling means a 12 GB clip
on an SMB share costs about 3 MiB of reads. The trade-off is that two files of the same size that
differ only outside the sampled regions would look identical. For camera footage this does not
happen in practice. Before deleting anything, use `verify`, which hashes every byte.

## Classification

Each source file is matched against the whole target index and gets exactly one status, checked
in this order:

| # | Status | Rule |
|---|--------|------|
| 1 | `present` | Identical fingerprint on the target **and** the same NFC file name. |
| 2 | `present_elsewhere` | Identical fingerprint, but only under other names. Every location is reported. |
| 3 | `only_in_bundle` | Identical fingerprint, but every match is inside a bundle (`in_bundle = 1`). |
| 4 | `same_name_different_content` | No identical content, but a file with the same NFC name exists. |
| 5 | `missing` | No identical content anywhere on the target. |

`present`, `present_elsewhere` and `only_in_bundle` together are **content found**.
`same_name_different_content` and `missing` together are **content not found**: those files are
not backed up on the target.

### Bundles

Folders with the extensions `.dra`, `.app`, `.fcpbundle`, `.RDC`, `.photoslibrary`, `.band` and
`.logicx` are macOS packages. They are scanned like normal folders, but their files get
`in_bundle = 1`.

A copy inside a bundle (for example a logo in `Project.dra/MediaFiles/...`) proves that the
content exists, but it is not a proper home for the file. So for rules 1 and 2, matches inside
bundles are ignored, and if they are the only matches the file is `only_in_bundle`.

**Exception:** if the source file is itself inside a bundle, bundle matches count as proper
homes. RED footage lives in `.RDC` folders, so `A001.RDC/A001_C001.R3D` matched against another
`A001.RDC/A001_C001.R3D` is `present`.

### Same name, different content

A name match (even name **and** size) is never treated as a copy. Sony FX3 cameras restart
`SW-FX30001.MP4` on every card, so different shoots share names and sometimes sizes. Only the
fingerprint decides.

When a same-name file exists, DriveLedger picks the closest candidate (longest shared path
suffix, then smallest size difference) and reports:

- `newer_side`: `source`, `target` or `same`, by modification time. Used for version drift, such as
  an Affinity `.af` file that was edited further on the target.
- hint `possibly_truncated`: the source is smaller than the candidate and its size is a multiple
  of 4096. That is the typical signature of an interrupted copy (like `SW-FX30157.MP4`, which
  stopped at 1,783,848,960 bytes).

All same-name candidates are listed in `target_locations`, closest first.

### Empty files

Every empty file has the same fingerprint, so content says nothing. An empty source file is
`present` only if an empty file with the same name exists on the target, and `missing` otherwise.
This matches the prototype.

### Unreadable source files

If a source file could not be read during the scan, it has no fingerprint. It is reported as
`missing` with the hint `source_unreadable`: DriveLedger cannot prove it is backed up.

### Names and Unicode

All names and paths are compared after NFC normalisation, so `Råmateriale` written as NFD on one
file system matches NFC on another.

## Unavailable targets

Before comparing against a live path, DriveLedger checks that it is mounted and readable. Under
`/Volumes/<name>`, the volume folder must be a real mount point, not an empty placeholder left by
an unmounted drive. If the check fails, the result is a `TargetUnavailable` error, never a list of
missing files. Scans refuse unmounted roots for the same reason, and a compare against an empty
target index carries a warning.

## Folder rollup

The summary groups source files by folder, cut to a configurable depth (default 3), with counts
and bytes for each status. That is the same view used during the manual checks, for example
`04_Partnere/Partnerslipp/Red-Bull`: 36.7 GB missing.

## Relation to the prototype oracle

`compare_content_sample.py` reports *found* and *missing*. For any source/target pair:

- prototype *found* = `present` + `present_elsewhere` + `only_in_bundle`
- prototype *missing* = `same_name_different_content` + `missing`

Phase 9 acceptance checks that these sets are identical on a real drive pair.
