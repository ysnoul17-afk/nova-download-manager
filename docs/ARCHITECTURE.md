# Architecture de Nova Download Manager

## Vue d'ensemble

```
┌──────────────────────────┐   Native Messaging (stdio, JSON)    ┌────────────────────────┐
│ Extension Chrome / Edge  │ ──────────────────────────────────▶ │ ndm-native-host.exe    │
│  détection des vidéos,   │                                      │ (hôte natif minimal)   │
│  bouton flottant,        │                                      └───────────┬────────────┘
│  interception            │                                                  │ canal nommé Windows
└──────────────────────────┘                                                  │ (utilisateur courant)
                                                                              ▼
                     ┌──────────────────────────────────────────────────────────────┐
                     │ Nova Download Manager (Tauri 2)                              │
                     │  Interface React/TypeScript ◀── commandes / événements ──▶   │
                     │  ndm-engine (Rust) : HTTP multisegment, file, reprise, SQLite│
                     │  ndm-media (phase 5) : yt-dlp, FFmpeg                        │
                     └──────────────────────────────────────────────────────────────┘
```

Seules les briques en gras dans le tableau ci-dessous existent aujourd'hui ; les autres arrivent dans
les phases indiquées, sans changement d'architecture.

## Organisation du monorepo

```
nova-download-manager/
├── Cargo.toml                 espace de travail Rust
├── crates/
│   └── ndm-engine/            moteur de téléchargement (bibliothèque Rust, sans interface)
│       ├── src/
│       └── tests/engine.rs    tests d'intégration contre un serveur HTTP local
├── apps/
│   └── desktop/               application Windows
│       ├── src/               interface React + TypeScript
│       └── src-tauri/         cœur Rust de l'application (commandes, zone de notification)
├── scripts/build-windows.ps1  compilation et production des installateurs
└── docs/
```

| Composant | Rôle | État |
|---|---|---|
| **`crates/ndm-engine`** | Moteur HTTP/HTTPS, file d'attente, reprise, limitation de vitesse, SQLite | Fait, testé |
| **`apps/desktop`** | Application Tauri : interface, commandes, zone de notification | Fait, testé sous Linux |
| **`apps/browser-extension`** | Extension MV3 Chrome/Edge (TypeScript, esbuild) | Fait, testé dans Chrome |
| **`crates/ndm-ipc`** | Protocole extension ↔ application, canal local, jeton | Fait, testé |
| **`crates/ndm-native-host`** | Hôte Native Messaging (`ndm-native-host.exe`) | Fait, testé |
| **`ndm-engine::media` et `::tools`** | yt-dlp, FFmpeg, HLS/DASH, installation vérifiée | Fait, testé |

**Écart avec la structure suggérée, et pourquoi.** Le cahier des charges proposait `packages/download-core`
et `services/aria2`. Le code Rust est rangé dans `crates/` (convention Cargo) et le code JavaScript dans
`apps/` et `packages/`, ce qui sépare clairement les deux chaînes de compilation. Les dossiers des phases
futures ne sont pas créés vides : ils apparaîtront avec leur code. Les tests unitaires vivent à côté du code
qu'ils testent (convention Rust), les tests d'intégration dans `crates/*/tests/`.

## Décision : moteur Rust natif plutôt qu'aria2

Le cahier des charges demandait d'évaluer aria2 comme moteur initial. Décision : **moteur Rust intégré**,
pour ces raisons.

| Critère | aria2 (processus externe + RPC) | Moteur Rust intégré |
|---|---|---|
| Distribution | Binaire tiers à embarquer, signer et mettre à jour | Compilé dans l'application |
| Communication | Serveur JSON-RPC local sur un port TCP | Appels directs, aucun port ouvert |
| Sécurité | Port local à protéger par jeton (exigence « pas de serveur local non sécurisé ») | Rien d'exposé |
| Segmentation dynamique type IDM | Segments fixes, pas de vol de travail | Vol de travail entre connexions |
| Carte des segments dans l'UI | Non exposée par l'API | Exposée directement |
| Messages d'erreur en français | Codes aria2 à traduire | Natifs |
| Protocoles FTP/BitTorrent/Metalink | Oui | Non (hors périmètre IDM) |

