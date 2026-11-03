// Script de contenu : un bouton « Télécharger cette vidéo » par lecteur vidéo de la page.
//
// - Les vidéos sont repérées au chargement puis à chaque ajout dans le DOM (pages dynamiques).
// - L'interface vit dans un shadow DOM séparé de la page : pas de conflit de styles, pas
//   d'interférence avec les contrôles du lecteur. Un seul bouton par vidéo, réutilisé.
// - Le pointeur est suivi par position (les lecteurs posent souvent des calques au-dessus
//   de la balise <video>), et le bouton reste affiché le temps d'aller du lecteur au bouton.
// - Seuls les clics réels de l'utilisateur sont pris en compte (event.isTrusted).

import {
  errorMessage,
  formatBytes,
  formatDuration,
  loadSettings,
  type Analysis,
  type MediaOption,
  type Msg,
  type Reply,
} from "./shared";

const MIN_WIDTH = 160;
const MIN_HEIGHT = 90;
const HIDE_DELAY = 700;
const CACHE_MS = 5 * 60_000;

interface Tracked {
  video: HTMLVideoElement;
  button?: HTMLButtonElement;
}

const tracked = new Map<HTMLVideoElement, Tracked>();
const cache = new Map<string, { at: number; reply: Reply<Analysis> }>();
let enabled = false;
let host: HTMLDivElement | null = null;
let shadow: ShadowRoot | null = null;
let shown: Tracked | null = null;
let menu: { el: HTMLDivElement; for: Tracked } | null = null;
let hideTimer: number | undefined;
let pointer = { x: -1, y: -1 };
let frame = 0;

function send<T>(msg: Msg): Promise<Reply<T>> {
  return new Promise((resolve) => {
    try {
      chrome.runtime.sendMessage(msg, (r?: Reply<T>) => {
        if (chrome.runtime.lastError || !r) {
          resolve({ ok: false, code: "extension_error", error: "L'extension a été rechargée : actualisez la page." });
        } else resolve(r);
      });
    } catch {
      resolve({ ok: false, code: "extension_error", error: "L'extension a été rechargée : actualisez la page." });
    }
  });
}

// ---------------------------------------------------------------------------
// Repérage des vidéos
// ---------------------------------------------------------------------------

function track(v: HTMLVideoElement) {
  if (!tracked.has(v)) tracked.set(v, { video: v });
}

function scan(root: ParentNode) {
  if (root instanceof HTMLVideoElement) track(root);
  root.querySelectorAll?.("video").forEach((v) => track(v));
}

/** Retire les vidéos sorties du document, avec leur bouton. */
function sweep() {
  for (const [v, t] of tracked) {
    if (!v.isConnected) {
      t.button?.remove();
      if (menu?.for === t) closeMenu();
      if (shown === t) shown = null;
      tracked.delete(v);
    }
  }
}

function usableRect(v: HTMLVideoElement): DOMRect | null {
  const r = v.getBoundingClientRect();
  if (r.width < MIN_WIDTH || r.height < MIN_HEIGHT) return null;
  if (r.bottom < 0 || r.right < 0 || r.top > innerHeight || r.left > innerWidth) return null;
  const style = getComputedStyle(v);
  if (style.visibility === "hidden" || style.display === "none" || Number(style.opacity) === 0) return null;
  return r;
}

/** Vidéo sous le pointeur ; en cas d'imbrication, la plus petite. */
function videoAt(x: number, y: number): Tracked | null {
  let best: Tracked | null = null;
  let bestArea = Infinity;
  for (const t of tracked.values()) {
    const r = usableRect(t.video);
    if (!r || x < r.left || x > r.right || y < r.top || y > r.bottom) continue;
    const area = r.width * r.height;
    if (area < bestArea) {
      best = t;
      bestArea = area;
    }
  }
  return best;
}

// ---------------------------------------------------------------------------
// Interface (shadow DOM)
// ---------------------------------------------------------------------------

