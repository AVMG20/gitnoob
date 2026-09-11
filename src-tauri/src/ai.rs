use serde::{Deserialize, Serialize};

use crate::config::{self, OPENROUTER_KEY};
use crate::conflict;
use crate::git_cmd;
use crate::state::AppState;

const BASE: &str = "https://openrouter.ai/api/v1";
/// Long enough that opening the picker twice does not hit the network twice,
/// short enough that new models show up the same day.
const MODEL_CACHE_SECS: u64 = 60 * 60 * 6;
/// Diffs get large; past this the model adds nothing but cost.
const MAX_DIFF_CHARS: usize = 48_000;
/// Git's empty tree, which is what a first commit is diffed against.
const EMPTY_TREE: &str = "4b825dc642cb6eb9a060e54bf8d69288fbee4904";

#[derive(Serialize, Deserialize, Clone)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub context_length: u64,
    /// US dollars per million tokens, which is how everyone quotes them.
    pub prompt_price: f64,
    pub completion_price: f64,
    pub description: String,
    /// True when the model takes images as well as text.
    pub multimodal: bool,
}

#[derive(Serialize)]
pub struct AiStatus {
    pub configured: bool,
    pub model: Option<String>,
    /// What the settings box starts from, and what "put the default back"
    /// puts back. Sent rather than repeated in the window, so the two cannot
    /// drift apart.
    pub default_commit_prompt: String,
}

#[derive(Serialize)]
pub struct CommitMessage {
    pub summary: String,
    pub body: String,
}

pub fn status(state: &AppState) -> AiStatus {
    let config = state.config();
    AiStatus {
        configured: config::secret_get(OPENROUTER_KEY).is_some()
            && config.global.ai.model.is_some(),
        model: config.global.ai.model.clone(),
        default_commit_prompt: DEFAULT_COMMIT_PROMPT.to_string(),
    }
}

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| e.to_string())
}

fn key() -> Result<String, String> {
    config::secret_get(OPENROUTER_KEY)
        .ok_or_else(|| "No OpenRouter API key set — add one in Settings › AI".to_string())
}

/// Fetches OpenRouter's catalogue, cached in memory.
///
/// This endpoint needs no key, so the picker works before the user has pasted
/// one and they can see prices while deciding.
pub async fn models(state: &AppState, refresh: bool) -> Result<Vec<Model>, String> {
    if !refresh {
        if let Some(cached) = state.cached_models(MODEL_CACHE_SECS) {
            return Ok(cached);
        }
    }

    let response = client()?
        .get(format!("{BASE}/models"))
        .send()
        .await
        .map_err(|e| format!("Could not reach OpenRouter: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("OpenRouter returned {}", response.status()));
    }
    let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;
    let items = body
        .get("data")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "OpenRouter sent an unexpected model list".to_string())?;

    let mut models: Vec<Model> = items
        .iter()
        .map(|item| {
            let pricing = item.get("pricing");
            // Prices arrive as per-token decimal strings.
            let price = |field: &str| {
                pricing
                    .and_then(|p| p.get(field))
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.parse::<f64>().ok())
                    .unwrap_or(0.0)
                    * 1_000_000.0
            };
            let modalities = item
                .get("architecture")
                .and_then(|a| a.get("input_modalities"))
                .and_then(|v| v.as_array())
                .map(|list| list.iter().filter_map(|v| v.as_str()).any(|m| m == "image"))
                .unwrap_or(false);

            Model {
                id: item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                name: item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                context_length: item
                    .get("context_length")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0),
                prompt_price: price("prompt"),
                completion_price: price("completion"),
                description: item
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .chars()
                    .take(400)
                    .collect(),
                multimodal: modalities,
            }
        })
        .filter(|model| !model.id.is_empty())
        .collect();

    models.sort_by(|a, b| a.id.cmp(&b.id));
    state.cache_models(models.clone());
    Ok(models)
}

/// Turns the chosen thinking level into OpenRouter's `reasoning` object.
///
/// OpenRouter takes one shape for every provider: an effort level, which it
/// translates into whatever the model underneath wants, or `enabled: false` to
/// switch thinking off on a model that would otherwise do it. Anything the
/// reasoning tokens are spent on is billed, so "off" is a real choice rather
/// than a placeholder.
fn reasoning_field(level: &str) -> serde_json::Value {
    match level {
        "minimal" | "low" | "medium" | "high" => serde_json::json!({ "effort": level }),
        // "off", and anything unrecognised: no thinking at all.
        _ => serde_json::json!({ "enabled": false }),
    }
}

/// What one chat completion came back with, before anyone reads meaning
/// into it.
///
/// Kept apart from the parsing because every provider on OpenRouter shapes the
/// same answer slightly differently: content as a string or as a list of
/// parts, thinking in a `reasoning` field or inline in `<think>` tags, and an
/// answer that ran out of tokens looking exactly like one that was never
/// written. The caller decides what counts as an answer.
#[derive(Debug, Default, Clone, PartialEq)]
struct Reply {
    /// The message content as sent, indentation and all. Blank when the
    /// model wrote none.
    content: String,
    /// Whatever the model thought before answering, when the provider sends
    /// it back. Some providers put the whole answer here by mistake.
    reasoning: String,
    /// OpenRouter's normalised reason, e.g. `stop` or `length`.
    finish: String,
    /// How many tokens went on thinking, when the provider counts them.
    reasoning_tokens: Option<u64>,
}

