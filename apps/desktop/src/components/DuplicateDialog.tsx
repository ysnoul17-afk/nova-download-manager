import { ExternalLink, FolderOpen, RotateCcw } from "lucide-react";
import type { DownloadInfo } from "../api";
import { formatBytes, formatDate } from "../format";
import { CategoryIcon, Modal } from "./common";

/**
 * Fichier(s) déjà téléchargé(s) : ouvrir l'existant, l'afficher, ou télécharger une nouvelle copie.
 * Utilisé pour un lien envoyé par le navigateur (un seul fichier) et pour l'ajout depuis NDM (un ou plusieurs).
 */
export function DuplicateDialog({
  items,
  onOpen,
  onReveal,
  onRedownload,
  onClose,
}: {
  items: DownloadInfo[];
  onOpen: (d: DownloadInfo) => void;
  onReveal: (d: DownloadInfo) => void;
  onRedownload: () => void;
  onClose: () => void;
}) {
  const single = items.length === 1 ? items[0] : null;
  return (
    <Modal
      title={single ? "Déjà téléchargé" : `${items.length} fichiers déjà téléchargés`}
      onClose={onClose}
      wide
      footer={
        <>
          <button className="btn btn-ghost grow-left" onClick={onRedownload}>
            <RotateCcw size={15} /> {single ? "Télécharger à nouveau" : "Tout télécharger à nouveau"}
          </button>
          {single ? (
            <>
              <button className="btn" onClick={() => onReveal(single)}>
                <FolderOpen size={15} /> Afficher dans le dossier
              </button>
              <button className="btn btn-primary" data-autofocus onClick={() => onOpen(single)}>
                <ExternalLink size={15} /> Ouvrir
              </button>
            </>
          ) : (
            <button className="btn btn-primary" data-autofocus onClick={onClose}>
              Ignorer
            </button>
          )}
        </>
      }
    >
      <p>{single ? "Ce fichier se trouve déjà sur votre ordinateur :" : "Ces fichiers se trouvent déjà sur votre ordinateur :"}</p>
      <ul className="dup-list">
        {items.map((d) => (
          <li key={d.id}>
            <CategoryIcon category={d.category} size={16} />
            <div className="dup-main">
              <div className="dup-name" title={d.filename}>
                {d.filename}
                {d.media_label && <span className="media-tag">{d.media_label}</span>}
              </div>
              <div className="muted">
                {formatBytes(d.total_size)}, terminé {formatDate(d.completed_at).toLowerCase()}
              </div>
            </div>
          </li>
        ))}
      </ul>
    </Modal>
  );
}
