import { ask } from "@tauri-apps/plugin-dialog";
import { ArrowDownToLine, Pause, Play, Plus, RotateCcw, Search, Square, Trash2 } from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { api, type DownloadInfo, type Priority } from "../api";
import {
  canPause,
  canResume,
  errorText,
  formatBytes,
  formatEta,
  formatSpeed,
  isActive,
  PRIORITY_LABEL,
  STATUS_LABEL,
} from "../format";
import { CategoryIcon, Modal, Progress, useToast } from "./common";
import { DetailPanel } from "./DetailPanel";

export type ListFilter =
  | { kind: "all" }
  | { kind: "active" }
  | { kind: "queued" }
  | { kind: "completed" }
  | { kind: "failed" }
  | { kind: "category"; category: DownloadInfo["category"] };

export function matchesFilter(d: DownloadInfo, f: ListFilter): boolean {
  switch (f.kind) {
    case "all":
      return true;
    case "active":
      return isActive(d.status);
    case "queued":
      return d.status === "queued" || d.status === "paused" || d.status === "stopped";
    case "completed":
      return d.status === "completed";
    case "failed":
      return d.status === "failed";
    case "category":
      return d.category === f.category;
  }
}

export function DownloadsView({
  title,
  items,
  filter,
  onAdd,
  searchRef,
  confirmExecutables,
}: {
  title: string;
  items: Map<string, DownloadInfo>;
  filter: ListFilter;
  onAdd: () => void;
  searchRef: React.RefObject<HTMLInputElement | null>;
  confirmExecutables: boolean;
}) {
  const toast = useToast();
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [anchor, setAnchor] = useState<string | null>(null);
  const [removing, setRemoving] = useState<string[] | null>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const rows = useMemo(() => {
    const q = search.trim().toLowerCase();
    return [...items.values()]
      .filter((d) => matchesFilter(d, filter))
      .filter((d) => !q || d.filename.toLowerCase().includes(q) || d.url.toLowerCase().includes(q))
      .sort((a, b) => b.seq - a.seq);
  }, [items, filter, search]);

  // La sélection ne garde que les éléments encore visibles.
  useEffect(() => {
    setSelected((prev) => {
      const visible = new Set(rows.map((r) => r.id));
      const next = new Set([...prev].filter((id) => visible.has(id)));
      return next.size === prev.size ? prev : next;
    });
  }, [rows]);

  const sel = rows.filter((r) => selected.has(r.id));
  const ids = sel.map((s) => s.id);
  const single = sel.length === 1 ? sel[0] : null;

  const run = (p: Promise<unknown>) => p.catch((e) => toast(errorText(e), "error"));

  const openFile = async (d: DownloadInfo) => {
    if (d.status !== "completed") return;
    if (confirmExecutables && d.category === "software") {
      const ok = await ask(
        `« ${d.filename} » est un programme. Ne l'ouvrez que si vous faites confiance à sa provenance.`,
        { title: "Ouvrir un programme téléchargé", kind: "warning", okLabel: "Ouvrir", cancelLabel: "Annuler" },
      );
      if (!ok) return;
    }
    run(api.openFile(d.id));
  };

  const onRowClick = (e: React.MouseEvent, d: DownloadInfo) => {
    if (e.shiftKey && anchor) {
      const a = rows.findIndex((r) => r.id === anchor);
      const b = rows.findIndex((r) => r.id === d.id);
      const [from, to] = a < b ? [a, b] : [b, a];
      setSelected(new Set(rows.slice(from, to + 1).map((r) => r.id)));
      return;
    }
    if (e.ctrlKey || e.metaKey) {
      setSelected((prev) => {
        const next = new Set(prev);
        if (next.has(d.id)) next.delete(d.id);
        else next.add(d.id);
        return next;
      });
    } else {
      setSelected(new Set([d.id]));
    }
    setAnchor(d.id);
  };

  const anyPausable = sel.some((s) => canPause(s.status));
  const anyResumable = sel.some((s) => canResume(s.status));

  const onKeyDown = (e: React.KeyboardEvent) => {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
    if (e.key === "a" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      setSelected(new Set(rows.map((r) => r.id)));
    } else if (e.key === "Delete" && ids.length) {
      setRemoving(ids);
    } else if (e.key === " " && ids.length) {
      e.preventDefault();
      if (anyPausable) run(api.pause(ids));
      else if (anyResumable) run(api.resume(ids));
    } else if (e.key === "Enter" && single) {
      openFile(single);
    } else if ((e.key === "ArrowDown" || e.key === "ArrowUp") && rows.length) {
      e.preventDefault();
      const cur = single ? rows.findIndex((r) => r.id === single.id) : -1;
      const next = Math.max(0, Math.min(rows.length - 1, cur + (e.key === "ArrowDown" ? 1 : -1)));
      setSelected(new Set([rows[next].id]));
      setAnchor(rows[next].id);
      listRef.current?.querySelectorAll<HTMLElement>(".dl-row")[next]?.scrollIntoView({ block: "nearest" });
    } else if (e.key === "Escape") {
      setSelected(new Set());
    }
  };

  return (
    <div className={`downloads ${single ? "with-detail" : ""}`}>
      <div className="main-col">
        <header className="topbar">
          <h1>{title}</h1>
          <div className="search">
            <Search size={16} aria-hidden />
            <input
              ref={searchRef}
              value={search}
              onChange={(e) => setSearch(e.target.value)}
              placeholder="Rechercher un nom ou une adresse"
              aria-label="Rechercher"
            />
          </div>
          <button className="btn btn-primary" onClick={onAdd} title="Ctrl+N">
            <Plus size={16} /> Ajouter
          </button>
        </header>

        <div className="toolbar" role="toolbar" aria-label="Actions sur la sélection">
          <button className="tool" disabled={!anyResumable} onClick={() => run(api.resume(ids))} title="Reprendre (Espace)">
            <Play size={16} /> <span className="tool-label">Reprendre</span>
          </button>
          <button className="tool" disabled={!anyPausable} onClick={() => run(api.pause(ids))} title="Pause (Espace)">
            <Pause size={16} /> <span className="tool-label">Pause</span>
          </button>
          <button
            className="tool"
            disabled={!sel.some((s) => canPause(s.status))}
            onClick={() => run(api.stop(ids))}
            title="Arrêter (le fichier partiel est conservé)"
          >
            <Square size={15} /> <span className="tool-label">Arrêter</span>
          </button>
          <button
            className="tool"
            disabled={!sel.length}
            onClick={() => run(api.restart(ids))}
            title="Recommencer depuis le début"
          >
            <RotateCcw size={16} /> <span className="tool-label">Relancer</span>
          </button>
          <button className="tool tool-danger" disabled={!sel.length} onClick={() => setRemoving(ids)} title="Supprimer (Suppr)">
            <Trash2 size={16} /> <span className="tool-label">Supprimer</span>
          </button>
          <span className="toolbar-sep" />
          <label className="tool-select">
            <span className="tool-label">Priorité</span>
            <select
              disabled={!sel.length}
              value={sel.length && sel.every((s) => s.priority === sel[0].priority) ? sel[0].priority : ""}
              onChange={(e) => e.target.value && run(api.setPriority(ids, e.target.value as Priority))}
            >
              <option value="" disabled>
                —
              </option>
              {(["high", "normal", "low"] as Priority[]).map((p) => (
                <option key={p} value={p}>
                  {PRIORITY_LABEL[p]}
                </option>
              ))}
            </select>
          </label>
          <span className="grow" />
          {sel.length > 1 && <span className="muted">{sel.length} sélectionnés</span>}
        </div>

        <div className="list" ref={listRef} tabIndex={0} onKeyDown={onKeyDown} role="listbox" aria-multiselectable>
          {rows.length === 0 ? (
            <Empty filtered={!!search || filter.kind !== "all"} onAdd={onAdd} />
          ) : (
            rows.map((d) => (
              <Row
                key={d.id}
                d={d}
                selected={selected.has(d.id)}
                onClick={(e) => onRowClick(e, d)}
                onDoubleClick={() => openFile(d)}
              />
            ))
          )}
        </div>
      </div>

      {single && <DetailPanel item={single} onClose={() => setSelected(new Set())} onOpenFile={openFile} />}

      {removing && (
        <RemoveDialog
          items={rows.filter((r) => removing.includes(r.id))}
          onClose={() => setRemoving(null)}
          onConfirm={async (deleteFiles) => {
            const target = removing;
            setRemoving(null);
            try {
              await api.remove(target, deleteFiles);
              toast(target.length === 1 ? "Téléchargement supprimé." : `${target.length} téléchargements supprimés.`);
            } catch (e) {
              toast(errorText(e), "error");
            }
          }}
        />
      )}
    </div>
  );
}