impl Reply {
    fn cut_off(&self) -> bool {
        self.finish == "length"
    }

    fn is_blank(&self) -> bool {
        self.content.trim().is_empty()
    }

    /// Why an answer is missing, in words that say what to change.
    fn missing(&self, model: &str) -> String {
        let thought = match self.reasoning_tokens {
            Some(n) if n > 0 => format!(" after {n} reasoning tokens"),
            _ if !self.reasoning.is_empty() => " after thinking".to_string(),
            _ => String::new(),
        };
        if self.cut_off() {
            format!(
                "{model} ran out of tokens{thought} before writing an answer. \
                 Raise Max tokens or turn Thinking off in Settings › AI"
            )
        } else if !self.reasoning.is_empty() {
            format!(
                "{model} put its whole answer in the reasoning field and none in the reply \
                 (finish reason: {}). Try Thinking off, or another model",
                self.finish
            )
        } else {
            format!(
                "{model} returned an empty answer (finish reason: {})",
                if self.finish.is_empty() {
                    "none"
                } else {
                    &self.finish
                }
            )
        }
    }
}

/// Reads one chat completion body into a [`Reply`].
///
/// A 200 is not always an answer: OpenRouter forwards a provider's failure as
/// `choices[0].error` with the status of the gateway, not the provider. That
/// is caught here so it reads as the failure it is rather than as an empty
/// answer.
fn parse_reply(body: &serde_json::Value) -> Result<Reply, String> {
    let choice = body
        .get("choices")
        .and_then(|c| c.get(0))
        .ok_or_else(|| "OpenRouter sent no choices back".to_string())?;

    if let Some(error) = choice.get("error") {
        let detail = error
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("no detail");
        return Err(format!("The provider failed: {detail}"));
    }

    let message = choice.get("message");
    let text = |field: &str| -> String {
        message
            .and_then(|m| m.get(field))
            .map(text_of)
            .unwrap_or_default()
    };
    let (content, inline_thinking) = strip_think_tags(&text("content"));
    let mut reasoning = text("reasoning");
    if reasoning.is_empty() {
        // DeepSeek's own name for the same field, which some hosts keep.
        reasoning = text("reasoning_content");
    }
    if reasoning.is_empty() {
        reasoning = inline_thinking;
    }

    let finish = choice
        .get("finish_reason")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let reasoning_tokens = body
        .get("usage")
        .and_then(|u| u.get("completion_tokens_details"))
        .and_then(|d| d.get("reasoning_tokens"))
        .and_then(|v| v.as_u64());

    Ok(Reply {
        content,
        reasoning: reasoning.trim().to_string(),
        finish,
        reasoning_tokens,
    })
}

/// The text of a content field, whichever of the two shapes it takes.
///
/// A plain string most of the time; a list of `{type: "text", text}` parts
/// from providers that speak the multimodal dialect even for text.
fn text_of(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(parts) => parts
            .iter()
            .filter_map(|part| {
                part.get("text")
                    .and_then(|t| t.as_str())
                    .or_else(|| part.as_str())
            })
            .collect::<Vec<_>>()
            .join(""),
        _ => String::new(),
    }
}

/// Splits `<think>…</think>` blocks out of an answer.
///
/// Returns the answer without them, and the thinking on its own. A hosted
/// model whose chat template leaks its thinking into the content is the usual
/// source; an unclosed `<think>` means the model never got to the answer, so
/// everything after it is thinking too.
fn strip_think_tags(text: &str) -> (String, String) {
    const OPEN: &str = "<think>";
    const CLOSE: &str = "</think>";
    let mut answer = String::new();
    let mut thinking = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        answer.push_str(&rest[..start]);
        let inner = &rest[start + OPEN.len()..];
        match inner.find(CLOSE) {
            Some(end) => {
                thinking.push_str(&inner[..end]);
                rest = &inner[end + CLOSE.len()..];
            }
            None => {
                thinking.push_str(inner);
                rest = "";
            }
        }
    }
    answer.push_str(rest);
    (answer, thinking)
}

