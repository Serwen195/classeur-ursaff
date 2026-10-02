# Développement et publication

## Architecture

```
crates/core/        classeur-core : tout ce qui est sensible, SANS dépendance à Tauri (testable seul)
  crypto.rs           Argon2id, AES-256-GCM, HKDF, noms de fichiers opaques
  vault.rs            vault.json : clé de données enveloppée par la phrase secrète
  store.rs            mois et pièces jointes chiffrés, validation, liste blanche des chemins
  sync.rs             libgit2 : clone, commit (liste blanche), fetch, fast-forward, fusion, push
  fsutil.rs           écriture atomique
src-tauri/          coque Tauri 2
  app.rs              état partagé, synchronisation en arrière-plan, verrouillage automatique
  commands.rs         commandes IPC appelées par l'interface
  secrets.rs          jeton dans le trousseau du système (keyring)
  config.rs           réglages non secrets ; remote_url.rs : validation de l'adresse du dépôt
  e2e_tests.rs        tests d'intégration des vraies commandes (voir ci-dessous)
  (capabilities/)     permissions de la page : aucun accès aux fichiers
src/                interface Svelte 5 + TypeScript
  lib/api.ts          unique point d'entrée vers le backend (Tauri, ou faux backend en dev)
  lib/contract.json   formes JSON partagées Rust ↔ TypeScript (vérifiées des deux côtés)
  lib/mock.ts         faux backend en mémoire — uniquement en mode développement
  lib/{money,totals,search,months,time}.ts   logique pure, testée
  components/         écrans
e2e/                parcours complets dans un navigateur (Playwright) contre le faux backend
.github/workflows/  ci.yml (tests) et release.yml (installeurs signés)
```

Pourquoi un crate `core` séparé : la partie qui garantit la confidentialité se teste sans fenêtre ni système graphique, et la CI la teste rapidement sur les trois OS.

## Prérequis

- Rust stable, Node 22+.
- Linux : `sudo apt install libwebkit2gtk-4.1-dev build-essential libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev pkg-config`.
- macOS : Xcode Command Line Tools. Windows : [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) et Build Tools C++.

## Commandes

```bash
npm install
npm run tauri dev        # application complète (vrai backend)
npm run dev              # interface seule dans un navigateur, avec le faux backend
                         #   http://127.0.0.1:1420/?scenario=setup|vault|locked|unlocked&offline=1&update=1
                         #   phrase du faux backend : « phrase secrete de test »

npm run check            # svelte-check (types)
npm test                 # tests unitaires de la logique de l'interface (vitest)
npm run build && npm run e2e     # parcours navigateur (PW_CHROMIUM=/chemin/chromium pour utiliser un Chromium existant)
cargo test --workspace   # cœur + coque
cargo clippy --workspace --all-targets -- -D warnings && cargo fmt --all -- --check
npm run tauri build      # installeurs de la plateforme courante
```

## Ce que couvrent les tests

