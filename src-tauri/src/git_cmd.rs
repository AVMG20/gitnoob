use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};

/// Result of running the `git` CLI.
#[derive(serde::Serialize, Debug)]
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

/// One `git` invocation, written the way it would be typed.
///
/// The window is a way to learn git, not only a way to avoid it, so every
/// command the app runs on the user's behalf is shown to them.
#[derive(Clone, Debug, serde::Serialize)]
pub struct GitCommand {
    /// The full command line, ready to paste into a terminal.
    pub line: String,
    pub ok: bool,
}

type Reporter = Arc<dyn Fn(GitCommand) + Send + Sync>;

/// Where to send each command as it runs. Set once at startup; a closure rather
/// than an `AppHandle` so the tests can watch it without a window.
static REPORTER: Mutex<Option<Reporter>> = Mutex::new(None);

pub fn report_to(reporter: impl Fn(GitCommand) + Send + Sync + 'static) {
    *REPORTER.lock().unwrap() = Some(Arc::new(reporter));
}

/// Commands that only ask a question. They are how the app fills the window —
/// several per refresh — and listing them would bury the one command the user
/// actually caused.
fn is_query(args: &[&str]) -> bool {
    const READS: &[&str] = &[
        "rev-parse",
        "merge-base",
        "status",
        "log",
        "show",
        "diff",
        "ls-files",
        "cat-file",
        "for-each-ref",
        "symbolic-ref",
    ];
    if args.first().is_some_and(|first| READS.contains(first)) {
        return true;
    }
    // The subcommands that share a verb with something that writes.
    match args {
        ["branch", rest @ ..] => rest
            .iter()
            .any(|arg| arg.starts_with("--format") || *arg == "--list"),
        ["stash", "list", ..] | ["stash", "show", ..] | ["config", "--get", ..] => true,
        _ => false,
    }
}

/// Renders an argument list as a command line, quoting only what needs it.
fn command_line(args: &[&str]) -> String {
    let mut line = String::from("git");
    for arg in args {
        line.push(' ');
        if arg.is_empty() || arg.contains(' ') {
            line.push_str(&format!("'{arg}'"));
        } else {
            line.push_str(arg);
        }
    }
    line
}

