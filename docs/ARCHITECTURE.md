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
| `apps/browser-extension` | Extension MV3 Chrome/Edge | Phase 3 |
| `crates/ndm-native-host` | Hôte Native Messaging | Phase 4 |
| `crates/ndm-media` | yt-dlp, FFmpeg, HLS/DASH | Phase 5 |
| `packages/shared-types` | Types partagés extension ↔ application | Phase 3 |

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

## Application (`apps/desktop`)

Le cœur Rust expose des commandes Tauri (ajout par lots, pause, reprise, arrêt, relance, suppression,
priorité, limite par fichier, historique, réglages, ouverture de fichier ou de dossier) et relaie les
événements du moteur vers l'interface. Fermer la fenêtre la réduit dans la zone de notification (réglable).
Une seule instance peut tourner : la relancer ramène la fenêtre existante.

Permissions de la fenêtre (`capabilities/default.json`) : uniquement les dialogues de choix de dossier et
de confirmation. Politique de sécurité du contenu stricte : aucun script ni ressource externe.

## Communication extension ↔ application (phases 3 et 4)

L'extension parle à `ndm-native-host.exe` par Native Messaging (le navigateur vérifie l'ID de l'extension
via `allowed_origins`). L'hôte relaie vers l'application par un canal nommé Windows restreint à
l'utilisateur courant. Aucun port réseau n'est ouvert. Les cookies d'un site ne sont transmis à NDM que si
l'utilisateur l'autorise explicitement pour ce téléchargement.
