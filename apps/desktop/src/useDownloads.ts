import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useState } from "react";
import { api, EVENT_ENGINE, EVENT_RESYNC, type DownloadInfo, type EngineEvent } from "./api";

export interface Stats {
  total_speed: number;
  active: number;
  queued: number;
}

/** État en direct des téléchargements, synchronisé par les événements du moteur. */
export function useDownloads() {
  const [items, setItems] = useState<Map<string, DownloadInfo>>(new Map());
  const [stats, setStats] = useState<Stats>({ total_speed: 0, active: 0, queued: 0 });
  const [loaded, setLoaded] = useState(false);

  const reload = useCallback(async () => {
    const list = await api.list();
    setItems(new Map(list.map((d) => [d.id, d])));
    setLoaded(true);
  }, []);

  useEffect(() => {
    let disposed = false;
    const unlisteners: (() => void)[] = [];

    (async () => {
      unlisteners.push(
        await listen<EngineEvent>(EVENT_ENGINE, ({ payload: ev }) => {
          if (ev.type === "stats") {
            setStats({ total_speed: ev.total_speed, active: ev.active, queued: ev.queued });
            return;
          }
          setItems((prev) => {
            const next = new Map(prev);
            switch (ev.type) {
              case "added":
              case "status_changed":
                next.set(ev.info.id, ev.info);
                break;
              case "progress":
                for (const i of ev.items) next.set(i.id, i);
                break;
              case "removed":
                next.delete(ev.id);
                break;
            }
            return next;
          });
        }),
      );
      unlisteners.push(await listen(EVENT_RESYNC, () => void reload()));
      if (disposed) unlisteners.forEach((u) => u());
      else await reload();
    })();

    return () => {
      disposed = true;
      unlisteners.forEach((u) => u());
    };
  }, [reload]);

  return { items, stats, loaded, reload };
}
