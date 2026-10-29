# Soundpad Improved

Soundboard pour Windows, pensée comme un Soundpad amélioré.

- **Plusieurs sons à la fois**, et le même son plusieurs fois par-dessus lui-même.
- **Réglage en direct de chaque son en cours** : volume, vitesse (et hauteur), boucle, pause, position.
- **Raccourcis globaux, même en jeu** : un par son, plus « tout arrêter », « pause / reprise de tout »,
  « monter » et « baisser le volume des sons en cours ». Toutes les touches marchent, Pause comprise.
- **Comportement par son** quand on le relance : le superposer, le redémarrer, ou l'arrêter (bascule).
  L'option « un seul son à la fois » reproduit Soundpad.
- **Volumes égalisés** : les sons trop forts sont baissés, les faibles remontés. Un limiteur évite la
  saturation quand plusieurs sons se superposent.
- **Les sons dans ton vrai micro**, comme Soundpad : un filtre Windows les ajoute au micro, toutes les
  applis les entendent, sans micro virtuel ni réglage dans Discord. Le câble virtuel VB-Cable reste
  possible, avec ta voix mélangée par l'appli. Un vumètre montre ce que les autres entendent.
- **Import de Soundpad en un clic** : sons, catégories et raccourcis. Le fichier de Soundpad est
  seulement lu, jamais modifié.
- Recherche insensible aux accents, tri par titre ou par sons les plus joués, favoris, catégories,
  glisser-déposer de fichiers et de dossiers.

## Pourquoi une appli à part, et pas un mod de Soundpad

Soundpad est un logiciel fermé, et son moteur ne joue qu'un son à la fois. Pour envoyer le son dans
Discord, il installe dans Windows un filtre audio (un APO, `UniteFx`) sur le micro lui-même : le
service audio de Windows y ajoute les sons, et toutes les applis reçoivent ce micro modifié. Cette
appli fait pareil avec son propre filtre (dossier `native/`). Elle sait aussi passer par un câble
audio virtuel gratuit, VB-Cable : elle joue alors dans ton casque et dans le câble, et Discord écoute
le câble comme un micro.

## Installation

Le plus simple : télécharge `Soundpad-Improved-<version>.exe` dans les Releases du dépôt. La CI en
publie une à chaque push sur `main` dont les tests passent. C'est un seul exe, sans installation, qui
démarre aussitôt ; le filtre micro est dedans. Il n'est pas signé, donc Windows SmartScreen peut
afficher « Windows a protégé votre ordinateur » : « Informations complémentaires », puis « Exécuter
quand même ». Il reprend la bibliothèque de la version Electron telle quelle.

Depuis les sources :

1. Installe Rust (https://rustup.rs) et le g++ de MSYS2 (UCRT64), dans le PATH.
2. Compile le filtre micro : `native\build.cmd`. L'appli l'embarque si `native\bin` existe au moment
   de la compiler.
3. `cargo run --release`, ou `cargo build --release` : l'exe est dans `target\release`.

Puis, dans l'appli :

4. Réglages, Envoi aux autres : **« Mon micro, avec le filtre (comme Soundpad) »**, puis
   « Installer le filtre ». Windows demande l'autorisation administrateur, et le son du PC se coupe
   une seconde le temps de redémarrer le service audio. Dans Discord, garde ton micro habituel.
   Coupe la suppression de bruit de Discord (Krisp), sinon elle efface la musique.
5. Ferme Soundpad pendant que tu utilises l'appli : deux applis ne peuvent pas prendre la même touche.
   En cas de conflit, la touche apparaît barrée en rouge sur le son.

Le filtre se pose sur chaque micro branché dont l'emplacement d'effet est libre ou tenu par Soundpad.
Il prend la place de celui de Soundpad, qui n'envoie plus de son tant qu'il est là. Les micros qui ont
déjà un effet du fabricant ne sont pas touchés. « Retirer le filtre » rend à chaque micro l'effet qu'il
avait avant, celui de Soundpad compris. Sans l'appli ouverte, le filtre laisse passer le micro tel quel.

Sans le filtre, avec VB-Cable : installe-le depuis https://vb-audio.com/Cable/ (lance
`VBCABLE_Setup_x64.exe` en administrateur, puis redémarre le PC), choisis « CABLE Input » dans
Envoi aux autres, et dans Discord le périphérique d'entrée **« CABLE Output (VB-Audio Virtual
Cable) »**. Pour réduire ton bruit de fond, coche « Réduction de bruit sur ma voix » dans l'appli.