| Niveau | Quoi |
|---|---|
| `crates/core` (54 tests + 2 sur serveur HTTP réel) | Chiffrement (vecteur Argon2id de référence calculé avec une autre implémentation, altération de chaque octet détectée, nonces uniques), coffre (mauvaise phrase, changement de phrase, paramètres hostiles), stockage (validation, noms opaques, rien de lisible sur disque, fichiers échangés détectés), Git (clone d'un dépôt vide, deux appareils, fusion, conflits, suppression contre modification, rejet de push, faux serveur HTTP qui refuse le jeton, panne réseau, **vrai serveur `git http-backend` protégé par jeton** (`tests/http_remote.rs`, lancé par la CI : `cargo test -p classeur-core --test http_remote -- --ignored`), **aucun octet de clair dans les objets du dépôt distant**, refus d'envoyer un fichier non chiffré). |
| `src-tauri` (14 tests d'intégration) | Les vraies commandes IPC, le vrai état partagé et de vraies synchronisations en arrière-plan sur de vrais dépôts Git locaux : cycle complet, deux appareils, modifications concurrentes, verrouillage automatique et effacement des copies temporaires, « Oublier ce poste », hors ligne, changement de phrase, rafales de sauvegardes ; formes JSON renvoyées à l'interface. |
| `src` (vitest, 40 tests) | Montants en centimes (aucun flottant), totaux, recherche, **contrat TypeScript ↔ Rust** : noms des commandes, des arguments (`contract.test.ts`) et des champs JSON (`contract.json`, vérifié aussi côté Rust). Une faute de frappe entre les deux côtés fait échouer la CI au lieu de n'apparaître que dans l'application réelle. |
| `e2e` (Playwright, 22 tests) | Assistant, déverrouillage, édition, pièces jointes, recherche, réglages, mises à jour (y compris l'échec d'installation), hors ligne. |

Le trousseau réel n'est pas exercé par la CI Linux. Test manuel (macOS, Windows, ou Linux avec un Secret Service déverrouillé) :
`cargo test -p classeur-urssaf -- --ignored keychain`. La CI le lance à titre informatif sur Windows et macOS.

## Publier une version

### Une fois pour toutes : clé de signature des mises à jour

La paire de clés est déjà créée : la **clé publique** est dans `src-tauri/tauri.conf.json` (`plugins.updater.pubkey`) ; c'est elle que les applications installées utilisent pour vérifier les mises à jour. La **clé privée** (non protégée par mot de passe) ne doit exister que dans deux endroits : le secret GitHub et votre sauvegarde personnelle.

1. Dans le dépôt GitHub : *Settings → Secrets and variables → Actions → New repository secret* → nom `TAURI_SIGNING_PRIVATE_KEY`, valeur : le **contenu** complet du fichier de clé privée. (Aucun second secret n'est nécessaire, la clé n'ayant pas de mot de passe.)
2. Sauvegardez la clé privée ailleurs (gestionnaire de mots de passe). **Ne la commitez jamais** (`*.key` est dans `.gitignore`). Si vous la perdez, les applications déjà installées ne pourront plus se mettre à jour : il faudra réinstaller manuellement une version portant une nouvelle clé publique.

Pour recréer une paire de clés : `npx tauri signer generate -w ~/.tauri/classeur-urssaf.key`, puis remplacez `pubkey` dans `tauri.conf.json` et le secret GitHub. Si vous ajoutez un mot de passe à la clé, créez aussi le secret `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`.

### À chaque version

1. Changez `version` dans `Cargo.toml` (`[workspace.package]`), commitez.
2. `git tag v0.2.0 && git push origin v0.2.0`.
3. Le workflow *Release* vérifie la clé publique, la cohérence tag/version, passe les tests, compile Windows, macOS (Apple Silicon + Intel) et Linux, signe les mises à jour et prépare une release **brouillon** contenant les installeurs et `latest.json`.
4. Testez un installeur, puis cliquez sur **Publish release**. Les applications installées proposeront la mise à jour au prochain lancement.

### Dépôt privé ou public ?

Le plugin de mise à jour télécharge `latest.json` et l'installeur **sans authentification**. Pour un dépôt GitHub *privé*, ces téléchargements répondent « 404 » : les mises à jour automatiques ne fonctionneraient pas. Options :

1. **Rendre public le dépôt de l'application** (recommandé). Il ne contient aucun secret : la clé de signature est dans les secrets Actions, les données sont dans l'autre dépôt (privé). Le code source n'a rien de confidentiel et un code public est plus facile à faire relire.
2. Garder le code privé et publier les releases dans un **second dépôt public** (par exemple `classeur-urssaf-releases`), en adaptant `release.yml` (`owner`/`repo` de `tauri-action`, avec un jeton ayant accès à ce dépôt) et l'URL dans `plugins.updater.endpoints`.

Ne mettez jamais de jeton dans l'application pour contourner ce point : il serait extractible du binaire.

### Signature des installeurs par les systèmes (optionnelle)

Sans certificat, Windows (SmartScreen) et macOS (Gatekeeper) affichent des avertissements au premier lancement (voir le README). Les supprimer demande un certificat Windows (signature de code) et un compte Apple Developer (signature + notarisation) ; `tauri-action` les gère via des secrets supplémentaires, voir la [documentation Tauri](https://tauri.app/distribute/). La signature des mises à jour décrite plus haut est indépendante et suffit à garantir que seul vous pouvez publier des mises à jour.

## Emplacement des données locales

| Système | Dossier de l'application (`data/` = copie locale du dépôt) |
|---|---|
| Windows | `%LOCALAPPDATA%\io.github.serwen195.classeur-urssaf\` |
| macOS | `~/Library/Application Support/io.github.serwen195.classeur-urssaf/` |
| Linux | `~/.local/share/io.github.serwen195.classeur-urssaf/` |

La configuration (adresse du dépôt, délai de verrouillage — pas de secret) est dans `config.json` du dossier de configuration correspondant.
