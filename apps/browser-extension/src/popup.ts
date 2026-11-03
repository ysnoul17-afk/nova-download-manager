// Fenêtre de l'extension : état de NDM, téléchargements en cours, fichiers de la page, réglages.

import { errorMessage, formatBytes, loadSettings, saveSettings, type Msg, type Reply } from "./shared";

interface StatusItem {
  id: string;
  filename: string;
  status: string;
  downloaded: number;
  total: number | null;
  speed: number;
  media_label: string | null;
  stage: string | null;
}

interface Status {
  version: string;
  active: number;
  queued: number;
  items: StatusItem[];
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

function send<T>(msg: Msg): Promise<Reply<T>> {
  return new Promise((resolve) =>
    chrome.runtime.sendMessage(msg, (r?: Reply<T>) =>
      resolve(r ?? { ok: false, code: "extension_error", error: "Pas de réponse de l'extension." }),
    ),
  );
}

const STATUS_LABEL: Record<string, string> = {
  queued: "En attente",
  probing: "Connexion…",
  downloading: "En cours",
  finalizing: "Finalisation…",
  paused: "En pause",
};

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

async function refresh() {
  const r = await send<Status>({ type: "status" });
  const state = $("state");
  const box = $("downloads");
  const open = $<HTMLButtonElement>("open");
  if (!r.ok) {
    state.className = r.code === "app_not_running" ? "state warn" : "state err";
    state.textContent = r.code === "app_not_running" ? "Non lancé" : "Indisponible";
    open.textContent = r.code === "app_not_running" ? "Lancer Nova Download Manager" : "Ouvrir Nova Download Manager";
    open.disabled = r.code !== "app_not_running";
    box.replaceChildren(el("p", "empty", errorMessage(r)));
    return;
  }
  const s = r.data!;
  state.className = "state ok";
  state.textContent = `Connecté, ${s.version}`;
  open.textContent = "Ouvrir Nova Download Manager";
  open.disabled = false;
  if (!s.items.length) {
    box.replaceChildren(el("p", "empty", "Aucun téléchargement en cours."));
    return;
  }
  box.replaceChildren(
    ...s.items.map((i) => {
      const item = el("div", "item");
      const top = el("div", "top");
      top.append(el("span", "fn", i.filename || "Analyse…"), el("span", "sp", i.speed ? `${formatBytes(i.speed)}/s` : ""));
      const bar = el("div", "bar");
      const fill = el("div");
      fill.style.width = i.total ? `${Math.min(100, (i.downloaded / i.total) * 100)}%` : "0";
      bar.append(fill);
      const parts = [i.stage || STATUS_LABEL[i.status] || i.status, i.media_label];
      if (i.total) parts.push(`${formatBytes(i.downloaded)} sur ${formatBytes(i.total)}`);
      item.append(top, bar, el("div", "sub", parts.filter(Boolean).join(", ")));
      return item;
    }),
  );
}

// ---------------------------------------------------------------------------
// Fichiers de la page
// ---------------------------------------------------------------------------

async function scanLinks() {
  const box = $("links");
  const settings = await loadSettings();
  const [tab] = await chrome.tabs.query({ active: true, currentWindow: true });
  if (tab?.id == null) return;
  const links: string[] = await new Promise((resolve) =>
    chrome.tabs.sendMessage(tab.id!, { type: "collect_links", extensions: settings.extensions }, (r?: string[]) => {
      void chrome.runtime.lastError;
      resolve(r ?? []);
    }),
  );
  if (!links.length) {
    box.replaceChildren(el("p", "empty", "Aucun lien vers un fichier reconnu sur cette page."));
    return;
  }
  const list = el("div", "links");
  for (const url of links) {
    const label = el("label");
    const cb = el("input");
    cb.type = "checkbox";
    cb.checked = true;
    cb.value = url;
    const name = decodeURIComponent(url.split(/[?#]/)[0].split("/").pop() || url);
    const span = el("span", "", name);
    span.title = url;
    label.append(cb, span);
    list.append(label);
  }
  const go = el("button", "btn", `Envoyer à NDM`);
  const out = el("p", "msg");
  const update = () => {
    const n = list.querySelectorAll("input:checked").length;
    go.textContent = n > 1 ? `Envoyer ${n} fichiers à NDM` : "Envoyer à NDM";
    go.disabled = n === 0;
  };
  list.addEventListener("change", update);
  update();
  go.addEventListener("click", async () => {
    go.disabled = true;
    const urls = [...list.querySelectorAll<HTMLInputElement>("input:checked")].map((c) => c.value);
    let okCount = 0;
    let lastErr = "";
    for (const url of urls) {
      const r = await send({ type: "download", url, referrer: tab.url });
      if (r.ok) okCount++;
      else lastErr = errorMessage(r);
    }
    out.className = okCount === urls.length ? "msg ok" : "msg err";
    out.textContent =
      okCount === urls.length
        ? `${okCount} fichier${okCount > 1 ? "s" : ""} envoyé${okCount > 1 ? "s" : ""} à NDM.`
        : `${okCount} sur ${urls.length} envoyés. ${lastErr}`;
    go.disabled = false;
    void refresh();
  });
  box.replaceChildren(list, go, out);
}

// ---------------------------------------------------------------------------
// Réglages
// ---------------------------------------------------------------------------

async function initSettings() {
  const s = await loadSettings();
  for (const r of document.querySelectorAll<HTMLInputElement>('input[name="mode"]')) {
    r.checked = r.value === s.mode;
    r.addEventListener("change", () => r.checked && saveSettings({ mode: r.value as "auto" | "manual" }));
  }
  const vb = $<HTMLInputElement>("videoButton");
  vb.checked = s.videoButton;
  vb.addEventListener("change", () => saveSettings({ videoButton: vb.checked }));

  const ck = $<HTMLInputElement>("sendCookies");
  const granted = await chrome.permissions.contains({ permissions: ["cookies"] });
  ck.checked = s.sendCookies && granted;
  ck.addEventListener("change", async () => {
    if (ck.checked) {
      // Autorisation demandée seulement au moment où l'utilisateur l'active.
      const ok = await chrome.permissions.request({ permissions: ["cookies"] });
      ck.checked = ok;
      await saveSettings({ sendCookies: ok });
    } else {
      await saveSettings({ sendCookies: false });
      await chrome.permissions.remove({ permissions: ["cookies"] });
    }
  });
}

$("open").addEventListener("click", async () => {
  const r = await send({ type: "show" });
  if (r.ok) window.close();
  else void refresh();
});
$("scan").addEventListener("click", () => void scanLinks());

void initSettings();
void refresh();
setInterval(refresh, 1000);
