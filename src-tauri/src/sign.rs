//! Whether a commit was signed, and by whom.
//!
//! Git already knows how to check a signature — through gpg, ssh-keygen or
//! whatever `gpg.format` names — and re-implementing that against three
//! toolchains would be re-implementing the one part that has to be right. So
//! every answer here is git's own, read out of the `%G` placeholders.
//!
//! Verification costs a subprocess per commit, which is why the graph asks for
//! a whole page in one command and only when the setting says to. The details
//! panel asks about one commit, which is cheap enough to do every time.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;

use crate::git_cmd;
use crate::state::AppState;

/// What git made of a commit's signature, from the `%G?` placeholder.
#[derive(Serialize, Debug, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// A good signature by a key the machine trusts.
    Good,
    /// The signature checks out, but nothing here vouches for the key: it is
    /// not in `allowed_signers`, or gpg does not trust it, or it has expired
    /// or been revoked. Real, but it names nobody you have agreed to believe.
    Untrusted,
    /// A good key, and the commit is not what it signed.
    Bad,
    /// There is a signature and git could not check it — usually the key is
    /// simply not on this machine.
    Unchecked,
    /// No signature at all, which is most commits in most repositories.
    None,
}

impl Verdict {
    /// git's own one-letter codes, from `git log --format=%G?`.
    fn read(code: &str) -> Verdict {
        match code {
            "G" => Verdict::Good,
            "B" => Verdict::Bad,
            // Good signatures with something wrong about the key: untrusted,
            // expired, made by an expired key, made by a revoked key. All four
            // mean the same thing to a reader — it is signed, but it does not
            // establish who by.
            "U" | "X" | "Y" | "R" => Verdict::Untrusted,
            "E" => Verdict::Unchecked,
            _ => Verdict::None,
        }
    }
}

/// One commit's signature, as much as git will say about it.
#[derive(Serialize, Debug, PartialEq, Eq, Default)]
pub struct Signature {
    pub verdict: Option<Verdict>,
    /// Who the key belongs to, in git's words.
    pub signer: Option<String>,
    /// The key it was made with.
    pub key: Option<String>,
    /// Its fingerprint, when git offers one.
    pub fingerprint: Option<String>,
    /// gpg's or ssh-keygen's own output, for the fold-out under the line.
    pub raw: Option<String>,
}

/// The mark one row of the graph carries.
#[derive(Serialize, Debug, PartialEq, Eq)]
pub struct Mark {
    pub verdict: Verdict,
    pub signer: Option<String>,
}

/// A page of the graph's worth of verdicts, keyed by commit.
///
/// One `git log` for the lot: verifying a hundred commits one at a time is a
/// hundred subprocesses, and on Windows a hundred process creations is the
/// difference between a page that draws and a page that hangs.
pub fn marks(state: &AppState, limit: usize) -> Result<HashMap<String, Mark>, String> {
    let root = state.path()?;
    let count = limit.to_string();
    let out = git_cmd::run(
        &root,
        &[
            "log",
            "--no-show-signature",
            "--format=%H%x1f%G?%x1f%GS",
            "-n",
            &count,
        ],
    )?;
    // A repository with no commits, or a HEAD that points nowhere, is not an
    // error worth a notice: there is simply nothing to mark.
    if !out.ok {
        return Ok(HashMap::new());
    }
    Ok(read_marks(&out.stdout))
}

fn read_marks(raw: &str) -> HashMap<String, Mark> {
    let mut found = HashMap::new();
    for line in raw.lines() {
        let mut parts = line.split('\u{1f}');
        let Some(oid) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let verdict = Verdict::read(parts.next().unwrap_or("N").trim());
        // An unsigned commit is the common case and carries no information;
        // leaving it out of the map keeps it out of the payload too.
        if verdict == Verdict::None {
            continue;
        }
        let signer = parts
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        found.insert(oid.to_string(), Mark { verdict, signer });
    }
    found
}

/// Everything git will say about one commit's signature.
pub fn of(state: &AppState, oid: &str) -> Result<Signature, String> {
    let root = state.path()?;
    let out = git_cmd::run(
        &root,
        &[
            "show",
            "--no-patch",
            "--no-show-signature",
            "--format=%G?%x1f%GS%x1f%GK%x1f%GF",
            oid,
        ],
    )?;
    if !out.ok {
        return Ok(Signature::default());
    }
    let mut found = read_one(&out.stdout);
    if found.verdict.is_some_and(|v| v != Verdict::None) {
        found.raw = raw_check(&root, oid);
    }
    Ok(found)
}

