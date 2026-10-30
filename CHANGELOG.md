# Changements

Chaque version mineure est publiée en plusieurs Releases, une par push sur `main` : 0.3.8 est la
huitième exécution de la CI pour la 0.3.

## 0.4

- **Nouveau nom : Bruitage.** Au premier lancement, la bibliothèque passe de
  `%APPDATA%\soundpad-improved` à `%APPDATA%\bruitage`, avec les sons téléchargés, et le lancement
  avec Windows suit. Si Soundpad Improved est encore ouvert, Bruitage demande de le quitter d'abord.
- Le filtre micro garde ses noms internes : un filtre déjà installé continue de marcher.
- Licence MIT, README complet, guide de contribution.

## 0.3 — 30 octobre 2026

- Zone de notification : fermer la fenêtre la range, les raccourcis restent actifs. Lancement avec
  Windows.
- Un raccourci par catégorie joue un de ses sons au hasard.
- Sélection de plusieurs sons, et déplacement d'un son en le glissant.
- Sons introuvables retrouvés dans un dossier.
- Opus pris en charge (libopus), dont les messages vocaux de Discord.
- Le silence au début des sons est coupé : ils partent dès la touche.
- Une nouvelle version publiée est signalée dans la fenêtre.
- `library.json.bak` garde la bibliothèque telle qu'au lancement.
- Optimisations : rien n'est dessiné fenêtre cachée ; le survol ne décode que le son où le pointeur
  s'arrête ; un raccourci de catégorie ne décode d'avance que son prochain son ; la fin d'une voix
  ne fait plus attendre la sortie audio.
- CI : mise en forme et clippy vérifiés.

## 0.2 — 29 octobre 2026

- Réécriture en Rust : un seul exe d'environ 13 Mo, fenêtre en une demi-seconde, filtre micro
  embarqué.
- Sons depuis un lien : Voicemod Tuna, MyInstants, ou un fichier audio en ligne.
- Copier-coller rétabli, barre de titre toujours sombre.

## 0.1 — 22 octobre 2026

- Première version, en Electron : soundboard polyphonique, filtre micro Windows, import de Soundpad,
  Release publiée par la CI.
