//! Per-profile SSH keys.
//!
//! A machine with a work account and a personal account has two keys, and ssh
//! left to itself offers whichever the agent hands over first — which is how
//! you end up pushing to a work GitLab as your personal self. Naming a key on
//! the profile pins it: every git command run while that profile is active
//! carries a `GIT_SSH_COMMAND` that uses that key and refuses to fall back to
//! any other.

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::Config;
use crate::git_cmd;

/// A key pair found in `~/.ssh`, for the picker in settings.
#[derive(serde::Serialize)]
pub struct SshKey {
    /// Full path to the private key.
    pub path: String,
    /// Bare file name, which is what the user recognises.
    pub name: String,
    /// `ssh-ed25519`, `ssh-rsa` and so on, read from the public half.
    pub kind: String,
    /// The trailing comment of the public key, usually an email address. This
    /// is normally the only thing distinguishing work from personal.
    pub comment: String,
}

/// `~/.ssh`, wherever home is on this platform.
pub fn ssh_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)?;
    Some(home.join(".ssh"))
}

/// Lists the key pairs in `~/.ssh`.
///
/// A private key is recognised by having a public half beside it, which is both
/// simpler and more reliable than sniffing file contents, and it skips
/// `config`, `known_hosts` and the rest without needing a list of names.
pub fn list_keys() -> Vec<SshKey> {
    let Some(dir) = ssh_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut keys: Vec<SshKey> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("pub") {
            continue;
        }
        let public = path.with_extension("pub");
        if !public.is_file() {
            continue;
        }

        let (kind, comment) = read_public(&public);
        keys.push(SshKey {
            name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: path.to_string_lossy().into_owned(),
            kind,
            comment,
        });
    }

    keys.sort_by(|a, b| a.name.cmp(&b.name));
    keys
}

/// Splits a `.pub` file into its type and its comment. A public key is one
/// line: type, base64 body, then whatever comment ssh-keygen was given.
fn read_public(path: &Path) -> (String, String) {
    let Ok(text) = std::fs::read_to_string(path) else {
        return (String::new(), String::new());
    };
    let line = text.lines().next().unwrap_or_default();
    let mut parts = line.splitn(3, ' ');
    let kind = parts.next().unwrap_or_default().to_string();
    let _body = parts.next();
    let comment = parts.next().unwrap_or_default().trim().to_string();
    (kind, comment)
}

/// The `GIT_SSH_COMMAND` for a key, or `None` when the profile names no key.
///
/// `IdentitiesOnly=yes` is the point of the exercise: without it ssh still
/// offers every key the agent holds and the server takes the first that works,
/// so pinning the key would change nothing.
pub fn command_for(key: Option<&str>) -> Option<String> {
    let key = key.map(str::trim).filter(|k| !k.is_empty())?;
    // git splits this string itself and hands the pieces to a shell, where a
    // backslash inside quotes is an escape rather than a path separator. ssh
    // accepts forward slashes on Windows, so use those and sidestep it. The
    // rest a shell would read as its own — a quote ending the argument, an
    // expansion — is escaped: a key path names a file and nothing else, and one
    // is settable by hand in the config file.
    let mut path = String::with_capacity(key.len());
    for c in key.chars() {
        match c {
            '\\' => path.push('/'),
            '"' | '$' | '`' => {
                path.push('\\');
                path.push(c);
            }
            _ => path.push(c),
        }
    }
    Some(format!("ssh -i \"{path}\" -o IdentitiesOnly=yes"))
}

/// Applies the active profile's key to every later git command.
pub fn apply(config: &Config) {
    let key = config.active().and_then(|p| p.ssh_key.clone());
    git_cmd::set_ssh_command(command_for(key.as_deref()));
}

/// What `ssh -T` had to say.
#[derive(serde::Serialize)]
pub struct SshTest {
    pub ok: bool,
    /// The forge's own greeting, or the failure, in one line.
    pub message: String,
    /// The account the forge recognised, when it named one.
    pub user: Option<String>,
}

