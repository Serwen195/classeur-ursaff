//! Validation de l'adresse du dépôt de données saisie par l'utilisateur.

/// N'accepte que `https://hôte/chemin/du/dépôt`, sans identifiants intégrés : le jeton d'accès
/// ne doit jamais se retrouver dans l'adresse (donc dans la configuration Git ou les journaux).
pub fn validate_remote_url(input: &str) -> Result<String, String> {
    let url = input.trim();
    let rest = url.strip_prefix("https://").ok_or_else(|| {
        "L'adresse doit commencer par https:// (les adresses SSH ne sont pas prises en charge).".to_string()
    })?;
    if rest.len() > 300 || rest.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err("Adresse invalide.".into());
    }
    if rest.contains(['?', '#', '\\']) {
        return Err("Adresse invalide : retirez les paramètres (?, #).".into());
    }
    let (authority, path) = rest.split_once('/').ok_or_else(|| {
        "Adresse incomplète. Exemple : https://github.com/votre-compte/classeur-urssaf-data".to_string()
    })?;
    if authority.contains('@') {
        return Err(
            "N'indiquez pas d'identifiant ni de jeton dans l'adresse : le jeton se saisit dans son propre champ.".into(),
        );
    }
    let host_ok = !authority.is_empty()
        && authority.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':'))
        && !authority.starts_with(['.', '-', ':']);
    if !host_ok {
        return Err("Nom d'hôte invalide.".into());
    }
    let path = path.trim_end_matches('/');
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() < 2 || segments.iter().any(|s| s.is_empty() || *s == "." || *s == "..") {
        return Err(
            "Adresse incomplète. Exemple : https://github.com/votre-compte/classeur-urssaf-data".into()
        );
    }
    Ok(format!("https://{authority}/{path}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_normal_urls_and_normalises() {
        assert_eq!(
            validate_remote_url("  https://github.com/moi/classeur-urssaf-data  ").unwrap(),
            "https://github.com/moi/classeur-urssaf-data"
        );
        assert_eq!(
            validate_remote_url("https://github.com/moi/data.git/").unwrap(),
            "https://github.com/moi/data.git"
        );
        assert!(validate_remote_url("https://git.example.org:8443/a/b/c").is_ok());
    }

    #[test]
    fn refuses_anything_that_could_leak_or_misdirect_a_token() {
        for bad in [
            "http://github.com/moi/data",
            "git@github.com:moi/data.git",
            "ssh://git@github.com/moi/data",
            "file:///etc/passwd",
            "/home/moi/data",
            "https://moi:ghp_secret@github.com/moi/data",
            "https://ghp_secret@github.com/moi/data",
            "https://github.com/moi",
            "https://github.com/",
            "https://github.com",
            "https://github.com/moi/../data",
            "https://github.com/moi/data?x=1",
            "https://github.com/moi/data#frag",
            "https://github.com/moi/da ta",
            "https://-evil/moi/data",
            "",
        ] {
            assert!(validate_remote_url(bad).is_err(), "{bad:?} aurait dû être refusée");
        }
    }
}