/// One chat completion round trip, returned as the model sent it.
///
/// `floor` is the fewest completion tokens the request may be allowed. The
/// settings cap is a cost control for commit messages; an answer that is a
/// block of code the user already has on screen needs room for that block,
/// and capping it below that room can only produce a cut-off answer.
///
/// An empty reply that was not cut off is asked for once more before it is
/// reported: providers behind OpenRouter do return nothing now and then, and
/// the second ask almost always lands.
async fn complete_raw(
    state: &AppState,
    system: &str,
    user: &str,
    floor: u32,
) -> Result<(Reply, String), String> {
    let config = state.config();
    let model = config
        .global
        .ai
        .model
        .clone()
        .ok_or_else(|| "No model chosen — pick one in Settings › AI".to_string())?;
    let key = key()?;
    let max_tokens = config.global.ai.max_tokens.max(floor);
    let mut reasoning = Some(reasoning_field(&config.global.ai.reasoning));
    let client = client()?;

    let mut reply = Reply::default();
    // Two goes at most: a retry for a blank reply, or one for a thinking
    // switch the model refused, and never a third.
    for attempt in 0..2 {
        let mut request = serde_json::json!({
            "model": model,
            "max_tokens": max_tokens,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user }
            ]
        });
        if let Some(reasoning) = &reasoning {
            request["reasoning"] = reasoning.clone();
        }
        let response = client
            .post(format!("{BASE}/chat/completions"))
            .bearer_auth(&key)
            // OpenRouter uses these for its own attribution listings.
            .header("HTTP-Referer", "https://github.com/gitnoob")
            .header("X-Title", "gitnoob")
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("Could not reach OpenRouter: {e}"))?;

        let status = response.status();
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("OpenRouter sent something unreadable: {e}"))?;

        if !status.is_success() {
            let detail = body
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(|v| v.as_str())
                .unwrap_or("no detail");
            // Some models think whether asked to or not, and OpenRouter
            // refuses the request rather than ignoring the switch. The
            // setting is a cost control, not a requirement, so the same
            // request goes again with the model's own default.
            if reasoning.is_some() && cannot_stop_thinking(detail) {
                reasoning = None;
                continue;
            }
            return Err(format!("{status}: {detail}"));
        }

        reply = parse_reply(&body)?;
        let worth_retrying = reply.is_blank() && reply.reasoning.is_empty() && !reply.cut_off();
        if !worth_retrying || attempt == 1 {
            break;
        }
    }
    Ok((reply, model))
}

/// Whether an error says the model will think regardless of the setting.
///
/// OpenRouter's wording as of this writing: "Reasoning is mandatory for this
/// endpoint and cannot be disabled." Matched loosely, since the sentence is
/// theirs to change.
fn cannot_stop_thinking(detail: &str) -> bool {
    let lower = detail.to_ascii_lowercase();
    lower.contains("reasoning")
        && (lower.contains("mandatory") || lower.contains("cannot be disabled"))
}

/// One chat completion round trip, as the text the model wrote.
async fn complete(state: &AppState, system: &str, user: String) -> Result<String, String> {
    let (reply, model) = complete_raw(state, system, &user, 0).await?;
    if reply.is_blank() {
        return Err(reply.missing(&model));
    }
    Ok(reply.content.trim().to_string())
}

/// Writes a commit message from what is staged.
pub async fn commit_message(state: &AppState) -> Result<CommitMessage, String> {
    let root = state.path()?;
    // Gather everything before the await; no git handles cross it.
    let diff = git_cmd::run_checked(
        &root,
        &[
            "diff",
            "--cached",
            "--no-color",
            "--unified=3",
            "--stat-width=200",
        ],
    )?;
    if diff.trim().is_empty() {
        return Err("Nothing is staged, so there is nothing to describe".to_string());
    }
    let files = git_cmd::run_checked(&root, &["diff", "--cached", "--name-status"])?;
    let recent = git_cmd::run_checked(&root, &["log", "-8", "--format=%s"]).unwrap_or_default();

    describe_change(state, "Staged diff", &files, &diff, &recent).await
}

/// Writes a commit message from a commit that already exists.
///
/// The same job as [`commit_message`] with a different subject: the message
/// being replaced is the one already on the commit, so what the model is shown
/// is that commit's own diff rather than the index. Its own subject is left
/// out of the recent ones deliberately — the point of asking is that the
/// message it has is not the wanted one.
pub async fn commit_message_for(state: &AppState, oid: &str) -> Result<CommitMessage, String> {
    let root = state.path()?;
    let oid = checked_ref(oid)?;
    let diff = git_cmd::run_checked(
        &root,
        &["show", "--no-color", "--unified=3", "--format=", &oid],
    )?;
    if diff.trim().is_empty() {
        return Err("That commit changed nothing, so there is nothing to describe".to_string());
    }
    let files = git_cmd::run_checked(&root, &["show", "--name-status", "--format=", &oid])?;
    // A root commit has no parent to walk back from; tone is a nicety anyway.
    let recent = git_cmd::run_checked(&root, &["log", "-8", "--format=%s", &format!("{oid}^")])
        .unwrap_or_default();

    describe_change(state, "Diff", &files, &diff, &recent).await
}

