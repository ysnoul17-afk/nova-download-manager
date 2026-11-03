# Guide d'utilisation de Nova Download Manager

## Ajouter des téléchargements

- Cliquez sur **Ajouter** (ou `Ctrl+N`), puis collez une ou plusieurs adresses. Un texte qui contient des
  adresses mêlées à autre chose fonctionne aussi : NDM en extrait les liens.
- Raccourci : collez une adresse (`Ctrl+V`) n'importe où dans la fenêtre pour ouvrir directement l'ajout.
- Options : dossier de destination, nom du fichier (pour une seule adresse), priorité, nombre de connexions,
  ajout sans démarrer.
- `Ctrl+Entrée` dans la zone d'adresses lance le téléchargement.

Les fichiers sont classés automatiquement par type (Vidéos, Musique, Documents, Images, Archives,
Logiciels, Autres), chacun dans son dossier. Si un fichier du même nom existe déjà, NDM ajoute « (1) ».

## Télécharger une vidéo

Collez l'adresse d'une page vidéo (YouTube, Vimeo, Dailymotion…) ou d'un flux `.m3u8` / `.mpd`, puis
**Télécharger**. NDM analyse la page et propose les qualités réellement disponibles (1080p, 720p…) avec leur
taille estimée, ainsi que l'audio seul en MP3 ou M4A. L'image et le son, souvent séparés, sont assemblés
automatiquement. Les sous-titres français ou anglais peuvent être intégrés à la vidéo.

La première fois, NDM propose d'installer **yt-dlp** et **FFmpeg**, deux logiciels libres téléchargés depuis
leurs dépôts officiels et vérifiés par empreinte. Ils se mettent à jour dans **Paramètres > Outils vidéo** :
si un site cesse de fonctionner, une mise à jour de yt-dlp suffit généralement.

Ne sont pas téléchargeables : les vidéos protégées par DRM (Netflix, Disney+…), les directs, et pour l'instant
les listes de lecture. Respectez les conditions d'utilisation des sites et les droits des auteurs.

## Gérer les téléchargements

| Action | Souris | Clavier |
|---|---|---|
| Sélectionner plusieurs éléments | `Ctrl`+clic, `Maj`+clic | `Ctrl+A` |
| Mettre en pause / reprendre | Barre d'outils | `Espace` |
| Supprimer | Barre d'outils | `Suppr` |
| Ouvrir un fichier terminé | Double-clic | `Entrée` |
| Se déplacer dans la liste | | `↑` `↓` |
| Rechercher | Champ de recherche | `Ctrl+F` |
| Paramètres | Menu de gauche | `Ctrl+,` |

- **Pause** suspend le téléchargement ; **Arrêter** fait de même mais le classe à part. Dans les deux cas,
  la reprise repart de là où elle s'était arrêtée, si le serveur le permet.
- **Relancer** recommence depuis le début.
- **Supprimer** retire l'élément de la liste. Cochez l'option pour effacer aussi le fichier du disque.

## Le panneau de détail

Sélectionnez un seul téléchargement pour afficher son détail. La bande colorée représente le fichier :
chaque portion correspond à une connexion, qui se remplit au fur et à mesure. On y règle aussi la priorité
et une vitesse maximale propre à ce fichier.

## Vitesse et file d'attente

- Le lien **Vitesse illimitée / Limite** en bas de la fenêtre bascule rapidement une limite globale.
  La valeur exacte se règle dans les paramètres.
- Au-delà du nombre de téléchargements simultanés choisi, les suivants attendent leur tour par ordre de
  priorité.

## Fermeture et reprise

Fermer la fenêtre réduit NDM dans la zone de notification ; les téléchargements continuent. Pour quitter,
utilisez **Quitter** dans le menu de l'icône. Les téléchargements interrompus (fermeture, coupure réseau,
redémarrage de Windows) reprennent automatiquement au lancement suivant.

## Historique

Tous les téléchargements sont conservés dans l'historique, avec recherche, filtres par statut, type et
période, et tri par date, taille, vitesse moyenne, type ou nom.

## Messages d'erreur courants

| Message | Que faire |
|---|---|
| Fichier introuvable sur le serveur (404) | Le lien est incorrect ou le fichier a été retiré. |
| Accès refusé (403) / Lien expiré (410) | Récupérez un nouveau lien depuis le site. |
| Adresse du serveur introuvable (erreur DNS) | Vérifiez l'adresse et votre connexion. |
| Espace disque insuffisant | Libérez de la place ou changez de dossier, puis reprenez. |
| Permission refusée | Choisissez un dossier dans lequel vous pouvez écrire. |