const CSS = `
:host { all: initial; }
* { box-sizing: border-box; font-family: "Segoe UI Variable Text", "Segoe UI", system-ui, sans-serif; }
.btn {
  position: fixed; display: none; align-items: center; gap: 6px; height: 32px; padding: 0 12px 0 10px;
  border: 0; border-radius: 16px; cursor: pointer; pointer-events: auto;
  background: rgba(26, 31, 54, 0.88); color: #fff; font-size: 13px; font-weight: 600; letter-spacing: .01em;
  box-shadow: 0 4px 14px rgba(0,0,0,.28); backdrop-filter: blur(6px); white-space: nowrap;
  transition: background .12s;
}
.btn.on { display: inline-flex; }
.btn:hover, .btn.open { background: #5b5bf6; }
.btn:focus-visible { outline: 2px solid #14b8c8; outline-offset: 2px; }
.btn svg { width: 16px; height: 16px; flex: none; }
.menu {
  position: fixed; width: 320px; max-height: min(460px, calc(100vh - 24px)); overflow: auto; pointer-events: auto;
  background: #fff; color: #1a1f36; border-radius: 12px; padding: 12px; font-size: 13px; line-height: 1.4;
  box-shadow: 0 18px 50px rgba(26,31,54,.28), 0 2px 6px rgba(26,31,54,.1);
}
@media (prefers-color-scheme: dark) {
  .menu { background: #1a1d28; color: #e7eaf3; }
  .menu .muted { color: #8b93aa; }
  .opt { border-color: #2a2e3c; }
  .opt:hover { background: #26284a; border-color: #7b7bff; }
}
.title { font-weight: 650; font-size: 14px; margin: 0 0 2px; display: -webkit-box; -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; }
.muted { color: #7a819c; }
.group { margin: 10px 0 4px; font-size: 12px; font-weight: 600; color: #7a819c; }
.opt {
  display: flex; align-items: center; gap: 8px; width: 100%; padding: 8px 10px; margin: 0 0 4px;
  border: 1px solid #dce1ec; border-radius: 8px; background: transparent; color: inherit; cursor: pointer; text-align: left; font-size: 13px;
}
.opt:hover { border-color: #5b5bf6; background: #e6e7ff; }
.opt b { font-weight: 650; min-width: 64px; }
.opt .c { font-size: 11px; font-weight: 600; color: #7a819c; }
.opt .s { margin-left: auto; color: #7a819c; font-variant-numeric: tabular-nums; }
.msg { margin: 6px 0 0; }
.err { color: #d64545; }
.ok { color: #1f9d6b; font-weight: 600; }
.link { background: none; border: 0; color: #5b5bf6; padding: 0; font: inherit; cursor: pointer; text-decoration: underline; }
.spin { display: inline-block; width: 12px; height: 12px; border: 2px solid currentColor; border-right-color: transparent; border-radius: 50%; animation: s .8s linear infinite; vertical-align: -2px; margin-right: 6px; }
@keyframes s { to { transform: rotate(360deg); } }
@media (prefers-reduced-motion: reduce) { .spin { animation: none; } }
`;

const ICON =
  '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M12 4v11"/><path d="m7 10 5 5 5-5"/><path d="M5 20h14"/></svg>';

function ensureLayer(): ShadowRoot {
  if (shadow && host?.isConnected) return shadow;
  host = document.createElement("div");
  host.setAttribute("data-ndm", "");
  host.style.cssText = "all:initial;position:fixed;inset:0 auto auto 0;width:0;height:0;z-index:2147483647;pointer-events:none;";
  shadow = host.attachShadow({ mode: "open" });
  const style = document.createElement("style");
  style.textContent = CSS;
  shadow.append(style);
  document.documentElement.append(host);
  for (const t of tracked.values()) t.button = undefined; // anciens boutons perdus avec l'ancien calque
  return shadow;
}

function buttonFor(t: Tracked): HTMLButtonElement {
  const root = ensureLayer();
  if (t.button?.isConnected) return t.button;
  const b = document.createElement("button");
  b.className = "btn";
  b.type = "button";
  b.innerHTML = `${ICON}<span>Télécharger cette vidéo</span>`;
  b.setAttribute("aria-haspopup", "menu");
  b.addEventListener("pointerenter", () => clearTimeout(hideTimer));
  b.addEventListener("pointerleave", scheduleHide);
  b.addEventListener("click", (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (!e.isTrusted) return; // clic simulé par la page : ignoré
    if (menu?.for === t) closeMenu();
    else openMenu(t);
  });
  root.append(b);
  t.button = b;
  return b;
}

function place(t: Tracked) {
  const b = t.button;
  const r = usableRect(t.video);
  if (!b || !r) {
    if (b && menu?.for !== t) b.classList.remove("on");
    return;
  }
  const w = b.offsetWidth || 190;
  b.style.left = `${Math.max(4, r.right - w - 10)}px`;
  b.style.top = `${Math.max(4, r.top + 10)}px`;
  if (menu?.for === t) placeMenu();
}

