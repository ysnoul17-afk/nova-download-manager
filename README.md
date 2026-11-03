# Nova Download Manager

Gestionnaire de téléchargements pour Windows, alternative libre à Internet Download Manager.
Application Tauri 2 (Rust + React/TypeScript) avec un moteur de téléchargement multisegment écrit en Rust.

## État d'avancement

| Phase | Contenu | État |
|---|---|---|
| 1. Fondations | Monorepo, application Tauri, interface React, SQLite, réglages, moteur, progression | Fait |
| 2. Moteur avancé | Multisegment, pause/reprise, simultanés, erreurs, historique, limite de vitesse | Fait |
| 3. Extension navigateur | Détection des vidéos, bouton flottant par lecteur | À faire |
| 4. Intégration navigateur | Native Messaging, synchronisation des états | À faire |
| 5. Vidéos avancées | yt-dlp, FFmpeg, HLS/DASH, choix de la qualité | À faire |
| 6. Finalisation | Notifications, planificateur, i18n, installateur signé, mises à jour | À faire |

## Ce qui a été testé

**Tests automatisés** (`cargo test -p ndm-engine`, 26 tests) contre un serveur HTTP local :

- téléchargement multisegment ;
- serveur sans reprise, taille inconnue, fichier vide ;
- coupures de connexion répétées, serveur limitant les connexions ;
- pause/reprise sans re-télécharger, survie à un redémarrage du moteur ;
- fichier modifié sur le serveur pendant une pause ;
- limite de vitesse, file d'attente et priorités, doublons de noms ;
- erreurs 404 et adresses invalides ;
- historique SQLite (recherche, filtres, tris, injection SQL) ;
- traversée de répertoires.

**Tests manuels de l'application complète** (Linux, écran virtuel) :

- ajout par lots ;
- 8 connexions par fichier et carte des segments ;
- pause au clavier ;
- plantage de l'application en plein transfert (`kill -9`) puis reprise automatique au relancement ;
- coupure puis retour du serveur ;
- intégrité vérifiée par SHA-256 identique à la source ;
- historique, paramètres, thème sombre.

**Non testé à ce stade** :

- compilation et exécution sous Windows ;
- zone de notification Windows ;
- installateurs NSIS/MSI ;
- fichiers de plusieurs gigaoctets ;
- erreurs DNS et disque plein réels (messages couverts par des tests unitaires seulement).

## Démarrage

- Compiler sous Windows : [docs/BUILD-WINDOWS.md](docs/BUILD-WINDOWS.md)
- Utilisation : [docs/GUIDE-UTILISATEUR.md](docs/GUIDE-UTILISATEUR.md)
- Architecture et choix techniques : [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
