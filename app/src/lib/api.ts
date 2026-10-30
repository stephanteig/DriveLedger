// Typed wrappers around the Tauri commands in src-tauri/src/commands.rs.
// Field names mirror the Rust structs (snake_case).

import { invoke } from "@tauri-apps/api/core";

export interface CommandError {
  kind: string;
  message: string;
}

export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e) return String((e as CommandError).message);
  return String(e);
}

export interface SkipList {
  names: string[];
  prefixes: string[];
}

export interface Settings {
  library: string | null;
  skip: SkipList;
  bundle_extensions: string[];
}

export interface SettingsView {
  settings: Settings;
  library: string;
  default_library: string;
  settings_path: string | null;
}

export interface VolumeInfo {
  name: string;
  uuid: string | null;
  fs_type: string | null;
  mount_point: string;
}

export interface VolumeStatus {
  volume: VolumeInfo;
  index_count: number;
  last_scanned_at: string | null;
}

export interface IndexSummary {
  id: string;
  path: string;
  volume_name: string;
  volume_uuid: string | null;
  root_path: string;
  scanned_at: string;
  file_count: number;
  total_bytes: number;
  error_count: number;
}

export type ScanPhase = "walking" | "processing" | "writing" | "done";

export interface ScanProgress {
  phase: ScanPhase;
  files_seen: number;
  files_done: number;
  bytes_hashed: number;
  errors: number;
  current_path: string;
}

export interface ScanInfo {
  volume_name: string;
  volume_uuid: string | null;
  fs_type: string | null;
  mount_point: string;
  root_path: string;
  started_at: string;
  finished_at: string | null;
  tool_version: string;
  file_count: number;
  total_bytes: number;
  error_count: number;
}

export interface ScanStats {
  files_hashed: number;
  files_reused: number;
  bytes_hashed: number;
  previous_index: string | null;
}

export interface ScanOutcome {
  index_path: string;
  info: ScanInfo;
  stats: ScanStats;
}

export interface ScanProgressEvent {
  scan_id: string;
  progress: ScanProgress;
}

export interface ScanFinishedEvent {
  scan_id: string;
  outcome: ScanOutcome | null;
  error: CommandError | null;
}

export const api = {
  getSettings: () => invoke<SettingsView>("get_settings"),
  saveSettings: (settings: Settings) => invoke<SettingsView>("save_settings", { settings }),
  listVolumes: () => invoke<VolumeStatus[]>("list_volumes"),
  listIndexes: () => invoke<IndexSummary[]>("list_saved_indexes"),
  startScan: (path: string, full = false) => invoke<string>("start_scan", { path, full }),
  cancelScan: (scanId: string) => invoke<void>("cancel_scan", { scanId }),
};

/** Index ID from an index file path. */
export function indexIdFromPath(path: string): string {
  const name = path.split("/").pop() ?? path;
  return name.replace(/\.dlidx\.sqlite$/, "");
}

// ---- Compare, verify, search ------------------------------------------------

export type Status =
  | "present"
  | "present_elsewhere"
  | "only_in_bundle"
  | "same_name_different_content"
  | "missing";

export const STATUSES: Status[] = [
  "present",
  "present_elsewhere",
  "only_in_bundle",
  "same_name_different_content",
  "missing",
];

export const STATUS_LABEL: Record<Status, string> = {
  present: "Present",
  present_elsewhere: "Present elsewhere",
  only_in_bundle: "Only in bundle",
  same_name_different_content: "Different version",
  missing: "Missing",
};

export const STATUS_TONE = {
  present: "cyan",
  present_elsewhere: "violet",
  only_in_bundle: "violet",
  same_name_different_content: "amber",
  missing: "magenta",
} as const satisfies Record<Status, string>;

export interface Totals {
  files: number;
  bytes: number;
}

export type StatusTotals = Record<Status, Totals>;

export interface Location {
  rel_path: string;
  size: number;
  modified: string | null;
  in_bundle: boolean;
}

export interface FileComparison {
  rel_path: string;
  size: number;
  modified: string | null;
  status: Status;
  target_locations: Location[];
  newer_side: "source" | "target" | "same" | null;
  hints: ("possibly_truncated" | "source_unreadable")[];
}

export interface FolderRollup {
  folder: string;
  total: Totals;
  by_status: StatusTotals;
}

export interface IndexRef {
  index_path: string | null;
  volume_name: string;
  root_path: string;
  scanned_at: string;
  file_count: number;
}

export interface CompareResult {
  source: IndexRef;
  target: IndexRef;
  source_subpath: string | null;
  summary: {
    total: Totals;
    content_found: Totals;
    content_not_found: Totals;
    by_status: StatusTotals;
  };
  folders: FolderRollup[];
  files: FileComparison[];
  warnings: string[];
}

export type VerifyStatus = "verified" | "mismatch" | "unreadable";

export interface VerifyItem {
  source_path: string;
  target_path: string;
  size: number;
  status: VerifyStatus;
  source_hash: string | null;
  target_hash: string | null;
  message: string | null;
}

export interface VerifyReport {
  summary: Record<VerifyStatus | "not_found_on_target", Totals>;
  items: VerifyItem[];
}

export interface FileRecord {
  rel_path: string;
  rel_path_nfc: string;
  name_nfc: string;
  ext: string;
  size: number;
  mtime: number | null;
  in_bundle: boolean;
  fingerprint: string | null;
  full_hash: string | null;
}

export interface SearchHit {
  index_id: string;
  volume_name: string;
  root_path: string;
  file: FileRecord;
}

export interface Page<T> {
  items: T[];
  total: number;
  next_cursor: string | null;
}

export interface SearchQuery {
  text?: string | null;
  glob?: string | null;
  extensions?: string[];
  min_size?: number | null;
  max_size?: number | null;
  modified_after?: string | null;
  modified_before?: string | null;
  fingerprint?: string | null;
}

export const compareApi = {
  compare: (source: string, target: string, subpath: string | null) =>
    invoke<CompareResult>("compare", { source, target, subpath, depth: 3 }),
  exportReport: (source: string, target: string, subpath: string | null, out: string, format: "tsv" | "json") =>
    invoke<void>("export_compare", { source, target, subpath, out, format }),
  verify: (source: string, target: string, paths: string[]) =>
    invoke<VerifyReport>("verify", { source, target, paths }),
  search: (query: SearchQuery, limit = 2000) => invoke<Page<SearchHit>>("search_files", { query, limit }),
  isAvailable: (path: string) => invoke<boolean>("is_available", { path }),
};

/** Join a scan root and a relative path. */
export function joinPath(root: string, rel: string): string {
  return root.endsWith("/") ? root + rel : `${root}/${rel}`;
}
