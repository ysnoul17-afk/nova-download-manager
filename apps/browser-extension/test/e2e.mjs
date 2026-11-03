// Test de bout en bout : vrai Chrome + extension + hôte natif + application NDM.
//
// Variables d'environnement :
//   CHROME      exécutable Chrome / Chrome for Testing
//   NDM_HOME    dossier personnel utilisé par l'application lancée (pour trouver ses fichiers)
//   PAGE        page de test (dix vidéos, vidéos dynamiques, blob, DRM, lien .zip)
//   NDM_APP_PID processus de l'application, arrêté pour tester le relancement automatique
//
// Le test s'arrête au premier échec avec un message explicite.

import puppeteer from "puppeteer-core";
import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";

const CHROME = process.env.CHROME ?? "google-chrome";
const HOME = process.env.NDM_HOME;
const PAGE = process.env.PAGE ?? "http://127.0.0.1:8765/ext/ten.html";
const DIST = resolve("dist");
const HOST = "app.novadm.host";
const EXT_ID = "eogmknmllnjfblmecmcngbbpgikhigjc";

const results = [];
function ok(name) {
  results.push(name);
  console.log(`  ✔ ${name}`);
}
function fail(name, detail) {
  console.error(`  ✘ ${name}\n    ${detail}`);
  process.exitCode = 1;
  throw new Error(name);
}
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
async function until(fn, ms, what) {
  const t = Date.now();
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() - t > ms) fail(what, "délai dépassé");
    await sleep(100);
  }
}

// Profil Chrome neuf ; la déclaration de l'hôte natif est copiée dans le profil.
const profile = join(HOME, "chrome-profile");
rmSync(profile, { recursive: true, force: true });
mkdirSync(join(profile, "NativeMessagingHosts"), { recursive: true });
copyFileSync(
  join(HOME, ".config", "google-chrome-for-testing", "NativeMessagingHosts", `${HOST}.json`),
  join(profile, "NativeMessagingHosts", `${HOST}.json`),
);

const browser = await puppeteer.launch({
  executablePath: CHROME,
  headless: false,
  userDataDir: profile,
  env: { ...process.env, HOME },
  defaultViewport: { width: 1280, height: 900 },
  args: [`--disable-extensions-except=${DIST}`, `--load-extension=${DIST}`, "--no-first-run", "--no-default-browser-check"],
});

