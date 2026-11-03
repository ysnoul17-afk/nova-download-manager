import { listen } from "@tauri-apps/api/event";
import { Download, RefreshCw } from "lucide-react";
import { useEffect, useState } from "react";
import { api, EVENT_UPDATE, type UpdateInfo, type UpdateProgress } from "../api";
import { errorText, formatBytes } from "../format";
import { Modal, useToast } from "./common";

/** Fenêtre d'installation : notes de version, progression, puis redémarrage automatique. */
export function UpdateDialog({ info, onClose }: { info: UpdateInfo; onClose: () => void }) {
  const toast = useToast();
  const [progress, setProgress] = useState<{ done: number; total: number | null } | null>(null);
  const [installing, setInstalling] = useState(false);

  useEffect(() => {
    let un: (() => void) | undefined;
    listen<UpdateProgress>(EVENT_UPDATE, ({ payload }) => {
      if (payload.phase === "download") setProgress({ done: payload.downloaded, total: payload.total });
      if (payload.phase === "installing") setInstalling(true);
    }).then((u) => (un = u));
    return () => un?.();
  }, []);

  const busy = progress !== null || installing;
  const install = async () => {
    setProgress({ done: 0, total: null });
    try {
      await api.installUpdate(); // l'application redémarre si tout va bien
    } catch (e) {
      toast(errorText(e), "error");
      setProgress(null);
      setInstalling(false);
    }
  };
  const pct = progress?.total ? (progress.done / progress.total) * 100 : 0;

  return (
    <Modal
      title={`Nova Download Manager ${info.version} est disponible`}
      onClose={busy ? () => {} : onClose}
      footer={
        <>
          <button className="btn" onClick={onClose} disabled={busy}>
            Plus tard
          </button>
          <button className="btn btn-primary" onClick={install} disabled={busy} data-autofocus>
            <Download size={16} /> {installing ? "Installation…" : busy ? "Téléchargement…" : "Installer et redémarrer"}
          </button>
        </>
      }
    >
      <p>
        Version actuelle : {info.current_version}. Les téléchargements en cours seront mis en pause puis reprendront
        après le redémarrage.
      </p>
      {info.notes && <pre className="release-notes">{info.notes}</pre>}
      {progress && (
        <div className="tool-progress">
          <div className="progress progress-downloading">
            <div className="progress-fill" style={{ width: `${pct}%` }} />
          </div>
          <span className="muted num">
            {formatBytes(progress.done)}
            {progress.total ? ` sur ${formatBytes(progress.total)}` : ""}
          </span>
        </div>
      )}
      <p className="setting-hint">La signature de la mise à jour est vérifiée avant l'installation.</p>
    </Modal>
  );
}

/** Section « Mises à jour » des paramètres. */
export function UpdatesPanel({
  version,
  autoCheck,
  onAutoCheck,
  onAvailable,
}: {
  version: string;
  autoCheck: boolean;
  onAutoCheck: (v: boolean) => void;
  onAvailable: (info: UpdateInfo) => void;
}) {
  const [state, setState] = useState<{ checking: boolean; text?: string; error?: boolean }>({ checking: false });

  const check = async () => {
    setState({ checking: true });
    try {
      const info = await api.checkUpdate();
      if (info) {
        setState({ checking: false, text: `Version ${info.version} disponible.` });
        onAvailable(info);
      } else {
        setState({ checking: false, text: "Vous avez la dernière version." });
      }
    } catch (e) {
      setState({ checking: false, text: errorText(e), error: true });
    }
  };

  return (
    <>
      <div className="setting">
        <div>
          <div className="setting-label">Version installée : {version}</div>
          {state.text && <div className={`setting-hint ${state.error ? "is-error" : ""}`}>{state.text}</div>}
        </div>
        <button className="btn" onClick={check} disabled={state.checking}>
          <RefreshCw size={15} className={state.checking ? "spin" : ""} /> Rechercher
        </button>
      </div>
      <label className="setting">
        <div>
          <div className="setting-label">Rechercher les mises à jour au démarrage</div>
          <div className="setting-hint">Rien n'est installé sans votre accord.</div>
        </div>
        <input type="checkbox" className="switch" checked={autoCheck} onChange={(e) => onAutoCheck(e.target.checked)} />
      </label>
    </>
  );
}
