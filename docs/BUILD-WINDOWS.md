# Compiler Nova Download Manager sous Windows

## Méthode rapide (script)

Dans PowerShell, depuis le dossier du projet :

```powershell
powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1 -InstallPrerequisites
```

Le script vérifie et installe au besoin (via `winget`) :

| Prérequis | Pourquoi | Taille approximative |
|---|---|---|
| Visual Studio Build Tools 2022, charge de travail « Développement Desktop en C++ » | Éditeur de liens MSVC et compilateur C (SQLite) | 2 à 4 Go |
| Rust (rustup, chaîne stable) | Compilation du moteur et de l'application | 1 Go |
| Node.js 20 ou plus (LTS) | Compilation de l'interface | 100 Mo |
| WebView2 Runtime | Moteur d'affichage (déjà présent sous Windows 11) | — |

Si le script s'arrête après avoir installé des outils, fermez PowerShell, rouvrez-le et relancez la commande
(sans `-InstallPrerequisites`).

Il lance ensuite les tests du moteur, puis la compilation. La première compilation prend 5 à 15 minutes.

## Erreur « os error 4551 » : Contrôle intelligent des applications

Si la compilation échoue avec « Une stratégie de contrôle d'application a bloqué ce fichier (os error 4551) »,
c'est le **Contrôle intelligent des applications** (Smart App Control) de Windows 11 qui bloque les programmes
non signés produits par le compilateur. Pour compiler :

1. `Windows + R`, puis `windowsdefender://appbrowser`.
2. **Paramètres du contrôle intelligent des applications** → **Désactivé**.

Depuis la mise à jour cumulative d'avril 2026 (KB5083769), il peut être réactivé au même endroit. Tant que NDM
n'est pas signé numériquement, il est lui aussi bloqué lorsque cette protection est active.

## Résultat

```
target\release\nova-download-manager.exe                                 application seule
target\release\bundle\nsis\Nova Download Manager_0.3.0_x64-setup.exe     installateur (recommandé)
target\release\bundle\msi\Nova Download Manager_0.3.0_x64_fr-FR.msi       installateur MSI
```

L'installateur NSIS s'installe pour l'utilisateur courant, sans droits administrateur. La désinstallation
se fait depuis « Paramètres > Applications ».

## Installer l'extension Chrome ou Edge

Le script construit l'extension dans `apps\browser-extension\dist`. Pour la charger :

- **Chrome** : `chrome://extensions`, activer **Mode développeur**, **Charger l'extension non empaquetée**,
  choisir le dossier `dist`.
- **Edge** : `edge://extensions`, activer **Mode développeur**, **Charger l'extension décompressée**,
  choisir le dossier `dist`.

Lancez NDM au moins une fois après l'installation : il se déclare alors auprès des navigateurs. Copiez de
préférence `dist` dans un dossier permanent (hors OneDrive), car le navigateur charge l'extension depuis cet
emplacement.

## Mettre à jour une installation existante

Quittez NDM (icône de la zone de notification → **Quitter**), relancez le script puis l'installateur : il
remplace l'ancienne version et conserve l'historique et les réglages.

## Méthode manuelle

```powershell
cargo test -p ndm-engine          # tests du moteur
cd apps\desktop
npm ci
npx tauri dev                     # lance l'application en mode développement
npx tauri build                   # produit les installateurs
```

## Données de l'application

`%APPDATA%\app.novadownloadmanager.desktop\ndm.sqlite3` contient l'historique et les réglages.
Les fichiers sont enregistrés par défaut dans `Téléchargements\NDM\<catégorie>`.

## Limites connues de cette version

- L'installateur n'est pas signé numériquement : Windows SmartScreen affichera un avertissement au premier
  lancement (« Informations complémentaires » puis « Exécuter quand même »). La signature et les mises à
  jour automatiques signées sont prévues en phase 6.
- Les tests vidéo de bout en bout (`crates/ndm-engine/tests/media.rs`) ne s'exécutent que si la variable
  `NDM_TEST_YTDLP` pointe vers yt-dlp et que `ffmpeg` est dans le PATH ; sinon ils sont sautés.
