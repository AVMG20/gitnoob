use std::path::Path;
use std::process::Command;

/// Result of running the `git` CLI.
#[derive(serde::Serialize)]
pub struct CmdOutput {
    pub argv: Vec<String>,
    pub ok: bool,
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Runs `git` inside `cwd`.
///
/// Every mutating operation goes through the CLI rather than libgit2 so that the
/// user's own configuration applies: credential helpers, SSH agent and
/// `~/.ssh/config`, hooks, commit signing, and `merge.conflictStyle`.
pub fn run(cwd: &Path, args: &[&str]) -> Result<CmdOutput, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        // Never let git stop and ask on stdin; we have no terminal to answer on.
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .map_err(|e| format!("Could not run git: {e}"))?;

    Ok(CmdOutput {
        argv: args.iter().map(|s| s.to_string()).collect(),
        ok: output.status.success(),
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
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

    let mut child = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
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
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}