function show(t: Tracked) {
  clearTimeout(hideTimer);
  // Un seul bouton visible à la fois (hors vidéo dont le menu est ouvert).
  for (const x of tracked.values()) if (x !== t && x !== menu?.for) x.button?.classList.remove("on");
  shown = t;
  const b = buttonFor(t);
  b.classList.add("on");
  place(t);
}

function scheduleHide() {
  clearTimeout(hideTimer);
  hideTimer = window.setTimeout(() => {
    if (!shown || menu?.for === shown) return;
    // Le pointeur est revenu sur la vidéo entre-temps : on garde le bouton.
    if (videoAt(pointer.x, pointer.y) === shown) return;
    shown.button?.classList.remove("on");
    shown = null;
  }, HIDE_DELAY);
}

// ---------------------------------------------------------------------------
// Menu
// ---------------------------------------------------------------------------

function closeMenu() {
  if (!menu) return;
  menu.for.button?.classList.remove("open");
  menu.el.remove();
  const t = menu.for;
  menu = null;
  if (shown !== t) t.button?.classList.remove("on"); // le pointeur est passé sur une autre vidéo
  else if (videoAt(pointer.x, pointer.y) !== t) scheduleHide();
}

function placeMenu() {
  if (!menu) return;
  const b = menu.for.button!.getBoundingClientRect();
  const m = menu.el;
  const w = 320;
  const left = Math.min(Math.max(8, b.right - w), innerWidth - w - 8);
  let top = b.bottom + 6;
  const h = m.offsetHeight || 200;
  if (top + h > innerHeight - 8) top = Math.max(8, b.top - h - 6);
  m.style.left = `${left}px`;
  m.style.top = `${top}px`;
}

function el<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function setBody(nodes: Node[]) {
  if (!menu) return;
  menu.el.replaceChildren(...nodes);
  placeMenu();
}

function message(text: string, cls = "msg muted", extra: Node[] = []): Node[] {
  const p = el("p", cls, text);
  return [el("div", "title", "Télécharger cette vidéo"), p, ...extra];
}

function loading(text: string): Node[] {
  const p = el("p", "msg muted");
  p.append(el("span", "spin"), document.createTextNode(text));
  return [el("div", "title", "Télécharger cette vidéo"), p];
}

/** Adresse à analyser pour une vidéo : fichier direct, lien parent ou page. */
function targetUrl(v: HTMLVideoElement): string {
  const src = v.currentSrc || v.src || v.querySelector("source")?.src || "";
  if (/^https?:/i.test(src)) return src;
  const a = v.closest("a[href]") as HTMLAnchorElement | null;
  if (a && /^https?:/i.test(a.href) && a.href !== location.href) return a.href;
  return location.href;
}

async function analyze(url: string): Promise<Reply<Analysis>> {
  const c = cache.get(url);
  if (c && Date.now() - c.at < CACHE_MS) return c.reply;
  const reply = await send<Analysis>({ type: "analyze", url });
  if (reply.ok) cache.set(url, { at: Date.now(), reply });
  return reply;
}

function openMenu(t: Tracked) {
  closeMenu();
  const root = ensureLayer();
  const m = el("div", "menu");
  m.setAttribute("role", "menu");
  m.addEventListener("pointerenter", () => clearTimeout(hideTimer));
  m.addEventListener("pointerdown", (e) => e.stopPropagation());
  root.append(m);
  menu = { el: m, for: t };
  t.button?.classList.add("open");
  t.button?.classList.add("on");

  // Contenu chiffré (Widevine, PlayReady, FairPlay) : on l'indique sans rien tenter.
  if (t.video.mediaKeys) {
    setBody(message("Cette vidéo est protégée par DRM : son téléchargement n'est pas possible.", "msg err"));
    return;
  }
  void begin(t);
}

/** Sites dont la page entière décrit la vidéo (analyse par yt-dlp plus fiable que les flux). */
const VIDEO_SITES = /(^|\.)(youtube\.com|youtu\.be|vimeo\.com|dailymotion\.com|twitch\.tv|tiktok\.com|x\.com|twitter\.com|instagram\.com|facebook\.com|reddit\.com|bilibili\.com)$/i;