/// Keeps Windows from opening a console window for a child process.
///
/// A GUI binary on Windows has no console, so starting a console program gives
/// it one — and every git call is a console program. In a release build that is
/// a black window appearing and vanishing for each command: hundreds of them
/// during a refresh, each one costing a window creation, which is most of why
/// the packaged app felt slow as well as looking broken. A debug build is
/// started from a terminal and inherits that one, which is why this never shows
/// while developing.
///
/// `CREATE_NO_WINDOW` says: run it, give it no console. Nothing else changes —
/// stdout and stderr are captured either way.
pub fn quiet(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

/// A `git` invocation with the environment every call needs.
fn git(cwd: &Path, args: &[&str]) -> Command {
    let mut command = Command::new("git");
    quiet(&mut command);
    command
        .args(args)
        .current_dir(cwd)
        // Never let git stop and ask on stdin; we have no terminal to answer on.
        .env("GIT_TERMINAL_PROMPT", "0")
        // Refusals and transport failures are matched against git's own English
        // wording elsewhere (`refused_over_local_changes`, `transport_hint`) to
        // decide when to auto-stash or how to explain a failure. A localized
        // git would translate the very words those matches look for, and the
        // match would just never fire.
        .env("LC_ALL", "C")
        .env("LANG", "C");
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

    let ok = output.status.success();
    if !is_query(args) {
        // Taken out of the lock before it is called: git runs on several
        // threads, and reporting one command should not hold up the next.
        let reporter = REPORTER.lock().unwrap().clone();
        if let Some(reporter) = reporter {
            reporter(GitCommand {
                line: command_line(args),
                ok,
            });
        }
    }

    Ok(CmdOutput {
        argv: args.iter().map(|s| s.to_string()).collect(),
        ok,
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: explained(&String::from_utf8_lossy(&output.stderr)),
    })
}

/// Runs a command the user typed themselves, as written.
///
/// Not reported: the prompt it was typed at already shows it, and this is the
/// one case where a read like `git log` is wanted in the log. There is no
/// terminal for an editor to open in, so anything that would start one —
/// `commit` without a message, `rebase -i` — takes what it is given instead of
/// hanging with the window waiting on it.
pub fn run_typed(cwd: &Path, args: &[&str]) -> Result<CmdOutput, String> {
    let output = git(cwd, args)
        .env("GIT_EDITOR", "true")
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

    let ok = output.status.success();
    if !is_query(args) {
        // Taken out of the lock before it is called: git runs on several
        // threads, and reporting one command should not hold up the next.
        let reporter = REPORTER.lock().unwrap().clone();
        if let Some(reporter) = reporter {
            reporter(GitCommand {
                line: command_line(args),
                ok,
            });
        }
    }

    Ok(CmdOutput {
        argv: args.iter().map(|s| s.to_string()).collect(),
        ok,
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: explained(&String::from_utf8_lossy(&output.stderr)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Held for as long as a test speaks about the pinned key.
    ///
    /// The pinned key is one global for the whole process and the tests run on
    /// several threads, so without this one test's `None` lands in the middle
    /// of another's `Some` and the failure is a coin toss rather than a bug.
    static PINNED: Mutex<()> = Mutex::new(());

    /// Takes the lock, ignoring a poisoning left behind by a failed test.
    fn pinned() -> std::sync::MutexGuard<'static, ()> {
        PINNED.lock().unwrap_or_else(|held| held.into_inner())
    }

    #[test]
    fn a_publickey_refusal_names_the_pinned_key() {
        let _held = pinned();
        set_ssh_command(Some(
            "ssh -i \"C:/Users/a/.ssh/id_work\" -o IdentitiesOnly=yes".to_string(),
        ));
        let out = explained("git@gitlab.com: Permission denied (publickey).");
        assert!(out.contains("C:/Users/a/.ssh/id_work"));
        set_ssh_command(None);
    }

    #[test]
    fn without_a_pinned_key_it_says_to_set_one() {
        let _held = pinned();
        set_ssh_command(None);
        let out = explained("git@github.com: Permission denied (publickey).");
        assert!(out.contains("No key is pinned"));
    }

    #[test]
    fn an_https_remote_with_no_helper_is_told_apart_from_a_key_problem() {
        let _held = pinned();
        set_ssh_command(None);
        let out = explained("fatal: could not read Username for 'https://github.com'");
        assert!(out.contains("HTTPS"));
    }

    #[test]
    fn a_command_line_is_rendered_the_way_it_would_be_typed() {
        assert_eq!(
            command_line(&["checkout", "main", "--"]),
            "git checkout main --"
        );
        assert_eq!(
            command_line(&["commit", "-m", "a message with spaces"]),
            "git commit -m 'a message with spaces'"
        );
    }

    #[test]
    fn questions_are_not_reported_but_changes_are() {
        assert!(is_query(&["status", "--porcelain"]));
        assert!(is_query(&["diff", "--cached"]));
        assert!(is_query(&["branch", "--format=%(refname)"]));
        assert!(is_query(&["stash", "list"]));
        assert!(is_query(&["stash", "show", "--name-only", "stash@{0}"]));
        assert!(!is_query(&["commit", "-m", "x"]));
        assert!(!is_query(&["branch", "-d", "old"]));
        assert!(!is_query(&["stash", "push"]));
        assert!(!is_query(&["push", "origin", "main"]));
    }

    #[test]
    fn an_ordinary_failure_is_left_alone() {
        let message = "error: pathspec 'nope' did not match any file(s) known to git";
        assert_eq!(explained(message), message);
    }
}
