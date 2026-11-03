// Compile l'hôte Native Messaging et le place là où Tauri l'attend
// (src-tauri/binaries/ndm-native-host-<triplet>[.exe]), pour qu'il soit installé à côté de l'application.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const release = process.argv.includes("--release");
const triple = execFileSync("rustc", ["--print", "host-tuple"], { encoding: "utf8" }).trim();
const ext = process.platform === "win32" ? ".exe" : "";

execFileSync("cargo", ["build", "-p", "ndm-native-host", ...(release ? ["--release"] : [])], {
  cwd: root,
  stdio: "inherit",
});
const built = join(root, "target", release ? "release" : "debug", `ndm-native-host${ext}`);
const dest = join(root, "apps", "desktop", "src-tauri", "binaries", `ndm-native-host-${triple}${ext}`);
mkdirSync(dirname(dest), { recursive: true });
copyFileSync(built, dest);
console.log(`hôte natif prêt : ${dest}`);