fn read_one(raw: &str) -> Signature {
    let line = raw.lines().next().unwrap_or_default();
    let mut parts = line.split('\u{1f}');
    let verdict = Verdict::read(parts.next().unwrap_or("N").trim());
    let mut next = || {
        parts
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Signature {
        verdict: Some(verdict),
        signer: next(),
        key: next(),
        fingerprint: next(),
        raw: None,
    }
}

/// gpg's or ssh-keygen's own words about the signature.
///
/// `verify-commit` writes them to stderr, and its exit status is the same
/// question `%G?` already answered — so a non-zero exit here is not a failure,
/// it is the bad signature we are trying to quote.
fn raw_check(root: &Path, oid: &str) -> Option<String> {
    let out = git_cmd::run(root, &["verify-commit", "--raw", oid]).ok()?;
    let text = if out.stderr.trim().is_empty() {
        out.stdout
    } else {
        out.stderr
    };
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// What this repository is set up to do when you commit.
#[derive(Serialize, Debug, PartialEq, Eq, Default)]
pub struct Setup {
    /// `commit.gpgsign` — whether a commit made here is signed.
    pub signs: bool,
    /// `tag.gpgsign`.
    pub signs_tags: bool,
    /// `gpg.format`: `openpgp` unless told otherwise.
    pub format: String,
    /// `user.signingkey`, which for ssh is a path or a literal key.
    pub key: Option<String>,
}

/// Read rather than assumed: these are ordinary git settings and the machine
/// may well have been set up outside this app, in which case what the profile
/// says is beside the point.
pub fn setup(state: &AppState) -> Result<Setup, String> {
    let root = state.path()?;
    let read = |key: &str| {
        let out = git_cmd::run(&root, &["config", "--get", key]).ok()?;
        out.ok
            .then(|| out.stdout.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    Ok(Setup {
        signs: read("commit.gpgsign").as_deref() == Some("true"),
        signs_tags: read("tag.gpgsign").as_deref() == Some("true"),
        format: read("gpg.format").unwrap_or_else(|| "openpgp".to_string()),
        key: read("user.signingkey"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_every_code_git_prints() {
        assert_eq!(Verdict::read("G"), Verdict::Good);
        assert_eq!(Verdict::read("B"), Verdict::Bad);
        assert_eq!(Verdict::read("E"), Verdict::Unchecked);
        assert_eq!(Verdict::read("N"), Verdict::None);
        for code in ["U", "X", "Y", "R"] {
            assert_eq!(Verdict::read(code), Verdict::Untrusted);
        }
        assert_eq!(Verdict::read(""), Verdict::None);
    }

    #[test]
    fn keeps_only_the_signed_rows() {
        let raw = concat!(
            "aaa\u{1f}G\u{1f}Ramon Robben\n",
            "bbb\u{1f}N\u{1f}\n",
            "ccc\u{1f}B\u{1f}A Contributor\n"
        );
        let found = read_marks(raw);
        assert_eq!(found.len(), 2);
        assert_eq!(found["aaa"].verdict, Verdict::Good);
        assert_eq!(found["aaa"].signer.as_deref(), Some("Ramon Robben"));
        assert_eq!(found["ccc"].verdict, Verdict::Bad);
        assert!(!found.contains_key("bbb"));
    }

    #[test]
    fn a_signer_with_no_name_is_not_an_empty_one() {
        let found = read_marks("aaa\u{1f}U\u{1f}\n");
        assert_eq!(found["aaa"].verdict, Verdict::Untrusted);
        assert_eq!(found["aaa"].signer, None);
    }

    #[test]
    fn reads_the_four_fields_of_one_commit() {
        let one = read_one("G\u{1f}Ramon Robben\u{1f}SHA256:0mB\u{1f}FINGER\n");
        assert_eq!(one.verdict, Some(Verdict::Good));
        assert_eq!(one.signer.as_deref(), Some("Ramon Robben"));
        assert_eq!(one.key.as_deref(), Some("SHA256:0mB"));
        assert_eq!(one.fingerprint.as_deref(), Some("FINGER"));
    }

    #[test]
    fn an_unsigned_commit_says_so_without_inventing_a_signer() {
        let one = read_one("N\u{1f}\u{1f}\u{1f}\n");
        assert_eq!(one.verdict, Some(Verdict::None));
        assert_eq!(one.signer, None);
        assert_eq!(one.key, None);
    }
}