## Utilisation

| Geste | Effet |
|---|---|
| Clic sur un son | Le joue, selon son comportement (superposer par défaut) |
| Clic droit, ou bouton `•••` | Modifier : titre, volume et vitesse par défaut, comportement, raccourci, catégories, favori |
| Panneau « En cours » | Volume, vitesse, position, boucle, pause et arrêt de chaque instance |
| Bouton « Garder » | Enregistre le volume et la vitesse de l'instance comme réglages du son |
| Ctrl+F | Recherche |
| Glisser des fichiers ou dossiers | Ajout ; dans une catégorie ouverte, ils y sont rangés |

Un raccourci global marche partout, mais sa touche n'arrive plus aux autres applis tant que
Soundpad Improved est ouvert. Préfère le pavé numérique, Pause, F13 à F24, ou une combinaison avec
Ctrl ou Alt.

## Données

La bibliothèque et les réglages sont dans `%APPDATA%\soundpad-improved\library.json`. Les fichiers
audio restent où ils sont : retirer un son de la bibliothèque ne supprime jamais le fichier.

## Formats

mp3, wav, ogg et webm (Vorbis), flac, m4a et aac, aiff. L'Opus n'est pas pris en charge : un tel
fichier affiche « Lecture impossible ».

## Développement

```
cargo test                            # logique, mélangeur, limiteur, anneau du filtre, décodage
native\build.cmd                      # filtre micro (g++ de MSYS2), avant cargo build pour l'embarquer
native\bin\test_apo.exe native\bin    # filtre micro de bout en bout, sans droits administrateur
cargo run
```

- `src/main.rs` : démarrage, une seule instance à la fois.
- `src/state.rs` : l'état de l'appli et ses actions (lecture, sauvegarde, raccourcis, filtre),
  partagés entre la fenêtre et le fil des raccourcis : un raccourci joue même fenêtre réduite.
- `src/ui/` : l'interface (egui). `mod.rs` la fenêtre, `dialogs.rs` l'éditeur de son et les
  réglages, `widgets.rs` les pads et curseurs, `theme.rs` couleurs et polices (celles de Windows),
  `capture.rs` la saisie d'un raccourci, qui lit l'état des touches de Windows.
- `src/audio/decode.rs` : fichier vers PCM stéréo 48 kHz (symphonia, rubato), gain d'égalisation.
- `src/audio/mixer.rs` : les voix, rendues par chaque sortie avec son propre curseur.
- `src/audio/engine.rs` : cache de décodage, sorties (cpal), ta voix et sa réduction de bruit
  (RNNoise), limiteur, et l'envoi au filtre micro au rythme de 48 kHz.
- `src/audio/ring.rs` : l'écriture dans la mémoire partagée du filtre (`native/shared.h`).
- `src/hotkeys.rs` : raccourcis globaux via l'API Windows `RegisterHotKey`, toutes touches comprises.
- `src/filter.rs` : état, installation et retrait du filtre, par `native/mic-filter.ps1` (embarqué avec
  la DLL). PowerShell est pris dans System32 ; il n'exécute le script que s'il a l'empreinte SHA-256
  de celui de l'exe, et la DLL est vérifiée une fois copiée dans Program Files.
- `src/model.rs`, `src/keys.rs`, `src/soundpad.rs`, `src/store.rs` : la bibliothèque, les noms de
  touches, l'import Soundpad, les fichiers.
- `native/apo.cpp` : le filtre, un APO d'effet de point de terminaison (EFX) chargé par `audiodg.exe`.
  Il ajoute au micro le son lu dans une mémoire partagée (`native/shared.h`), avec un
  rééchantillonnage qui suit la dérive entre l'horloge de l'appli et celle du micro.
- `native/mic-filter.ps1` : état, installation et retrait du filtre (registre du micro, sauvegarde de
  l'effet d'origine dans `HKLM\SOFTWARE\Soundpad Improved\Backup`).

Variables de test : `SPI_USER_DATA=<dossier>` isole le profil ; `SPI_MIC_SHM=Local\<nom>` remplace la
mémoire partagée du filtre, pour la tester avec
`native\bin\test_apo.exe native\bin --listen Local\<nom> <secondes>` à la place d'audiodg.
