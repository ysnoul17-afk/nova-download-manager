import { CheckCircle2, CircleAlert, Clock, Download, History, Layers, Settings } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, CATEGORIES, EVENT_CLIPBOARD, EVENT_UPDATE, type DownloadInfo, type UiSettings, type UpdateInfo, type UpdateProgress } from "./api";
import { AddDialog } from "./components/AddDialog";
import { CategoryIcon, ToastProvider, useToast } from "./components/common";
import { DownloadsView, matchesFilter, type ListFilter } from "./components/DownloadsView";
import { HistoryView } from "./components/HistoryView";
import { SettingsView } from "./components/SettingsView";
import { UpdateDialog } from "./components/Updates";
import { CATEGORY_LABEL, errorText, formatSpeed } from "./format";
import { useDownloads } from "./useDownloads";

type View = { kind: "downloads"; filter: ListFilter } | { kind: "history" } | { kind: "settings" };

const MAIN_FILTERS: { filter: ListFilter; label: string; icon: typeof Layers }[] = [
  { filter: { kind: "all" }, label: "Tous", icon: Layers },
  { filter: { kind: "active" }, label: "En cours", icon: Download },
  { filter: { kind: "queued" }, label: "En attente", icon: Clock },
  { filter: { kind: "completed" }, label: "Terminés", icon: CheckCircle2 },
  { filter: { kind: "failed" }, label: "Échecs", icon: CircleAlert },
];

function sameFilter(a: ListFilter, b: ListFilter) {
  return a.kind === b.kind && (a.kind !== "category" || (b.kind === "category" && a.category === b.category));
}

function titleFor(f: ListFilter) {
  if (f.kind === "category") return CATEGORY_LABEL[f.category];
  return MAIN_FILTERS.find((m) => sameFilter(m.filter, f))?.label ?? "Téléchargements";
}

const DEFAULT_UI: UiSettings = {
  theme: "system",
  close_to_tray: true,
  confirm_open_executables: true,
  notifications: true,
  clipboard_watch: false,
  auto_update_check: true,
};

export default function App() {
  return (
    <ToastProvider>
      <Shell />
    </ToastProvider>
  );
}

