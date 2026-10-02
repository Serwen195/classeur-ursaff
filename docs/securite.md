# Sécurité : modèle, formats et limites

## Ce qui est protégé, contre qui

| Menace | Protégé ? |
|---|---|
| Quelqu'un lit le dépôt Git (fuite du jeton, erreur de visibilité, GitHub, sauvegarde du disque) | **Oui** : seuls des blobs chiffrés et authentifiés y figurent. |
| Quelqu'un modifie, échange ou rejoue des fichiers du dépôt | **Détecté** : AES-GCM authentifie chaque fichier et l'associe à son emplacement (voir « AAD »). Un fichier altéré n'est pas déchiffré et est signalé dans l'interface. *Un ancien état valide rejoué (retour en arrière) n'est pas détecté.* |
| Vol de l'ordinateur éteint | **Oui** si le disque est chiffré par ailleurs ; sinon seule la copie locale du dépôt (déjà chiffrée) est exposée, jamais la phrase ni la clé. |
| Vol du jeton GitHub | Limité par sa portée (un seul dépôt) ; le jeton donne accès aux fichiers *chiffrés*, pas aux données. Il est dans le trousseau du système, jamais dans un fichier. |
| Logiciel malveillant sur le poste pendant que le classeur est déverrouillé | **Non.** Il peut lire la mémoire ou l'écran. |
| Perte de la phrase secrète | Irrécupérable, par construction. |

## Cryptographie

- **Dérivation** : Argon2id (RFC 9106, version 0x13), 64 Mio de mémoire, 3 passes, 4 voies, sel aléatoire de 128 bits propre à chaque coffre. Paramètres stockés dans `vault.json` ; bornés à la lecture (un dépôt altéré ne peut pas faire allouer des gigaoctets). La phrase est normalisée en Unicode NFC avant dérivation (un « é » saisi sous macOS ou Windows donne la même clé).
- **Clé de données** : 256 bits aléatoires, générée à la création du coffre. Elle est *enveloppée* (chiffrée par AES-256-GCM) avec la clé dérivée de la phrase et stockée dans `vault.json`. Changer la phrase ne réécrit donc que ce fichier.
- **Sous-clés** : HKDF-SHA256 à partir de la clé de données, avec séparation de domaine : une clé de chiffrement, une clé pour nommer les fichiers.
- **Chiffrement** : AES-256-GCM, nonce aléatoire de 96 bits par chiffrement. Format d'un blob : `"CUR1" ‖ nonce (12) ‖ texte chiffré ‖ tag (16)`.
- **AAD** (données authentifiées associées) : `"CUR1"` + un contexte qui lie le blob à son emplacement : `record-file:<nom>` pour un mois, `file:<id>` pour une pièce jointe, `vault-key` pour la clé enveloppée. Copier un fichier sous un autre nom le rend indéchiffrable.
- **Aléa** : `getrandom` (générateur du système).
- **Mémoire** : les clés sont dans des tampons `Zeroizing` effacés à la libération ; verrouiller supprime la session. Limite : la phrase transite par le moteur web (JavaScript) au moment de la saisie ; ces copies-là ne peuvent pas être effacées de façon garantie.

## Disposition du dépôt de données

```
vault.json                     en clair, sans secret : format, version, paramètres Argon2id, sel, clé enveloppée
records/<32 hex>.enc           un fichier par mois ; le nom = HKDF(clé, "record:AAAA-MM") tronqué à 128 bits
files/<32 hex>.enc             une pièce jointe ; le nom est un identifiant aléatoire
README.md, .gitattributes, .gitignore    en clair, génériques
```

Un enregistrement de mois contient, chiffrés : année, mois, montant (en centimes), date de déclaration, statut, notes, et la liste de ses pièces jointes (identifiant, **nom d'origine**, type, taille). Les noms de fichiers du dépôt ne révèlent ni le mois, ni le nom d'une pièce.

**Ce qui reste visible** pour quelqu'un qui lit le dépôt : le nombre et la taille approximative des fichiers (donc, grossièrement, le nombre de mois renseignés et la taille des pièces), l'historique des commits (dates et fréquence), et le fait que c'est un coffre Classeur URSSAF. Les messages de commit sont génériques (« Mise à jour du classeur »).

## Garde-fous « rien en clair »

1. **Liste blanche à l'écriture** : seuls `vault.json`, `README.md`, `.gitattributes`, `.gitignore`, `records/<32 hex>.enc` et `files/<32 hex>.enc` sont jamais indexés. Tout autre fichier présent dans le dossier de données est ignoré (et signalé dans *Réglages*).
2. **Relecture avant envoi** : avant chaque `push`, chaque commit sortant est relu ; un chemin hors liste blanche, ou un fichier `.enc` qui ne commence pas par l'en-tête `CUR1`, **bloque l'envoi**.
3. **Aucune conversion** : `.gitattributes` (`* -text`, `*.enc binary`) et `core.autocrlf=false` empêchent toute conversion de fin de ligne des fichiers chiffrés.
4. **Tests** : la suite d'intégration relit *tous les objets Git* du dépôt distant (blobs, arbres, commits) après des enregistrements contenant des marqueurs reconnaissables, et vérifie qu'aucun n'apparaît.

## Jeton d'accès

- Stocké dans le trousseau du système (Trousseau d'accès, Gestionnaire d'identifiants, Secret Service), relu à chaque synchronisation, jamais écrit dans la configuration, un fichier ou l'adresse du dépôt (une adresse contenant des identifiants est refusée).
- Transmis à libgit2 uniquement au moment de l'authentification HTTPS ; proposé une seule fois par requête (un jeton refusé ne provoque pas de boucle).
- Seules les adresses `https://` sont acceptées.

## Interface

Politique de sécurité du contenu stricte (`default-src 'self'`, pas de script distant, pas d'`eval`) et aucune requête réseau depuis la page. La page web n'a **aucun accès aux fichiers** : ses permissions Tauri se limitent aux événements, à la mise à jour et au redémarrage (voir `src-tauri/capabilities/default.json`, vérifié par un test). Les sélecteurs « Ajouter des fichiers » et « Enregistrer sous… » sont ouverts **côté Rust** : la page ne choisit jamais un chemin d'écriture, donc une éventuelle faille d'injection dans la page ne donnerait pas la possibilité d'écraser des fichiers. Seuls les chemins fournis par un glisser-déposer transitent par la page, et ils sont limités aux PDF et images. « Ouvrir » n'accepte que des PDF et des images, et l'extension du fichier temporaire est déduite du type validé, pas du nom.

## Mises à jour

Chaque mise à jour est signée (minisign, clé Ed25519) ; l'application ne l'installe que si la signature correspond à la clé publique intégrée à l'application. La clé privée de signature vit uniquement dans les secrets GitHub Actions du dépôt de l'application.
