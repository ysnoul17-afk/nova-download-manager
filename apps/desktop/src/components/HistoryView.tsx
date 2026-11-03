import { ArrowDown, ArrowUp, FolderOpen, Search } from "lucide-react";
import { useEffect, useState } from "react";
import { api, CATEGORIES, type Category, type DownloadInfo, type DownloadStatus, type SortKey } from "../api";
import { CATEGORY_LABEL, errorText, formatBytes, formatDate, formatSpeed, STATUS_LABEL } from "../format";
import { CategoryIcon, StatusBadge, useToast } from "./common";

const SORTS: { key: SortKey; label: string }[] = [
  { key: "added_at", label: "Date d'ajout" },
  { key: "completed_at", label: "Date de fin" },
  { key: "size", label: "Taille" },
  { key: "speed", label: "Vitesse moyenne" },
  { key: "type", label: "Type de fichier" },
  { key: "name", label: "Nom" },
];

const STATUSES: DownloadStatus[] = ["completed", "failed", "paused", "stopped", "queued", "downloading"];

/** Historique complet, interrogé dans la base SQLite. */
export function HistoryView({ refreshKey }: { refreshKey: number }) {
  const toast = useToast();
  const [search, setSearch] = useState("");
  const [status, setStatus] = useState<DownloadStatus | "">("");
  const [category, setCategory] = useState<Category | "">("");
  const [from, setFrom] = useState("");
  const [to, setTo] = useState("");
  const [sort, setSort] = useState<SortKey>("added_at");
  const [desc, setDesc] = useState(true);
  const [rows, setRows] = useState<DownloadInfo[]>([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let stale = false;
    const t = setTimeout(async () => {
      try {
        const res = await api.history({
          search: search || null,
          statuses: status ? [status] : [],
          categories: category ? [category] : [],
          // Dates locales : du début du jour « du » à la fin du jour « au ».
          from_ms: from ? new Date(`${from}T00:00:00`).getTime() : null,
          to_ms: to ? new Date(`${to}T23:59:59.999`).getTime() : null,
          sort,
          descending: desc,
          limit: 2000,
        });
        if (!stale) setRows(res);
      } catch (e) {
        if (!stale) toast(errorText(e), "error");
      } finally {
        if (!stale) setLoading(false);
      }
    }, 180);
    return () => {
      stale = true;
      clearTimeout(t);
    };
  }, [search, status, category, from, to, sort, desc, refreshKey, toast]);

  const hasFilters = !!(search || status || category || from || to);

  return (
    <div className="history">
      <header className="topbar">
        <h1>Historique</h1>
        <div className="search">
          <Search size={16} aria-hidden />
          <input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Rechercher" aria-label="Rechercher" />
        </div>
      </header>

      <div className="filters">
        <label className="field-inline">
          Statut
          <select value={status} onChange={(e) => setStatus(e.target.value as DownloadStatus | "")}>
            <option value="">Tous</option>
            {STATUSES.map((s) => (
              <option key={s} value={s}>
                {STATUS_LABEL[s]}
              </option>
            ))}
          </select>
        </label>
        <label className="field-inline">
          Type
          <select value={category} onChange={(e) => setCategory(e.target.value as Category | "")}>
            <option value="">Tous</option>
            {CATEGORIES.map((c) => (
              <option key={c} value={c}>
                {CATEGORY_LABEL[c]}
              </option>
            ))}
          </select>
        </label>
        <label className="field-inline">
          Du
          <input type="date" value={from} max={to || undefined} onChange={(e) => setFrom(e.target.value)} />
        </label>
        <label className="field-inline">
          au
          <input type="date" value={to} min={from || undefined} onChange={(e) => setTo(e.target.value)} />
        </label>
        <span className="grow" />
        <label className="field-inline">
          Trier par
          <select value={sort} onChange={(e) => setSort(e.target.value as SortKey)}>
            {SORTS.map((s) => (
              <option key={s.key} value={s.key}>
                {s.label}
              </option>
            ))}
          </select>
        </label>
        <button
          className="icon-btn"
          onClick={() => setDesc((d) => !d)}
          aria-label={desc ? "Ordre décroissant" : "Ordre croissant"}
          title={desc ? "Décroissant" : "Croissant"}
        >
          {desc ? <ArrowDown size={16} /> : <ArrowUp size={16} />}
        </button>
        {hasFilters && (
          <button
            className="btn btn-ghost"
            onClick={() => {
              setSearch("");
              setStatus("");
              setCategory("");
              setFrom("");
              setTo("");
            }}
          >
            Effacer les filtres
          </button>
        )}
      </div>

      <div className="table-wrap">
        <table className="table">
          <thead>
            <tr>
              <th>Nom</th>
              <th>Type</th>
              <th className="r">Taille</th>
              <th className="r">Vitesse moy.</th>
              <th>Statut</th>
              <th>Ajouté</th>
              <th aria-label="Actions" />
            </tr>
          </thead>
          <tbody>
            {rows.map((d) => (
              <tr key={d.id}>
                <td>
                  <div className="name-cell">
                    <CategoryIcon category={d.category} size={16} />
                    <span title={d.url}>{d.filename || d.url}</span>
                  </div>
                </td>
                <td>{CATEGORY_LABEL[d.category]}</td>
                <td className="r num">{formatBytes(d.total_size)}</td>
                <td className="r num">{formatSpeed(d.avg_speed)}</td>
                <td>
                  <StatusBadge status={d.status} />
                </td>
                <td className="num">{formatDate(d.added_at)}</td>
                <td className="r">
                  <button
                    className="icon-btn"
                    title="Afficher dans le dossier"
                    aria-label="Afficher dans le dossier"
                    onClick={() => api.openFolder(d.id).catch((e) => toast(errorText(e), "error"))}
                  >
                    <FolderOpen size={16} />
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        {!loading && rows.length === 0 && (
          <p className="empty">{hasFilters ? "Aucun résultat pour ces filtres." : "L'historique est vide pour l'instant."}</p>
        )}
      </div>
      <footer className="statusline muted">{rows.length.toLocaleString("fr-FR")} élément{rows.length > 1 ? "s" : ""}</footer>
    </div>
  );
}