/// Tries an authenticated connection to a forge over ssh.
///
/// Neither GitHub nor GitLab gives a shell, so a working key still exits
/// non-zero on GitHub. The greeting is the real signal, not the exit status.
pub fn test(host: &str, key: Option<&str>) -> Result<SshTest, String> {
    let host = host.trim();
    if host.is_empty() {
        return Err("This profile has no host to connect to".to_string());
    }

    let mut command = Command::new("ssh");
    command
        .arg("-T")
        // Never sit waiting for a passphrase or a host-key question: there is
        // no terminal behind this window to answer on.
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("StrictHostKeyChecking=accept-new")
        .arg("-o")
        .arg("ConnectTimeout=10");
    if let Some(key) = key.map(str::trim).filter(|k| !k.is_empty()) {
        command.arg("-o").arg("IdentitiesOnly=yes");
        command.arg("-i").arg(key);
    }
    command.arg(format!("git@{host}"));

    let output = command
        .output()
        .map_err(|e| format!("Could not run ssh: {e}"))?;

    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(interpret(&text))
}

/// Reads a forge's ssh greeting.
fn interpret(text: &str) -> SshTest {
    let line = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty() && !l.starts_with("Warning: Permanently added"))
        .unwrap_or("")
        .to_string();

    // GitHub: "Hi octocat! You've successfully authenticated, but GitHub does
    // not provide shell access."
    // GitLab: "Welcome to GitLab, @octocat!"
    let ok = line.contains("successfully authenticated") || line.contains("Welcome to GitLab");
    let user = if ok { extract_user(&line) } else { None };

    let message = if !ok && line.contains("Permission denied") {
        format!("{line} — the forge did not accept this key. Add its public half to your account, or pick a different key.")
    } else if line.is_empty() {
        "ssh said nothing at all, which usually means it could not reach the host".to_string()
    } else {
        line
    };

    SshTest { ok, message, user }
}

/// Pulls the account name out of a greeting, so the profile can show whose key
/// this actually is.
fn extract_user(line: &str) -> Option<String> {
    if let Some(rest) = line.strip_prefix("Hi ") {
        let name = rest.split(['!', ' ']).next()?.trim();
        return (!name.is_empty()).then(|| name.to_string());
    }
    if let Some(at) = line.find('@') {
        let name: String = line[at + 1..]
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
            .collect();
        return (!name.is_empty()).then_some(name);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_the_key_and_forbids_the_agents_others() {
        let command = command_for(Some("/home/a/.ssh/id_ed25519")).unwrap();
        assert_eq!(
            command,
            "ssh -i \"/home/a/.ssh/id_ed25519\" -o IdentitiesOnly=yes"
        );
    }

    #[test]
    fn turns_windows_separators_into_the_ones_ssh_reads() {
        let command = command_for(Some(r"C:\Users\a\.ssh\id_ed25519")).unwrap();
        assert!(command.contains("C:/Users/a/.ssh/id_ed25519"));
        assert!(!command.contains('\\'));
    }

    #[test]
    fn a_quote_in_the_path_cannot_open_a_second_option() {
        let command = command_for(Some(r#"/home/a/id" -o ProxyCommand=whatever "x"#)).unwrap();
        assert_eq!(
            command,
            r#"ssh -i "/home/a/id\" -o ProxyCommand=whatever \"x" -o IdentitiesOnly=yes"#
        );
    }

    #[test]
    fn an_expansion_in_the_path_stays_part_of_the_path() {
        let command = command_for(Some("/home/a/$(id)/`id`")).unwrap();
        assert_eq!(
            command,
            r#"ssh -i "/home/a/\$(id)/\`id\`" -o IdentitiesOnly=yes"#
        );
    }

    #[test]
    fn no_key_leaves_ssh_alone() {
        assert!(command_for(None).is_none());
        assert!(command_for(Some("   ")).is_none());
    }

    #[test]
    fn reads_a_github_greeting() {
        let result = interpret(
            "Hi octocat! You've successfully authenticated, but GitHub does not provide shell access.\n",
        );
        assert!(result.ok);
        assert_eq!(result.user.as_deref(), Some("octocat"));
    }

    #[test]
    fn reads_a_gitlab_greeting() {
        let result = interpret("Welcome to GitLab, @octocat!\n");
        assert!(result.ok);
        assert_eq!(result.user.as_deref(), Some("octocat"));
    }

    #[test]
    fn a_rejected_key_says_what_to_do_about_it() {
        let result = interpret(
            "Warning: Permanently added 'github.com' to the list of known hosts.\ngit@github.com: Permission denied (publickey).\n",
        );
        assert!(!result.ok);
        assert!(result.message.contains("Permission denied"));
        assert!(result.message.contains("Add its public half"));
    }

    #[test]
    fn silence_is_reported_as_unreachable() {
        let result = interpret("");
        assert!(!result.ok);
        assert!(result.message.contains("could not reach"));
    }
}