/// Writes one message for the commits a squash would fold.
///
/// The dialog starts from git's own join — every message one after another —
/// which says what each commit did and never what the fold as a whole did. So
/// the model is shown the range as a single diff, the way the folded commit
/// will read afterwards, together with the messages being replaced: they carry
/// the reasoning that the diff alone does not.
pub async fn squash_message(state: &AppState, oids: &[String]) -> Result<CommitMessage, String> {
    let root = state.path()?;
    let mut wanted: Vec<String> = Vec::new();
    for oid in oids {
        let oid = checked_ref(oid)?;
        if !wanted.contains(&oid) {
            wanted.push(oid);
        }
    }
    if wanted.len() < 2 {
        return Err("Squashing folds commits together, so it takes at least two.".to_string());
    }

    let ordered = crate::work::in_history_order(state, &wanted)?;
    let oldest = ordered.first().expect("checked non-empty above");
    let newest = ordered.last().expect("checked non-empty above");
    // The parent of the oldest, or the empty tree when the run starts at the
    // root: a first commit has nothing behind it to diff against.
    let base = git_cmd::run(&root, &["rev-parse", "--verify", &format!("{oldest}^")])
        .ok()
        .filter(|out| out.ok)
        .map(|out| out.stdout.trim().to_string())
        .filter(|oid| !oid.is_empty())
        .unwrap_or_else(|| EMPTY_TREE.to_string());

    let diff = git_cmd::run_checked(
        &root,
        &[
            "diff",
            "--no-color",
            "--unified=3",
            "--stat-width=200",
            &base,
            newest,
        ],
    )?;
    if diff.trim().is_empty() {
        return Err(
            "Those commits change nothing together, so there is nothing to describe.".to_string(),
        );
    }
    let files = git_cmd::run_checked(&root, &["diff", "--name-status", &base, newest])?;
    // What the commits say about themselves, which is the part of the join
    // worth keeping. Subjects from before the run set the tone.
    let folded = git_cmd::run_checked(
        &root,
        &[
            "log",
            "--reverse",
            "--format=%B",
            &format!("{base}..{newest}"),
        ],
    )
    .unwrap_or_default();
    let recent =
        git_cmd::run_checked(&root, &["log", "-8", "--format=%s", &base]).unwrap_or_default();

    let truncated = diff.chars().count() > MAX_DIFF_CHARS;
    let short: String = diff.chars().take(MAX_DIFF_CHARS).collect();
    let system = commit_prompt(&state.config().global.ai);
    let prompt = format!(
        "These commits are being folded into one. Their own messages:\n{folded}\n\n         Files changed:\n{files}\n\nRecent commit subjects in this repository:\n{recent}\n\n         The folded diff{}:\n{short}",
        if truncated { " (truncated)" } else { "" }
    );

    let answer = complete(state, &system, prompt).await?;
    Ok(split_message(&answer))
}

/// Asks the model for a subject and body describing one set of changes.
async fn describe_change(
    state: &AppState,
    label: &str,
    files: &str,
    diff: &str,
    recent: &str,
) -> Result<CommitMessage, String> {
    let truncated = diff.chars().count() > MAX_DIFF_CHARS;
    let diff: String = diff.chars().take(MAX_DIFF_CHARS).collect();
    // Whatever the user has in the settings box, which is the default until
    // they change it.
    let system = commit_prompt(&state.config().global.ai);

    let prompt = format!(
        "Files changed:\n{files}\n\nRecent commit subjects in this repository, \
         for the vocabulary this codebase uses:\n{recent}\n\n{label}{}:\n{diff}",
        if truncated { " (truncated)" } else { "" }
    );

    let answer = complete(state, &system, prompt).await?;
    Ok(split_message(&answer))
}

/// The instructions a commit message is written under.
///
/// An empty box means the default rather than no instructions at all: a model
/// told nothing writes a paragraph of prose and calls it a commit message.
pub fn commit_prompt(ai: &config::Ai) -> String {
    match ai.commit_prompt.as_deref().map(str::trim) {
        Some(text) if !text.is_empty() => text.to_string(),
        _ => DEFAULT_COMMIT_PROMPT.to_string(),
    }
}

/// Writes the title and description of a review from the commits behind it.
///
/// The commits are the argument the branch is making, in the author's own
/// words; the diffstat is there so a subject like "fix the thing" can still be
/// placed in the right part of the codebase.
pub async fn review_message(
    state: &AppState,
    source: String,
    target: String,
) -> Result<CommitMessage, String> {
    let root = state.path()?;
    let source = checked_ref(&source)?;
    let target = resolve_ref(&root, &checked_ref(&target)?)?;
    if source == target {
        return Err("The two branches are the same".to_string());
    }

    let range = format!("{target}..{source}");
    let log = git_cmd::run_checked(
        &root,
        &[
            "log",
            "--no-color",
            "--max-count=40",
            "--format=%s%n%b%n---",
            &range,
        ],
    )?;
    if log.trim().is_empty() {
        return Err(format!(
            "{source} has no commits that {target} does not already have"
        ));
    }
    // Three dots: what the branch did, not what happened on the target while
    // it was away.
    let stat = git_cmd::run_checked(
        &root,
        &[
            "diff",
            "--stat",
            "--stat-width=200",
            &format!("{target}...{source}"),
        ],
    )
    .unwrap_or_default();

    let truncated = log.chars().count() > MAX_DIFF_CHARS;
    let log: String = log.chars().take(MAX_DIFF_CHARS).collect();
    let stat: String = stat.chars().take(8_000).collect();

    let prompt = format!(
        "Branch: {source}\nMerging into: {target}\n\n\
         Commits on the branch, newest first, each ending with `---`{}:\n{log}\n\n\
         Files changed:\n{stat}",
        if truncated { " (truncated)" } else { "" }
    );

    let answer = complete(state, REVIEW_SYSTEM, prompt).await?;
    Ok(split_message(&answer))
}

/// Refuses a branch name git would read as an option.
///
/// The arguments never reach a shell, so this is not about quoting: a branch
/// called `--all` would simply be understood as a flag.
fn checked_ref(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("No branch given".to_string());
    }
    if name.starts_with('-') {
        return Err(format!("{name} is not a usable branch name"));
    }
    Ok(name.to_string())
}

