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
