# Publier une version et activer les mises à jour automatiques

Les mises à jour sont publiées dans les « Releases » du dépôt GitHub
`ysnoul17-afk/nova-download-manager`. NDM vérifie au démarrage le fichier
`https://github.com/ysnoul17-afk/nova-download-manager/releases/latest/download/latest.json`, télécharge
l'installateur indiqué et **vérifie sa signature** avec la clé publique intégrée à l'application avant de
l'installer. Une mise à jour non signée par votre clé privée est refusée.

## Mise en place (une seule fois)

1. **Créer le dépôt** `nova-download-manager` sur github.com, **public** : les téléchargements des Releases
   d'un dépôt privé ne sont pas accessibles à l'application sans identifiants.
2. **Ajouter deux secrets** : dépôt → *Settings* → *Secrets and variables* → *Actions* → *New repository secret* :
   - `TAURI_SIGNING_PRIVATE_KEY` : le contenu du fichier `ndm-updater.key` (une seule ligne) ;
   - `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` : le mot de passe fourni avec la clé.
3. **Envoyer le code** sur la branche `main`.

Conservez la clé privée et son mot de passe dans un gestionnaire de mots de passe. **Ne les ajoutez jamais
au dépôt.** Si vous les perdez, les versions déjà installées ne pourront plus recevoir de mises à jour
(il faudra réinstaller manuellement une version contenant une nouvelle clé publique).

## Publier une version

1. Mettre le même numéro dans `Cargo.toml` (`[workspace.package]`), `crates/ndm-engine/Cargo.toml`,
   `apps/desktop/package.json` et `apps/desktop/src-tauri/tauri.conf.json`.
2. Décrire les nouveautés dans `CHANGELOG.md`, sous un titre `## X.Y.Z` : ce texte est affiché dans la
   fenêtre de mise à jour de NDM et sur la page de la version.
3. Envoyer le code sur `main`. Le workflow *Publication* (`.github/workflows/release.yml`) voit qu'aucune
   étiquette `vX.Y.Z` n'existe pour ce numéro et publie la version ; un envoi sans changement de numéro ne
   publie rien. Il peut aussi être lancé à la main depuis l'onglet *Actions*.
4. Sur un serveur Windows de GitHub : tests, compilation, signature des artefacts de mise à jour, création de
   l'étiquette et de la Release avec l'installateur `.exe`, le `.msi`, `latest.json` et l'extension en `.zip`.
   Durée : 15 à 25 minutes.
5. Au démarrage suivant, les NDM installés affichent « Version x.y.z disponible » dans la barre d'état.

La compilation locale (`scripts\build-windows.ps1`) ne produit pas d'artefacts de mise à jour et n'a pas
besoin de la clé privée.
