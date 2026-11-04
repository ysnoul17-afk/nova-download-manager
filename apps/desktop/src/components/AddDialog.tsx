import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, Loader2 } from "lucide-react";
import { useMemo, useState } from "react";
import { api, type DownloadInfo, type MediaInfo, type MediaSpec, type Priority } from "../api";
import { errorText, PRIORITY_LABEL } from "../format";
import { Modal, useToast } from "./common";
import { DuplicateDialog } from "./DuplicateDialog";
import { MediaPicker } from "./MediaPicker";
import { ToolsPanel } from "./ToolsPanel";

/** Extrait les URL http(s) d'un texte collé (une par ligne, ou mêlées à du texte). */
export function extractUrls(text: string): string[] {
  // S'arrête aussi au début d'une autre adresse : « …/a.zipHttps://…/b.zip » donne deux adresses.
  const found = text.match(/https?:\/\/[^\s"'<>]+?(?=https?:\/\/|[\s"'<>]|$)/gi) ?? [];
  return [...new Set(found.map((u) => u.replace(/[),.;]+$/, "")))];
}

type Step =
  | { kind: "input" }
  | { kind: "analyzing" }
  | { kind: "media"; info: MediaInfo }
  | { kind: "tools" }
  | { kind: "page"; message: string }
  | { kind: "error"; message: string }
  | { kind: "duplicates"; items: DownloadInfo[] };

export function AddDialog({ onClose, initialText = "" }: { onClose: () => void; initialText?: string }) {
  const toast = useToast();
  const [text, setText] = useState(initialText);
  const [directory, setDirectory] = useState<string | null>(null);
  const [filename, setFilename] = useState("");
  const [priority, setPriority] = useState<Priority>("normal");
  const [startPaused, setStartPaused] = useState(false);
  const [connections, setConnections] = useState<number | null>(null);
  const [step, setStep] = useState<Step>({ kind: "input" });
  const [busy, setBusy] = useState(false);

  const urls = useMemo(() => extractUrls(text), [text]);
  const single = urls.length === 1;

  const pickFolder = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Dossier de destination" });
    if (typeof dir === "string") setDirectory(dir);
  };

  /** Ajout direct (fichiers, ou lots : les sites vidéo connus passent par yt-dlp). */
  const addFiles = async (only?: string[], allowDuplicates = false) => {
    setBusy(true);
    try {
      const res = await api.add({
        urls: only ?? urls,
        directory,
        filename: single && filename.trim() ? filename.trim() : null,
        priority,
        startPaused,
        connections,
        allowDuplicates,
      });
      if (res.failed.length) toast(res.failed.map((f) => `${f.url} : ${f.error}`).join("\n"), "error");
      if (res.added.length) {
        toast(res.added.length === 1 ? "Téléchargement ajouté." : `${res.added.length} téléchargements ajoutés.`);
      }
      if (res.duplicates.length) {
        setStep({ kind: "duplicates", items: res.duplicates });
      } else if (res.added.length) {
        onClose();
      }
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };

  /** Une seule adresse : on regarde d'abord s'il s'agit d'un fichier ou d'une page vidéo. */
  const analyze = async () => {
    setStep({ kind: "analyzing" });
    try {
      const r = await api.analyzeUrl(urls[0]);
      switch (r.kind) {
        case "file":
          setStep({ kind: "input" });
          await addFiles();
          return;
        case "media":
          setStep({ kind: "media", info: r.info });
          return;
        case "needs_tools":
          setStep({ kind: "tools" });
          return;
        case "page":
          setStep({ kind: "page", message: r.message });
          return;
        case "error":
          setStep({ kind: "error", message: r.message });
          return;
      }
    } catch (e) {
      setStep({ kind: "error", message: errorText(e) });
    }
  };

  const submit = () => {
    if (urls.length === 0 || busy) return;
    // Un nom de fichier imposé signifie un fichier direct.
    if (single && !filename.trim()) analyze();
    else addFiles();
  };

  const addMedia = async (spec: MediaSpec) => {
    setBusy(true);
    try {
      await api.addMedia({ url: urls[0], spec, directory, priority, startPaused });
      toast(`Vidéo ajoutée : ${spec.label}.`);
      onClose();
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };

  // --- Étapes après analyse -------------------------------------------------

  if (step.kind === "duplicates") {
    const run = (p: Promise<unknown>) => p.then(onClose).catch((e) => toast(errorText(e), "error"));
    return (
      <DuplicateDialog
        items={step.items}
        onOpen={(d) => run(api.openFile(d.id))}
        onReveal={(d) => run(api.openFolder(d.id))}
        onRedownload={() => addFiles(step.items.map((d) => d.url), true)}
        onClose={onClose}
      />
    );
  }

  if (step.kind === "media") {
    return (
      <Modal
        title="Télécharger cette vidéo"
        onClose={onClose}
        wide
        footer={
          <>
            <button className="btn btn-ghost grow-left" onClick={() => setStep({ kind: "input" })}>
              Retour
            </button>
            <button className="btn" onClick={onClose}>
              Annuler
            </button>
            <button className="btn btn-primary" type="submit" form="media-picker" disabled={busy} data-autofocus>
              Télécharger
            </button>
          </>
        }
      >
        <MediaPicker info={step.info} onChoose={addMedia} />
      </Modal>
    );
  }

  if (step.kind === "tools") {
    return (
      <Modal
        title="Installer les outils vidéo"
        onClose={onClose}
        footer={
          <>
            <button className="btn btn-ghost grow-left" onClick={() => setStep({ kind: "input" })}>
              Retour
            </button>
            <button className="btn" onClick={onClose}>
              Plus tard
            </button>
          </>
        }
      >
        <ToolsPanel compact onReady={analyze} />
      </Modal>
    );
  }

  if (step.kind === "page" || step.kind === "error") {
    return (
      <Modal
        title={step.kind === "page" ? "Aucune vidéo sur cette page" : "Analyse impossible"}
        onClose={onClose}
        footer={
          <>
            <button className="btn btn-ghost grow-left" onClick={() => setStep({ kind: "input" })}>
              Retour
            </button>
            <button className="btn" onClick={() => addFiles()} disabled={busy}>
              {step.kind === "page" ? "Enregistrer la page web" : "Télécharger quand même"}
            </button>
            <button className="btn btn-primary" onClick={onClose} data-autofocus>
              Fermer
            </button>
          </>
        }
      >
        <p>{step.message}</p>
        <p className="muted">{urls[0]}</p>
      </Modal>
    );
  }

  // --- Saisie ---------------------------------------------------------------

  const analyzing = step.kind === "analyzing";
  return (
    <Modal
      title="Ajouter des téléchargements"
      onClose={onClose}
      wide
      footer={
        <>
          <span className="muted grow">
            {analyzing
              ? "Analyse de l'adresse…"
              : urls.length === 0
                ? "Aucune adresse détectée"
                : urls.length === 1
                  ? "1 adresse"
                  : `${urls.length} adresses`}
          </span>
          <button className="btn" onClick={onClose}>
            Annuler
          </button>
          <button className="btn btn-primary" disabled={urls.length === 0 || busy || analyzing} onClick={submit}>
            {analyzing && <Loader2 size={16} className="spin" />}
            {startPaused ? "Ajouter en pause" : "Télécharger"}
          </button>
        </>
      }
    >
      <label className="field">
        <span>Adresses de fichiers ou de vidéos (une par ligne)</span>
        <textarea
          data-autofocus
          rows={5}
          value={text}
          spellCheck={false}
          disabled={analyzing}
          placeholder="https://exemple.com/fichier.zip ou https://www.youtube.com/watch?v=…"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) submit();
          }}
        />
      </label>
      {urls.length > 1 && (
        <p className="setting-hint">
          En lot, les vidéos des sites reconnus (YouTube, Vimeo…) sont téléchargées en meilleure qualité MP4.
        </p>
      )}

      {single && (
        <label className="field">
          <span>Nom du fichier (facultatif, fichiers uniquement)</span>
          <input value={filename} onChange={(e) => setFilename(e.target.value)} placeholder="Déduit automatiquement" />
        </label>
      )}

      <div className="field">
        <span>Dossier de destination</span>
        <div className="row">
          <input readOnly value={directory ?? "Dossier de la catégorie (automatique)"} className="grow" />
          <button className="btn" onClick={pickFolder}>
            <FolderOpen size={16} /> Choisir…
          </button>
          {directory && (
            <button className="btn btn-ghost" onClick={() => setDirectory(null)}>
              Automatique
            </button>
          )}
        </div>
      </div>

      <div className="grid-3">
        <label className="field">
          <span>Priorité</span>
          <select value={priority} onChange={(e) => setPriority(e.target.value as Priority)}>
            {(["high", "normal", "low"] as Priority[]).map((p) => (
              <option key={p} value={p}>
                {PRIORITY_LABEL[p]}
              </option>
            ))}
          </select>
        </label>
        <label className="field">
          <span>Connexions (fichiers)</span>
          <select
            value={connections ?? ""}
            onChange={(e) => setConnections(e.target.value ? Number(e.target.value) : null)}
          >
            <option value="">Réglage par défaut</option>
            {[1, 2, 4, 8, 16, 24, 32].map((n) => (
              <option key={n} value={n}>
                {n}
              </option>
            ))}
          </select>
        </label>
        <label className="check">
          <input type="checkbox" checked={startPaused} onChange={(e) => setStartPaused(e.target.checked)} />
          Ajouter sans démarrer
        </label>
      </div>
    </Modal>
  );
}
