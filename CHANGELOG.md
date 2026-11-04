# Nouveautés de Nova Download Manager

Chaque section `## X.Y.Z` est affichée dans la fenêtre de mise à jour de cette version.

## 0.4.2

Correctifs de sécurité (voir docs/AUDIT-SECURITE.md) :
- Les cookies d'un site ne sont plus transmis au site vers lequel il redirige le téléchargement.
- Les fichiers téléchargés portent la marque « provenance Internet » de Windows : SmartScreen et le mode protégé d'Office s'appliquent comme pour un téléchargement du navigateur.
- Les scripts et programmes (.exe, .js, .vbs, .hta, .lnk, .iso…) demandent confirmation avant d'être ouverts, quelle que soit leur catégorie.
- Les cookies ne sont plus conservés une fois le téléchargement terminé.
- Les noms de fichiers ne peuvent plus être déguisés par des caractères Unicode invisibles.
- Les redirections de HTTPS vers HTTP sont refusées.

## 0.4.1

- Une notification « Repris par Nova Download Manager » signale quand l'extension prend en charge un téléchargement du navigateur.
- Un fichier déjà téléchargé n'est plus dupliqué : NDM propose de l'ouvrir, de l'afficher dans son dossier ou de le télécharger à nouveau.

## 0.4.0

- Notifications Windows à la fin ou en cas d'échec d'un téléchargement.
- Proposition des liens de fichiers copiés (réglage facultatif).
- Lancement au démarrage de Windows.
- Mises à jour automatiques signées.