Le seul avantage décisif d'aria2 (FTP, BitTorrent) est hors du périmètre demandé. Si ce besoin apparaît,
aria2 pourra être ajouté comme moteur secondaire derrière l'API `Engine` sans toucher à l'interface.

## Moteur (`ndm-engine`)

**Analyse.** `GET` avec `Range: bytes=0-0` (plus fiable que `HEAD`, souvent mal géré). On en déduit la
taille, le support des plages d'octets, le nom (Content-Disposition RFC 5987, puis URL, puis type MIME),
l'ETag et Last-Modified.

**Segmentation dynamique.** Le fichier démarre en N segments (8 par défaut, réglable de 1 à 32, jamais
plus petits que 1 Mo). Quand une connexion termine, elle scinde en deux le segment actif qui a le plus de
travail restant et en prend la seconde moitié : les connexions rapides absorbent le travail des lentes.

**Plus de connexions n'est pas toujours mieux.** Si le serveur refuse des connexions (429, 503) alors que
d'autres fonctionnent, la connexion refusée se retire silencieusement au lieu d'échouer.

**Écriture.** Tampon de 512 Ko par connexion, écritures positionnelles (`seek_write` sous Windows) dans un
fichier pré-alloué `nom.ext.ndmpart`. La pré-allocation détecte tôt un disque plein. Le fichier n'est
renommé qu'après vérification que sa taille est exacte.

**Reprise.** L'état des segments est enregistré dans SQLite toutes les 2 secondes et à chaque changement
d'état. À la reprise : nouvelle analyse, puis requêtes avec `If-Range`. Si le fichier distant a changé
(ETag, date ou taille différents, ou réponse `200` au lieu de `206`), le téléchargement repart proprement
de zéro au lieu de produire un fichier corrompu. Testé : pause, fermeture, plantage (`kill -9`), coupure
du serveur.

**File d'attente.** Nombre de téléchargements simultanés réglable (1 à 16), priorités Haute / Normale /
Basse, puis ordre d'arrivée. À la fermeture de l'application, les téléchargements en cours reprennent
automatiquement au lancement suivant.

**Vitesse.** Seau de jetons global et par fichier, modifiable pendant le téléchargement.

**Persistance (SQLite, `ndm.sqlite3`).** Table `downloads` avec colonnes indexées (statut, date, catégorie)
pour la recherche, les filtres et les tris, plus l'enregistrement complet en JSON. Table `settings` pour
les réglages. Mode WAL, schéma versionné (`PRAGMA user_version`).

**Sécurité.** Seuls `http` et `https` sont acceptés. Noms de fichiers nettoyés (caractères interdits,
noms réservés Windows comme `CON`, longueur), aucune séparation de chemin possible dans un nom. Dossiers
de destination : chemins absolus uniquement, `..` refusé. Recherche SQL paramétrée. L'interface ne peut
ouvrir un fichier que par son identifiant de téléchargement, jamais par un chemin libre, et une
confirmation est demandée avant d'ouvrir un programme. Aucun fichier n'est exécuté automatiquement.

## Vidéo (`media.rs`, `tools.rs`)

**Analyse.** `yt-dlp -J` renvoie les formats d'une page. NDM en tire des options simples : une par résolution
(meilleur flux vidéo à cette hauteur, plus la meilleure piste audio si elle est séparée), plus l'audio seul en
MP3 et M4A. La taille de chaque option est estimée (taille annoncée, sinon débit × durée). Les formats marqués
DRM sont écartés ; si tous le sont, la vidéo est refusée avec une explication. Listes de lecture et directs sont
refusés explicitement pour l'instant.

**Téléchargement.** Une vidéo est un téléchargement comme un autre dans le moteur (file d'attente, priorité,
pause, historique). yt-dlp est lancé sans console, dans son propre groupe de processus : la pause arrête tout
l'arbre de processus (`taskkill /T` sous Windows), et la reprise relance yt-dlp qui continue ses fichiers
partiels. La progression est lue sur une ligne formatée par `--progress-template`, en suivant les flux
successifs (piste vidéo, piste audio) puis les étapes FFmpeg (fusion, conversion, sous-titres). Le nom final est
choisi par NDM (titre nettoyé, sans collision), et les fichiers intermédiaires sont supprimés en cas d'abandon.

