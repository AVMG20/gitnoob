use std::path::Path;
use std::process::Command;
use std::sync::Mutex;

/// Result of running the `git` CLI.
#[derive(serde::Serialize)]
pub struct CmdOutput {
    pub argv: Vec<String>,
    pub ok: bool,
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// The `GIT_SSH_COMMAND` the active profile asks for, or `None` to leave ssh
/// alone.
///
/// This is process-wide rather than carried through every call site because
/// there is only ever one active profile, and threading it through the hundred
/// or so `run` calls would say nothing a reader does not already know.
static SSH_COMMAND: Mutex<Option<String>> = Mutex::new(None);

/// Points every later git command at a profile's key. Called when a profile is
/// activated or edited, and once at startup.
pub fn set_ssh_command(command: Option<String>) {
    *SSH_COMMAND.lock().unwrap() = command;
}

pub fn ssh_command() -> Option<String> {
    SSH_COMMAND.lock().unwrap().clone()
}

/// A `git` invocation with the environment every call needs.
fn git(cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(cwd)
        // Never let git stop and ask on stdin; we have no terminal to answer on.
        .env("GIT_TERMINAL_PROMPT", "0");
    if let Some(ssh) = ssh_command() {
        command.env("GIT_SSH_COMMAND", ssh);
    }
    command
}

/// Runs `git` inside `cwd`.
///
/// Every mutating operation goes through the CLI rather than libgit2 so that the
/// user's own configuration applies: credential helpers, SSH agent and
/// `~/.ssh/config`, hooks, commit signing, and `merge.conflictStyle`.
pub fn run(cwd: &Path, args: &[&str]) -> Result<CmdOutput, String> {
    let output = git(cwd, args)
        .output()
        .map_err(|e| format!("Could not run git: {e}"))?;

    Ok(CmdOutput {
        argv: args.iter().map(|s| s.to_string()).collect(),
        ok: output.status.success(),
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: explained(&String::from_utf8_lossy(&output.stderr)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_publickey_refusal_names_the_pinned_key() {
        set_ssh_command(Some(
            "ssh -i \"C:/Users/a/.ssh/id_work\" -o IdentitiesOnly=yes".to_string(),
        ));
        let out = explained("git@gitlab.com: Permission denied (publickey).");
        assert!(out.contains("C:/Users/a/.ssh/id_work"));
        set_ssh_command(None);
    }

    #[test]
    fn without_a_pinned_key_it_says_to_set_one() {
        set_ssh_command(None);
        let out = explained("git@github.com: Permission denied (publickey).");
        assert!(out.contains("No key is pinned"));
    }

    #[test]
    fn an_https_remote_with_no_helper_is_told_apart_from_a_key_problem() {
        set_ssh_command(None);
        let out = explained("fatal: could not read Username for 'https://github.com'");
        assert!(out.contains("HTTPS"));
    }

    #[test]
    fn an_ordinary_failure_is_left_alone() {
        let message = "error: pathspec 'nope' did not match any file(s) known to git";
        assert_eq!(explained(message), message);
    }
}

/// Adds a line saying what to do about a transport failure, where git's own
/// message says only that one happened.
///
/// "Permission denied (publickey)" is the classic: true, unhelpful, and on a
/// machine with a work key and a personal key it never says which one was
/// offered. Anything not recognised is passed through untouched.
fn explained(stderr: &str) -> String {
    let Some(hint) = transport_hint(stderr) else {
        return stderr.to_string();
    };
    format!("{}\n\n{hint}", stderr.trim_end())
}

fn transport_hint(stderr: &str) -> Option<String> {
    if stderr.contains("Permission denied (publickey") {
        return Some(match ssh_command() {
            Some(command) => format!(
                "The forge refused the key this profile pins. Offered: {}. Either its public half is not on the account, or this profile should point at the other key.",
                key_in(&command).unwrap_or(command)
            ),
            None => "No key is pinned to this profile, so ssh offered whatever the agent held and the forge accepted none of it. Set an SSH key on the profile in Settings.".to_string(),
        });
    }
    if stderr.contains("Host key verification failed") {
        return Some(
            "The host is not in ~/.ssh/known_hosts yet. Run the connection test on the profile once — it accepts a first-time host key — or ssh to the host by hand."
                .to_string(),
        );
    }
    if stderr.contains("could not read Username") || stderr.contains("terminal prompts disabled") {
        return Some(
            "This remote is HTTPS and no credential helper answered. Either configure one, or switch the remote to ssh so the profile's key is used."
                .to_string(),
        );
    }
    None
}

/// Pulls the path back out of a `GIT_SSH_COMMAND` so a message can name it.
fn key_in(command: &str) -> Option<String> {
    let after = command.split("-i ").nth(1)?;
    Some(after.trim_start_matches('"').split('"').next()?.to_string())
}

/// Like [`run`], but turns a non-zero exit status into an `Err` carrying git's
/// own message. Use this where the caller has nothing useful to say about a
/// failure beyond passing it on.
pub fn run_checked(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let out = run(cwd, args)?;
    if out.ok {
        Ok(out.stdout)
    } else {
        let msg = if out.stderr.trim().is_empty() {
            out.stdout
        } else {
            out.stderr
        };
        Err(msg.trim().to_string())
    }
}

/// Runs `git` with text on stdin, for the commands that take a patch.
pub fn run_with_input(cwd: &Path, args: &[&str], input: &str) -> Result<CmdOutput, String> {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = git(cwd, args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not run git: {e}"))?;

    child
        .stdin
        .as_mut()
        .ok_or_else(|| "git would not take input".to_string())?
        .write_all(input.as_bytes())
        .map_err(|e| format!("Could not write the patch to git: {e}"))?;
    // Close stdin so git stops waiting for more.
    drop(child.stdin.take());

    let output = child
        .wait_with_output()
        .map_err(|e| format!("git did not finish: {e}"))?;

    Ok(CmdOutput {
        argv: args.iter().map(|s| s.to_string()).collect(),
        ok: output.status.success(),
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: explained(&String::from_utf8_lossy(&output.stderr)),
    })
}
