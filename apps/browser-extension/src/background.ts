// Service worker : relais vers l'application (Native Messaging), interception des téléchargements,
// menu contextuel et repérage des flux vidéo (HLS/DASH) chargés par les pages.

import { errorMessage, HOST_NAME, loadSettings, type MediaSpecLike, type Msg, type Reply } from "./shared";

// ---------------------------------------------------------------------------
// Native Messaging
// ---------------------------------------------------------------------------

function native<T = unknown>(message: Record<string, unknown>): Promise<Reply<T>> {
  return new Promise((resolve) => {
    try {
      chrome.runtime.sendNativeMessage(HOST_NAME, message, (response?: Reply<T>) => {
        const err = chrome.runtime.lastError;
        if (err || !response) {
          resolve({ ok: false, code: "native_unavailable", error: err?.message ?? "Pas de réponse." });
        } else {
          resolve(response);
        }
      });
    } catch (e) {
      resolve({ ok: false, code: "native_unavailable", error: String(e) });
    }
  });
}

async function cookieHeader(url: string): Promise<string | undefined> {
  const settings = await loadSettings();
  if (!settings.sendCookies || !chrome.cookies) return undefined;
  try {
    const cookies = await chrome.cookies.getAll({ url });
    if (!cookies.length) return undefined;
    return cookies.map((c) => `${c.name}=${c.value}`).join("; ");
  } catch {
    return undefined;
  }
}

async function sendDownload(url: string, opts: { referrer?: string; filename?: string; size?: number }) {
  return native<{ id: string; filename: string }>({
    type: "download",
    url,
    referrer: opts.referrer || undefined,
    filename: opts.filename || undefined,
    size: opts.size && opts.size > 0 ? opts.size : undefined,
    user_agent: navigator.userAgent,
    cookies: await cookieHeader(url),
  });
}

// ---------------------------------------------------------------------------
// Interception des téléchargements (mode automatique)
// ---------------------------------------------------------------------------

