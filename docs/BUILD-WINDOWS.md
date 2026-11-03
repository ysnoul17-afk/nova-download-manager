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

## Résultat

```
target\release\nova-download-manager.exe                                 application seule
target\release\bundle\nsis\Nova Download Manager_0.1.0_x64-setup.exe     installateur (recommandé)
target\release\bundle\msi\Nova Download Manager_0.1.0_x64_fr-FR.msi       installateur MSI
```

L'installateur NSIS s'installe pour l'utilisateur courant, sans droits administrateur. La désinstallation
se fait depuis « Paramètres > Applications ».

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
- Testé sous Linux (application complète, interface comprise). La compilation Windows n'a pas pu être
  vérifiée dans l'environnement de développement : le premier passage du script fait foi.
