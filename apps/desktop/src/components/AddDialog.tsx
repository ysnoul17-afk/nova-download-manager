import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen } from "lucide-react";
import { useMemo, useState } from "react";
import { api, type Priority } from "../api";
import { errorText, PRIORITY_LABEL } from "../format";
import { Modal, useToast } from "./common";

/** Extrait les URL http(s) d'un texte collé (une par ligne, ou mêlées à du texte). */
function extractUrls(text: string): string[] {
  // S'arrête aussi au début d'une autre adresse : « …/a.zipHttps://…/b.zip » donne deux adresses.
  const found = text.match(/https?:\/\/[^\s"'<>]+?(?=https?:\/\/|[\s"'<>]|$)/gi) ?? [];
  return [...new Set(found.map((u) => u.replace(/[),.;]+$/, "")))];
}

export function AddDialog({ onClose, initialText = "" }: { onClose: () => void; initialText?: string }) {
  const toast = useToast();
  const [text, setText] = useState(initialText);
  const [directory, setDirectory] = useState<string | null>(null);
  const [filename, setFilename] = useState("");
  const [priority, setPriority] = useState<Priority>("normal");
  const [startPaused, setStartPaused] = useState(false);
  const [connections, setConnections] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);

  const urls = useMemo(() => extractUrls(text), [text]);
  const single = urls.length === 1;

  const pickFolder = async () => {
    const dir = await open({ directory: true, multiple: false, title: "Dossier de destination" });
    if (typeof dir === "string") setDirectory(dir);
  };

  const submit = async () => {
    if (urls.length === 0) return;
    setBusy(true);
    try {
      const res = await api.add({
        urls,
        directory,
        filename: single && filename.trim() ? filename.trim() : null,
        priority,
        startPaused,
        connections,
      });
      if (res.failed.length) {
        toast(res.failed.map((f) => `${f.url} : ${f.error}`).join("\n"), "error");
      }
      if (res.added.length) {
        toast(res.added.length === 1 ? "Téléchargement ajouté." : `${res.added.length} téléchargements ajoutés.`);
        onClose();
      }
    } catch (e) {
      toast(errorText(e), "error");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title="Ajouter des téléchargements"
      onClose={onClose}
      wide
      footer={
        <>
          <span className="muted grow">
            {urls.length === 0 ? "Aucune adresse détectée" : urls.length === 1 ? "1 adresse" : `${urls.length} adresses`}
          </span>
          <button className="btn" onClick={onClose}>
            Annuler
          </button>
          <button className="btn btn-primary" disabled={urls.length === 0 || busy} onClick={submit}>
            {startPaused ? "Ajouter en pause" : "Télécharger"}
          </button>
        </>
      }
    >
      <label className="field">
        <span>Adresses (une par ligne, ou collez un texte qui en contient)</span>
        <textarea
          data-autofocus
          rows={5}
          value={text}
          spellCheck={false}
          placeholder="https://exemple.com/fichier.zip"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) submit();
          }}
        />
      </label>

      {single && (
        <label className="field">
          <span>Nom du fichier (facultatif)</span>
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
          <span>Connexions</span>
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