function extensionOf(name: string): string {
  const clean = name.split(/[?#]/)[0];
  const base = clean.split(/[\\/]/).pop() ?? "";
  const i = base.lastIndexOf(".");
  return i > 0 ? base.slice(i + 1).toLowerCase() : "";
}

/** Téléchargements que l'on a laissés au navigateur : on ne les intercepte pas une seconde fois. */
const passThrough = new Set<string>();

chrome.downloads.onDeterminingFilename.addListener((item, suggest) => {
  (async () => {
    const settings = await loadSettings();
    const url = item.finalUrl || item.url;
    const ext = extensionOf(item.filename) || extensionOf(url);
    const candidate =
      settings.mode === "auto" &&
      /^https?:/i.test(url) &&
      !passThrough.has(url) &&
      item.byExtensionId !== chrome.runtime.id &&
      settings.extensions.includes(ext);
    if (!candidate) {
      suggest();
      return;
    }
    const r = await sendDownload(url, {
      referrer: item.referrer,
      filename: item.filename.split(/[\\/]/).pop(),
      size: item.fileSize,
    });
    if (r.ok) {
      // NDM a accepté et vérifié le lien : le navigateur abandonne le sien.
      chrome.downloads.cancel(item.id, () => {
        void chrome.runtime.lastError;
        chrome.downloads.erase({ id: item.id }, () => void chrome.runtime.lastError);
      });
      flashBadge("✓", "#1f9d6b");
    } else {
      // Application absente, lien non récupérable… : le téléchargement normal continue.
      passThrough.add(url);
      setTimeout(() => passThrough.delete(url), 60_000);
    }
    suggest();
  })();
  return true; // réponse asynchrone
});

function flashBadge(text: string, color: string) {
  chrome.action.setBadgeBackgroundColor({ color });
  chrome.action.setBadgeText({ text });
  setTimeout(() => chrome.action.setBadgeText({ text: "" }), 2500);
}

// ---------------------------------------------------------------------------
// Menu contextuel
// ---------------------------------------------------------------------------

chrome.runtime.onInstalled.addListener(() => {
  chrome.contextMenus.removeAll(() => {
    chrome.contextMenus.create({
      id: "ndm-link",
      title: "Télécharger avec Nova Download Manager",
      contexts: ["link"],
      targetUrlPatterns: ["http://*/*", "https://*/*"],
    });
    chrome.contextMenus.create({
      id: "ndm-media",
      title: "Télécharger avec Nova Download Manager",
      contexts: ["video", "audio", "image"],
    });
  });
});

chrome.contextMenus.onClicked.addListener(async (info, tab) => {
  const url = info.menuItemId === "ndm-link" ? info.linkUrl : info.srcUrl;
  if (!url) return;
  if (!/^https?:/i.test(url)) {
    // Vidéo diffusée en flux (blob:) : c'est le bouton sur le lecteur qui sait la retrouver.
    if (tab?.id != null) {
      chrome.tabs.sendMessage(tab.id, { type: "open_menu_at_video", src: url }, () => void chrome.runtime.lastError);
    }
    return;
  }
  const r = await sendDownload(url, { referrer: info.pageUrl });
  flashBadge(r.ok ? "✓" : "!", r.ok ? "#1f9d6b" : "#d64545");
  if (!r.ok) console.warn("NDM :", errorMessage(r));
});

// ---------------------------------------------------------------------------
// Flux vidéo repérés dans le trafic des pages (HLS, DASH)
// ---------------------------------------------------------------------------

const STREAM_RE = /\.(m3u8|mpd)(\?|#|$)/i;

async function rememberStream(tabId: number, url: string) {
  const key = `streams:${tabId}`;
  const s = await chrome.storage.session.get(key);
  const list = (s[key] as string[] | undefined) ?? [];
  if (list.includes(url)) return;
  list.push(url);
  await chrome.storage.session.set({ [key]: list.slice(-20) });
}

chrome.webRequest.onBeforeRequest.addListener(
  (d) => {
    if (d.tabId >= 0 && STREAM_RE.test(d.url)) void rememberStream(d.tabId, d.url);
  },
  { urls: ["http://*/*", "https://*/*"], types: ["xmlhttprequest", "media", "other"] },
);

chrome.tabs.onRemoved.addListener((tabId) => void chrome.storage.session.remove(`streams:${tabId}`));
chrome.tabs.onUpdated.addListener((tabId, change) => {
  if (change.url) void chrome.storage.session.remove(`streams:${tabId}`);
});

// ---------------------------------------------------------------------------
// Messages du script de contenu et de la fenêtre
// ---------------------------------------------------------------------------

async function handle(msg: Msg, sender: chrome.runtime.MessageSender): Promise<Reply> {
  switch (msg.type) {
    case "analyze":
      return native({ type: "analyze", url: msg.url });
    case "download":
      return sendDownload(msg.url, { referrer: msg.referrer ?? sender.tab?.url, filename: msg.filename });
    case "download_media": {
      const o = msg.option;
      const spec: MediaSpecLike = {
        format: o.format,
        subtitles: false,
        label: `${o.label} ${o.container}`,
        title: msg.title,
        est_size: o.est_size,
      };
      return native({ type: "download_media", url: msg.url, spec });
    }
    case "detected_streams": {
      const tabId = sender.tab?.id;
      if (tabId == null) return { ok: true, data: [] };
      const key = `streams:${tabId}`;
      const s = await chrome.storage.session.get(key);
      return { ok: true, data: (s[key] as string[] | undefined) ?? [] };
    }
    case "status":
      // La fenêtre de l'extension ne doit pas lancer l'application à chaque ouverture.
      return native({ type: "status", launch: false });
    case "show":
      return native({ type: "show" });
  }
}

chrome.runtime.onMessage.addListener((msg: Msg, sender, sendResponse) => {
  // Seuls nos propres scripts peuvent nous parler.
  if (sender.id !== chrome.runtime.id) return false;
  handle(msg, sender).then(sendResponse);
  return true;
});