function Shell() {
  const toast = useToast();
  const { items, stats } = useDownloads();
  const [view, setView] = useState<View>({ kind: "downloads", filter: { kind: "all" } });
  const [adding, setAdding] = useState(false);
  const [ui, setUi] = useState<UiSettings>(DEFAULT_UI);
  const [historyKey, setHistoryKey] = useState(0);
  const [limit, setLimit] = useState(0);
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    api.getUi().then(setUi).catch(() => {});
    api.getConfig().then((c) => setLimit(c.global_speed_limit)).catch(() => {});
  }, []);

  // Thème : clair, sombre ou celui de Windows.
  useEffect(() => {
    const root = document.documentElement;
    if (ui.theme === "system") root.removeAttribute("data-theme");
    else root.setAttribute("data-theme", ui.theme);
  }, [ui.theme]);

  // L'historique se met à jour quand un téléchargement change d'état.
  const statusSignature = useMemo(() => [...items.values()].map((i) => i.id + i.status).join("|"), [items]);
  useEffect(() => setHistoryKey((k) => k + 1), [statusSignature]);

  // Raccourcis globaux.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.ctrlKey || e.metaKey;
      if (mod && e.key.toLowerCase() === "n") {
        e.preventDefault();
        setAdding(true);
      } else if (mod && e.key.toLowerCase() === "f") {
        e.preventDefault();
        if (view.kind !== "downloads") setView({ kind: "downloads", filter: { kind: "all" } });
        setTimeout(() => searchRef.current?.focus(), 0);
      } else if (mod && e.key === ",") {
        e.preventDefault();
        setView({ kind: "settings" });
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [view.kind]);

  // Coller une adresse n'importe où (hors champ de saisie) ouvre la fenêtre d'ajout.
  const [pasted, setPasted] = useState("");
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [showUpdate, setShowUpdate] = useState(false);

  // Lien copié dans une autre application (si le réglage est activé) : fenêtre d'ajout pré-remplie.
  // Mise à jour trouvée au démarrage : bandeau discret dans la barre d'état.
  useEffect(() => {
    const uns: (() => void)[] = [];
    listen<string>(EVENT_CLIPBOARD, ({ payload }) => {
      setPasted(payload);
      setAdding(true);
    }).then((u) => uns.push(u));
    listen<UpdateProgress>(EVENT_UPDATE, ({ payload }) => {
      if (payload.phase === "available") setUpdate(payload.info);
    }).then((u) => uns.push(u));
    return () => uns.forEach((u) => u());
  }, []);
  useEffect(() => {
    const onPaste = (e: ClipboardEvent) => {
      const t = e.target as HTMLElement;
      if (t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement || adding) return;
      const text = e.clipboardData?.getData("text") ?? "";
      if (/https?:\/\//i.test(text)) {
        setPasted(text);
        setAdding(true);
      }
    };
    window.addEventListener("paste", onPaste);
    return () => window.removeEventListener("paste", onPaste);
  }, [adding]);

  const count = useCallback((f: ListFilter) => {
    let n = 0;
    for (const d of items.values()) if (matchesFilter(d, f)) n++;
    return n;
  }, [items]);

  const list: DownloadInfo[] = useMemo(() => [...items.values()], [items]);
  const hasPausable = list.some((d) => d.status === "queued" || d.status === "downloading" || d.status === "probing");
  const hasResumable = list.some((d) => d.status === "paused" || d.status === "stopped");

  const toggleLimit = async () => {
    try {
      const cfg = await api.getConfig();
      // Bascule rapide : illimité ↔ 1 Mo/s (réglable dans les paramètres).
      const next = cfg.global_speed_limit ? 0 : 1024 * 1024;
      const saved = await api.setConfig({ ...cfg, global_speed_limit: next });
      setLimit(saved.global_speed_limit);
      toast(next ? "Vitesse limitée à 1 Mo/s." : "Vitesse illimitée.");
    } catch (e) {
      toast(errorText(e), "error");
    }
  };

  return (
    <div className="app">
      <nav className="sidebar" aria-label="Navigation">
        <div className="brand">
          <img src="/logo.svg" alt="" width={28} height={28} />
          <span>Nova</span>
        </div>

        <div className="nav-group">
          {MAIN_FILTERS.map(({ filter, label, icon: Icon }) => {
            const n = count(filter);
            const on = view.kind === "downloads" && sameFilter(view.filter, filter);
            return (
              <button
                key={label}
                className={`nav-item ${on ? "on" : ""}`}
                aria-current={on ? "page" : undefined}
                onClick={() => setView({ kind: "downloads", filter })}
              >
                <Icon size={17} strokeWidth={1.8} />
                <span className="grow">{label}</span>
                {n > 0 && <span className={`count ${filter.kind === "failed" ? "count-alert" : ""}`}>{n}</span>}
              </button>
            );
          })}
        </div>

        <div className="nav-heading">Catégories</div>
        <div className="nav-group">
          {CATEGORIES.map((c) => {
            const filter: ListFilter = { kind: "category", category: c };
            const n = count(filter);
            const on = view.kind === "downloads" && sameFilter(view.filter, filter);
            return (
              <button
                key={c}
                className={`nav-item nav-cat ${on ? "on" : ""}`}
                aria-current={on ? "page" : undefined}
                onClick={() => setView({ kind: "downloads", filter })}
              >
                <CategoryIcon category={c} size={16} />
                <span className="grow">{CATEGORY_LABEL[c]}</span>
                {n > 0 && <span className="count">{n}</span>}
              </button>
            );
          })}
        </div>

        <div className="grow" />
        <div className="nav-group">
          <button
            className={`nav-item ${view.kind === "history" ? "on" : ""}`}
            onClick={() => setView({ kind: "history" })}
          >
            <History size={17} strokeWidth={1.8} />
            <span className="grow">Historique</span>
          </button>
          <button
            className={`nav-item ${view.kind === "settings" ? "on" : ""}`}
            onClick={() => setView({ kind: "settings" })}
          >
            <Settings size={17} strokeWidth={1.8} />
            <span className="grow">Paramètres</span>
          </button>
        </div>
      </nav>

      <main className="content">
        {view.kind === "downloads" && (
          <DownloadsView
            title={titleFor(view.filter)}
            items={items}
            filter={view.filter}
            onAdd={() => setAdding(true)}
            searchRef={searchRef}
            confirmExecutables={ui.confirm_open_executables}
          />
        )}
        {view.kind === "history" && <HistoryView refreshKey={historyKey} />}
        {view.kind === "settings" && (
          <SettingsView
            ui={ui}
            onUiChange={setUi}
            onUpdateAvailable={(info) => {
              setUpdate(info);
              setShowUpdate(true);
            }}
          />
        )}

        <footer className="statusbar">
          <span className={`pulse ${stats.active ? "on" : ""}`} aria-hidden />
          <span className="num">
            {stats.active ? `${stats.active} en cours` : "Aucun transfert"}
            {stats.queued ? `, ${stats.queued} en attente` : ""}
          </span>
          <span className="statusbar-speed num">{stats.active ? formatSpeed(stats.total_speed) : ""}</span>
          <span className="grow" />
          {update && (
            <button className="link-btn update-link" onClick={() => setShowUpdate(true)}>
              Version {update.version} disponible
            </button>
          )}
          <button className="link-btn" onClick={toggleLimit} title="Basculer la limite de vitesse globale">
            {limit ? `Limite : ${formatSpeed(limit)}` : "Vitesse illimitée"}
          </button>
          <button className="link-btn" disabled={!hasResumable} onClick={() => api.resumeAll()}>
            Tout reprendre
          </button>
          <button className="link-btn" disabled={!hasPausable} onClick={() => api.pauseAll()}>
            Tout mettre en pause
          </button>
        </footer>
      </main>

      {showUpdate && update && <UpdateDialog info={update} onClose={() => setShowUpdate(false)} />}
      {adding && (
        <AddDialog
          key={pasted}
          initialText={pasted}
          onClose={() => {
            setAdding(false);
            setPasted("");
          }}
        />
      )}
    </div>
  );
}
