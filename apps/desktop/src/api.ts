// Types et appels vers le cœur Rust. Les noms de champs suivent la sérialisation serde.
import { invoke } from "@tauri-apps/api/core";

export type DownloadStatus =
  | "queued"
  | "probing"
  | "downloading"
  | "paused"
  | "finalizing"
  | "completed"
  | "failed"
  | "stopped";

export type Priority = "low" | "normal" | "high";

export type Category = "video" | "music" | "document" | "image" | "archive" | "software" | "other";

export const CATEGORIES: Category[] = ["video", "music", "document", "image", "archive", "software", "other"];

export interface DownloadInfo {
  id: string;
  url: string;
  final_url: string | null;
  filename: string;
  directory: string;
  category: Category;
  status: DownloadStatus;
  priority: Priority;
  total_size: number | null;
  downloaded: number;
  speed: number;
  avg_speed: number | null;
  eta_secs: number | null;
  active_connections: number;
  max_connections: number;
  resumable: boolean;
  speed_limit: number;
  error: string | null;
  added_at: number;
  completed_at: number | null;
  seq: number;
}

export type EngineEvent =
  | { type: "added"; info: DownloadInfo }
  | { type: "progress"; items: DownloadInfo[] }
  | { type: "status_changed"; info: DownloadInfo }
  | { type: "removed"; id: string }
  | { type: "stats"; total_speed: number; active: number; queued: number };

export interface EngineConfig {
  base_directory: string;
  category_directories: Partial<Record<Category, string>>;
  max_concurrent_downloads: number;
  connections_per_download: number;
  min_segment_size: number;
  global_speed_limit: number;
  max_retries: number;
  user_agent: string;
  connect_timeout_secs: number;
  read_timeout_secs: number;
}

export interface UiSettings {
  theme: "system" | "light" | "dark";
  close_to_tray: boolean;
  confirm_open_executables: boolean;
}

export type SortKey = "added_at" | "completed_at" | "size" | "speed" | "type" | "name";

export interface HistoryQuery {
  search?: string | null;
  statuses?: DownloadStatus[];
  categories?: Category[];
  from_ms?: number | null;
  to_ms?: number | null;
  sort?: SortKey;
  descending?: boolean;
  limit?: number | null;
}

export interface SegmentView {
  start: number;
  end: number | null;
  downloaded: number;
  active: boolean;
}

export interface AddOptions {
  urls: string[];
  directory?: string | null;
  filename?: string | null;
  startPaused?: boolean;
  priority?: Priority;
  connections?: number | null;
}

export interface AddResult {
  added: DownloadInfo[];
  failed: { url: string; error: string }[];
}

export const api = {
  list: () => invoke<DownloadInfo[]>("list_downloads"),
  add: (options: AddOptions) => invoke<AddResult>("add_downloads", { options }),
  pause: (ids: string[]) => invoke<void>("pause_downloads", { ids }),
  resume: (ids: string[]) => invoke<void>("resume_downloads", { ids }),
  stop: (ids: string[]) => invoke<void>("stop_downloads", { ids }),
  restart: (ids: string[]) => invoke<void>("restart_downloads", { ids }),
  remove: (ids: string[], deleteFiles: boolean) => invoke<void>("remove_downloads", { ids, deleteFiles }),
  setPriority: (ids: string[], priority: Priority) => invoke<void>("set_priority", { ids, priority }),
  setSpeedLimit: (id: string, bytesPerSec: number) => invoke<void>("set_download_speed_limit", { id, bytesPerSec }),
  segments: (id: string) => invoke<SegmentView[]>("get_segments", { id }),
  pauseAll: () => invoke<void>("pause_all"),
  resumeAll: () => invoke<void>("resume_all"),
  history: (query: HistoryQuery) => invoke<DownloadInfo[]>("query_history", { query }),
  openFile: (id: string) => invoke<void>("open_file", { id }),
  openFolder: (id: string) => invoke<void>("open_folder", { id }),
  getConfig: () => invoke<EngineConfig>("get_engine_config"),
  setConfig: (config: EngineConfig) => invoke<EngineConfig>("set_engine_config", { config }),
  getUi: () => invoke<UiSettings>("get_ui_settings"),
  setUi: (settings: UiSettings) => invoke<void>("set_ui_settings", { settings }),
  categoryDirs: () => invoke<Record<Category, string>>("get_category_directories"),
  appInfo: () => invoke<{ version: string; data_dir: string }>("app_info"),
};

export const EVENT_ENGINE = "ndm://engine";
export const EVENT_RESYNC = "ndm://resync";
