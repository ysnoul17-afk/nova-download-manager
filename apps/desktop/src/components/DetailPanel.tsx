import { Copy, ExternalLink, FolderOpen, X } from "lucide-react";
import { api, type DownloadInfo, type Priority } from "../api";
import {
  CATEGORY_LABEL,
  errorText,
  formatBytes,
  formatDate,
  formatEta,
  formatSpeed,
  isActive,
  PRIORITY_LABEL,
} from "../format";
import { CategoryIcon, StatusBadge, useToast } from "./common";
import { SegmentMap } from "./SegmentMap";

const LIMITS = [0, 128, 256, 512, 1024, 2048, 5120, 10240]; // Ko/s

export function DetailPanel({
  item,
  onClose,
  onOpenFile,
}: {
  item: DownloadInfo;
  onClose: () => void;
  onOpenFile: (item: DownloadInfo) => void;
}) {
  const toast = useToast();
  const run = (p: Promise<unknown>) => p.catch((e) => toast(errorText(e), "error"));
  const pct = item.total_size ? Math.min(100, (item.downloaded / item.total_size) * 100) : null;

  const copyUrl = async () => {
    try {
      await navigator.clipboard.writeText(item.url);
      toast("Adresse copiée.");
    } catch {
      toast("Impossible d'accéder au presse-papiers.", "error");
    }
  };

  return (
    <aside className="detail" aria-label="Détails du téléchargement">
      <header className="detail-head">
        <CategoryIcon category={item.category} size={22} />
        <div className="detail-title">
          <h2 title={item.filename}>{item.filename || "Analyse en cours…"}</h2>
          <StatusBadge status={item.status} />
        </div>
        <button className="icon-btn" onClick={onClose} aria-label="Fermer le panneau">
          <X size={18} />
        </button>
      </header>

      <section className="detail-progress">
        <div className="detail-percent">
          <span className="big num">{pct == null ? "—" : `${pct.toLocaleString("fr-FR", { maximumFractionDigits: pct >= 99.95 || pct === 0 ? 0 : 1, minimumFractionDigits: pct >= 99.95 || pct === 0 ? 0 : 1 })} %`}</span>
          <span className="muted num">
            {formatBytes(item.downloaded)} sur {formatBytes(item.total_size)}
          </span>
        </div>
        <SegmentMap item={item} />
        <div className="detail-legend muted">
          {item.status === "downloading" && item.active_connections === 0
            ? "Connexion perdue : nouvelle tentative automatique…"
            : isActive(item.status)
            ? `${item.active_connections} connexion${item.active_connections > 1 ? "s" : ""} active${item.active_connections > 1 ? "s" : ""} sur ${item.max_connections}`
            : item.status === "completed"
              ? "Fichier complet, taille vérifiée"
              : item.resumable
                ? "Reprise possible à tout moment"
                : "Ce serveur ne permet pas la reprise : une interruption recommence le fichier"}
        </div>
      </section>

      {item.error && <p className="detail-error">{item.error}</p>}

      <dl className="facts">
        <dt>Vitesse</dt>
        <dd className="num">{item.status === "completed" ? formatSpeed(item.avg_speed) + " en moyenne" : formatSpeed(item.speed)}</dd>
        <dt>Temps restant</dt>
        <dd className="num">{isActive(item.status) ? formatEta(item.eta_secs) : "—"}</dd>
        <dt>Type</dt>
        <dd>{CATEGORY_LABEL[item.category]}</dd>
        <dt>Ajouté</dt>
        <dd>{formatDate(item.added_at)}</dd>
        {item.completed_at && (
          <>
            <dt>Terminé</dt>
            <dd>{formatDate(item.completed_at)}</dd>
          </>
        )}
        <dt>Dossier</dt>
        <dd className="path" title={item.directory}>
          {item.directory}
        </dd>
        <dt>Adresse</dt>
        <dd className="path url" title={item.url}>
          {item.url}
        </dd>
      </dl>

      <div className="detail-controls">
        <label className="field">
          <span>Priorité</span>
          <select value={item.priority} onChange={(e) => run(api.setPriority([item.id], e.target.value as Priority))}>
            {(["high", "normal", "low"] as Priority[]).map((p) => (
              <option key={p} value={p}>
                {PRIORITY_LABEL[p]}
              </option>
            ))}
          </select>
        </label>
        {item.status !== "completed" && (
          <label className="field">
            <span>Vitesse max.</span>
            <select
              value={Math.round(item.speed_limit / 1024)}
              onChange={(e) => run(api.setSpeedLimit(item.id, Number(e.target.value) * 1024))}
            >
              {LIMITS.map((k) => (
                <option key={k} value={k}>
                  {k === 0 ? "Illimitée" : k >= 1024 ? `${k / 1024} Mo/s` : `${k} Ko/s`}
                </option>
              ))}
              {!LIMITS.includes(Math.round(item.speed_limit / 1024)) && (
                <option value={Math.round(item.speed_limit / 1024)}>{formatSpeed(item.speed_limit)}</option>
              )}
            </select>
          </label>
        )}
      </div>

      <footer className="detail-actions">
        {item.status === "completed" && (
          <button className="btn btn-primary" onClick={() => onOpenFile(item)}>
            <ExternalLink size={16} /> Ouvrir
          </button>
        )}
        <button className="btn" onClick={() => run(api.openFolder(item.id))}>
          <FolderOpen size={16} /> Afficher dans le dossier
        </button>
        <button className="btn btn-ghost" onClick={copyUrl}>
          <Copy size={16} /> Copier l'adresse
        </button>
      </footer>
    </aside>
  );
}
