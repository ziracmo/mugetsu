# 月 Mugetsu

Client Git de bureau pour macOS, en Rust, pensé pour la vitesse. Inspiré de Fork (densité, efficacité) et de GitKraken (graphe coloré), avec un thème « cyber dark » animé.

## Fonctionnalités

- **Onglets multi-dépôts** : seul l'onglet actif est rendu, les autres gardent leurs données en mémoire sans coût d'affichage. Session restaurée au lancement.
- **Graphe de commits** rendu GPU (GPUI), lanes néon, liste virtualisée : seules les lignes visibles sont dessinées, même sur 100k+ commits.
- **Sidebar** : branches (ahead/behind, upstream disparu), remotes et leurs branches, worktrees (clic pour l'ouvrir dans un onglet), tags, stashes.
- **Diff** unifié avec coloration syntaxique et changements intra-ligne.
- **Changements** : indexer ou désindexer par fichier ou par hunk, commit, amend.
- **Fetch automatique** toutes les 10 s sur le dépôt actif et toutes les 5 min sur les autres (et sur tous quand l'app est en arrière-plan). Une pastille magenta pulse sur l'onglet quand un remote a bougé.
- **Pull requests GitHub** : un clic sur une branche affiche sa PR (#numéro, couleur selon l'état) ; un clic sur la puce ouvre la PR.
- Pull (fast-forward), push (crée l'upstream si besoin), stash et pop, checkout par double-clic.

## Raccourcis

| Action | Touche |
| --- | --- |
| Ouvrir un dépôt | ⌘O |
| Fermer l'onglet | ⌘W |
| Onglet suivant / précédent | ⌃Tab / ⌃⇧Tab |
| Historique / Changements | ⌘1 / ⌘2 |
| Fetch | ⌘⇧F |
| Rafraîchir | ⌘R |
| Commit précédent / suivant | ↑ ↓ (ou k j) |

## Lancer

```sh
cargo run --release -p mugetsu-app -- ~/code/mon-depot ~/code/autre-depot
scripts/bundle-macos.sh   # produit target/release/Mugetsu.app
```

Prérequis : Rust stable récent, Xcode Command Line Tools (`git`). Pour les PR : `gh auth login` ou une variable `GITHUB_TOKEN`.

## Architecture

```
crates/
  mugetsu-git    moteur Git : historique via gix (in-process, décodage parallèle),
                 tout le reste via le CLI git (config, hooks, credentials, fsmonitor)
                 graph.rs : placement des lanes en O(n log n), mémoire O(n)
  mugetsu-forge  GitHub : URL de remote → owner/repo, PR d'une branche
  mugetsu-app    UI GPUI (gpui-kit) : workspace, onglets, vues
```

Ordres de grandeur mesurés sur git.git (85 593 commits, 1 017 refs, 4 cœurs) : refs en 40 ms, historique complet en 0,66 s avec commit-graph, placement du graphe en 13 ms, segments de 60 lignes visibles en 0,1 ms.

## Tests

```sh
cargo test --workspace
```

- `mugetsu-git` : tests unitaires (parsers, graphe, tri) et tests d'intégration sur de vrais dépôts temporaires.
- `mugetsu-app` : tests d'intégration UI dans des fenêtres headless (clic sur une branche, double-clic pour checkout, navigation au clavier) et parcours de bout en bout (indexer puis commiter depuis l'UI, pastille d'un onglet en arrière-plan après un fetch).
