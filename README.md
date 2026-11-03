# Nova Download Manager

Gestionnaire de téléchargements pour Windows, alternative libre à Internet Download Manager.
Application Tauri 2 (Rust + React/TypeScript) avec un moteur de téléchargement multisegment écrit en Rust.

## État d'avancement

| Phase | Contenu | État |
|---|---|---|
| 1. Fondations | Monorepo, application Tauri, interface React, SQLite, réglages, moteur, progression | Fait |
| 2. Moteur avancé | Multisegment, pause/reprise, simultanés, erreurs, historique, limite de vitesse | Fait |
| 5. Vidéos avancées | yt-dlp, FFmpeg, HLS/DASH, choix de la qualité | Fait (réalisée avant la phase 3) |
| 3. Extension navigateur | Détection des vidéos, bouton flottant par lecteur, interception | Fait |
| 4. Intégration navigateur | Native Messaging, canal local sécurisé, relancement automatique | Fait |
| 6. Finalisation | Notifications, planificateur, i18n, installateur signé, mises à jour | À faire |

## Ce qui a été testé

**Tests automatisés** (50 tests Rust : moteur, vidéo, communication, application) contre des serveurs HTTP locaux :

- téléchargement multisegment ;
- serveur sans reprise, taille inconnue, fichier vide ;
- coupures de connexion répétées, serveur limitant les connexions ;
- pause/reprise sans re-télécharger, survie à un redémarrage du moteur ;
- fichier modifié sur le serveur pendant une pause ;
- limite de vitesse, file d'attente et priorités, doublons de noms ;
- erreurs 404 et adresses invalides ;
- historique SQLite (recherche, filtres, tris, injection SQL) ;
- traversée de répertoires ;
- vidéo avec le vrai yt-dlp et FFmpeg sur un flux HLS multi-qualités (image et son séparés, comme YouTube) :
  analyse des qualités, respect de la résolution choisie et fusion de l'audio vérifiées par ffprobe,
  extraction MP3, page HTML5, ajout par lots, pause qui arrête réellement yt-dlp puis reprise ;
- analyse de formats façon YouTube (estimation des tailles, rejet du DRM, listes de lecture, directs) ;
- installation de yt-dlp depuis GitHub avec vérification de l'empreinte SHA-256, extraction sécurisée de FFmpeg ;
- communication navigateur : format Native Messaging, jeton (mauvais jeton refusé), requêtes invalides
  refusées (`file://`, `javascript:`, injection d'en-têtes), application absente détectée.

**Test de bout en bout de l'extension** (`apps/browser-extension/test/e2e.mjs`, 20 vérifications dans un vrai
Chrome avec l'hôte natif et l'application) : page à dix vidéos (un bouton chacune, aucun doublon, placement,
survol lecteur → bouton), clic simulé par la page ignoré, vidéos ajoutées dynamiquement, vidéo `blob:` dont le
flux HLS est repéré puis assemblé, vidéo protégée par DRM expliquée, interception d'un `.zip`, fenêtre de
l'extension, application arrêtée puis relancée automatiquement.

**Tests manuels de l'application complète** (Linux, écran virtuel) :

- ajout par lots ;
- 8 connexions par fichier et carte des segments ;
- pause au clavier ;
- plantage de l'application en plein transfert (`kill -9`) puis reprise automatique au relancement ;
- coupure puis retour du serveur ;
- intégrité vérifiée par SHA-256 identique à la source ;
- historique, paramètres, thème sombre ;
- vidéo : installation guidée de yt-dlp, choix de la qualité, étapes affichées en direct.

**Non testé à ce stade** :

- utilisation sous Windows : la compilation, les tests du moteur et la production des installateurs NSIS/MSI
  ont réussi sous Windows 11, mais l'application installée reste à valider (zone de notification comprise) ;
- fichiers de plusieurs gigaoctets ;
- erreurs DNS et disque plein réels (messages couverts par des tests unitaires seulement) ;
- YouTube réel : testé une fois par l'utilisateur sous Windows (1080p, image et son assemblés) ; les autres
  sites réels ne sont pas testés ;
- extension sous Windows (Chrome, Edge) : testée sous Linux avec Chrome for Testing uniquement ;
- installation automatique de FFmpeg (Windows uniquement : archive officielle vérifiée ici, extraction testée).

## Démarrage

- Compiler sous Windows : [docs/BUILD-WINDOWS.md](docs/BUILD-WINDOWS.md)
- Utilisation : [docs/GUIDE-UTILISATEUR.md](docs/GUIDE-UTILISATEUR.md)
- Architecture et choix techniques : [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
