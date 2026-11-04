# Audit de sécurité — Nova Download Manager 0.4.1

Date : 4 novembre 2026. Périmètre : moteur de téléchargement, application Tauri, hôte Native Messaging,
canal local, extension Chrome/Edge, intégration yt-dlp/FFmpeg, mises à jour, chaîne de publication GitHub.

**Méthode.** Relecture du code du point de vue d'un attaquant, selon quatre profils : un site web malveillant
(page visitée ou serveur de fichiers), un autre utilisateur de la même machine, une compromission de la chaîne
de publication, et une personne qui accède au dépôt public. Chaque défaut retenu a été **reproduit par un test**
avant correction ; chaque correction est couverte par un test automatisé ou vérifiée dans l'application.

**Limite.** Cet audit a été réalisé par l'outil qui a écrit le code. Il ne remplace pas une revue par un
expert indépendant, à prévoir si l'application est diffusée largement.

## Synthèse

| # | Gravité | Constat | État |
|---|---|---|---|
| 1 | Élevée | Cookies d'un site envoyés au site vers lequel il redirige | Corrigé (0.4.2) |
| 2 | Élevée | Fichiers téléchargés sans « marque de provenance Internet » | Corrigé (0.4.2) |
| 3 | Élevée | Scripts (.js, .vbs, .hta, .scr, .lnk…) ouverts sans avertissement | Corrigé (0.4.2) |
| 4 | Moyenne | Cookies conservés en clair dans la base après la fin du téléchargement | Corrigé (0.4.2) |
| 5 | Moyenne | Actions GitHub non figées alors qu'elles reçoivent la clé de signature | Corrigé (0.4.2) |
| 6 | Moyenne | Noms de fichiers trompeurs par caractères Unicode d'inversion (RLO) | Corrigé (0.4.2) |
| 7 | Faible | Redirection HTTPS → HTTP acceptée | Corrigé (0.4.2) |
| 8 | Faible | Extensions (plugins) yt-dlp chargées depuis le profil utilisateur | Corrigé (0.4.2) |
| 9 | Faible | Outils système (reg, taskkill) appelés sans chemin complet | Corrigé (0.4.2) |
| 10 | Faible | Adresse de mise à jour modifiable par variable d'environnement | Corrigé (0.4.2) |
| 11 | Faible | Canal nommé avec la protection Windows par défaut | Accepté (voir détail) |
| 12 | Faible | Interface de l'extension accessible au JavaScript de la page | Accepté (voir détail) |
| 13 | Info | Adresse e-mail personnelle visible dans l'historique Git public | Corrigé (historique réécrit) |

## Constats corrigés

**1. Fuite de cookies après redirection (élevée).** Quand l'extension transmet les cookies d'un site A et que
le fichier est servi après redirection par un site B (CDN, hébergeur), la bibliothèque réseau retirait bien les
cookies lors de la redirection, mais le moteur adressait ensuite ses connexions directement à B **avec tous les
en-têtes d'origine**. B recevait alors la session de l'utilisateur sur A.
*Correction :* cookies et en-têtes d'authentification ne sont envoyés qu'à la même origine (schéma, hôte, port)
que l'adresse d'origine. *Test :* `cookies_do_not_follow_redirects_to_another_site` (site A qui redirige vers un
site B qui enregistre les cookies reçus).

**2. Absence de « Mark of the Web » (élevée).** Windows reconnaît un fichier venu d'Internet grâce au flux NTFS
`Zone.Identifier`, que les navigateurs ajoutent. Sans lui, un programme téléchargé par NDM n'était pas vérifié par
SmartScreen à l'ouverture, et un document Office ne s'ouvrait pas en mode protégé : ses **macros n'étaient pas
bloquées**.
*Correction :* chaque fichier terminé (fichiers et vidéos) reçoit `ZoneId=3` (Internet), avec l'adresse
d'origine et le référent, sans identifiants éventuellement présents dans l'adresse. *Test :* contenu vérifié par
`zone_identifier_content` ; écriture du flux propre à Windows, à confirmer sur la première version publiée
(vérifiée sous Windows, voir plus bas).

**3. Ouverture de scripts sans avertissement (élevée).** La confirmation ne portait que sur la catégorie
« Logiciels ». Un `.js`, `.vbs`, `.hta`, `.scr` ou `.lnk` (classé « Autres ») s'exécutait au double-clic.
*Correction :* liste de 40 types qui exécutent du code ou en contiennent (dont les images disque `.iso`/`.vhd`,
qui servaient à contourner la marque Internet). La vérification est faite **par l'application** : l'interface ne
peut pas ouvrir un tel fichier sans confirmation explicite, qui affiche la provenance. *Tests :* `executables`, et
vérifié dans l'application avec un `.js`.

**4. Cookies conservés au repos (moyenne).** Les en-têtes transmis par l'extension, cookies compris, restaient
dans `ndm.sqlite3` indéfiniment.
*Correction :* effacés dès la fin du téléchargement, et purgés au démarrage pour les données des versions
précédentes. Ils ne sont gardés que le temps de permettre la reprise d'un téléchargement interrompu.
*Test :* même test que le n° 1 (lecture directe de la base).

