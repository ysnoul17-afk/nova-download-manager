// Types et réglages partagés entre le service worker, le script de contenu et la fenêtre.

export const HOST_NAME = "app.novadm.host";

export interface Settings {
  /** auto : NDM prend en charge les téléchargements reconnus ; manual : le navigateur les garde. */
  mode: "auto" | "manual";
  /** Extensions interceptées en mode automatique. */
  extensions: string[];
  /** Afficher « Télécharger cette vidéo » sur les lecteurs. */
  videoButton: boolean;
  /** Transmettre les cookies du site (autorisation « cookies » demandée à l'activation). */
  sendCookies: boolean;
}

export const DEFAULT_EXTENSIONS =
  "3gp 7z aac apk avi bin bz2 dmg exe flac flv gz img iso m4a m4v mkv mov mp3 mp4 mpeg mpg msi msix ogg opus pdf rar tar tgz wav webm wma wmv xz zip zst";

export const DEFAULT_SETTINGS: Settings = {
  mode: "auto",
  extensions: DEFAULT_EXTENSIONS.split(" "),
  videoButton: true,
  sendCookies: false,
};

export async function loadSettings(): Promise<Settings> {
  const s = await chrome.storage.local.get("settings");
  return { ...DEFAULT_SETTINGS, ...(s.settings ?? {}) };
}

export async function saveSettings(patch: Partial<Settings>): Promise<Settings> {
  const next = { ...(await loadSettings()), ...patch };
  await chrome.storage.local.set({ settings: next });
  return next;
}

/** Réponse de l'hôte natif / de l'application. */
export interface Reply<T = unknown> {
  ok: boolean;
  data?: T;
  error?: string;
  /** native_unavailable, app_not_running, app_not_installed, invalid_request, probe_failed… */
  code?: string;
}

export type MediaFormat =
  | { type: "best" }
  | { type: "video"; max_height: number }
  | { type: "audio_mp3" }
  | { type: "audio_m4a" };

export interface MediaOption {
  id: string;
  label: string;
  container: string;
  format: MediaFormat;
  est_size: number | null;
}

export interface MediaInfo {
  url: string;
  title: string;
  duration_secs: number | null;
  thumbnail: string | null;
  site: string | null;
  options: MediaOption[];
  drm_filtered: boolean;
}

export type Analysis =
  | { kind: "file"; filename: string | null; size: number | null; mime: string | null }
  | { kind: "media"; info: MediaInfo }
  | { kind: "page"; message: string }
  | { kind: "needs_tools"; missing: string[] }
  | { kind: "error"; message: string };

/** Messages du script de contenu et de la fenêtre vers le service worker. */
export type Msg =
  | { type: "analyze"; url: string }
  | { type: "download"; url: string; referrer?: string; filename?: string }
  | { type: "download_media"; url: string; option: MediaOption; title: string }
  | { type: "detected_streams" }
  | { type: "status" }
  | { type: "show" };

const UNITS = ["o", "Ko", "Mo", "Go", "To"];
export function formatBytes(n: number | null | undefined): string {
  if (n == null) return "";
  let v = n;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  const d = i === 0 ? 0 : v < 10 ? 1 : 0;
  return `${v.toLocaleString("fr-FR", { maximumFractionDigits: d, minimumFractionDigits: d })} ${UNITS[i]}`;
}

export function formatDuration(secs: number | null | undefined): string {
  if (!secs) return "";
  const s = Math.round(secs);
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = String(s % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${r}` : `${m}:${r}`;
}

/** Message lisible pour une réponse en erreur. */
export function errorMessage(r: Reply): string {
  switch (r.code) {
    case "native_unavailable":
      return "Nova Download Manager n'est pas installé, ou l'extension ne peut pas le joindre.";
    case "app_not_running":
      return "Nova Download Manager n'est pas lancé.";
    case "app_not_installed":
      return "Nova Download Manager est introuvable : réinstallez l'application.";
    default:
      return r.error || "Erreur inconnue.";
  }
}

/** Choix envoyé à l'application (même forme que MediaSpec côté Rust). */
export interface MediaSpecLike {
  format: MediaFormat;
  subtitles: boolean;
  label: string;
  title: string;
  est_size: number | null;
}
