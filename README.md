# Classeur URSSAF

Un classeur de bureau (Windows, macOS, Linux) pour **ranger et retrouver vos déclarations mensuelles de chiffre d'affaires** à l'URSSAF.

- **Il ne calcule rien.** Pas de cotisations, pas de seuils : uniquement la saisie de ce que vous avez déclaré, vos pièces jointes et une addition du CA déclaré par année.
- **Tout est chiffré avant de quitter l'ordinateur** (AES-256-GCM, clé protégée par votre phrase secrète via Argon2id). Aucune donnée en clair n'est jamais écrite dans le dépôt Git ni envoyée nulle part.
- **Vos appareils restent synchronisés** grâce à un dépôt Git *privé* de votre choix, mis à jour à chaque lancement et après chaque modification.
- **L'application se met à jour toute seule** (mises à jour signées).

## Ce que contient le classeur

Pour chacun des 12 mois d'une année : le **montant du CA déclaré**, la **date de déclaration**, le **statut** (À déclarer · Déclaré · Payé), des **notes libres** et des **pièces jointes** (PDF ou images : accusé de déclaration, justificatif de paiement…).
Une **recherche** (mois, année, montant, statut, notes, nom d'une pièce jointe — insensible aux accents) et le **total annuel** complètent l'ensemble.

Le total « CA déclaré » additionne les mois **Déclaré** et **Payé**. Un montant saisi sur un mois encore « À déclarer » n'est pas compté, mais il est signalé à part (« + 875,00 € saisi, pas encore déclaré ») pour que rien ne passe inaperçu.

## Comment ça marche

```
   votre poste                                   GitHub (dépôt privé)
 ┌────────────────────────┐   git pull / push   ┌───────────────────────────┐
 │ Classeur URSSAF        │ ──────────────────► │ vault.json   (sel, clé    │
 │  phrase secrète        │   uniquement des    │              enveloppée)  │
 │   └─ Argon2id ─► clé   │   fichiers chiffrés │ records/<opaque>.enc      │
 │  AES-256-GCM           │ ◄────────────────── │ files/<opaque>.enc        │
 │  jeton ► trousseau OS  │                     └───────────────────────────┘
 └────────────────────────┘
```

Détails et limites : [docs/securite.md](docs/securite.md).

## Installation

Téléchargez l'installeur de votre système depuis la page **Releases** du dépôt :

| Système | Fichier | Remarque |
|---|---|---|
| Windows | `…-setup.exe` (ou `.msi`) | SmartScreen peut afficher « Windows a protégé votre ordinateur » : *Informations complémentaires → Exécuter quand même*. L'installeur n'est pas signé avec un certificat commercial. |
| macOS | `….dmg` (Apple Silicon `aarch64`, Intel `x64`) | Au premier lancement : clic droit sur l'application → *Ouvrir*. Si macOS la déclare « endommagée » : `xattr -dr com.apple.quarantine "/Applications/Classeur URSSAF.app"`. L'application n'est pas notarisée. |
| Linux | `.AppImage` ou `.deb` | Un **service de trousseau** (GNOME Keyring, KWallet…) doit être installé et déverrouillé : c'est là que le jeton GitHub est rangé. |

## Première utilisation

1. **Créez le dépôt de données** sur GitHub : un dépôt **privé et vide** (sans README), par exemple `classeur-urssaf-data`.
2. **Créez un jeton d'accès** : *Settings → Developer settings → Personal access tokens → Fine-grained tokens*. Limitez-le à **ce seul dépôt**, permission **Contents : Read and write**. Choisissez la durée de validité qui vous convient (le jeton peut être remplacé plus tard dans *Réglages*).
3. **Lancez l'application**, collez l'adresse du dépôt (`https://github.com/vous/classeur-urssaf-data`) et le jeton. Le jeton est rangé dans le trousseau du système ; il n'est écrit dans aucun fichier.
4. **Choisissez votre phrase secrète** (12 caractères minimum ; plusieurs mots au hasard font une très bonne phrase).

> ⚠️ **La phrase secrète n'est enregistrée nulle part.** Si vous la perdez, vos données sont définitivement irrécupérables : personne ne peut les restaurer. Notez-la dans un endroit sûr (gestionnaire de mots de passe, papier).

## Au quotidien

- À chaque lancement, l'application récupère la dernière version des données (`git pull`) et cherche une mise à jour d'elle-même, puis demande la phrase secrète.
- Chaque enregistrement est validé (`commit`) puis envoyé (`push`) automatiquement. Le badge en haut indique l'état : *Synchronisé*, *Synchronisation…*, *Hors ligne — modifications en attente* (elles partiront dès le retour du réseau), *Accès refusé* (jeton expiré : *Réglages → Remplacer le jeton*).
- Le classeur se **verrouille** à la demande (bouton *Verrouiller*) et automatiquement après une période d'inactivité (10 minutes par défaut, réglable). Verrouiller efface la clé de la mémoire de l'application et supprime les copies temporaires déchiffrées.
- Pièces jointes : *Ouvrir* (copie temporaire dans un dossier privé, supprimée au verrouillage et à la fermeture), *Aperçu* pour les images (en mémoire uniquement), *Enregistrer sous…* pour en garder une copie déchiffrée où vous voulez. PDF et images uniquement, 20 Mo maximum par fichier.

## Plusieurs appareils

Sur un second ordinateur : installez l'application, collez la **même adresse de dépôt** et un jeton (le même ou un autre), puis saisissez la **même phrase secrète**. Si deux appareils modifient le *même mois* avant de se synchroniser, la version modifiée le plus récemment est conservée et l'application vous le signale ; l'autre version reste dans l'historique Git (chiffrée). Une suppression ne l'emporte jamais sur une modification.

Changer la phrase secrète (*Réglages*) ne rechiffre pas les données : seule la clé qui les protège est ré-enveloppée.

## Mises à jour de l'application

L'application interroge `https://github.com/Serwen195/classeur-ursaff/releases/latest/download/latest.json`, vérifie la **signature** de la mise à jour avec la clé publique intégrée, puis propose *Installer et redémarrer*.

> ⚠️ Ce mécanisme ne peut pas lire les releases d'un dépôt GitHub **privé** (le téléchargement n'est pas authentifié). Le dépôt *de l'application* doit donc être **public** — il ne contient aucun secret : vos données sont dans l'autre dépôt, privé. Voir [docs/developpement.md](docs/developpement.md#dépôt-privé-ou-public-).

## Limites à connaître

- Pas de récupération de phrase secrète (c'est voulu : c'est ce qui garantit que personne d'autre ne peut lire vos données).
- Taille, nombre et dates de modification des fichiers sont visibles dans le dépôt Git ; leur contenu, leurs noms, les mois concernés et les noms des pièces jointes ne le sont pas.
- Sur un poste déjà compromis (logiciel malveillant, enregistreur de frappe), aucun chiffrement ne protège pendant que le classeur est déverrouillé.
- Le chiffrement repose sur des bibliothèques éprouvées ([RustCrypto](https://github.com/RustCrypto)) mais **le projet n'a pas fait l'objet d'un audit indépendant**.

## Développement

Voir [docs/developpement.md](docs/developpement.md) : prérequis, tests, structure du code et procédure de publication d'une version.
