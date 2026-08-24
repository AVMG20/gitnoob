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
    pub commit_style: String,
}

#[derive(Serialize)]
pub struct CommitMessage {
    pub summary: String,
    pub body: String,
}

pub fn status(state: &AppState) -> AiStatus {
    let config = state.config();
    AiStatus {
        configured: config::secret_get(OPENROUTER_KEY).is_some() && config.global.ai.model.is_some(),
        model: config.global.ai.model.clone(),
        commit_style: config.global.ai.commit_style.clone(),
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
                .map(|list| {
                    list.iter()
                        .filter_map(|v| v.as_str())
                        .any(|m| m == "image")
                })
                .unwrap_or(false);

            Model {
                id: item.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                name: item.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
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

/// One chat completion round trip.
async fn complete(state: &AppState, system: &str, user: String) -> Result<String, String> {
    let config = state.config();
    let model = config
        .global
        .ai
        .model
        .clone()
        .ok_or_else(|| "No model chosen — pick one in Settings › AI".to_string())?;
    let key = key()?;

    let response = client()?
        .post(format!("{BASE}/chat/completions"))
        .bearer_auth(key)
        // OpenRouter uses these for its own attribution listings.
        .header("HTTP-Referer", "https://github.com/gitnoob")
        .header("X-Title", "gitnoob")
        .json(&serde_json::json!({
            "model": model,
            "max_tokens": config.global.ai.max_tokens,
            "reasoning": reasoning_field(&config.global.ai.reasoning),
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": user }
            ]
        }))
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
        return Err(format!("{status}: {detail}"));
    }

    body.get("choices")
        .and_then(|c| c.get(0))
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "The model returned an empty answer".to_string())
}

/// Writes a commit message from what is staged.
pub async fn commit_message(state: &AppState) -> Result<CommitMessage, String> {
    let root = state.path()?;
    // Gather everything before the await; no git handles cross it.
    let diff = git_cmd::run_checked(
        &root,
        &["diff", "--cached", "--no-color", "--unified=3", "--stat-width=200"],
    )?;
    if diff.trim().is_empty() {
        return Err("Nothing is staged, so there is nothing to describe".to_string());
    }
    let files = git_cmd::run_checked(&root, &["diff", "--cached", "--name-status"])?;
    let recent = git_cmd::run_checked(&root, &["log", "-8", "--format=%s"]).unwrap_or_default();

    let truncated = diff.chars().count() > MAX_DIFF_CHARS;
    let diff: String = diff.chars().take(MAX_DIFF_CHARS).collect();
    let style = state.config().global.ai.commit_style;

    let system = if style == "conventional" {
        COMMIT_SYSTEM_CONVENTIONAL
    } else {
        COMMIT_SYSTEM_PLAIN
    };

    let prompt = format!(
        "Files changed:\n{files}\n\nRecent commit subjects in this repository, \
         to match tone and conventions:\n{recent}\n\nStaged diff{}:\n{diff}",
        if truncated { " (truncated)" } else { "" }
    );

    let answer = complete(state, system, prompt).await?;
    Ok(split_message(&answer))
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

    let answer = complete(state, CONFLICT_SYSTEM, prompt).await?;
    Ok(strip_fences(&answer))
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
    let opens = lines.first().is_some_and(|l| l.trim_start().starts_with("```"));
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

const COMMIT_SYSTEM_PLAIN: &str = "\
You write git commit messages for a working developer. Reply with the message \
and nothing else: no preamble, no markdown, no code fences, no quotes.

Line 1 is the summary: imperative mood, no trailing period, under 72 \
characters, specific about what changed. Never start with a type prefix like \
feat: or fix: unless the repository's own recent subjects use one.

Then a blank line, then a short body of one to three sentences explaining WHY \
the change was made and anything a reviewer could not see from the diff. Skip \
the body entirely for a small, self-evident change. Do not list the files; the \
diff already says that. Do not pad with filler.";

const COMMIT_SYSTEM_CONVENTIONAL: &str = "\
You write git commit messages in Conventional Commits form. Reply with the \
message and nothing else: no preamble, no markdown, no code fences.

Line 1: `type(scope): summary` where type is one of feat, fix, docs, style, \
refactor, perf, test, build, ci, chore. The scope is optional. Imperative \
mood, no trailing period, under 72 characters total.

Then a blank line, then one to three sentences on WHY the change was made. \
Skip the body for a trivial change. Do not list files.";

const CONFLICT_SYSTEM: &str = "\
You resolve a single git merge conflict.

You are given the lines around the conflict, the common ancestor when it is \
available, and both sides. Work out what each side was trying to do and produce \
the text that keeps BOTH intentions where they are compatible. If they are \
genuinely incompatible, keep the side whose change is clearly newer or more \
specific.

Reply with the resolved lines only. No conflict markers. No markdown, no code \
fences, no commentary, no explanation. Preserve the file's existing \
indentation style and language exactly. If the right answer is to keep one side \
unchanged, output that side verbatim.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_thinking_level_becomes_an_effort() {
        assert_eq!(reasoning_field("high"), serde_json::json!({ "effort": "high" }));
        assert_eq!(
            reasoning_field("minimal"),
            serde_json::json!({ "effort": "minimal" })
        );
    }

    #[test]
    fn off_switches_thinking_off_rather_than_omitting_it() {
        assert_eq!(reasoning_field("off"), serde_json::json!({ "enabled": false }));
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
    fn drops_a_label_the_model_prefixed() {
        let message = split_message("Summary: Tidy the parser");
        assert_eq!(message.summary, "Tidy the parser");
    }
}