**5. Chaîne de publication (moyenne).** Les actions GitHub utilisées (dont celle qui reçoit la clé de signature des
mises à jour) étaient désignées par une étiquette modifiable. Une action compromise aurait pu voler la clé et
signer une mise à jour piégée, que tous les NDM installés auraient acceptée.
*Correction :* actions figées par empreinte de commit, droits du jeton GitHub réduits au minimum par tâche
(lecture seule pour la vérification de version), dépendances Rust verrouillées (`--locked`), identifiants Git non
conservés après extraction.

**6. Noms trompeurs (moyenne).** Un serveur pouvait nommer un fichier `facture[RLO]fdp.exe`, affiché
« factureexe.pdf ».
*Correction :* les caractères invisibles et de contrôle de direction sont supprimés des noms. *Test :*
`sanitize_windows`.

**7 à 10. Durcissements (faibles).**
- Redirection de HTTPS vers HTTP refusée (le fichier transiterait en clair). *Test :* `https_to_http_is_a_downgrade`.
- yt-dlp lancé sans configuration ni extensions du profil (`--ignore-config`, `--no-plugin-dirs` pour les versions
  qui le permettent). *Test :* `plugin_isolation_only_for_recent_ytdlp`.
- `reg.exe` et `taskkill.exe` appelés par leur chemin complet dans `System32`.
- L'adresse de mise à jour de test (`NDM_UPDATE_ENDPOINT`) n'existe plus dans les versions publiées.

## Risques acceptés

**11. Canal nommé.** Le canal entre l'hôte natif et l'application utilise la protection par défaut de Windows :
les autres comptes de la machine peuvent le lire mais pas y écrire, et les accès réseau sont refusés. Un autre
compte local pourrait créer ce canal avant NDM et recevoir les adresses (et cookies, si activés) envoyées par
l'extension. Cela suppose un second compte Windows malveillant sur le même ordinateur. Sur un PC personnel à
compte unique, le risque est négligeable. Amélioration possible : liste d'accès explicite limitée au compte
courant et vérification de l'identité du serveur par le client.

**12. Interface de l'extension.** Le bouton vidéo est dans un shadow DOM « ouvert », que le JavaScript de la page
peut lire ou masquer. La page ne peut pas déclencher de téléchargement (clics simulés refusés) et contrôle de
toute façon la vidéo qu'elle affiche. Un mode « fermé » est possible mais compliquerait les tests de bout en bout.

## Points vérifiés sans défaut

- **Aucun port réseau ouvert.** Communication extension → application par Native Messaging puis canal local avec
  jeton de 256 bits (comparaison en temps constant), refus des clients distants.
- **Validation des requêtes** de l'extension : types autorisés, adresses `http(s)` seulement (`file:`,
  `javascript:` refusés), pas de retour à la ligne dans les champs (injection d'en-têtes), taille bornée.
  L'extension ne peut pas choisir le dossier de destination.
- **Traversée de répertoires** : noms nettoyés (séparateurs, noms réservés Windows, points finaux), dossiers
  absolus sans `..`, extraction de FFmpeg sous des noms fixés par NDM (archive piégée testée).
- **SQL** : requêtes paramétrées, tri construit à partir d'une liste fermée (injection testée).
- **Interface** : aucun HTML injecté (React échappe tout), politique de sécurité du contenu stricte, permissions
  Tauri minimales, l'interface ouvre un fichier par identifiant et jamais par chemin libre.
- **yt-dlp** : arguments passés sans shell, adresse après `--` (pas d'injection d'option), `%` échappé dans le
  modèle de nom.
- **Outils** : empreinte SHA-256 vérifiée avant installation.
- **Mises à jour** : signature minisign vérifiée avec la clé publique intégrée (mise à jour signée par une autre
  clé refusée, testé), HTTPS obligatoire.
- **TLS** : certificats vérifiés (aucune option de contournement).
- **Dépôt public** : aucun secret versionné ; clé privée et mot de passe uniquement dans les secrets GitHub ;
  une copie (fork) ou une proposition de modification ne peut pas déclencher de publication.

**13. Adresse e-mail dans l'historique (info).** Les enregistrements publiés portaient l'adresse personnelle
de l'auteur. *Correction :* historique réécrit avec l'adresse anonyme fournie par GitHub
(`…@users.noreply.github.com`), anciennes versions et étiquettes supprimées, option GitHub « Block command line
pushes that expose my email » activée.

## Recommandations pour l'utilisateur

1. **Activer la double authentification GitHub** : la prise de contrôle du compte permettrait de publier une
   mise à jour piégée (si la clé de signature était aussi obtenue).
2. **Conserver la clé de signature** dans un gestionnaire de mots de passe, et supprimer le fichier texte reçu.
3. **Contrôle intelligent des applications** désactivé : c'est la principale baisse de protection du PC. Le
   réactiver suppose de signer NDM avec un certificat reconnu (payant).
4. **Laisser activée** la confirmation avant d'ouvrir un programme téléchargé (réglage par défaut).
5. **N'activer la transmission des cookies** que si nécessaire.
6. **Suspendre l'application Claude sur GitHub** quand le projet n'est pas en cours de modification.

## Vérifié sur Windows (0.4.2)

- Propriétés d'un `.exe` téléchargé par NDM → onglet *Général* : mention « Ce fichier provient d'un autre
  ordinateur… » présente (marque Internet effective).
- Double-clic sur ce `.exe` dans NDM : demande de confirmation affichée, avec la provenance réelle du fichier.
