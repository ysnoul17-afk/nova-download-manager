import { listen } from "@tauri-apps/api/event";
import { CheckCircle2, Download, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { api, EVENT_TOOLS, type Tool, type ToolProgress, type ToolsStatus } from "../api";
import { errorText, formatBytes } from "../format";
import { useToast } from "./common";

const TOOL_LABEL: Record<Tool, string> = { yt_dlp: "yt-dlp", ffmpeg: "FFmpeg" };
const TOOL_ROLE: Record<Tool, string> = {
  yt_dlp: "Analyse les pages vidéo et récupère les flux",
  ffmpeg: "Assemble l'image et le son, convertit en MP3",
};
// Tailles indicatives des téléchargements officiels.
const TOOL_SIZE: Record<Tool, string> = { yt_dlp: "environ 18 Mo", ffmpeg: "environ 200 Mo" };

/**
 * État et installation de yt-dlp et FFmpeg. Utilisé dans les paramètres et
 * à la première tentative de téléchargement vidéo (`compact`).
 */
export function ToolsPanel({
  compact,
  onReady,
}: {
  compact?: boolean;
  /** Appelé quand les deux outils sont disponibles. */
  onReady?: () => void;
}) {
  const toast = useToast();
  const [status, setStatus] = useState<ToolsStatus | null>(null);
  const [progress, setProgress] = useState<Partial<Record<Tool, ToolProgress>>>({});
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    api.toolsStatus().then(setStatus).catch((e) => toast(errorText(e), "error"));
    let un: (() => void) | undefined;
    listen<ToolProgress>(EVENT_TOOLS, ({ payload }) => setProgress((p) => ({ ...p, [payload.tool]: payload }))).then(
      (u) => (un = u),
    );
    return () => un?.();
  }, [toast]);

  const install = async (which: Tool[]) => {
    setBusy(true);
    setProgress({});
    try {
      const s = await api.installTools(which);
      setStatus(s);
      toast(which.length > 1 ? "Outils vidéo installés." : `${TOOL_LABEL[which[0]]} est à jour.`);
      if (s.ytdlp && s.ffmpeg) onReady?.();
    } catch (e) {
      toast(errorText(e), "error");
      api.toolsStatus().then(setStatus).catch(() => {});
    } finally {
      setBusy(false);
    }
  };

  if (!status) return <p className="muted">Recherche des outils…</p>;

  const missing: Tool[] = [];
  if (!status.ytdlp) missing.push("yt_dlp");
  if (!status.ffmpeg && status.ffmpeg_installable) missing.push("ffmpeg");
  const rows: { tool: Tool; info: ToolsStatus["ytdlp"] }[] = [
    { tool: "yt_dlp", info: status.ytdlp },
    { tool: "ffmpeg", info: status.ffmpeg },
  ];

  return (
    <div className={`tools ${compact ? "tools-compact" : ""}`}>
      {compact && (
        <p>
          Pour télécharger des vidéos, NDM utilise deux logiciels libres reconnus. Ils sont téléchargés depuis leurs
          dépôts officiels et leur empreinte est vérifiée avant installation.
        </p>
      )}
      {rows.map(({ tool, info }) => {
        const p = progress[tool];
        const pct = p && p.phase === "download" && p.total ? (p.downloaded / p.total) * 100 : null;
        return (
          <div className="tool-row" key={tool}>
            <div className="tool-main">
              <div className="setting-label with-icon">
                {info ? <CheckCircle2 size={16} className="ok-icon" /> : <Download size={16} className="muted" />}
                {TOOL_LABEL[tool]}
                {info?.version && <span className="muted num">{info.version}</span>}
              </div>
              <div className="setting-hint">
                {TOOL_ROLE[tool]}.{" "}
                {info
                  ? info.managed
                    ? "Installé par NDM."
                    : "Trouvé sur le système."
                  : tool === "ffmpeg" && !status.ffmpeg_installable
                    ? "Installez-le avec le gestionnaire de paquets du système."
                    : `Non installé (${TOOL_SIZE[tool]}).`}
              </div>
              {p?.phase === "download" && (
                <div className="tool-progress">
                  <div className="progress progress-downloading">
                    <div className="progress-fill" style={{ width: `${pct ?? 0}%` }} />
                  </div>
                  <span className="muted num">
                    {formatBytes(p.downloaded)}
                    {p.total ? ` sur ${formatBytes(p.total)}` : ""}
                  </span>
                </div>
              )}
              {p?.phase === "error" && <div className="dl-sub is-error">{p.error}</div>}
            </div>
            {!compact && (info?.managed || (!info && (tool === "yt_dlp" || status.ffmpeg_installable))) && (
              <button className="btn" disabled={busy} onClick={() => install([tool])}>
                {info ? (
                  <>
                    <RefreshCw size={15} /> Mettre à jour
                  </>
                ) : (
                  <>
                    <Download size={15} /> Installer
                  </>
                )}
              </button>
            )}
          </div>
        );
      })}
      {compact && missing.length > 0 && (
        <button className="btn btn-primary" disabled={busy} onClick={() => install(missing)}>
          <Download size={16} /> {busy ? "Installation…" : "Installer les outils vidéo"}
        </button>
      )}
      {!compact && status.ytdlp?.managed && (
        <p className="setting-hint">
          Les sites vidéo changent souvent : si un téléchargement échoue, mettez d'abord yt-dlp à jour.
        </p>
      )}
    </div>
  );
}
