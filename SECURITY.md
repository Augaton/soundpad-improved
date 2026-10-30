# Sécurité

Bruitage installe, avec ton autorisation administrateur, un filtre audio que le service audio de
Windows charge dans ton micro. Une faille dans l'appli ou dans ce filtre peut donc toucher tout le
système : merci de la signaler en privé.

## Signaler une faille

Ne l'écris pas dans une issue publique. Utilise le signalement privé de GitHub : onglet
**Security** du dépôt, puis **Report a vulnerability**. Décris le problème, comment le reproduire,
et la version concernée.

Tu recevras une réponse dès que possible. Le correctif est publié dans une nouvelle Release, puis
la faille est décrite publiquement, avec ton nom si tu le souhaites.

## Versions suivies

Seule la dernière Release reçoit des correctifs : l'appli signale elle-même qu'une version plus
récente existe.

## Ce que le projet fait déjà

- Le script du filtre n'est exécuté par PowerShell (pris dans System32 par son chemin complet) que
  s'il a l'empreinte SHA-256 de celui embarqué dans l'exe.
- La DLL du filtre est vérifiée une fois copiée dans Program Files, où seuls les administrateurs
  écrivent, avant que le service audio la charge.
- Les sons en ligne sont téléchargés en HTTPS seulement, avec une taille plafonnée, et gardés
  seulement s'ils se décodent.
- Une bibliothèque lue sur le disque est traitée comme non fiable : champs validés, chemins de
  périphérique refusés, décodage borné en durée et protégé des plantages.