function Row({
  d,
  selected,
  onClick,
  onDoubleClick,
}: {
  d: DownloadInfo;
  selected: boolean;
  onClick: (e: React.MouseEvent) => void;
  onDoubleClick: () => void;
}) {
  const progress = d.total_size ? d.downloaded / d.total_size : d.status === "completed" ? 1 : null;
  const live = isActive(d.status);
  const reconnecting = d.status === "downloading" && d.active_connections === 0;
  const sub =
    d.status === "failed" && d.error
      ? d.error
      : reconnecting
        ? `Reconnexion au serveur… (${formatBytes(d.downloaded)} sur ${formatBytes(d.total_size)})`
        : d.status === "finalizing" && d.stage
        ? `${d.stage}…`
        : d.status === "downloading"
        ? `${d.stage ? `${d.stage} : ` : ""}${formatBytes(d.downloaded)} sur ${d.media_label && d.total_size ? "≈ " : ""}${formatBytes(d.total_size)}`
        : d.status === "completed"
          ? formatBytes(d.total_size)
          : `${STATUS_LABEL[d.status]}${d.total_size ? `, ${formatBytes(d.downloaded)} sur ${formatBytes(d.total_size)}` : ""}`;

  return (
    <div
      className={`dl-row ${selected ? "selected" : ""} st-${d.status}`}
      role="option"
      aria-selected={selected}
      onClick={onClick}
      onDoubleClick={onDoubleClick}
    >
      <CategoryIcon category={d.category} />
      <div className="dl-name">
        <div className="dl-filename" title={d.filename}>
          {d.filename || hostOf(d.url)}
          {d.media_label && <span className="media-tag">{d.media_label}</span>}
          {d.priority !== "normal" && <span className={`prio prio-${d.priority}`}>{PRIORITY_LABEL[d.priority]}</span>}
        </div>
        <div className={`dl-sub ${d.status === "failed" ? "is-error" : ""}`} title={sub}>
          {sub}
        </div>
      </div>
      <div className="dl-progress">
        <Progress value={live && !d.total_size ? null : progress} status={d.status} />
      </div>
      <div className="dl-speed num">{d.status === "downloading" && !reconnecting ? formatSpeed(d.speed) : ""}</div>
      <div className="dl-eta num">
        {d.status === "downloading" && !reconnecting ? formatEta(d.eta_secs) : d.status === "completed" ? <ArrowDownToLine size={14} aria-label="Terminé" /> : ""}
      </div>
    </div>
  );
}