/// Finds the ref a branch name means here.
///
/// The target of a review is often a branch nobody has checked out — `develop`
/// exists on the forge and locally only as `origin/develop` — so a plain name
/// that resolves to nothing is tried again against each remote.
fn resolve_ref(root: &std::path::Path, name: &str) -> Result<String, String> {
    let exists = |candidate: &str| {
        git_cmd::run_checked(
            root,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{candidate}^{{commit}}"),
            ],
        )
        .is_ok()
    };
    if exists(name) {
        return Ok(name.to_string());
    }
    let remotes = git_cmd::run_checked(root, &["remote"]).unwrap_or_default();
    for remote in remotes.lines().map(str::trim).filter(|r| !r.is_empty()) {
        let candidate = format!("{remote}/{name}");
        if exists(&candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "{name} is not a branch this repository knows about"
    ))
}

/// Resolves one conflict region.
///
/// The model gets the merge base as well as both sides, which is what lets it
/// tell an addition from a deletion instead of guessing.
pub async fn resolve_conflict(
    state: &AppState,
    path: String,
    index: usize,
) -> Result<Vec<String>, String> {
    let file = conflict::read(state, &path)?;
    let mut ours = Vec::new();
    let mut base = Vec::new();
    let mut theirs = Vec::new();
    let mut has_base = false;
    let mut before: Vec<String> = Vec::new();
    let mut after: Vec<String> = Vec::new();
    let mut seen = false;

    // Walk the blocks once, keeping the context on either side of the region.
    for block in &file.blocks {
        match block {
            conflict::Block::Context { lines } => {
                if seen {
                    if after.len() < 15 {
                        after.extend(lines.iter().take(15 - after.len()).cloned());
                    }
                } else {
                    before = lines.iter().rev().take(15).rev().cloned().collect();
                }
            }
            conflict::Block::Conflict {
                index: i,
                ours: o,
                base: b,
                theirs: t,
                has_base: hb,
                ..
            } => {
                if *i == index {
                    ours = o.clone();
                    base = b.clone();
                    theirs = t.clone();
                    has_base = *hb;
                    seen = true;
                } else if !seen {
                    before.clear();
                }
            }
        }
    }

    if !seen {
        return Err(format!("No conflict number {index} in {path}"));
    }

    let prompt = format!(
        "File: {path}\n\n\
         Lines before the conflict:\n{}\n\n\
         {}OUR side:\n{}\n\nTHEIR side:\n{}\n\n\
         Lines after the conflict:\n{}",
        before.join("\n"),
        if has_base {
            format!("COMMON ANCESTOR:\n{}\n\n", base.join("\n"))
        } else {
            String::new()
        },
        ours.join("\n"),
        theirs.join("\n"),
        after.join("\n")
    );

    // Room for the answer: it is at most about the size of both sides put
    // together, and a cap below that can only cut it off.
    let floor = u32::try_from(
        ours.iter()
            .chain(&theirs)
            .map(|l| l.len() + 1)
            .sum::<usize>()
            / 2
            + 512,
    )
    .unwrap_or(u32::MAX);

    let (reply, model) = complete_raw(state, CONFLICT_SYSTEM, &prompt, floor).await?;
    resolution_of(&reply, &model)
}

/// The line the model is told to put before its answer, and the one after.
///
/// An answer with edges is one that can be found wherever the model put it:
/// after a sentence of commentary it was told not to write, inside a fence it
/// was told not to add, or in the reasoning field on a host whose chat
/// template loses the final turn. It is also the only way an empty
/// resolution — both sides deleted the lines — can be told from no answer.
const RESOLUTION_OPEN: &str = "=====BEGIN RESOLUTION=====";
const RESOLUTION_CLOSE: &str = "=====END RESOLUTION=====";

/// Reads the resolved lines out of a reply.
///
/// Looks for the delimited block in the content first and the reasoning
/// second. Without a delimited block, the content is the answer if there is
/// one, minus any fence the model added anyway.
fn resolution_of(reply: &Reply, model: &str) -> Result<Vec<String>, String> {
    if reply.cut_off() {
        return Err(if reply.is_blank() {
            reply.missing(model)
        } else {
            format!(
                "{model} ran out of tokens part way through the answer. \
                 Raise Max tokens or turn Thinking off in Settings › AI"
            )
        });
    }
    if let Some(lines) = delimited(&reply.content).or_else(|| delimited(&reply.reasoning)) {
        return Ok(lines);
    }
    if reply.is_blank() {
        return Err(reply.missing(model));
    }
    Ok(strip_fences(reply.content.trim_matches(['\r', '\n'])))
}

/// The lines between the delimiters, when both are there in order.
///
/// A fence the model wrapped the block in, inside or outside the delimiters,
/// is dropped; the delimiter lines themselves may carry stray whitespace.
fn delimited(text: &str) -> Option<Vec<String>> {
    let lines: Vec<&str> = text.lines().collect();
    let open = lines.iter().position(|l| l.trim() == RESOLUTION_OPEN)?;
    let close = lines
        .iter()
        .skip(open + 1)
        .position(|l| l.trim() == RESOLUTION_CLOSE)
        .map(|at| at + open + 1)?;
    let inner = strip_fences(&lines[open + 1..close].join("\n"));
    Some(inner)
}