async function begin(t: Tracked) {
  const src = t.video.currentSrc || t.video.src;
  // Vidéo alimentée par JavaScript (blob:) hors sites connus : sur une page à plusieurs vidéos,
  // l'analyse de la page pourrait renvoyer une autre vidéo. Les flux chargés par la page d'abord.
  if (src.startsWith("blob:") && !VIDEO_SITES.test(location.hostname)) {
    const streams = await send<string[]>({ type: "detected_streams" });
    if (menu?.for !== t) return;
    const list = streams.ok ? streams.data ?? [] : [];
    if (list.length) {
      setBody(streamsView(t, list, true));
      return;
    }
  }
  void showAnalysis(t, targetUrl(t.video));
}

async function showAnalysis(t: Tracked, url: string, fromStream = false) {
  setBody(loading("Recherche des formats disponibles…"));
  const r = await analyze(url);
  if (menu?.for !== t) return; // menu fermé entre-temps

  if (!r.ok) {
    setBody(message(errorMessage(r), "msg err", openAppLink(r.code)));
    return;
  }
  const a = r.data!;
  switch (a.kind) {
    case "file":
      setBody(fileView(t, url, a));
      return;
    case "media":
      setBody(mediaView(t, url, a.info));
      return;
    case "needs_tools":
      setBody(
        message("Les outils vidéo de Nova Download Manager ne sont pas encore installés.", "msg", [
          button("Ouvrir Nova Download Manager pour les installer", () => send({ type: "show" })),
        ]),
      );
      return;
    case "page":
    case "error": {
      // La page n'est pas reconnue : on propose les flux vidéo repérés dans le trafic.
      if (!fromStream) {
        const streams = await send<string[]>({ type: "detected_streams" });
        if (menu?.for !== t) return;
        const list = streams.ok ? streams.data ?? [] : [];
        if (list.length) {
          setBody(streamsView(t, list));
          return;
        }
      }
      setBody(message(a.message || "Aucune vidéo téléchargeable n'a été trouvée.", "msg err"));
    }
  }
}

function button(text: string, onClick: () => void): HTMLButtonElement {
  const b = el("button", "link", text);
  b.type = "button";
  b.addEventListener("click", (e) => {
    if (e.isTrusted) onClick();
  });
  return b;
}

function openAppLink(code?: string): Node[] {
  return code === "app_not_running" ? [button("Lancer Nova Download Manager", () => send({ type: "show" }))] : [];
}

function head(title: string, meta: string): Node[] {
  return [el("div", "title", title), el("div", "muted", meta)];
}

function optionButton(label: string, container: string, size: number | null, onPick: () => Promise<void>) {
  const b = el("button", "opt");
  b.type = "button";
  b.setAttribute("role", "menuitem");
  b.append(el("b", "", label), el("span", "c", container), el("span", "s", size ? `≈ ${formatBytes(size)}` : ""));
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    if (!e.isTrusted) return;
    void onPick();
  });
  return b;
}

async function sent(r: Reply) {
  if (!menu) return;
  if (r.ok) {
    setBody(message("Ajouté à Nova Download Manager.", "msg ok"));
    setTimeout(closeMenu, 1600);
  } else {
    setBody(message(errorMessage(r), "msg err"));
  }
}