function hostOf(url: string) {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}

function Empty({ filtered, onAdd }: { filtered: boolean; onAdd: () => void }) {
  if (filtered) return <p className="empty">Aucun téléchargement ne correspond.</p>;
  return (
    <div className="empty">
      <p>Collez une adresse pour lancer votre premier téléchargement.</p>
      <button className="btn btn-primary" onClick={onAdd}>
        <Plus size={16} /> Ajouter un téléchargement
      </button>
    </div>
  );
}

function RemoveDialog({
  items,
  onClose,
  onConfirm,
}: {
  items: DownloadInfo[];
  onClose: () => void;
  onConfirm: (deleteFiles: boolean) => void;
}) {
  const [deleteFiles, setDeleteFiles] = useState(false);
  const completed = items.filter((i) => i.status === "completed").length;
  return (
    <Modal
      title={items.length === 1 ? "Supprimer ce téléchargement ?" : `Supprimer ${items.length} téléchargements ?`}
      onClose={onClose}
      footer={
        <>
          <button className="btn" onClick={onClose}>
            Annuler
          </button>
          <button className="btn btn-danger" data-autofocus onClick={() => onConfirm(deleteFiles)}>
            Supprimer
          </button>
        </>
      }
    >
      <p>
        {items.length === 1 ? `« ${items[0].filename || items[0].url} »` : "Les éléments sélectionnés"} seront retirés de la liste
        et de l'historique. Les fichiers partiels sont effacés.
      </p>
      {completed > 0 && (
        <label className="check">
          <input type="checkbox" checked={deleteFiles} onChange={(e) => setDeleteFiles(e.target.checked)} />
          Supprimer aussi {completed === 1 ? "le fichier téléchargé" : `les ${completed} fichiers téléchargés`} du disque
        </label>
      )}
    </Modal>
  );
}
