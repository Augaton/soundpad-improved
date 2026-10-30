# Contribuer à Bruitage

Merci de vouloir aider. Ce guide dit comment signaler un problème, proposer une idée et soumettre
du code.

## Signaler un bug ou proposer une idée

Ouvre une [issue](https://github.com/Augaton/bruitage/issues/new/choose) avec le modèle qui
correspond. Pour un bug, donne la version de Bruitage (en bas des réglages), ta version de Windows,
le mode d'envoi (filtre micro ou VB-Cable) et les étapes pour le reproduire.

Une faille de sécurité ne va pas dans une issue publique : suis [SECURITY.md](SECURITY.md).

## Préparer l'environnement

Bruitage ne tourne que sous Windows 10 ou 11 en 64 bits. Il faut :

- [Rust](https://rustup.rs), dans sa dernière version stable : la CI s'en sert, et chaque version de
  clippy ajoute des règles ;
- [CMake](https://cmake.org), pour compiler libopus ;
- le g++ de [MSYS2](https://www.msys2.org) (UCRT64), pour le filtre micro.

Tous trois doivent être dans le PATH. Puis :

```
native\build.cmd        # le filtre micro, embarqué par l'exe si native\bin existe
cargo run
```

Pour essayer sans toucher à ta bibliothèque de tous les jours, lance l'appli avec
`SPI_USER_DATA=<un dossier>` : elle utilise ce profil et tourne à côté de l'autre.

## Avant d'envoyer une pull request

La CI refuse une pull request qui ne passe pas ces vérifications :

```
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
native\bin\test_apo.exe native\bin    # si tu as touché au filtre (native\)
```

Et à la main : lance l'appli, et vérifie le changement dans la vraie fenêtre.

## Règles du projet

- **Le code reste simple.** Avant d'ajouter une fonction, cherche si le dépôt ou la bibliothèque
  standard l'offre déjà. Pas de nouvelle dépendance pour quelques lignes, pas d'abstraction pour
  un seul usage.
- **Une logique non triviale a son test** : une branche, une boucle, un calcul.
- **On ne rogne jamais** la validation des données lues sur le disque ou le réseau, ni la gestion
  d'erreur qui évite de perdre la bibliothèque, ni l'accessibilité.
- **L'interface parle français**, en phrases courtes, et tutoie. Les commentaires du code sont en
  anglais.
- **Le filtre micro est délicat** : il tourne dans le service audio de Windows. Tout changement dans
  `native/` passe par `test_apo.exe`, et le script `mic-filter.ps1` est vérifié par empreinte
  SHA-256 au lancement.
- **`library.json` garde son format** : une bibliothèque existante doit toujours se lire.

## Messages de commit

En français, avec un titre court qui dit ce qui change pour l'utilisateur, et un corps en
liste quand il y a plusieurs changements. Par exemple :

```
Raccourci par catégorie, sons introuvables retrouvés

- Un raccourci par catégorie joue un de ses sons au hasard.
- Les sons dont le fichier a été déplacé se retrouvent dans un dossier.
```

## Publication

Chaque push sur `main` dont la CI passe publie une Release, numérotée `<majeur>.<mineur>` de
`Cargo.toml`, puis le numéro d'exécution de la CI. Pour une nouvelle version mineure, change la
version dans `Cargo.toml` et note-la dans le [CHANGELOG](CHANGELOG.md).

## Licence

En contribuant, tu acceptes que ta contribution soit publiée sous la [licence MIT](LICENSE) du
projet.
