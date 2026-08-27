//! The faces beside a name, fetched once and remembered.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::avatar;
use crate::config::ForgeKind;
use crate::state::AppState;

use super::*;

/// Faces already fetched this run, by profile id. `None` records a profile
/// whose forge had nothing to show, so it is not asked again on every opening.
pub(super) static FACES: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

/// The faces of every profile, so the switcher shows accounts rather than a
/// list of names.
///
/// One request per profile, once per run: the menu is opened often and the
/// answer does not change between openings.
pub async fn faces(state: &AppState) -> HashMap<String, String> {
    let profiles: Vec<(String, ForgeKind, String, String)> = {
        let config = state.config();
        config
            .profiles
            .iter()
            .filter(|profile| profile.forge != ForgeKind::None)
            .filter_map(|profile| {
                let token = config::secret_get(&config::forge_key(&profile.id))?;
                let host = if profile.host.is_empty() {
                    profile.forge.default_host().to_string()
                } else {
                    profile.host.clone()
                };
                Some((profile.id.clone(), profile.forge, host, token))
            })
            .collect()
    };

    let mut found = HashMap::new();
    for (id, kind, host, token) in profiles {
        let known = FACES
            .lock()
            .unwrap()
            .as_ref()
            .and_then(|seen| seen.get(&id).cloned());
        if let Some(known) = known {
            if let Some(picture) = known {
                found.insert(id, picture);
            }
            continue;
        }
        let picture = one_face(kind, &host, &token).await;
        FACES
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(id.clone(), picture.clone());
        if let Some(picture) = picture {
            found.insert(id, picture);
        }
    }
    found
}

pub(super) async fn one_face(kind: ForgeKind, host: &str, token: &str) -> Option<String> {
    let base = api_base(kind, host);
    let body: serde_json::Value = client()
        .ok()?
        .get(format!("{base}/user"))
        .bearer_auth(token)
        .send()
        .await
        .ok()
        .filter(|response| response.status().is_success())?
        .json()
        .await
        .ok()?;
    let url = body
        .get("avatar_url")?
        .as_str()
        .filter(|url| !url.is_empty())?;
    avatar::from_url(url).await
}

/// Faces already fetched this run, by URL. Each one is a request and a base64
/// blob, and a project's reviews name the same few people over and over.
pub(super) static PEOPLE: Mutex<Option<HashMap<String, Option<String>>>> = Mutex::new(None);

/// A picture for somebody a review names, fetched at most once per URL.
pub(super) async fn face(url: &str) -> Option<String> {
    if url.is_empty() {
        return None;
    }
    if let Some(known) = PEOPLE.lock().unwrap().as_ref().and_then(|map| map.get(url)) {
        return known.clone();
    }
    let found = avatar::from_url(url).await;
    PEOPLE
        .lock()
        .unwrap()
        .get_or_insert_with(HashMap::new)
        .insert(url.to_string(), found.clone());
    found
}

/// Reads one person out of a forge's JSON, whichever forge wrote it, along with
/// where their picture lives.
///
/// GitHub gives an account a `login` and nothing else here; GitLab gives a
/// `username` and a display name. Taking both and falling back keeps one
/// reader for the two shapes. Kept apart from fetching the picture so the
/// reading can be checked without a network.
pub(super) fn read_person(value: &serde_json::Value) -> (Person, String) {
    let login = {
        let github = string(value, &["login"]);
        if github.is_empty() {
            string(value, &["username"])
        } else {
            github
        }
    };
    let name = {
        let given = string(value, &["name"]);
        if given.is_empty() {
            login.clone()
        } else {
            given
        }
    };
    (
        Person {
            login,
            name,
            avatar: None,
        },
        string(value, &["avatar_url"]),
    )
}

/// One person, with their picture fetched.
pub(super) async fn person(value: &serde_json::Value) -> Person {
    let (mut read, picture) = read_person(value);
    read.avatar = face(&picture).await;
    read
}

/// The same, for the arrays of people a review carries. Anyone the forge lists
/// without an account name is not somebody to show.
pub(super) async fn people(value: Option<&serde_json::Value>) -> Vec<Person> {
    let Some(items) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        if read_person(item).0.login.is_empty() {
            continue;
        }
        out.push(person(item).await);
    }
    out
}

/// A review's labels. GitHub writes its colours without the `#`; GitLab writes
/// them with one, and only when asked for the detailed form.
pub(super) fn labels(value: Option<&serde_json::Value>) -> Vec<Label> {
    let Some(items) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(|item| {
            // GitLab without `with_labels_details` gives bare strings.
            if let Some(name) = item.as_str() {
                return Some(Label {
                    name: name.to_string(),
                    color: String::new(),
                });
            }
            let name = string(item, &["name"]);
            if name.is_empty() {
                return None;
            }
            let color = string(item, &["color"]);
            let color = if color.is_empty() || color.starts_with('#') {
                color
            } else {
                format!("#{color}")
            };
            Some(Label { name, color })
        })
        .collect()
}
