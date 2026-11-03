import { useEffect, useState } from "react";
import { api, type DownloadInfo, type SegmentView } from "../api";
import { isActive } from "../format";

/**
 * Le fichier représenté comme une bande : chaque segment est une portion,
 * remplie au fur et à mesure par sa connexion. Les segments actifs sont mis en évidence.
 */
export function SegmentMap({ item }: { item: DownloadInfo }) {
  const [segments, setSegments] = useState<SegmentView[]>([]);
  const live = isActive(item.status);

  useEffect(() => {
    let stop = false;
    const load = () =>
      api
        .segments(item.id)
        .then((s) => !stop && setSegments(s))
        .catch(() => {});
    load();
    const t = live ? setInterval(load, 400) : undefined;
    return () => {
      stop = true;
      if (t) clearInterval(t);
    };
  }, [item.id, live, item.status]);

  const total = item.total_size;
  if (!total || segments.length === 0) {
    const done = item.status === "completed";
    return (
      <div className="segmap" aria-label="Progression">
        <div className={`segmap-single ${done ? "done" : live ? "live" : ""}`} />
      </div>
    );
  }

  return (
    <div className="segmap" aria-label={`${segments.length} segments`}>
      {segments.map((s) => {
        const end = s.end ?? total - 1;
        const len = end + 1 - s.start;
        const left = (s.start / total) * 100;
        const width = (len / total) * 100;
        const fill = len > 0 ? Math.min(1, s.downloaded / len) * 100 : 100;
        return (
          <div
            key={s.start}
            className={`seg ${s.active ? "seg-active" : ""} ${fill >= 100 ? "seg-done" : ""}`}
            style={{ left: `${left}%`, width: `${width}%` }}
          >
            <div className="seg-fill" style={{ width: `${fill}%` }} />
          </div>
        );
      })}
    </div>
  );
}
