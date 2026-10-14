//! The name and address a repository commits with, and the profile behind them.
//!
//! A profile is a person, and choosing one is the whole statement, so this runs
//! by itself whenever a repository is opened. What it will not do by itself is
//! write a GitHub profile's address into a GitLab project, or replace an address
//! the repository already commits with: opening a repository under the wrong
//! profile used to rewrite its `user.email` and say so in one line of the log.
//! Those cases are now said, and done only when the user says yes (`force`).

use std::collections::HashSet;
use std::sync::Mutex;

use serde::Serialize;

use crate::config::{Config, ForgeKind, Profile};
use crate::forge;
use crate::git_cmd;
use crate::state::AppState;

/// Writes the profile's identity and signing into the repository's local
/// config. `Ok(None)` means there was nothing to do and nothing to say.
pub fn apply(state: &AppState, force: bool) -> Result<Option<String>, String> {
    let root = state.path()?;
    let config = state.config();
    let Some(profile) = config.profile_for(&root.to_string_lossy()) else {
        return Ok(None);
    };
    let slug = forge::remote_slug(state);
    if !force && !belongs(&config, profile, slug.as_ref()) {
        let host = slug.map(|s| s.host).unwrap_or_default();
        // Said once a session for each repository, not on every open: it is
        // the same news every time, and it buried everything else in the log.
        let told = format!("{}\u{0}{}\u{0}{host}", root.display(), profile.id);
        if !TOLD
            .lock()
            .unwrap()
            .get_or_insert_with(HashSet::new)
            .insert(told)
        {
            return Ok(None);
        }
        return Ok(Some(format!(
            "Left this repository's identity alone: it is on {host}, and the {} profile is for {}",
            profile.name,
            forge::profile_host(profile)
        )));
    }
    let mut said: Vec<String> = Vec::new();

    if let (Some(name), Some(email)) = (
        profile.git_name.clone().filter(|s| !s.trim().is_empty()),
        profile.git_email.clone().filter(|s| !s.trim().is_empty()),
    ) {
        let local_email = configured_locally(&root, "user.email");
        if !force && local_email.as_deref().is_some_and(|here| here != email) {
            said.push(format!(
                "This repository commits as {}; the {} profile would make it {email}. Kept as it is",
                local_email.unwrap_or_default(),
                profile.name
            ));
        } else if configured(&root, "user.name").as_deref() != Some(name.as_str())
            || configured(&root, "user.email").as_deref() != Some(email.as_str())
        {
            git_cmd::run_checked(&root, &["config", "--local", "user.name", &name])?;
            git_cmd::run_checked(&root, &["config", "--local", "user.email", &email])?;
            said.push(format!("Committing here as {name} <{email}>"));
        }
    }

    // Signing is applied separately from the identity, because a profile may
    // well set one and not the other. Each setting the profile has no opinion
    // about is left exactly as the machine had it: a `None` here means "say
    // nothing", which is not the same as "off".
    let mut signing = Vec::new();
    if let Some(key) = profile.signing_key.clone().filter(|s| !s.trim().is_empty()) {
        signing.push(("user.signingkey", key));
    }
    if let Some(format) = profile
        .signing_format
        .clone()
        .filter(|s| !s.trim().is_empty())
    {
        signing.push(("gpg.format", format));
    }
    if let Some(on) = profile.sign_commits {
        signing.push(("commit.gpgsign", on.to_string()));
    }
    if let Some(on) = profile.sign_tags {
        signing.push(("tag.gpgsign", on.to_string()));
    }

    let mut signing_changed = false;
    for (key, value) in signing {
        if configured(&root, key).as_deref() == Some(value.as_str()) {
            continue;
        }
        git_cmd::run_checked(&root, &["config", "--local", key, &value])?;
        signing_changed = true;
    }
    if signing_changed {
        said.push(match profile.sign_commits {
            Some(false) => "Commits here are not signed".to_string(),
            _ => "Signing set up for this repository".to_string(),
        });
    }

    Ok((!said.is_empty()).then(|| said.join(". ")))
}

/// The repositories already told their identity was left alone, this session.
static TOLD: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// Whether a profile's identity goes into a repository.
///
/// It does unless the repository is on a forge another profile is for: a
/// GitHub profile's address does not go into a GitLab project that has a
/// GitLab profile of its own. A host no profile claims — Bitbucket, a Gitea
/// at work — gets the profile's identity and signing, as it always did: a
/// profile is a person, and that person commits there too.
pub fn belongs(config: &Config, profile: &Profile, slug: Option<&forge::RepoSlug>) -> bool {
    if forge::identity_fits(profile, slug) {
        return true;
    }
    let Some(slug) = slug else { return true };
    !config
        .profiles
        .iter()
        .any(|other| other.forge != ForgeKind::None && forge::fit(other, slug) != forge::Fit::No)
}

/// What a repository commits as against what its profile would make it.
#[derive(Serialize, Debug, Default)]
pub struct IdentityCheck {
    pub local_name: Option<String>,
    pub local_email: Option<String>,
    /// The profile the repository answers to.
    pub profile: Option<String>,
    pub name: Option<String>,
    pub email: Option<String>,
    /// Whether that profile belongs on this repository's host at all.
    pub fits: bool,
    /// Whether the repository already commits as somebody else.
    pub differs: bool,
}

/// What `apply` would be asked to change, so the window can ask first.
pub fn check(state: &AppState) -> Result<IdentityCheck, String> {
    let root = state.path()?;
    let config = state.config();
    let profile = config.profile_for(&root.to_string_lossy());
    let slug = forge::remote_slug(state);
    let local_email = configured_locally(&root, "user.email");
    let email = profile.and_then(|p| p.git_email.clone().filter(|e| !e.trim().is_empty()));
    let differs = matches!((&local_email, &email), (Some(here), Some(want)) if here != want);
    Ok(IdentityCheck {
        local_name: configured_locally(&root, "user.name"),
        local_email,
        profile: profile.map(|p| p.name.clone()),
        name: profile.and_then(|p| p.git_name.clone()),
        email,
        fits: profile
            .map(|p| belongs(&config, p, slug.as_ref()))
            .unwrap_or(true),
        differs,
    })
}

/// What this repository's own config says, ignoring what it inherits.
pub fn configured_locally(root: &std::path::Path, key: &str) -> Option<String> {
    let out = git_cmd::run(root, &["config", "--local", "--get", key]).ok()?;
    out.ok
        .then(|| out.stdout.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// What git would use for a setting in this repository, local or inherited.
pub fn configured(root: &std::path::Path, key: &str) -> Option<String> {
    let out = git_cmd::run(root, &["config", "--get", key]).ok()?;
    out.ok
        .then(|| out.stdout.trim().to_string())
        .filter(|value| !value.is_empty())
}