/// Splits a model's answer into a summary line and a body.
fn split_message(answer: &str) -> CommitMessage {
    // Models like to wrap prose in fences even when told not to.
    let cleaned = strip_fences(answer).join("\n");
    let mut lines = cleaned.lines();
    let summary = lines
        .next()
        .unwrap_or("")
        .trim()
        .trim_start_matches("Summary:")
        .trim()
        .to_string();
    let body = lines.collect::<Vec<_>>().join("\n").trim().to_string();
    CommitMessage { summary, body }
}

/// Removes a surrounding markdown code fence, if the answer has one.
fn strip_fences(answer: &str) -> Vec<String> {
    let lines: Vec<&str> = answer.lines().collect();
    let opens = lines
        .first()
        .is_some_and(|l| l.trim_start().starts_with("```"));
    let closes = lines.len() > 1 && lines.last().is_some_and(|l| l.trim() == "```");
    let slice = if opens && closes {
        &lines[1..lines.len() - 1]
    } else if opens {
        &lines[1..]
    } else {
        &lines[..]
    };
    slice.iter().map(|l| l.to_string()).collect()
}

/// What the model is told when the settings box has not been changed.
///
/// One line, most of the time. A body is the exception rather than the shape,
/// because a commit that needs three sentences of explanation is rarer than
/// the models writing them believe.
///
/// It says nothing about `feat:` and `fix:` on purpose. A type prefix is a
/// convention a repository either has or does not, and a default that reaches
/// for one writes it into repositories that never asked. Anyone who wants them
/// has the settings box to say so.
pub const DEFAULT_COMMIT_PROMPT: &str = "\
You write git commit messages for a working developer. Reply with the message \
and nothing else: no preamble, no markdown, no code fences, no quotes.

Line 1 is the message, and usually the whole of it: imperative mood, no \
trailing period, under 72 characters, specific about what changed.

Add a body only where the summary cannot carry the change on its own. When you \
do, leave a blank line after the summary and keep it to one or two sentences \
on WHY. Most commits need no body at all. Never list the files, never restate \
the diff, never pad.";

/// Kept for the settings that were written before the box existed: a config
/// that said `conventional` is migrated into the box with this text, so the
/// choice survives the change rather than being quietly dropped.
pub const CONVENTIONAL_COMMIT_PROMPT: &str = "\
You write git commit messages in Conventional Commits form. Reply with the \
message and nothing else: no preamble, no markdown, no code fences.

Line 1: `type(scope): summary` where type is one of feat, fix, docs, style, \
refactor, perf, test, build, ci, chore. The scope is optional. Imperative \
mood, no trailing period, under 72 characters total.

Then a blank line, then one to three sentences on WHY the change was made. \
Skip the body for a trivial change. Do not list files.";

const REVIEW_SYSTEM: &str = "\
You write the title and description of a pull request, for the reviewer who \
has to read it. Reply with the text and nothing else: no preamble, no code \
fences, no quotes.

Line 1 is the title: what the branch does as a whole, imperative mood, no \
trailing period, under 70 characters. It is not a list of the commits; if they \
are one piece of work, name the piece. Never prefix it with the branch name or \
a ticket number that is already in the branch name.

Then a blank line, then the description in plain GitHub-flavoured markdown, \
usually 60 to 200 words:

- A short paragraph on WHY the change was made and what problem it solves.
- A `## What changed` list of the substantial changes, one line each, only \
where there is more than one worth separating. Group by intent, not by file, \
and leave out anything mechanical.
- A `## How to test` list, only when there is something a reviewer should \
actually run or click.

Write what the commits and the file list support and nothing more. Do not \
invent tickets, screenshots, breaking changes or migration steps. Do not \
thank anyone, do not summarise your own summary, and do not add a checklist \
the project has not asked for.";

const CONFLICT_SYSTEM: &str = "\
You resolve a single git merge conflict.

You are given the lines around the conflict, the common ancestor when it is \
available, and both sides. Work out what each side was trying to do and produce \
the text that keeps BOTH intentions where they are compatible. If they are \
genuinely incompatible, keep the side whose change is clearly newer or more \
specific.

Reply with the resolved lines between two marker lines, exactly like this:

=====BEGIN RESOLUTION=====
<the resolved lines>
=====END RESOLUTION=====