**Outils.** yt-dlp et FFmpeg ne sont pas embarqués dans l'installateur : ils évoluent trop vite (yt-dlp suit
les changements de YouTube). NDM les télécharge à la demande depuis leurs dépôts GitHub officiels et compare
leur SHA-256 au fichier de sommes publié avec la version ; en cas d'écart, rien n'est installé. Seuls
`ffmpeg.exe` et `ffprobe.exe` sont extraits de l'archive, sous des noms fixés par NDM. Les outils déjà présents
sur le système (PATH) sont aussi détectés.

## Application (`apps/desktop`)

Le cœur Rust expose des commandes Tauri (ajout par lots, pause, reprise, arrêt, relance, suppression,
priorité, limite par fichier, historique, réglages, ouverture de fichier ou de dossier) et relaie les
événements du moteur vers l'interface. Fermer la fenêtre la réduit dans la zone de notification (réglable).
Une seule instance peut tourner : la relancer ramène la fenêtre existante.

Permissions de la fenêtre (`capabilities/default.json`) : uniquement les dialogues de choix de dossier et
de confirmation. Politique de sécurité du contenu stricte : aucun script ni ressource externe.

## Extension et communication avec l'application

**Chaîne.** L'extension appelle `chrome.runtime.sendNativeMessage("app.novadm.host", …)`. Le navigateur lance
`ndm-native-host.exe` (installé à côté de l'application) seulement si l'identifiant de l'extension figure dans
`allowed_origins` ; cet identifiant est fixe (`eogmknmllnjfblmecmcngbbpgikhigjc`) grâce à la clé publique du
manifeste. L'hôte valide le message (type autorisé, adresses `http(s)` seulement, pas de retour à la ligne
dans les champs, taille bornée) puis le relaie à l'application par un **canal nommé Windows** (clients distants
refusés, première instance exigée pour empêcher l'usurpation) accompagné d'un **jeton de 256 bits** lu dans le
dossier de données de l'utilisateur. Aucun port réseau n'est ouvert. Si l'application ne répond pas, l'hôte la
lance avec `--minimized` (fenêtre masquée) puis réessaie pendant 20 secondes.

**Déclaration.** À chaque démarrage, l'application écrit le manifeste de l'hôte et l'enregistre pour Chrome,
Edge, Chromium et Brave (`HKCU\Software\…\NativeMessagingHosts`), ce qui suit les mises à jour et les
déplacements de l'installation.

**Requêtes.** `ping`, `download` (lien intercepté ou envoyé : l'application **vérifie d'abord** que le lien
renvoie bien un fichier, sinon le navigateur garde son téléchargement), `analyze` (fichier, vidéo, page),
`download_media` (qualité choisie), `status` (fenêtre de l'extension, sans lancer l'application), `show`.

**Bouton vidéo (script de contenu).** Les `<video>` sont repérées au chargement et par un `MutationObserver`
(pages dynamiques, vidéos retirées). Le pointeur est suivi par position, car les lecteurs posent des calques au
dessus de la balise vidéo ; en cas d'imbrication, la plus petite vidéo l'emporte. Un seul bouton par vidéo, dans
un shadow DOM ouvert attaché à la racine du document (aucun conflit de styles, aucune interférence avec les
contrôles du lecteur) ; seuls les clics réels (`event.isTrusted`) sont acceptés, donc la page ne peut pas
déclencher de téléchargement à la place de l'utilisateur. Cible analysée : la source HTTP de la vidéo si elle
existe, sinon le lien qui l'entoure, sinon la page ; pour une vidéo `blob:` hors des sites connus, les flux
HLS/DASH repérés dans le trafic de l'onglet (`webRequest`, observation seule) sont proposés en premier. Une
vidéo avec des clés de chiffrement (`video.mediaKeys`) est signalée comme protégée, sans tentative.

**Interception.** En mode automatique, `downloads.onDeterminingFilename` retient les fichiers dont l'extension
figure dans la liste ; si NDM accepte, le téléchargement du navigateur est annulé, sinon il continue normalement
(application absente, lien non récupérable).

**Permissions.** `nativeMessaging`, `downloads`, `contextMenus`, `storage`, `webRequest` et l'accès aux pages
`http(s)` (bouton vidéo, repérage des flux). `cookies` est **facultative** : demandée seulement si l'utilisateur
active « Transmettre les cookies du site ». Les cookies ne sont alors envoyés qu'à NDM, sur la machine.
