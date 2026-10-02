use std::io;

/// Erreurs du cœur. Les messages sont en français : ils sont affichés tels quels à l'utilisateur.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Phrase secrète incorrecte.")]
    WrongPassphrase,

    #[error("Données illisibles ou altérées : {0}")]
    Corrupt(String),

    #[error("{0}")]
    InvalidInput(String),

    #[error("Aucun coffre n'existe encore dans ce dossier.")]
    NoVault,

    #[error("Un coffre existe déjà dans ce dossier.")]
    VaultExists,

    #[error("Erreur de fichier : {0}")]
    Io(#[from] io::Error),

    #[error("Erreur Git : {0}")]
    Git(String),

    #[error(
        "Authentification refusée par le dépôt distant (jeton invalide, expiré ou sans droit d'écriture)."
    )]
    AuthFailed,

    #[error("Dépôt distant injoignable : {0}")]
    Network(String),

    #[error("Conflit de synchronisation impossible à résoudre automatiquement : {0}")]
    Conflict(String),

    #[error("Refus de valider un fichier non chiffré dans le dépôt : {0}")]
    PlaintextRefused(String),
}

impl From<git2::Error> for Error {
    fn from(e: git2::Error) -> Self {
        use git2::{ErrorClass, ErrorCode};
        if e.code() == ErrorCode::Auth {
            return Error::AuthFailed;
        }
        // Un 401/403 renvoyé par le serveur remonte parfois en classe Http sans code Auth.
        let msg = e.message().to_string();
        if matches!(e.class(), ErrorClass::Http) && (msg.contains("401") || msg.contains("403")) {
            return Error::AuthFailed;
        }
        // Une connexion refusée ou une résolution DNS en échec remontent parfois en classe Os/Git :
        // on reconnaît aussi les messages typiques pour distinguer « hors ligne » d'une vraie erreur.
        let lower = msg.to_lowercase();
        let looks_offline = [
            "failed to connect",
            "connection refused",
            "connection reset",
            "could not resolve",
            "failed to resolve",
            "timed out",
            "timeout",
            "unreachable",
            "no route to host",
            "network",
        ]
        .iter()
        .any(|k| lower.contains(k));
        if looks_offline || matches!(e.class(), ErrorClass::Net | ErrorClass::Http | ErrorClass::Ssl) {
            return Error::Network(msg);
        }
        Error::Git(msg)
    }
}

pub type Result<T> = std::result::Result<T, Error>;