function fileView(_t: Tracked, url: string, a: Extract<Analysis, { kind: "file" }>): Node[] {
  const name = a.filename || decodeURIComponent(url.split(/[?#]/)[0].split("/").pop() || "video");
  const ext = (name.split(".").pop() || "vidéo").toUpperCase();
  return [
    ...head(name, "Fichier vidéo direct"),
    el("div", "group", "Fichier"),
    optionButton("Original", ext, a.size, async () => {
      setBody(loading("Envoi à Nova Download Manager…"));
      sent(await send({ type: "download", url, referrer: location.href, filename: a.filename ?? undefined }));
    }),
  ];
}

function mediaView(_t: Tracked, url: string, info: Extract<Analysis, { kind: "media" }>["info"]): Node[] {
  const meta = [info.site, formatDuration(info.duration_secs)].filter(Boolean).join(", ");
  const nodes: Node[] = head(info.title, meta);
  const pick = (o: MediaOption) => async () => {
    setBody(loading("Envoi à Nova Download Manager…"));
    sent(await send({ type: "download_media", url, option: o, title: info.title }));
  };
  const video = info.options.filter((o) => o.format.type === "video" || o.format.type === "best");
  const audio = info.options.filter((o) => o.format.type === "audio_mp3" || o.format.type === "audio_m4a");
  if (video.length) {
    nodes.push(el("div", "group", "Vidéo"));
    for (const o of video) nodes.push(optionButton(o.label, o.container, o.est_size, pick(o)));
  }
  if (audio.length) {
    nodes.push(el("div", "group", "Audio seul"));
    for (const o of audio) nodes.push(optionButton(o.label, o.container, o.est_size, pick(o)));
  }
  if (info.drm_filtered) nodes.push(el("p", "msg muted", "Certaines qualités sont protégées par DRM."));
  return nodes;
}

function streamsView(t: Tracked, streams: string[], offerPage = false): Node[] {
  const nodes: Node[] = head("Flux vidéo repérés sur cette page", "Choisissez celui de cette vidéo");
  streams
    .slice()
    .reverse()
    .forEach((s, i) => {
      const kind = /\.mpd/i.test(s) ? "DASH" : "HLS";
      nodes.push(optionButton(`Flux ${streams.length - i}`, kind, null, async () => void showAnalysis(t, s, true)));
    });
  if (offerPage) {
    const p = el("p", "msg");
    p.append(button("Analyser plutôt la page entière", () => void showAnalysis(t, location.href, true)));
    nodes.push(p);
  }
  return nodes;
}

// ---------------------------------------------------------------------------
// Événements
// ---------------------------------------------------------------------------

let overUi = false;

function onPointerMove(e: PointerEvent) {
  pointer = { x: e.clientX, y: e.clientY };
  // composedPath() n'est lisible que pendant la diffusion de l'événement : on le lit tout de suite.
  overUi = !!host && e.composedPath().includes(host);
  if (frame) return;
  frame = requestAnimationFrame(() => {
    frame = 0;
    if (!enabled) return;
    // Pointeur sur notre propre interface (bouton, menu) : rien ne change.
    if (overUi) return;
    const t = videoAt(pointer.x, pointer.y);
    if (t) show(t);
    else if (shown && menu?.for !== shown) scheduleHide();
  });
}

function reposition() {
  if (shown) place(shown);
  else if (menu) place(menu.for);
}

function start() {
  if (enabled) return;
  enabled = true;
  scan(document);
  addEventListener("pointermove", onPointerMove, { capture: true, passive: true });
  addEventListener("scroll", reposition, { capture: true, passive: true });
  addEventListener("resize", reposition, { passive: true });
  document.addEventListener(
    "pointerdown",
    (e) => {
      if (menu && host && !e.composedPath().includes(host)) closeMenu();
    },
    true,
  );
  addEventListener("keydown", (e) => {
    if (e.key === "Escape") closeMenu();
  });
}

function stop() {
  enabled = false;
  closeMenu();
  for (const t of tracked.values()) t.button?.classList.remove("on");
  shown = null;
}

const observer = new MutationObserver((records) => {
  for (const r of records) for (const n of r.addedNodes) if (n.nodeType === 1) scan(n as Element);
  if (records.some((r) => r.removedNodes.length)) sweep();
});
observer.observe(document.documentElement, { childList: true, subtree: true });

// Lecteur redimensionné ou déplacé sans défilement (mode cinéma, SPA…).
setInterval(() => {
  if (shown || menu) reposition();
}, 300);

// Menu contextuel « Télécharger avec NDM » sur une vidéo en flux (blob:).
chrome.runtime.onMessage.addListener((msg: { type: string; src?: string }) => {
  if (msg.type !== "open_menu_at_video" || !enabled) return;
  const t = [...tracked.values()].find((x) => x.video.currentSrc === msg.src || x.video.src === msg.src);
  if (t) {
    show(t);
    openMenu(t);
  }
});

loadSettings().then((s) => (s.videoButton ? start() : undefined));
chrome.storage.onChanged.addListener((changes) => {
  const s = changes.settings?.newValue as { videoButton?: boolean } | undefined;
  if (!s) return;
  if (s.videoButton) start();
  else stop();
});

// Liens vers des fichiers téléchargeables (fenêtre de l'extension, « Fichiers de cette page »).
chrome.runtime.onMessage.addListener((msg: { type: string; extensions?: string[] }, _s, reply) => {
  if (msg.type !== "collect_links" || window !== window.top) return;
  const exts = new Set((msg.extensions ?? []).map((e) => e.toLowerCase()));
  const seen = new Set<string>();
  for (const a of document.querySelectorAll<HTMLAnchorElement>("a[href]")) {
    const href = a.href;
    if (!/^https?:/i.test(href) || seen.has(href)) continue;
    const ext = href.split(/[?#]/)[0].split("/").pop()?.split(".").pop()?.toLowerCase() ?? "";
    if (exts.has(ext)) seen.add(href);
    if (seen.size >= 200) break;
  }
  reply([...seen]);
});
