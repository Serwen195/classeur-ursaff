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
        if looks_offline(&msg) || matches!(e.class(), ErrorClass::Net | ErrorClass::Http | ErrorClass::Ssl) {
            return Error::Network(msg);
        }
        Error::Git(msg)
    }
}

/// Reconnaît les messages d'une panne réseau (hors ligne, DNS, connexion refusée ou expirée).
/// Ils varient selon le système : libgit2 relaie le texte de l'API réseau native (WinHTTP sous
/// Windows, résolveur et sockets POSIX ailleurs), d'où une liste plutôt qu'un code d'erreur.
pub fn looks_offline(message: &str) -> bool {
    const MARKERS: &[&str] = &[
        // POSIX (Linux, macOS)
        "failed to connect",
        "connection refused",
        "connection reset",
        "connection was aborted",
        "could not resolve",
        "failed to resolve",
        "nodename nor servname",
        "name or service not known",
        "temporary failure in name resolution",
        "timed out",
        "timeout",
        "unreachable",
        "no route to host",
        "network is down",
        "network",
        // WinHTTP (Windows)
        "failed to send request",
        "could not be established",
        "could not be resolved",
        "server name or address",
        "connection with the server",
    ];
    let lower = message.to_lowercase();
    MARKERS.iter().any(|m| lower.contains(m))
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;
    use git2::{ErrorClass, ErrorCode};

    #[test]
    fn offline_messages_from_every_platform_are_recognised() {
        for msg in [
            // Linux
            "failed to connect to 127.0.0.1: Connection refused",
            "failed to resolve address for github.com: Temporary failure in name resolution",
            "failed to connect to github.com: Network is unreachable",
            // macOS
            "failed to resolve address for github.com: nodename nor servname provided, or not known",
            "failed to connect to github.com: Operation timed out",
            // Windows (WinHTTP)
            "failed to send request: A connection with the server could not be established\r\n",
            "failed to send request: The server name or address could not be resolved\r\n",
            "failed to send request: The operation timed out\r\n",
            "failed to send request: The connection with the server was terminated abnormally\r\n",
        ] {
            assert!(looks_offline(msg), "non reconnu : {msg}");
            let e: Error = git2::Error::new(ErrorCode::GenericError, ErrorClass::Os, msg).into();
            assert!(matches!(e, Error::Network(_)), "{msg} -> {e:?}");
        }
    }

    #[test]
    fn ordinary_git_errors_are_not_mistaken_for_being_offline() {
        for msg in [
            "reference 'refs/heads/main' not found",
            "object not found - no match for id",
            "the repository is corrupt",
            "invalid path 'records/x.enc'",
        ] {
            assert!(!looks_offline(msg), "à tort réseau : {msg}");
            let e: Error = git2::Error::new(ErrorCode::GenericError, ErrorClass::Reference, msg).into();
            assert!(matches!(e, Error::Git(_)), "{msg} -> {e:?}");
        }
    }

    #[test]
    fn auth_errors_stay_auth_errors() {
        let e: Error = git2::Error::new(ErrorCode::Auth, ErrorClass::Http, "authentication required").into();
        assert!(matches!(e, Error::AuthFailed));
    }
}