try {
  console.log("Extension");
  const sw = await browser.waitForTarget((t) => t.type() === "service_worker" && t.url().includes(EXT_ID), { timeout: 10000 });
  ok(`service worker chargé avec l'identifiant fixe ${EXT_ID}`);

  const page = await browser.newPage();
  await page.goto(PAGE, { waitUntil: "networkidle0" });

  const shadowQuery = (sel) =>
    page.evaluate((sel) => {
      const root = document.querySelector("[data-ndm]")?.shadowRoot;
      return root ? [...root.querySelectorAll(sel)].map((e) => {
        const r = e.getBoundingClientRect();
        return { x: r.x, y: r.y, w: r.width, h: r.height, text: e.textContent, cls: e.className };
      }) : [];
    }, sel);

  async function hoverVideo(selector, index = 0) {
    const box = await page.evaluate(
      (selector, index) => {
        const v = document.querySelectorAll(selector)[index];
        v.scrollIntoView({ block: "center" });
        const r = v.getBoundingClientRect();
        return { x: r.x, y: r.y, w: r.width, h: r.height };
      },
      selector,
      index,
    );
    await sleep(150);
    await page.mouse.move(box.x + box.w / 2, box.y + box.h / 2, { steps: 4 });
    await sleep(200);
    return box;
  }

  // --- Dix vidéos : un bouton chacune, au bon endroit, sans doublon ---------
  console.log("Dix vidéos");
  for (let i = 0; i < 10; i++) {
    const box = await hoverVideo("#grid video", i);
    const visible = await shadowQuery(".btn.on");
    if (visible.length !== 1) fail(`un seul bouton visible sur la vidéo ${i + 1}`, `${visible.length} visibles`);
    const b = visible[0];
    const inside = b.x >= box.x && b.x + b.w <= box.x + box.w + 1 && b.y >= box.y && b.y < box.y + box.h / 3;
    if (!inside) fail(`bouton placé en haut du lecteur ${i + 1}`, JSON.stringify({ b, box }));
  }
  ok("un bouton visible par vidéo survolée, en haut à droite du lecteur, malgré le calque posé dessus");
  const all = await shadowQuery(".btn");
  if (all.length !== 10) fail("aucun bouton en double", `${all.length} boutons pour 10 vidéos`);
  ok("10 boutons pour 10 vidéos, aucun doublon après 10 survols");

  // Du lecteur au bouton : le bouton ne disparaît pas.
  await hoverVideo("#grid video", 0);
  let [btn] = await shadowQuery(".btn.on");
  await page.mouse.move(btn.x + btn.w / 2, btn.y + btn.h / 2, { steps: 8 });
  await sleep(900);
  if ((await shadowQuery(".btn.on")).length !== 1) fail("le bouton reste visible quand on le rejoint", "disparu");
  ok("le bouton reste visible en passant du lecteur au bouton");

  // Hors de toute vidéo : il disparaît.
  await page.mouse.move(5, 5, { steps: 4 });
  await sleep(1000);
  if ((await shadowQuery(".btn.on")).length !== 0) fail("le bouton disparaît hors du lecteur", "toujours visible");
  ok("le bouton disparaît quand le pointeur quitte le lecteur");

  // --- Menu sur une vidéo directe → envoi à NDM -----------------------------
  console.log("Menu et envoi à NDM");
  await hoverVideo("#grid video", 0);
  [btn] = await shadowQuery(".btn.on");
  await page.mouse.click(btn.x + btn.w / 2, btn.y + btn.h / 2);
  const opt = await until(async () => (await shadowQuery(".menu .opt"))[0], 15000, "menu avec les formats disponibles");
  if (!/Original/.test(opt.text) || !/MP4/.test(opt.text)) fail("format proposé pour un fichier direct", opt.text);
  ok(`menu ouvert, format réellement disponible proposé : « ${opt.text.replace(/\s+/g, " ").trim()} »`);
  // Le menu ne ferme pas le bouton et reste ancré dessous.
  const [menu] = await shadowQuery(".menu");
  if (!(menu.y > btn.y)) fail("menu sous le bouton", JSON.stringify({ menu, btn }));
  await page.mouse.click(opt.x + opt.w / 2, opt.y + opt.h / 2);
  await until(async () => (await shadowQuery(".menu .ok")).length, 10000, "confirmation « Ajouté à NDM »");
  ok("vidéo envoyée : « Ajouté à Nova Download Manager »");
  const videos = join(HOME, "Downloads", "NDM", "Vidéos");
  await until(() => existsSync(videos) && readdirSync(videos).some((f) => f.startsWith("clip-demo")), 15000, "fichier reçu par NDM");
  ok("NDM a téléchargé le fichier dans Vidéos");

  // Clic simulé par la page : ignoré. (Le menu de confirmation se ferme seul d'abord.)
  await until(async () => (await shadowQuery(".menu")).length === 0, 5000, "fermeture du menu de confirmation");
  await hoverVideo("#grid video", 3);
  const fake = await page.evaluate(() => {
    const b = document.querySelector("[data-ndm]").shadowRoot.querySelector(".btn.on");
    b.click();
    return !!document.querySelector("[data-ndm]").shadowRoot.querySelector(".menu");
  });
  if (fake) fail("clic simulé par la page ignoré", "le menu s'est ouvert");
  ok("un clic simulé par le JavaScript de la page n'ouvre pas le menu");

  // --- Vidéos ajoutées dynamiquement ----------------------------------------
  console.log("Pages dynamiques");
  await page.waitForSelector("#dyn video:nth-child(1), #dyn .player video", { timeout: 5000 });
  for (let i = 0; i < 3; i++) {
    await hoverVideo("#dyn video", i);
    if ((await shadowQuery(".btn.on")).length !== 1) fail(`bouton sur la vidéo ajoutée ${i + 1}`, "absent");
  }
  ok("les 3 vidéos ajoutées après le chargement ont leur bouton");
  await page.evaluate(() => document.querySelector("#dyn .player").remove());
  await sleep(300);
  if ((await shadowQuery(".btn")).length !== 12) fail("bouton retiré avec sa vidéo", `${(await shadowQuery(".btn")).length} boutons`);
  ok("le bouton d'une vidéo retirée de la page est supprimé");

  // --- Vidéo en blob: → flux HLS repéré → qualités --------------------------
  console.log("Vidéo en flux (blob:)");
  await page.waitForFunction(() => document.getElementById("blobvid").src.startsWith("blob:"));
  await hoverVideo("#blobvid");
  [btn] = await shadowQuery(".btn.on");
  await page.mouse.click(btn.x + btn.w / 2, btn.y + btn.h / 2);
  const stream = await until(async () => (await shadowQuery(".menu .opt")).find((o) => /HLS/.test(o.text)), 10000, "flux HLS repéré");
  ok("vidéo blob: : le flux HLS chargé par la page est proposé");
  await page.mouse.click(stream.x + stream.w / 2, stream.y + stream.h / 2);
  const q = await until(async () => {
    const o = await shadowQuery(".menu .opt");
    return o.length >= 3 ? o : null;
  }, 30000, "qualités du flux");
  const labels = q.map((o) => o.text.replace(/\s+/g, " ").trim());
  if (!labels.some((l) => l.startsWith("1080p")) || !labels.some((l) => /MP3/.test(l)))
    fail("qualités réellement disponibles", labels.join(" | "));
  ok(`qualités du flux : ${labels.join(" | ")}`);
  const p480 = q.find((o) => o.text.trim().startsWith("480p"));
  await page.mouse.click(p480.x + p480.w / 2, p480.y + p480.h / 2);
  await until(async () => (await shadowQuery(".menu .ok")).length, 10000, "envoi de la qualité 480p");
  ok("qualité 480p envoyée à NDM");
  await until(() => readdirSync(videos).some((f) => f.startsWith("master") && f.endsWith(".mp4")), 60000, "vidéo HLS assemblée par NDM");
  ok("NDM a assemblé la vidéo HLS (image + son) en MP4");

  // --- DRM -------------------------------------------------------------------
  console.log("DRM");
  const drm = await page.evaluate(() => document.body.dataset.drm);
  if (drm === "ready") {
    await page.keyboard.press("Escape");
    await hoverVideo("#drmvid");
    [btn] = await shadowQuery(".btn.on");
    await page.mouse.click(btn.x + btn.w / 2, btn.y + btn.h / 2);
    const m = await until(async () => (await shadowQuery(".menu .err"))[0], 5000, "message DRM").catch(async (e) => {
      console.error("    menu :", JSON.stringify(await shadowQuery(".menu")), "boutons :", JSON.stringify(await shadowQuery(".btn.on")));
      console.error("    mediaKeys :", await page.evaluate(() => String(document.getElementById("drmvid").mediaKeys)));
      throw e;
    });
    if (!/DRM/.test(m.text)) fail("explication DRM", m.text);
    ok("vidéo protégée : explication affichée, aucune tentative de téléchargement");
  } else {
    console.log(`  (ClearKey indisponible dans ce navigateur : ${drm})`);
  }

  // --- Interception d'un téléchargement du navigateur ------------------------
  console.log("Interception");
  await page.keyboard.press("Escape");
  await page.click("#zip");
  const archives = join(HOME, "Downloads", "NDM", "Archives");
  await until(() => existsSync(join(archives, "album-live.zip")), 15000, "zip repris par NDM");
  await sleep(1500);
  const browserCopy = readdirSync(join(HOME, "Downloads")).filter((f) => f.startsWith("album-live"));
  if (browserCopy.length) fail("téléchargement du navigateur annulé", browserCopy.join(", "));
  ok("clic sur un .zip : NDM le télécharge, le navigateur abandonne le sien");

  // --- Fenêtre de l'extension -------------------------------------------------
  console.log("Fenêtre de l'extension");
  const popup = await browser.newPage();
  await popup.goto(`chrome-extension://${EXT_ID}/popup.html`);
  const state = await until(async () => {
    const t = await popup.$eval("#state", (e) => e.textContent);
    return t && t !== "…" ? t : null;
  }, 10000, "état de la connexion");
  if (!/Connecté/.test(state)) fail("fenêtre : connexion à NDM", state);
  ok(`fenêtre : « ${state} »`);
  await popup.close();

  // --- Application arrêtée : relancée automatiquement -------------------------
  if (process.env.NDM_APP_PID) {
    console.log("Perte de connexion");
    process.kill(Number(process.env.NDM_APP_PID), "SIGTERM");
    await sleep(1500);
    const popup2 = await browser.newPage();
    await popup2.goto(`chrome-extension://${EXT_ID}/popup.html`);
    const st = await until(async () => {
      const t = await popup2.$eval("#state", (e) => e.textContent);
      return t && t !== "…" ? t : null;
    }, 10000, "état sans application");
    if (!/Non lancé/.test(st)) fail("fenêtre : application arrêtée détectée", st);
    ok("application arrêtée : la fenêtre l'indique sans la relancer");
    await popup2.close();

    await page.bringToFront();
    await hoverVideo("#grid video", 6);
    [btn] = await shadowQuery(".btn.on");
    await page.mouse.click(btn.x + btn.w / 2, btn.y + btn.h / 2);
    await until(async () => (await shadowQuery(".menu .opt"))[0], 30000, "relancement automatique de NDM");
    ok("bouton vidéo avec application arrêtée : NDM est relancé automatiquement et répond");
  }

  console.log(`\n${results.length} vérifications réussies.`);
} catch (e) {
  if (!process.exitCode) {
    console.error(e);
    process.exitCode = 1;
  }
} finally {
  await browser.close();
}
