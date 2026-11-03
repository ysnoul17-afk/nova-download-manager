import type { Category, DownloadStatus, Priority } from "./api";

const UNITS = ["o", "Ko", "Mo", "Go", "To"];

export function formatBytes(n: number | null | undefined): string {
  if (n == null) return "—";
  let v = n;
  let i = 0;
  while (v >= 1024 && i < UNITS.length - 1) {
    v /= 1024;
    i++;
  }
  const digits = i === 0 ? 0 : v < 10 ? 2 : v < 100 ? 1 : 0;
  return `${v.toLocaleString("fr-FR", { maximumFractionDigits: digits, minimumFractionDigits: digits })} ${UNITS[i]}`;
}

export function formatSpeed(bps: number | null | undefined): string {
  if (!bps) return "—";
  return `${formatBytes(bps)}/s`;
}

export function formatEta(secs: number | null | undefined): string {
  if (secs == null) return "—";
  if (secs < 60) return `${secs} s`;
  const m = Math.floor(secs / 60);
  if (m < 60) return `${m} min ${String(secs % 60).padStart(2, "0")} s`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h} h ${String(m % 60).padStart(2, "0")} min`;
  return `${Math.floor(h / 24)} j`;
}

export function formatDate(ms: number | null | undefined): string {
  if (!ms) return "—";
  const d = new Date(ms);
  const today = new Date();
  const sameDay = d.toDateString() === today.toDateString();
  const time = d.toLocaleTimeString("fr-FR", { hour: "2-digit", minute: "2-digit" });
  if (sameDay) return `Aujourd'hui, ${time}`;
  return `${d.toLocaleDateString("fr-FR", { day: "numeric", month: "short", year: "numeric" })}, ${time}`;
}

export const STATUS_LABEL: Record<DownloadStatus, string> = {
  queued: "En attente",
  probing: "Connexion…",
  downloading: "En cours",
  paused: "En pause",
  finalizing: "Finalisation…",
  completed: "Terminé",
  failed: "Échec",
  stopped: "Arrêté",
};

export const CATEGORY_LABEL: Record<Category, string> = {
  video: "Vidéos",
  music: "Musique",
  document: "Documents",
  image: "Images",
  archive: "Archives",
  software: "Logiciels",
  other: "Autres",
};

export const PRIORITY_LABEL: Record<Priority, string> = {
  high: "Haute",
  normal: "Normale",
  low: "Basse",
};

export function isActive(s: DownloadStatus) {
  return s === "probing" || s === "downloading" || s === "finalizing";
}

export function canPause(s: DownloadStatus) {
  return s === "queued" || isActive(s);
}

export function canResume(s: DownloadStatus) {
  return s === "paused" || s === "stopped" || s === "failed";
}

export function errorText(e: unknown): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return String(e);
}