Nothing else: no conflict markers, no markdown, no code fences, no commentary, \
no explanation before or after the markers. Preserve the file's existing \
indentation style and language exactly. If the right answer is to keep one side \
unchanged, output that side verbatim. If the right answer is to delete the \
lines entirely, put nothing between the markers.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_box_means_the_default_rather_than_no_instructions() {
        let mut ai = config::Ai::default();
        assert_eq!(commit_prompt(&ai), DEFAULT_COMMIT_PROMPT);

        ai.commit_prompt = Some(String::new());
        assert_eq!(commit_prompt(&ai), DEFAULT_COMMIT_PROMPT);

        ai.commit_prompt = Some("   \n  ".to_string());
        assert_eq!(commit_prompt(&ai), DEFAULT_COMMIT_PROMPT);
    }

    #[test]
    fn what_is_in_the_box_is_what_the_model_is_told() {
        let ai = config::Ai {
            commit_prompt: Some("  Only ever write one line.  ".to_string()),
            ..Default::default()
        };
        assert_eq!(commit_prompt(&ai), "Only ever write one line.");
    }

    #[test]
    fn the_default_asks_for_one_line_and_a_body_only_where_it_is_needed() {
        // The wording is the feature: a default that asks for three sentences
        // of body gets three sentences of body on every commit.
        assert!(DEFAULT_COMMIT_PROMPT.contains("usually the whole of it"));
        assert!(DEFAULT_COMMIT_PROMPT.contains("Add a body only where"));
        assert!(DEFAULT_COMMIT_PROMPT.contains("Most commits need no body at all"));
    }

    #[test]
    fn the_default_asks_for_no_type_prefix() {
        // A prefix is a convention a repository either has or does not, and
        // the default writing `feat:` into one that never used it is the thing
        // people turn the whole feature off over. Conventional Commits are
        // still there for anyone who wants them, in the settings box.
        assert!(!DEFAULT_COMMIT_PROMPT.contains("feat:"));
        assert!(!DEFAULT_COMMIT_PROMPT.contains("fix:"));
        assert!(CONVENTIONAL_COMMIT_PROMPT.contains("feat"));
    }

    #[test]
    fn a_thinking_level_becomes_an_effort() {
        assert_eq!(
            reasoning_field("high"),
            serde_json::json!({ "effort": "high" })
        );
        assert_eq!(
            reasoning_field("minimal"),
            serde_json::json!({ "effort": "minimal" })
        );
    }

    #[test]
    fn off_switches_thinking_off_rather_than_omitting_it() {
        assert_eq!(
            reasoning_field("off"),
            serde_json::json!({ "enabled": false })
        );
        // An unknown value from a hand-edited config must not turn thinking on.
        assert_eq!(
            reasoning_field("whatever"),
            serde_json::json!({ "enabled": false })
        );
    }

    #[test]
    fn splits_a_summary_from_a_body() {
        let message = split_message("Add a thing\n\nBecause it was missing.");
        assert_eq!(message.summary, "Add a thing");
        assert_eq!(message.body, "Because it was missing.");
    }

    #[test]
    fn tolerates_a_summary_only_answer() {
        let message = split_message("Fix the off-by-one");
        assert_eq!(message.summary, "Fix the off-by-one");
        assert_eq!(message.body, "");
    }

    #[test]
    fn strips_fences_a_model_added_anyway() {
        let message = split_message("```\nAdd a thing\n\nBody here.\n```");
        assert_eq!(message.summary, "Add a thing");
        assert_eq!(message.body, "Body here.");

        assert_eq!(
            strip_fences("```rust\nlet x = 1;\n```"),
            vec!["let x = 1;".to_string()]
        );
        assert_eq!(
            strip_fences("no fence here"),
            vec!["no fence here".to_string()]
        );
    }

    #[test]
    fn refuses_a_branch_name_git_would_read_as_a_flag() {
        assert_eq!(checked_ref(" main ").unwrap(), "main");
        assert!(checked_ref("--all").is_err());
        assert!(checked_ref("   ").is_err());
    }

    #[test]
    fn drops_a_label_the_model_prefixed() {
        let message = split_message("Summary: Tidy the parser");
        assert_eq!(message.summary, "Tidy the parser");
    }

    fn reply(body: serde_json::Value) -> Reply {
        parse_reply(&body).unwrap()
    }

    #[test]
    fn reads_content_as_a_string_or_as_parts() {
        let plain = reply(serde_json::json!({
            "choices": [{ "message": { "content": "  hello \n" }, "finish_reason": "stop" }]
        }));
        assert_eq!(plain.content, "  hello \n");
        assert_eq!(plain.finish, "stop");

        let parts = reply(serde_json::json!({
            "choices": [{ "message": { "content": [
                { "type": "text", "text": "hel" },
                { "type": "text", "text": "lo" }
            ] } }]
        }));
        assert_eq!(parts.content, "hello");
    }

    #[test]
    fn a_null_content_with_reasoning_is_not_an_answer() {
        let r = reply(serde_json::json!({
            "choices": [{
                "message": { "content": null, "reasoning": "Let me think..." },
                "finish_reason": "length"
            }],
            "usage": { "completion_tokens_details": { "reasoning_tokens": 1500 } }
        }));
        assert_eq!(r.content, "");
        assert_eq!(r.reasoning, "Let me think...");
        assert!(r.cut_off());
        let why = r.missing("deepseek/deepseek-v4.1-flash");
        assert!(why.contains("ran out of tokens"), "{why}");
        assert!(why.contains("1500 reasoning tokens"), "{why}");
        assert!(why.contains("Max tokens"), "{why}");
    }

    #[test]
    fn reasoning_content_is_read_under_deepseeks_name_too() {
        let r = reply(serde_json::json!({
            "choices": [{
                "message": { "content": "", "reasoning_content": "thinking" },
                "finish_reason": "stop"
            }]
        }));
        assert_eq!(r.reasoning, "thinking");
        assert!(r.missing("m").contains("reasoning field"));
    }

    #[test]
    fn inline_think_tags_are_thinking_not_answer() {
        let r = reply(serde_json::json!({
            "choices": [{
                "message": { "content": "<think>\nhmm\n</think>\nAdd a thing" },
                "finish_reason": "stop"
            }]
        }));
        assert_eq!(r.content.trim(), "Add a thing");
        assert_eq!(r.reasoning, "hmm");

        // Never closed: the model ran out before it got to the answer.
        let (answer, thinking) = strip_think_tags("<think>still going");
        assert_eq!(answer, "");
        assert_eq!(thinking, "still going");
        assert_eq!(
            strip_think_tags("no tags"),
            ("no tags".to_string(), String::new())
        );
    }

    #[test]
    fn a_first_line_keeps_its_indentation() {
        let r = reply(serde_json::json!({
            "choices": [{ "message": { "content": "    let x = 1;\n    let y = 2;\n" }, "finish_reason": "stop" }]
        }));
        assert_eq!(
            resolution_of(&r, "m").unwrap(),
            vec!["    let x = 1;".to_string(), "    let y = 2;".to_string()]
        );
    }

    #[test]
    fn a_model_that_must_think_is_recognised_from_the_refusal() {
        assert!(cannot_stop_thinking(
            "Reasoning is mandatory for this endpoint and cannot be disabled."
        ));
        assert!(!cannot_stop_thinking("Invalid API key"));
        assert!(!cannot_stop_thinking(
            "reasoning.effort must be one of low, medium, high"
        ));
    }

    #[test]
    fn a_provider_error_inside_a_200_is_an_error() {
        let err = parse_reply(&serde_json::json!({
            "choices": [{ "error": { "message": "upstream overloaded" }, "finish_reason": "error" }]
        }))
        .unwrap_err();
        assert!(err.contains("upstream overloaded"), "{err}");

        let err = parse_reply(&serde_json::json!({ "id": "x" })).unwrap_err();
        assert!(err.contains("no choices"), "{err}");
    }

    #[test]
    fn an_empty_answer_says_so_without_guessing() {
        let r = reply(serde_json::json!({
            "choices": [{ "message": { "content": "" }, "finish_reason": "stop" }]
        }));
        assert_eq!(
            r.missing("m"),
            "m returned an empty answer (finish reason: stop)"
        );
    }

    fn done(content: &str, reasoning: &str) -> Reply {
        Reply {
            content: content.to_string(),
            reasoning: reasoning.to_string(),
            finish: "stop".to_string(),
            reasoning_tokens: None,
        }
    }

    #[test]
    fn the_system_prompt_asks_for_the_markers_the_parser_looks_for() {
        assert!(CONFLICT_SYSTEM.contains(RESOLUTION_OPEN));
        assert!(CONFLICT_SYSTEM.contains(RESOLUTION_CLOSE));
    }

    #[test]
    fn a_delimited_answer_is_found_wherever_the_model_put_it() {
        let block =
            "=====BEGIN RESOLUTION=====\n  let x = 1;\n  let y = 2;\n=====END RESOLUTION=====";
        let want = vec!["  let x = 1;".to_string(), "  let y = 2;".to_string()];

        assert_eq!(resolution_of(&done(block, ""), "m").unwrap(), want);
        // Commentary around it, which the model was told not to write.
        let chatty = format!("Sure, here is the merge:\n{block}\nLet me know if that helps.");
        assert_eq!(resolution_of(&done(&chatty, ""), "m").unwrap(), want);
        // Fenced, inside the markers.
        let fenced = "=====BEGIN RESOLUTION=====\n```rust\n  let x = 1;\n  let y = 2;\n```\n=====END RESOLUTION=====";
        assert_eq!(resolution_of(&done(fenced, ""), "m").unwrap(), want);
        // Stray whitespace on the marker lines.
        let loose =
            " =====BEGIN RESOLUTION===== \n  let x = 1;\n  let y = 2;\n=====END RESOLUTION=====  ";
        assert_eq!(resolution_of(&done(loose, ""), "m").unwrap(), want);
        // The whole answer landed in the reasoning field.
        assert_eq!(
            resolution_of(&done("", &format!("I will keep both.\n{block}")), "m").unwrap(),
            want
        );
    }

    #[test]
    fn an_empty_resolution_is_an_answer_when_it_is_delimited() {
        let block = "=====BEGIN RESOLUTION=====\n=====END RESOLUTION=====";
        assert_eq!(
            resolution_of(&done(block, ""), "m").unwrap(),
            Vec::<String>::new()
        );
        // But nothing at all is still nothing.
        assert!(resolution_of(&done("", ""), "m").is_err());
        assert!(resolution_of(&done("", "only thinking"), "m").is_err());
    }

    #[test]
    fn an_undelimited_answer_is_still_taken_minus_its_fence() {
        assert_eq!(
            resolution_of(&done("```\nlet x = 1;\n```", ""), "m").unwrap(),
            vec!["let x = 1;".to_string()]
        );
        // Only an opening marker: not a block, so the content stands as is.
        let half = "=====BEGIN RESOLUTION=====\nlet x = 1;";
        assert_eq!(
            resolution_of(&done(half, ""), "m").unwrap(),
            vec![
                "=====BEGIN RESOLUTION=====".to_string(),
                "let x = 1;".to_string()
            ]
        );
    }

    #[test]
    fn a_cut_off_resolution_is_refused_rather_than_written_half() {
        let mut r = done("=====BEGIN RESOLUTION=====\nlet x = 1;", "");
        r.finish = "length".to_string();
        let err = resolution_of(&r, "m").unwrap_err();
        assert!(err.contains("part way"), "{err}");

        let mut r = done("", "thinking");
        r.finish = "length".to_string();
        assert!(resolution_of(&r, "m")
            .unwrap_err()
            .contains("ran out of tokens"));
    }
}
