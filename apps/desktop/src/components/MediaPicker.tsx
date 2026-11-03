import { Film, Music } from "lucide-react";
import { useState } from "react";
import type { MediaInfo, MediaOption, MediaSpec } from "../api";
import { formatBytes, formatEta } from "../format";

/** Choix de la qualité d'une vidéo analysée. */
export function MediaPicker({
  info,
  onChoose,
}: {
  info: MediaInfo;
  onChoose: (spec: MediaSpec) => void;
}) {
  const [selected, setSelected] = useState<MediaOption>(info.options[0]);
  const [subs, setSubs] = useState(false);
  const isAudio = selected.format.type === "audio_mp3" || selected.format.type === "audio_m4a";
  const video = info.options.filter((o) => o.format.type === "video" || o.format.type === "best");
  const audio = info.options.filter((o) => o.format.type === "audio_mp3" || o.format.type === "audio_m4a");
  const hasSubs = info.subtitle_langs.some((l) => l.startsWith("fr") || l.startsWith("en"));

  const spec = (): MediaSpec => ({
    format: selected.format,
    subtitles: subs && !isAudio,
    label: `${selected.label} ${selected.container}`,
    title: info.title,
    est_size: selected.est_size,
  });

  return (
    <form
      id="media-picker"
      className="media-picker"
      onSubmit={(e) => {
        e.preventDefault();
        onChoose(spec());
      }}
    >
      <div className="media-head">
        {info.thumbnail ? (
          <img className="media-thumb" src={info.thumbnail} alt="" referrerPolicy="no-referrer" />
        ) : (
          <div className="media-thumb media-thumb-empty">
            <Film size={28} />
          </div>
        )}
        <div className="media-meta">
          <div className="media-title" title={info.title}>
            {info.title}
          </div>
          <div className="muted">
            {[info.uploader, info.site, info.duration_secs ? formatEta(Math.round(info.duration_secs)) : null]
              .filter(Boolean)
              .join(", ")}
          </div>
        </div>
      </div>

      {video.length > 0 && (
        <fieldset className="quality-group">
          <legend>
            <Film size={15} /> Vidéo
          </legend>
          {video.map((o) => (
            <QualityRow key={o.id} o={o} checked={selected.id === o.id} onPick={() => setSelected(o)} />
          ))}
        </fieldset>
      )}
      {audio.length > 0 && (
        <fieldset className="quality-group">
          <legend>
            <Music size={15} /> Audio seul
          </legend>
          {audio.map((o) => (
            <QualityRow key={o.id} o={o} checked={selected.id === o.id} onPick={() => setSelected(o)} />
          ))}
        </fieldset>
      )}

      {hasSubs && !isAudio && (
        <label className="check">
          <input type="checkbox" checked={subs} onChange={(e) => setSubs(e.target.checked)} />
          Intégrer les sous-titres (français ou anglais)
        </label>
      )}
      {info.drm_filtered && (
        <p className="setting-hint">Certaines qualités sont protégées par DRM et ne peuvent pas être téléchargées.</p>
      )}
    </form>
  );
}

function QualityRow({ o, checked, onPick }: { o: MediaOption; checked: boolean; onPick: () => void }) {
  return (
    <label className={`quality ${checked ? "on" : ""}`}>
      <input type="radio" name="quality" checked={checked} onChange={onPick} />
      <span className="quality-label">{o.label}</span>
      <span className="quality-container">{o.container}</span>
      <span className="grow" />
      <span className="muted num">{o.est_size ? `≈ ${formatBytes(o.est_size)}` : ""}</span>
    </label>
  );
}
