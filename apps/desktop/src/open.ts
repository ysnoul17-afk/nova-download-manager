import { ask } from "@tauri-apps/plugin-dialog";
import { api, type DownloadInfo } from "./api";

/** Même valeur que `NEEDS_CONFIRMATION` côté Rust. */
const NEEDS_CONFIRMATION = "NDM_CONFIRM_EXECUTABLE";

/**
 * Ouvre un fichier téléchargé. Si c'est un programme (exe, js, vbs, lnk, iso…), l'application refuse
 * tant que l'utilisateur n'a pas confirmé : la question est posée ici, puis l'ouverture est relancée.
 */
export async function openDownloaded(d: DownloadInfo): Promise<void> {
  try {
    await api.openFile(d.id);
  } catch (e) {
    if (e !== NEEDS_CONFIRMATION) throw e;
    const ok = await ask(
      `« ${d.filename} » peut exécuter un programme sur votre ordinateur. Ne l'ouvrez que si vous faites confiance à sa provenance.\n\nProvenance : ${hostOf(d.final_url ?? d.url)}`,
      { title: "Ouvrir un fichier exécutable", kind: "warning", okLabel: "Ouvrir", cancelLabel: "Annuler" },
    );
    if (ok) await api.openFile(d.id, true);
  }
}

function hostOf(url: string): string {
  try {
    return new URL(url).host;
  } catch {
    return url;
  }
}
