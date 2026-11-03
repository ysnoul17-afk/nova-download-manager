import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, RotateCcw } from "lucide-react";
import { useEffect, useState } from "react";
import { api, CATEGORIES, type Category, type EngineConfig, type UiSettings } from "../api";
import { CATEGORY_LABEL, errorText } from "../format";
import { CategoryIcon, useToast } from "./common";

export function SettingsView({ ui, onUiChange }: { ui: UiSettings; onUiChange: (s: UiSettings) => void }) {
  const toast = useToast();
  const [cfg, setCfg] = useState<EngineConfig | null>(null);
  const [dirs, setDirs] = useState<Record<Category, string> | null>(null);
  const [info, setInfo] = useState<{ version: string; data_dir: string } | null>(null);

  useEffect(() => {
    api.getConfig().then(setCfg).catch((e) => toast(errorText(e), "error"));
    api.categoryDirs().then(setDirs).catch(() => {});
    api.appInfo().then(setInfo).catch(() => {});
  }, [toast]);

  /** Enregistre immédiatement chaque modification (pas de bouton « Enregistrer » à oublier). */
  const save = async (next: EngineConfig) => {
    setCfg(next);
    try {
      const saved = await api.setConfig(next);
      setCfg(saved);
      setDirs(await api.categoryDirs());
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const saveUi = async (next: UiSettings) => {
    onUiChange(next);
    try {
      await api.setUi(next);
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  const pick = async (title: string) => {
    const d = await open({ directory: true, multiple: false, title });
    return typeof d === "string" ? d : null;
  };

  if (!cfg) return <div className="settings" />;

  const limitKo = Math.round(cfg.global_speed_limit / 1024);

  return (
    <div className="settings">
      <header className="topbar">
        <h1>Paramètres</h1>
      </header>
      <div className="settings-body">
        <section className="panel">
          <h2>Apparence et comportement</h2>
          <div className="setting">
            <div>
              <div className="setting-label">Thème</div>
            </div>
            <div className="segmented" role="radiogroup" aria-label="Thème">
              {(
                [
                  ["system", "Système"],
                  ["light", "Clair"],
                  ["dark", "Sombre"],
                ] as const
              ).map(([v, label]) => (
                <button
                  key={v}
                  role="radio"
                  aria-checked={ui.theme === v}
                  className={ui.theme === v ? "on" : ""}
                  onClick={() => saveUi({ ...ui, theme: v })}
                >
                  {label}
                </button>
              ))}
            </div>
          </div>
          <Toggle
            label="Réduire dans la zone de notification à la fermeture"
            hint="Les téléchargements continuent fenêtre fermée. Quittez depuis l'icône de la zone de notification."
            checked={ui.close_to_tray}
            onChange={(v) => saveUi({ ...ui, close_to_tray: v })}
          />
          <Toggle
            label="Demander confirmation avant d'ouvrir un programme téléchargé"
            checked={ui.confirm_open_executables}
            onChange={(v) => saveUi({ ...ui, confirm_open_executables: v })}
          />
        </section>

        <section className="panel">
          <h2>Téléchargements</h2>
          <NumberSetting
            label="Téléchargements simultanés"
            hint="Les suivants attendent dans la file, par ordre de priorité."
            min={1}
            max={16}
            value={cfg.max_concurrent_downloads}
            onChange={(v) => save({ ...cfg, max_concurrent_downloads: v })}
          />
          <NumberSetting
            label="Connexions par fichier"
            hint="Plus n'est pas toujours plus rapide : certains serveurs limitent ou refusent les connexions multiples. NDM s'adapte automatiquement."
            min={1}
            max={32}
            value={cfg.connections_per_download}
            onChange={(v) => save({ ...cfg, connections_per_download: v })}
          />
          <NumberSetting
            label="Vitesse maximale globale"
            hint="En Ko/s. 0 = illimitée."
            min={0}
            max={10_000_000}
            step={128}
            value={limitKo}
            unit="Ko/s"
            onChange={(v) => save({ ...cfg, global_speed_limit: v * 1024 })}
          />
          <NumberSetting
            label="Nouvelles tentatives en cas d'erreur"
            min={0}
            max={50}
            value={cfg.max_retries}
            onChange={(v) => save({ ...cfg, max_retries: v })}
          />
          <NumberSetting
            label="Délai sans données avant reconnexion"
            min={5}
            max={300}
            unit="s"
            value={cfg.read_timeout_secs}
            onChange={(v) => save({ ...cfg, read_timeout_secs: v })}
          />
        </section>

        <section className="panel">
          <h2>Dossiers</h2>
          <div className="setting">
            <div>
              <div className="setting-label">Dossier principal</div>
              <div className="setting-hint">Chaque catégorie y a son sous-dossier, sauf si vous en choisissez un autre.</div>
            </div>
            <div className="dir-pick">
              <span className="path" title={cfg.base_directory}>
                {cfg.base_directory}
              </span>
              <button
                className="btn"
                onClick={async () => {
                  const d = await pick("Dossier principal des téléchargements");
                  if (d) save({ ...cfg, base_directory: d });
                }}
              >
                <FolderOpen size={16} /> Changer
              </button>
            </div>
          </div>
          {dirs &&
            CATEGORIES.map((c) => {
              const custom = cfg.category_directories[c];
              return (
                <div className="setting" key={c}>
                  <div className="setting-label with-icon">
                    <CategoryIcon category={c} size={16} /> {CATEGORY_LABEL[c]}
                  </div>
                  <div className="dir-pick">
                    <span className={`path ${custom ? "" : "muted"}`} title={dirs[c]}>
                      {dirs[c]}
                    </span>
                    <button
                      className="btn"
                      onClick={async () => {
                        const d = await pick(`Dossier pour « ${CATEGORY_LABEL[c]} »`);
                        if (d) save({ ...cfg, category_directories: { ...cfg.category_directories, [c]: d } });
                      }}
                    >
                      Changer
                    </button>
                    {custom && (
                      <button
                        className="icon-btn"
                        title="Revenir au dossier par défaut"
                        aria-label="Revenir au dossier par défaut"
                        onClick={() => {
                          const next = { ...cfg.category_directories };
                          delete next[c];
                          save({ ...cfg, category_directories: next });
                        }}
                      >
                        <RotateCcw size={15} />
                      </button>
                    )}
                  </div>
                </div>
              );
            })}
        </section>

        {info && (
          <p className="about muted">
            Nova Download Manager {info.version}. Données locales : {info.data_dir}
          </p>
        )}
      </div>
    </div>
  );
}

function Toggle({
  label,
  hint,
  checked,
  onChange,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <label className="setting">
      <div>
        <div className="setting-label">{label}</div>
        {hint && <div className="setting-hint">{hint}</div>}
      </div>
      <input type="checkbox" className="switch" checked={checked} onChange={(e) => onChange(e.target.checked)} />
    </label>
  );
}

function NumberSetting({
  label,
  hint,
  value,
  min,
  max,
  step = 1,
  unit,
  onChange,
}: {
  label: string;
  hint?: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  unit?: string;
  onChange: (v: number) => void;
}) {
  const [draft, setDraft] = useState(String(value));
  useEffect(() => setDraft(String(value)), [value]);
  const commit = () => {
    const n = Math.round(Number(draft));
    if (Number.isFinite(n)) {
      const clamped = Math.max(min, Math.min(max, n));
      if (clamped !== value) onChange(clamped);
      setDraft(String(clamped));
    } else setDraft(String(value));
  };
  return (
    <label className="setting">
      <div>
        <div className="setting-label">{label}</div>
        {hint && <div className="setting-hint">{hint}</div>}
      </div>
      <span className="num-input">
        <input
          type="number"
          min={min}
          max={max}
          step={step}
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={commit}
          onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
        />
        {unit && <span className="muted">{unit}</span>}
      </span>
    </label>
  );
}
