use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use git2::{Branch, BranchType, Oid, Repository, Revwalk, Sort};
use serde::Serialize;

use crate::refs;
use crate::state::AppState;

#[derive(Serialize)]
pub struct RefLabel {
    pub kind: String,
    pub name: String,
    /// The checked-out branch, or a detached HEAD.
    pub head: bool,
}

/// A single line segment to draw in one row's graph cell.
///
/// `y` is expressed in thirds of a row so the frontend never has to know the
/// layout rules: 0 is the top edge, 1 the centre (where the node sits), 2 the
/// bottom edge.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub x1: usize,
    pub y1: u8,
    pub x2: usize,
    pub y2: u8,
    pub color: usize,
    /// A line that is not history, drawn broken: a stash hanging off the
    /// commit it was made on, or a branch whose work reached another line by a
    /// squash or a rebase rather than by a merge.
    pub dashed: bool,
    /// A line the upstream has and no local branch does yet. Drawn held back,
    /// so what is still to be pulled reads as sitting above where you stand
    /// rather than as part of it.
    pub faint: bool,
    /// Part of the line HEAD is on, from the commit you stand on down to where
    /// that line runs into another. Drawn a shade brighter than the rest.
    pub current: bool,
}

#[derive(Serialize)]
pub struct GraphRow {
    pub oid: String,
    pub short: String,
    pub summary: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    pub parents: Vec<String>,
    pub lane: usize,
    pub color: usize,
    pub width: usize,
    pub segments: Vec<Segment>,
    pub labels: Vec<RefLabel>,
    /// On a local branch but not yet on its upstream. Drawn hollow, so the
    /// boundary between what the remote has and what it does not is visible in
    /// the graph rather than only in an ahead count.
    pub unpushed: bool,
    /// On the upstream of a local branch but on no local branch yet: what a
    /// pull would bring in. The mirror of `unpushed`.
    pub unpulled: bool,
    /// Branch tips whose work this commit carries without being a merge of
    /// them: the result of a squash merge, or the last of a rebase. Git records
    /// no link between the two, so it is found by comparing what they change.
    pub carries: Vec<String>,
    /// Which stash this row is, when it is one. A stash is a commit, so it
    /// draws like one — with its own mark, on a broken line to the commit it
    /// was made on.
    pub stash: Option<usize>,
}

#[derive(Serialize)]
pub struct GraphPage {
    pub rows: Vec<GraphRow>,
    /// True when the walk stopped at `limit` and older commits remain.
    pub has_more: bool,
}

/// The walk the graph draws: every ref, newest first, a branch's commits kept
/// together.
///
/// Shared with `depth`, which has to count the very same rows in the very same
/// order or the number it reports is not the row the graph puts the commit on.
fn walk(repo: &Repository) -> Result<Revwalk<'_>, String> {
    let mut walk = repo.revwalk().map_err(err)?;
    // Topological order keeps a branch's commits contiguous; the time secondary
    // sort keeps the result close to what the user expects to read.
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME)
        .map_err(err)?;
    // Push every ref, not just HEAD, so unmerged branches are visible.
    walk.push_glob("refs/heads/*").map_err(err)?;
    let _ = walk.push_glob("refs/remotes/*");
    let _ = walk.push_glob("refs/tags/*");
    let _ = walk.push_head();
    Ok(walk)
}

/// How far back a commit sits in the same walk the graph draws.
///
/// The graph loads a page at a time, and in a repository with tens of
/// thousands of commits a branch tip is very often older than the page: the
/// row is simply not there to scroll to, so clicking the branch appears to do
/// nothing. Answering how deep it is lets the frontend load exactly enough to
/// have the row before it asks for it.
///
/// `None` means the commit is not in the walk at all — nothing points at it,
/// or it is deeper than any page anybody should be asked to wait for.
pub fn depth(state: &AppState, oid: &str) -> Result<Option<usize>, String> {
    let repo = state.repo()?;
    let wanted = Oid::from_str(oid).map_err(err)?;

    for (index, found) in walk(&repo)?.enumerate() {
        if index >= DEPTH_CAP {
            break;
        }
        if found.map_err(err)? == wanted {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

/// Walks history and assigns each commit a lane, producing ready-to-draw line
/// segments.
///
/// The lane table holds, per column, the commit that column is currently
/// waiting to reach. A commit claims the column that was waiting for it (or a
/// free one if it is the tip of a line), then hands that column to its first
/// parent and allocates columns for any further parents. Because every segment
/// is derived from the lane table immediately before and after that step, a row
/// can be drawn in isolation — which is what makes the list cheap to virtualize.
pub fn build(state: &AppState, limit: usize) -> Result<GraphPage, String> {
    let repo = state.repo()?;
    let labels = refs::labels_by_oid(&repo);
    let Against { unpushed, unpulled } = against_upstreams(&repo, limit);

    // Read the commits first, then work out the picture. The two are separate
    // jobs, and only the second one is hard enough to be worth testing on
    // histories that would be a chore to build a repository for.
    let mut commits: Vec<Commit> = Vec::with_capacity(limit.min(4096));
    let mut has_more = false;
    for oid in walk(&repo)? {
        let oid = oid.map_err(err)?;
        if commits.len() >= limit {
            has_more = true;
            break;
        }
        let commit = repo.find_commit(oid).map_err(err)?;
        let author = commit.author();
        commits.push(Commit {
            oid,
            parents: commit.parent_ids().collect(),
            summary: commit.summary().unwrap_or("").to_string(),
            author: author.name().unwrap_or("").to_string(),
            email: author.email().unwrap_or("").to_string(),
            time: commit.time().seconds(),
            unpulled: unpulled.contains(&oid),
            carries: Vec::new(),
            stash: None,
        });
    }

    let anchor = trunk_tip(&repo);
    let mut commits = with_stashes(state, &repo, commits);
    link_folded(&repo, &mut commits, anchor, &labels);
    let head = repo.head().ok().and_then(|h| h.target());
    let plotted = plot(&commits, anchor, head);

    let rows = commits
        .into_iter()
        .zip(plotted)
        .map(|(commit, place)| GraphRow {
            oid: commit.oid.to_string(),
            short: commit.oid.to_string()[..7].to_string(),
            summary: commit.summary,
            author: commit.author,
            email: commit.email,
            time: commit.time,
            parents: commit.parents.iter().map(|p| p.to_string()).collect(),
            lane: place.lane,
            color: place.color,
            width: place.width,
            segments: place.segments,
            labels: labels
                .get(&commit.oid.to_string())
                .map(|v| {
                    v.iter()
                        .map(|d| RefLabel {
                            kind: d.kind.clone(),
                            name: d.name.clone(),
                            head: d.head,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            unpushed: unpushed.contains(&commit.oid),
            unpulled: commit.unpulled,
            carries: commit.carries.iter().map(|p| p.to_string()).collect(),
            stash: commit.stash,
        })
        .collect();

    Ok(GraphPage { rows, has_more })
}

/// Puts each stash into the list directly above the commit it was made on.
///
/// A stash is a commit whose first parent is where HEAD stood at the time, so
/// it belongs in the picture hanging off that row — which is what makes it
/// findable at all. Its other parents (the index, and the untracked files) are
/// bookkeeping rather than history, so only the first is kept: nothing should
/// draw a line to them.
///
/// A stash whose parent is older than this page is left out. A line to a row
/// that is not there is a line to nowhere.
fn with_stashes(state: &AppState, repo: &Repository, commits: Vec<Commit>) -> Vec<Commit> {
    let Ok(entries) = crate::work::stash_list(state) else {
        return commits;
    };
    if entries.is_empty() {
        return commits;
    }

    // Keyed by the commit each one hangs off, newest first within a parent so
    // `stash@{0}` sits closest to the top.
    let mut hanging: HashMap<Oid, Vec<Commit>> = HashMap::new();
    for entry in entries {
        let Ok(oid) = Oid::from_str(&entry.oid) else {
            continue;
        };
        let Ok(commit) = repo.find_commit(oid) else {
            continue;
        };
        let Some(parent) = commit.parent_ids().next() else {
            continue;
        };
        let author = commit.author();
        hanging.entry(parent).or_default().push(Commit {
            oid,
            parents: vec![parent],
            // The tidied message from the list, not git's own "WIP on main:
            // 0e8a1f2 …", which says nothing the row does not already say.
            summary: entry.message,
            author: author.name().unwrap_or("").to_string(),
            email: author.email().unwrap_or("").to_string(),
            time: entry.time,
            unpulled: false,
            carries: Vec::new(),
            stash: Some(entry.index),
        });
    }

    let mut out = Vec::with_capacity(commits.len() + hanging.len());
    for commit in commits {
        if let Some(stashes) = hanging.remove(&commit.oid) {
            out.extend(stashes);
        }
        out.push(commit);
    }
    out
}

/// A commit as the plotter needs it: an id, its parents, and the details that
/// travel with the row.
pub struct Commit {
    pub oid: Oid,
    pub parents: Vec<Oid>,
    pub summary: String,
    pub author: String,
    pub email: String,
    pub time: i64,
    /// On an upstream and on no local branch: not pulled yet.
    pub unpulled: bool,
    /// Branch tips this commit is the squash or the rebase of. Drawn as a
    /// broken line from here to each, the way a merge's second parent is drawn
    /// solid.
    pub carries: Vec<Oid>,
    /// Set when this row is a stash rather than a commit of the history.
    pub stash: Option<usize>,
}

/// Where one row's node sits and what lines cross it.
pub struct Place {
    pub lane: usize,
    pub color: usize,
    pub width: usize,
    pub segments: Vec<Segment>,
}

/// One column of the picture: what it is waiting for and how its line looks.
///
/// The look travels with the column rather than with the commit that set it,
/// because a line is drawn one row at a time and the rows a line merely passes
/// through know nothing about the commit it left. A stash's line has to stay
/// broken all the way down to the commit the stash hangs off, and the line of
/// commits still to be pulled has to stay held back until it lands on the one
/// you have.
#[derive(Clone, Default)]
struct Lane {
    /// The commit this column is waiting to reach; `None` for an empty column,
    /// which keeps the colour it last wore so it is not handed out twice.
    waiting: Option<Oid>,
    color: usize,
    dashed: bool,
    faint: bool,
    current: bool,
}

impl Lane {
    fn clear(&mut self) {
        let color = self.color;
        *self = Lane {
            color,
            ..Lane::default()
        };
    }

    /// A segment of this column's line between two points of a row.
    fn segment(&self, x1: usize, y1: u8, x2: usize, y2: u8) -> Segment {
        Segment {
            x1,
            y1,
            x2,
            y2,
            color: self.color,
            dashed: self.dashed,
            faint: self.faint,
            current: self.current,
        }
    }
}

/// Turns a list of commits, newest first, into the lines to draw.
///
/// `anchor` is the commit whose line owns the leftmost column — the trunk's
/// tip. `head` is the commit you are standing on: its line is marked from there
/// down, so the frontend can bring it forward.
fn plot(commits: &[Commit], anchor: Option<Oid>, head: Option<Oid>) -> Vec<Place> {
    let mut lanes: Vec<Lane> = Vec::new();
    let mut next_color = 0usize;
    let mut places: Vec<Place> = Vec::with_capacity(commits.len());

    // The trunk takes the leftmost lane, whatever order the walk happens to
    // reach it in. Left to first come first served, a branch whose tip is merely
    // newer takes the trunk's column, and the trunk is then drawn stepping
    // sideways around it on its way down — which reads as the branch and the
    // trunk swapping places rather than as a branch leaving.
    //
    // It is the trunk that is anchored here and not HEAD, because HEAD moves.
    // Pinning the column to wherever you are standing means the whole graph
    // slides sideways, redrawing history that has not changed, every time you
    // stand somewhere else: a checkout, or a commit that makes your branch the
    // newest one. Pinning it to `main` gives every other branch a fixed edge to
    // be read against, and leaves the picture still when only you have moved.
    //
    // The lane is only reserved: nothing is drawn in it until the commit itself
    // turns up, since above that row there is no line to draw.
    let mut reserved = None;
    if let Some(anchor) = anchor {
        lanes.push(Lane {
            waiting: Some(anchor),
            color: TRUNK_COLOR,
            ..Lane::default()
        });
        reserved = Some(0);
    }

    for commit in commits {
        let oid = commit.oid;

        let before = lanes.clone();
        let reserved_before = reserved;

        // 1. Claim a lane. Scanning from the left means a commit that both the
        //    trunk and a branch built on it are waiting for is drawn on the
        //    trunk, which is the line it belongs to.
        let lane = match lanes.iter().position(|l| l.waiting == Some(oid)) {
            Some(i) => i,
            None => {
                let i = open(&mut lanes, &mut next_color);
                lanes[i].waiting = Some(oid);
                i
            }
        };
        let color = lanes[lane].color;
        {
            let here = &mut lanes[lane];
            // The line leaving this row is broken if a stash is what is
            // leaving, and held back if the commit is one you do not have yet.
            // An ordinary commit taking the lane over makes it plain again.
            here.dashed = commit.stash.is_some();
            here.faint = commit.unpulled;
            // Your line is bright from where you stand downwards. Not above:
            // what sits above HEAD is the rest of the branch, or the upstream's
            // commits, and neither is where you are.
            if head == Some(oid) {
                here.current = true;
            }
        }
        if reserved == Some(lane) {
            reserved = None;
        }

        // 2. Release any other lane that was also waiting for this commit —
        //    several children merging back into one line.
        for (i, slot) in lanes.iter_mut().enumerate() {
            if i != lane && slot.waiting == Some(oid) {
                slot.clear();
            }
        }

        // 3. Hand the lane to the first parent, and give every other parent a
        //    lane of its own unless it is already tracked.
        let parents = &commit.parents;
        match parents.split_first() {
            None => lanes[lane].clear(),
            Some((first, rest)) => {
                // The lane carries on to the first parent even when another
                // lane is already waiting for that same commit. Both hold it
                // until the row it lands on, where they meet at its node —
                // which is the row the branch actually rejoined. Collapsing the
                // duplicate here instead would move the join a row early and
                // draw the branch sliding into its neighbour's column before
                // there is anything there to join.
                lanes[lane].waiting = Some(*first);
                for parent in rest {
                    if lanes.iter().any(|l| l.waiting == Some(*parent)) {
                        continue;
                    }
                    let i = open(&mut lanes, &mut next_color);
                    lanes[i].waiting = Some(*parent);
                    lanes[i].faint = commit.unpulled;
                }
            }
        }
        // A branch this commit is the squash or rebase of gets a lane exactly
        // as a merge's second parent does, on a broken line: the work came
        // from there, but git has no record of it.
        for tip in &commit.carries {
            if lanes.iter().any(|l| l.waiting == Some(*tip)) {
                continue;
            }
            let i = open(&mut lanes, &mut next_color);
            lanes[i].waiting = Some(*tip);
            lanes[i].dashed = true;
            lanes[i].faint = commit.unpulled;
        }

        trim(&mut lanes);

        // 4. Turn the before/after tables into segments.
        // Where a line ends up, preferring the lane it is already in. Two lines
        // waiting for the same commit both hold it, so a plain search finds
        // whichever comes first and would draw a line that is staying put as
        // stepping into its neighbour.
        let find = |want: &Oid, prefer: usize| -> Option<usize> {
            if lanes.get(prefer).and_then(|slot| slot.waiting.as_ref()) == Some(want) {
                return Some(prefer);
            }
            lanes
                .iter()
                .position(|slot| slot.waiting.as_ref() == Some(want))
        };

        let mut segments = Vec::new();
        for (x, was) in before.iter().enumerate() {
            // A lane still only reserved holds a commit the walk has not
            // reached, so there is no line above this row to come down from.
            if reserved_before == Some(x) {
                continue;
            }
            let Some(waiting) = was.waiting else { continue };
            if waiting == oid {
                // Incoming line ends at this row's node.
                segments.push(was.segment(x, 0, lane, 1));
            } else if let Some(to) = find(&waiting, x) {
                // A line that just passes this row by.
                segments.push(was.segment(x, 0, to, 2));
            }
        }
        for (i, parent) in parents.iter().enumerate() {
            let Some(to) = find(parent, if i == 0 { lane } else { usize::MAX }) else {
                continue;
            };
            segments.push(Segment {
                x1: lane,
                y1: 1,
                x2: to,
                y2: 2,
                // The first parent continues this commit's line, so it keeps its
                // colour and its brightness; a merge's other parents belong to
                // the lines they join.
                color: if i == 0 { color } else { lanes[to].color },
                dashed: commit.stash.is_some(),
                faint: commit.unpulled,
                current: i == 0 && lanes[lane].current,
            });
        }
        for tip in &commit.carries {
            let Some(to) = find(tip, usize::MAX) else {
                continue;
            };
            segments.push(Segment {
                x1: lane,
                y1: 1,
                x2: to,
                y2: 2,
                color: lanes[to].color,
                dashed: true,
                faint: commit.unpulled,
                current: false,
            });
        }

        // A lane that something has just merged into is no longer merely
        // reserved: there is a line in it from this row down to the commit it
        // is waiting for, and the rows in between have to draw it. Without
        // this, a merge into the checked-out branch from above puts a line into
        // the lane and every row below hides it, so the line stops in mid-air.
        if let Some(kept) = reserved {
            if segments.iter().any(|s| s.x2 == kept && s.y2 == 2) {
                reserved = None;
            }
        }

        let width = before.len().max(lanes.len()).max(lane + 1);

        // Lines that live entirely past the right-hand edge of the column are
        // never seen, and a repository with a few hundred remote branches has
        // far more of those than of the ones on show — fifty-odd segments a row
        // where a dozen are drawn. Dropping them here rather than in the
        // frontend keeps them out of the payload as well as out of the
        // document. A line with one end inside the edge is kept whole: it is
        // drawn running off the side, which is what it does.
        segments.retain(|s| s.x1 < DRAWN_LANES || s.x2 < DRAWN_LANES);

        places.push(Place {
            lane,
            color,
            width,
            segments,
        });
    }

    places
}

/// What the local branches and their upstreams each have that the other does
/// not.
struct Against {
    /// On a local branch but not on its upstream.
    unpushed: HashSet<Oid>,
    /// On an upstream but on no local branch at all.
    unpulled: HashSet<Oid>,
}

/// The commits a local branch has and its upstream does not, and the other way
/// round.
///
/// An ahead count says how many there are; this says which, so the graph can
/// draw the boundary between what the remote knows about and what is still only
/// here — and, the other way, between what is here and what a pull would bring.
/// Branches with no upstream contribute nothing: everything on them is unpushed
/// in a sense, but there is nowhere it was meant to go.
///
/// Unpushed is measured branch by branch, because it is a statement about one
/// branch and its own remote. Unpulled is measured against every local branch
/// at once: a commit the upstream has is only "not here yet" if no branch here
/// has it, and a topic branch rebased onto a fresher `origin/main` than the
/// local `main` has every one of those commits already.
fn against_upstreams(repo: &Repository, limit: usize) -> Against {
    let mut unpushed = HashSet::new();
    let mut unpulled = HashSet::new();
    let Ok(branches) = repo.branches(Some(BranchType::Local)) else {
        return Against { unpushed, unpulled };
    };

    let mut locals = Vec::new();
    let mut upstreams = Vec::new();
    for (branch, _) in branches.flatten() {
        let Some(local) = branch.get().target() else {
            continue;
        };
        locals.push(local);
        let Some(upstream) = branch.upstream().ok().and_then(|u| u.get().target()) else {
            continue;
        };
        if local == upstream {
            continue;
        }
        upstreams.push(upstream);

        let Ok(mut walk) = repo.revwalk() else {
            continue;
        };
        if walk.push(local).is_err() || walk.hide(upstream).is_err() {
            continue;
        }
        unpushed.extend(walk.flatten().take(limit));
    }

    if !upstreams.is_empty() {
        if let Ok(mut walk) = repo.revwalk() {
            for upstream in upstreams {
                let _ = walk.push(upstream);
            }
            for local in locals {
                let _ = walk.hide(local);
            }
            unpulled.extend(walk.flatten().take(limit));
        }
    }

    Against { unpushed, unpulled }
}

/// The tip of the branch this repository is organised around.
///
/// The branch the sidebar was told about, or `main` and `master` looked for
/// locally and then on a remote, so a clone that has never checked the default
/// branch out still has a trunk. A repository that uses neither name falls back
/// to whatever HEAD is on: the column is then no steadier than HEAD is, but it
/// is still steadier than handing it to whichever branch was committed to last.
///
/// A local branch that has fallen behind its upstream anchors the column at the
/// upstream's tip instead. Anchored at the local tip, the commits still to be
/// pulled were the one line in the picture that could not have the column,
/// and were drawn as a branch off to the side that rejoined at your commit —
/// the shape of a merge, for a fast-forward. Anchored where the line actually
/// ends, they sit straight above you, which is what they are.
fn trunk_tip(repo: &Repository) -> Option<Oid> {
    if let Some(name) = refs::trunk_of(repo) {
        if let Ok(branch) = repo.find_branch(&name, BranchType::Local) {
            return furthest(repo, &branch);
        }
        if let Ok(branch) = repo.find_branch(&name, BranchType::Remote) {
            return branch.get().target();
        }
    }
    let head = repo.head().ok()?;
    if head.is_branch() {
        return furthest(repo, &Branch::wrap(head));
    }
    head.target()
}

/// A local branch's tip, or its upstream's when that is strictly ahead of it.
fn furthest(repo: &Repository, branch: &Branch) -> Option<Oid> {
    let local = branch.get().target()?;
    let upstream = branch
        .upstream()
        .ok()
        .and_then(|u| u.get().target())
        .or_else(|| {
            // No upstream set, but a remote copy under the same name is what
            // a fetch would have brought, and what "behind" is measured
            // against in the sidebar.
            let name = branch.name().ok().flatten()?;
            repo.find_branch(&format!("origin/{name}"), BranchType::Remote)
                .ok()
                .and_then(|b| b.get().target())
        });
    match upstream {
        Some(up) if up != local && repo.graph_descendant_of(up, local).unwrap_or(false) => Some(up),
        _ => Some(local),
    }
}

// --- squash and rebase merges ------------------------------------------------

/// Finds the branches whose work landed somewhere without a merge commit, and
/// links each to the commit that carries it.
///
/// A merge leaves a commit with two parents and the picture draws the join. A
/// squash leaves one commit with one parent, and a rebase leaves copies with
/// new ids: git keeps no record of where either came from, so the branch is
/// drawn dangling as though nothing had happened to it — and its work is
/// somewhere in the trunk with nothing to say so.
///
/// What a squash does keep is the change itself. The diff from a branch's fork
/// point to its tip and the diff the squash commit makes are the same patch,
/// and git's own `patch-id` is a hash of a patch that ignores where in the file
/// it landed and what the message said. So every branch tip nothing is built
/// on has its patch measured, and every single-parent commit near it is asked
/// whether it makes the same one. A rebase is the same question one commit at
/// a time: when every commit on the branch has a copy, the branch is linked to
/// the newest copy.
///
/// Only the commits on this page are looked at, and only against the trunk's
/// fork point: a branch squashed into some other branch is matched when that
/// branch had nothing of its own at the time, which is the usual case, and
/// left alone otherwise. A wrong link is worse than a missing one.
fn link_folded(
    repo: &Repository,
    commits: &mut [Commit],
    anchor: Option<Oid>,
    labels: &HashMap<String, Vec<refs::Decoration>>,
) {
    let Some(anchor) = anchor else { return };

    // A tip is a commit a branch points at that nothing on the page has as a
    // parent. Anything built on is a branch still going, not one folded away.
    let has_child: HashSet<Oid> = commits
        .iter()
        .flat_map(|c| c.parents.iter().copied())
        .collect();
    let tips: Vec<usize> = commits
        .iter()
        .enumerate()
        .filter(|(_, c)| c.stash.is_none() && !has_child.contains(&c.oid) && c.oid != anchor)
        .filter(|(_, c)| {
            labels
                .get(&c.oid.to_string())
                .is_some_and(|v| v.iter().any(|d| d.kind == "local" || d.kind == "remote"))
        })
        .map(|(at, _)| at)
        .collect();
    if tips.is_empty() {
        return;
    }

    let links = {
        let rows: &[Commit] = commits;
        // The patch each single-parent commit makes, worked out once per row
        // however many tips ask.
        let mut memo: HashMap<usize, Option<Oid>> = HashMap::new();
        let mut patch_of = |at: usize| -> Option<Oid> {
            *memo.entry(at).or_insert_with(|| {
                let commit = &rows[at];
                match (commit.stash, commit.parents.as_slice()) {
                    (None, [parent]) => patch_id(repo, *parent, commit.oid),
                    _ => None,
                }
            })
        };

        let mut links: Vec<(usize, Oid)> = Vec::new();
        for tip in tips {
            let tip_oid = rows[tip].oid;
            let Ok(base) = repo.merge_base(anchor, tip_oid) else {
                continue;
            };
            // Already in the trunk the ordinary way, or ahead of it: nothing
            // to find.
            if base == tip_oid || base == anchor {
                continue;
            }
            // Where the copy could be: above the tip, nearest first. A squash
            // is made after the work it squashes, and the walk puts a newer
            // commit above an older one it does not descend from. Nothing
            // below is looked at: the branch's own commits are there, and a
            // one-commit branch makes the same patch its squash would.
            let near: Vec<usize> = (tip.saturating_sub(FOLD_REACH)..tip).rev().collect();

            // The whole branch as one patch, which is what a squash makes of it.
            if let Some(wanted) = patch_id(repo, base, tip_oid) {
                if let Some(&at) = near.iter().find(|&&at| patch_of(at) == Some(wanted)) {
                    links.push((at, tip_oid));
                    continue;
                }
            }

            // Or one commit at a time, which is what a rebase makes of it.
            // Bounded: a branch a hundred commits long is not one anybody
            // rebase-merged.
            let Some(own) = commits_between(repo, base, tip_oid, REBASE_CAP) else {
                continue;
            };
            let mut left: HashSet<Oid> = own
                .iter()
                .filter_map(|&oid| repo.find_commit(oid).ok())
                .filter(|c| c.parent_count() == 1)
                .filter_map(|c| patch_id(repo, c.parent_id(0).ok()?, c.id()))
                .collect();
            if left.len() != own.len() {
                continue;
            }
            let mut newest: Option<usize> = None;
            for &at in &near {
                let Some(id) = patch_of(at) else { continue };
                if left.remove(&id) {
                    newest = Some(newest.map_or(at, |best| best.min(at)));
                    if left.is_empty() {
                        break;
                    }
                }
            }
            if let (true, Some(at)) = (left.is_empty(), newest) {
                links.push((at, tip_oid));
            }
        }
        links
    };

    for (at, tip) in links {
        commits[at].carries.push(tip);
    }
}

/// The commits on `tip`'s side of `base`, or `None` when there are more than
/// `cap` of them.
fn commits_between(repo: &Repository, base: Oid, tip: Oid, cap: usize) -> Option<Vec<Oid>> {
    let mut walk = repo.revwalk().ok()?;
    walk.push(tip).ok()?;
    walk.hide(base).ok()?;
    let mut out = Vec::new();
    for oid in walk.flatten() {
        if out.len() >= cap {
            return None;
        }
        out.push(oid);
    }
    Some(out)
}

/// The patch-id of the change from one commit's tree to another's, or `None`
/// when the two trees are the same: an empty patch matches every other empty
/// patch, and says nothing.
///
/// Remembered across calls, process-wide. A commit's content never changes, so
/// neither does the answer, and the graph is rebuilt after every action: the
/// diffs are only ever taken once per commit rather than once per refresh.
fn patch_id(repo: &Repository, from: Oid, to: Oid) -> Option<Oid> {
    static CACHE: OnceLock<PatchCache> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(known) = cache.lock().ok().and_then(|c| c.get(&(from, to)).copied()) {
        return known;
    }

    let old = repo.find_commit(from).ok()?.tree().ok()?;
    let new = repo.find_commit(to).ok()?.tree().ok()?;
    let diff = repo.diff_tree_to_tree(Some(&old), Some(&new), None).ok()?;
    let id = if diff.deltas().len() == 0 {
        None
    } else {
        diff.patchid(None).ok()
    };

    if let Ok(mut cache) = cache.lock() {
        // Bounded rather than forever: a session that has walked a very large
        // history has walked it, and starting over costs one refresh.
        if cache.len() >= PATCH_CACHE {
            cache.clear();
        }
        cache.insert((from, to), id);
    }
    id
}

/// How many lanes the frontend has room for. Kept in step with `MAX_LANES`
/// there, which is where the column's width is actually decided: the svg is
/// drawn that many lanes wide whatever the user does to the column, so a line
/// beyond it has nowhere to appear.
///
/// Everything past the last lane is drawn *in* the last lane, so a repository
/// with more branches in flight than there are lanes ends with several unrelated
/// lines collapsed into one column. Set well past the point where a graph is
/// still readable so that the picture runs out of usable width before it runs
/// out of lanes. There is no matching limit on colour: the palette is walked
/// with a modulo, so lanes beyond it wear a shade already in use rather than
/// having none.
const DRAWN_LANES: usize = 28;

/// How far the search for a commit's row will walk before giving up. Counting
/// oids is cheap; a page big enough to hold a row this far back is not, so
/// there is no point finding one.
const DEPTH_CAP: usize = 100_000;

/// The longest branch worth checking commit by commit for a rebase.
const REBASE_CAP: usize = 48;

/// How many rows above a branch tip are asked whether they carry its work.
/// Far enough that a branch squashed a page ago is still found; near enough
/// that a page set to thousands of rows does not diff every one of them for
/// every dangling tip on the first load.
const FOLD_REACH: usize = 500;

/// How many patch-ids to remember before starting over.
const PATCH_CACHE: usize = 100_000;

/// The patch-id of the change between two commits, by the pair.
type PatchCache = Mutex<HashMap<(Oid, Oid), Option<Oid>>>;

/// How many colours the frontend cycles through. Kept in step with
/// `LANE_COLORS` there, which is where they are actually chosen.
const PALETTE: usize = 10;
/// The trunk's colour. Nothing else is given it while the trunk is alive — the
/// leftmost lane holds a commit from before the first row onwards, so it is
/// always in the "already taken" set below — and so the column that never moves
/// never changes shade either.
const TRUNK_COLOR: usize = 0;

/// Picks a colour for a lane that has just opened.
///
/// Walking the palette in order keeps neighbours apart until it wraps, at which
/// point two lines running side by side can come out the same shade — which is
/// exactly when the colour was carrying the most weight. So the search steps
/// past any colour a live lane is already wearing, and only repeats one when
/// there are more lines on screen than there are colours to tell them apart.
fn pick_color(lanes: &[Lane], next: &mut usize) -> usize {
    let taken: HashSet<usize> = lanes
        .iter()
        .filter(|lane| lane.waiting.is_some())
        .map(|lane| lane.color)
        .collect();

    for step in 0..PALETTE {
        let candidate = (*next + step) % PALETTE;
        if !taken.contains(&candidate) {
            *next = candidate + 1;
            return candidate;
        }
    }
    let candidate = *next % PALETTE;
    *next += 1;
    candidate
}

/// Opens a lane for a line that is starting: the first empty one, or a new one
/// on the right, freshly coloured and otherwise plain.
///
/// The trunk's column is never among them: it holds the trunk's tip from before
/// the first row and the trunk's own history from then on, so it is never empty
/// for anything else to move into.
///
/// Coloured while it is still empty on purpose: an empty lane still carries
/// whatever colour last ran through it, and a lane counted as live would rule
/// that colour out for the line about to take the lane over.
fn open(lanes: &mut Vec<Lane>, next_color: &mut usize) -> usize {
    let i = match lanes.iter().position(|l| l.waiting.is_none()) {
        Some(i) => i,
        None => {
            lanes.push(Lane::default());
            lanes.len() - 1
        }
    };
    let color = pick_color(lanes, next_color);
    lanes[i].color = color;
    i
}

/// Drops trailing empty lanes so the graph column stays as narrow as the history
/// actually needs.
fn trim(lanes: &mut Vec<Lane>) {
    while lanes.last().is_some_and(|l| l.waiting.is_none()) {
        lanes.pop();
    }
}

fn err(e: git2::Error) -> String {
    e.message().to_string()
}

#[cfg(test)]
mod tests {
    use std::path::Path;
    use std::process::Command;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// The number `depth` reports has to be the row `build` draws, or the graph
    /// loads a page that still does not reach the commit.
    #[test]
    fn a_commit_is_found_at_the_row_the_graph_puts_it_on() {
        let root = std::env::temp_dir().join(format!("gitnoob-depth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        git(&root, &["init", "--quiet", "--initial-branch=main"]);
        git(&root, &["config", "user.email", "test@example.com"]);
        git(&root, &["config", "user.name", "Test"]);

        let mut old = String::new();
        for step in 0..12 {
            std::fs::write(root.join("log.txt"), format!("{step}\n")).unwrap();
            git(&root, &["add", "-A"]);
            git(
                &root,
                &["commit", "--quiet", "-m", &format!("commit {step}")],
            );
            // A branch left behind partway, the shape that goes missing from a
            // page in a repository big enough for one.
            if step == 3 {
                git(&root, &["branch", "left-behind"]);
                old = git(&root, &["rev-parse", "HEAD"]).trim().to_string();
            }
        }

        let state = AppState::new(root.join("config"));
        state.set_path(root.clone());

        let page = build(&state, 100).unwrap();
        let drawn = page.rows.iter().position(|row| row.oid == old).unwrap();
        assert_eq!(depth(&state, &old).unwrap(), Some(drawn));
        // A page that stops short of it is exactly the case being fixed: the
        // row is missing, and the depth is what says how far to reach.
        let short = build(&state, drawn).unwrap();
        assert!(short.rows.iter().all(|row| row.oid != old));
        assert!(depth(&state, &old).unwrap().unwrap() >= short.rows.len());

        let unknown = "0".repeat(40);
        assert_eq!(depth(&state, &unknown).unwrap(), None);

        let _ = std::fs::remove_dir_all(&root);
    }

    use super::*;

    /// A commit id from a small number, so a test history reads as 1, 2, 3.
    fn id(n: u32) -> Oid {
        Oid::from_str(&format!("{n:040x}")).unwrap()
    }

    fn commit(n: u32, parents: &[u32]) -> Commit {
        Commit {
            oid: id(n),
            parents: parents.iter().map(|p| id(*p)).collect(),
            summary: format!("commit {n}"),
            author: "Tester".into(),
            email: "tester@example.com".into(),
            time: 0,
            unpulled: false,
            carries: Vec::new(),
            stash: None,
        }
    }

    /// A stash hanging off `parent`, as `with_stashes` builds one.
    fn stash(n: u32, parent: u32, index: usize) -> Commit {
        Commit {
            stash: Some(index),
            ..commit(n, &[parent])
        }
    }

    #[test]
    fn a_stash_hangs_off_its_commit_on_a_broken_line() {
        // 1 ← 2 ← 3, with a stash made while standing on 2.
        let commits = vec![
            commit(1, &[2]),
            stash(9, 2, 0),
            commit(2, &[3]),
            commit(3, &[]),
        ];
        assert!(faults(&commits, None, None).is_empty());

        let places = plot(&commits, None, None);
        // Its own lane, beside the line it hangs off.
        assert_ne!(places[1].lane, places[2].lane);
        // Everything leaving the stash is drawn broken.
        let leaving: Vec<&Segment> = places[1]
            .segments
            .iter()
            .filter(|seg| seg.x1 == places[1].lane && seg.y1 == 1)
            .collect();
        assert!(!leaving.is_empty());
        assert!(leaving.iter().all(|seg| seg.dashed));

        // And the line arriving at the commit it was made on is broken too, so
        // it does not turn solid half way down.
        let arriving: Vec<&Segment> = places[2]
            .segments
            .iter()
            .filter(|seg| seg.x1 == places[1].lane)
            .collect();
        assert!(
            !arriving.is_empty(),
            "the stash's line has to land somewhere"
        );
        assert!(arriving.iter().all(|seg| seg.dashed));
    }

    #[test]
    fn the_history_itself_is_never_drawn_broken() {
        let commits = vec![
            commit(1, &[2]),
            stash(9, 2, 0),
            commit(2, &[3]),
            commit(3, &[]),
        ];
        let places = plot(&commits, None, None);
        // Row 0 and row 3 have nothing to do with the stash.
        for at in [0usize, 3] {
            assert!(
                places[at].segments.iter().all(|seg| !seg.dashed),
                "row {at} should be solid"
            );
        }
        // The commit the stash hangs off carries on downwards solidly: only
        // the line coming in from the stash's lane is broken.
        let onward: Vec<&Segment> = places[2]
            .segments
            .iter()
            .filter(|seg| seg.y1 == 1)
            .collect();
        assert!(onward.iter().all(|seg| !seg.dashed));
    }

    #[test]
    fn two_stashes_on_one_commit_both_stay_broken_all_the_way_down() {
        // Both made while standing on 2, newest first.
        let commits = vec![
            commit(1, &[2]),
            stash(9, 2, 0),
            stash(8, 2, 1),
            commit(2, &[3]),
            commit(3, &[]),
        ];
        assert!(faults(&commits, None, None).is_empty());

        let places = plot(&commits, None, None);
        let first = places[1].lane;
        // The older stash's row has the newer one's line passing through it,
        // and a line passing a row by must not come out solid.
        let passing: Vec<&Segment> = places[2]
            .segments
            .iter()
            .filter(|seg| seg.x1 == first)
            .collect();
        assert!(
            !passing.is_empty(),
            "the first stash's line passes this row"
        );
        assert!(
            passing.iter().all(|seg| seg.dashed),
            "a stash line stays broken where it merely passes by"
        );

        // Every segment reaching the commit both were made on is broken.
        let lanes = [places[1].lane, places[2].lane];
        let arriving: Vec<&Segment> = places[3]
            .segments
            .iter()
            .filter(|seg| lanes.contains(&seg.x1))
            .collect();
        assert_eq!(arriving.len(), 2);
        assert!(arriving.iter().all(|seg| seg.dashed));
    }

    #[test]
    fn a_lane_a_stash_gave_up_is_solid_again_for_whatever_takes_it() {
        // The stash's lane is freed at 2 and reused by an unrelated branch.
        let commits = vec![
            commit(1, &[2]),
            stash(9, 2, 0),
            commit(2, &[3]),
            commit(7, &[3]),
            commit(3, &[]),
        ];
        assert!(faults(&commits, None, None).is_empty());
        let places = plot(&commits, None, None);
        assert!(
            places[3].segments.iter().all(|seg| !seg.dashed),
            "an ordinary commit taking a freed lane draws a solid line"
        );
        assert!(places[4].segments.iter().all(|seg| !seg.dashed));
    }

    /// Every complaint the picture could make about itself.
    ///
    /// A line is drawn one row at a time, so the only thing holding it together
    /// is that what leaves the bottom of a row arrives at the top of the next
    /// one, in the same lane and the same colour. Where that fails the user
    /// sees exactly what a broken graph looks like: a line that stops in
    /// mid-air, or a stub that starts from nothing.
    fn faults(commits: &[Commit], anchor: Option<Oid>, head: Option<Oid>) -> Vec<String> {
        let places = plot(commits, anchor, head);
        let mut out = Vec::new();

        // Two lines can leave a row in the same lane and colour — a branch
        // rejoining the line it came from is drawn as both, one on top of the
        // other — so what matters is which lanes carry a line, not how many
        // times each was drawn.
        let ends = |place: &Place| {
            let mut v: Vec<(usize, usize)> = place
                .segments
                .iter()
                .filter(|s| s.y2 == 2)
                .map(|s| (s.x2, s.color))
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let starts = |place: &Place| {
            let mut v: Vec<(usize, usize)> = place
                .segments
                .iter()
                .filter(|s| s.y1 == 0)
                .map(|s| (s.x1, s.color))
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        };

        for (row, place) in places.iter().enumerate() {
            for segment in &place.segments {
                if segment.y2 == 1 && segment.x2 != place.lane {
                    out.push(format!("row {row}: a line arrives away from the node"));
                }
                if segment.y1 == 1 && segment.x1 != place.lane {
                    out.push(format!("row {row}: a line leaves from beside the node"));
                }
                if segment.x1 >= place.width || segment.x2 >= place.width {
                    out.push(format!(
                        "row {row}: a line is drawn outside the row's width"
                    ));
                }
            }
            // A commit with parents keeps its line going; only a root ends one.
            // A branch it carries the work of gets a line too.
            let leaving = place.segments.iter().filter(|s| s.y1 == 1).count();
            let expected = commits[row].parents.len() + commits[row].carries.len();
            if leaving != expected {
                out.push(format!(
                    "row {row}: {expected} lines expected but {leaving} leaving the node"
                ));
            }
            if let Some(next) = places.get(row + 1) {
                if ends(place) != starts(next) {
                    out.push(format!(
                        "rows {row}/{}: {:?} leaves the bottom but {:?} arrives at the top",
                        row + 1,
                        ends(place),
                        starts(next)
                    ));
                }
            }
        }
        out
    }

    fn check(commits: &[Commit], anchor: Option<Oid>) {
        let faults = faults(commits, anchor, None);
        assert!(faults.is_empty(), "{}", faults.join("\n"));
    }

    #[test]
    fn draws_a_straight_history() {
        let history = [commit(1, &[2]), commit(2, &[3]), commit(3, &[])];
        check(&history, Some(id(1)));
    }

    #[test]
    fn draws_a_branch_that_was_merged_back() {
        // 1 merges the branch 2 into the trunk 3; both reach 4.
        let history = [
            commit(1, &[3, 2]),
            commit(2, &[4]),
            commit(3, &[4]),
            commit(4, &[5]),
            commit(5, &[]),
        ];
        check(&history, Some(id(1)));
    }

    #[test]
    fn draws_a_branch_nobody_merged() {
        // 1 is the tip of a branch off 3; the trunk carries on through 2.
        let history = [
            commit(1, &[3]),
            commit(2, &[3]),
            commit(3, &[4]),
            commit(4, &[]),
        ];
        check(&history, Some(id(2)));
    }

    #[test]
    fn draws_a_history_whose_head_is_not_the_newest_commit() {
        // What a branch switch leaves behind: HEAD is 3, and the branch tip 1
        // is newer than it. The lane reserved for HEAD must not be drawn as a
        // line before the walk reaches it.
        let history = [
            commit(1, &[2]),
            commit(2, &[4]),
            commit(3, &[4]),
            commit(4, &[5]),
            commit(5, &[]),
        ];
        check(&history, Some(id(3)));
    }

    #[test]
    fn keeps_the_line_a_merge_puts_into_the_lane_head_is_waiting_in() {
        // The shape a branch switch leaves: HEAD is 5, further down the list,
        // and the newest commit is a merge whose second parent is HEAD. The
        // merge puts a line into the lane reserved for HEAD, and every row
        // between the two has to carry it.
        let history = [
            commit(1, &[2, 5]),
            commit(2, &[3]),
            commit(3, &[4]),
            commit(4, &[5]),
            commit(5, &[6]),
            commit(6, &[]),
        ];
        check(&history, Some(id(5)));

        let places = plot(&history, Some(id(5)), None);
        for (row, place) in places.iter().enumerate().take(4).skip(1) {
            assert!(
                place.segments.iter().any(|s| s.x1 == 0 && s.y1 == 0),
                "row {row} drops the line the merge put into HEAD's lane"
            );
        }
    }

    #[test]
    fn keeps_the_trunk_in_the_leftmost_lane() {
        // A branch off 4 whose tip, 1, is newer than the trunk's tip, 2. Being
        // first in the list must not win the branch the trunk's column: that is
        // the shape that made the whole graph slide sideways the moment
        // somebody committed to a branch.
        let history = [
            commit(1, &[4]),
            commit(2, &[3]),
            commit(3, &[4]),
            commit(4, &[]),
        ];
        check(&history, Some(id(2)));

        let places = plot(&history, Some(id(2)), None);
        assert_eq!(
            places[0].lane, 1,
            "the newer branch took the trunk's column"
        );
        assert_eq!(places[1].lane, 0, "the trunk is not in the leftmost column");
        assert_eq!(places[2].lane, 0);
        assert_eq!(places[1].color, TRUNK_COLOR);
    }

    #[test]
    fn keeps_lines_that_share_a_row_in_different_colours() {
        // Four lines leave the merge at once. A colour that repeats among them
        // is a colour saying two lines are one.
        let history = [
            commit(1, &[2, 3, 4, 5]),
            commit(2, &[6]),
            commit(3, &[6]),
            commit(4, &[6]),
            commit(5, &[6]),
            commit(6, &[]),
        ];
        let places = plot(&history, Some(id(1)), None);
        let colours: HashSet<usize> = places[0]
            .segments
            .iter()
            .filter(|s| s.y2 == 2)
            .map(|s| s.color)
            .collect();
        assert_eq!(colours.len(), 4, "two of the four lines share a colour");
    }

    #[test]
    fn draws_an_octopus_merge() {
        let history = [
            commit(1, &[2, 3, 4]),
            commit(2, &[5]),
            commit(3, &[5]),
            commit(4, &[5]),
            commit(5, &[]),
        ];
        check(&history, Some(id(1)));
    }

    #[test]
    fn draws_history_that_runs_off_the_bottom() {
        // The walk stops at a limit, so lines are still in flight on the last
        // row. They may run off the bottom, but nothing may vanish before it.
        let history = [commit(1, &[3]), commit(2, &[4])];
        check(&history, Some(id(1)));
    }

    /// Histories nobody would think to write down by hand.
    ///
    /// The picture only breaks on the awkward ones — a branch merged twice, a
    /// lane freed and taken by something else two rows later — so the shapes
    /// are generated rather than chosen, and the same seeds run every time.
    #[test]
    fn draws_whatever_shape_history_takes() {
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut random = move |bound: u32| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % bound as u64) as u32
        };

        for round in 0..400 {
            let count = 6 + random(24);
            let mut history = Vec::new();
            for n in 1..=count {
                // Parents are always further down the list, which is what a
                // topological walk guarantees and what makes the shape a DAG.
                let older = count - n;
                let mut parents = Vec::new();
                if older > 0 {
                    parents.push(n + 1 + random(older.min(4)));
                    // Every so often a merge, and rarely an octopus.
                    if random(4) == 0 && older > 2 {
                        parents.push(n + 1 + random(older));
                    }
                    if random(24) == 0 && older > 3 {
                        parents.push(n + 1 + random(older));
                    }
                    parents.dedup();
                }
                let mut made = commit(n, &parents);
                // Now and then a commit that is the squash of some older one,
                // and a stretch of commits not pulled yet.
                if random(6) == 0 && older > 1 {
                    let tip = n + 1 + random(older);
                    if !parents.contains(&tip) {
                        made.carries.push(id(tip));
                    }
                }
                made.unpulled = random(5) == 0;
                history.push(made);
            }

            // Sometimes the walk stopped at a limit, leaving lines in flight on
            // the last row, and sometimes HEAD is an older commit than the tip.
            let cut = history.len() - random(3) as usize;
            let history = &history[..cut];
            let head = history
                .get(random(history.len() as u32) as usize)
                .map(|c| c.oid);

            let standing = history
                .get(random(history.len() as u32) as usize)
                .map(|c| c.oid);
            let faults = faults(history, head, standing);
            assert!(
                faults.is_empty(),
                "round {round}:\n{}\nhistory: {:?}",
                faults.join("\n"),
                history
                    .iter()
                    .map(|c| (
                        c.summary.clone(),
                        c.parents
                            .iter()
                            .map(|p| p.to_string()[38..].to_string())
                            .collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn brightens_the_line_you_stand_on_down_to_where_it_leaves_the_trunk() {
        // HEAD is 2, on a branch off 4. Above it, 1 is the same line carried on
        // — the upstream's newer commit, say — and 3 is the trunk beside it.
        let history = [
            commit(1, &[2]),
            commit(2, &[4]),
            commit(3, &[4]),
            commit(4, &[5]),
            commit(5, &[]),
        ];
        assert!(faults(&history, Some(id(3)), Some(id(2))).is_empty());
        let places = plot(&history, Some(id(3)), Some(id(2)));
        let mine = places[1].lane;

        // Nothing above where you stand is bright: that is the rest of the
        // branch, not where you are.
        assert!(places[0].segments.iter().all(|s| !s.current));
        assert!(places[1]
            .segments
            .iter()
            .filter(|s| s.y1 == 0)
            .all(|s| !s.current));
        // From your node down, it is.
        assert!(places[1]
            .segments
            .iter()
            .filter(|s| s.y1 == 1)
            .all(|s| s.current));
        // The trunk's row beside it: your line passes by bright, the trunk's
        // own line stays plain.
        let passing: Vec<&Segment> = places[2].segments.iter().filter(|s| s.x1 == mine).collect();
        assert!(!passing.is_empty());
        assert!(passing.iter().all(|s| s.current));
        assert!(places[2]
            .segments
            .iter()
            .filter(|s| s.x1 == places[2].lane)
            .all(|s| !s.current));
        // Where the branch left the trunk: bright into the node, plain out of
        // it — below here the history is the trunk's, not the branch's.
        assert!(places[3]
            .segments
            .iter()
            .any(|s| s.y2 == 1 && s.x1 == mine && s.current));
        assert!(places[3]
            .segments
            .iter()
            .filter(|s| s.y1 == 1)
            .all(|s| !s.current));
        assert!(places[4].segments.iter().all(|s| !s.current));
    }

    #[test]
    fn standing_on_the_trunk_brightens_it_all_the_way_down() {
        let history = [commit(1, &[2]), commit(2, &[3]), commit(3, &[])];
        let places = plot(&history, Some(id(1)), Some(id(2)));
        assert!(places[0].segments.iter().all(|s| !s.current));
        assert!(places[1]
            .segments
            .iter()
            .filter(|s| s.y1 == 1)
            .all(|s| s.current));
        assert!(places[2]
            .segments
            .iter()
            .filter(|s| s.y1 == 0)
            .all(|s| s.current));
    }

    #[test]
    fn a_branch_folded_in_hangs_off_the_commit_that_carries_it() {
        // 1 is the squash of the branch whose tip is 3; the trunk is 1 ← 2 ← 4.
        let mut history = vec![
            commit(1, &[2]),
            commit(2, &[4]),
            commit(3, &[4]),
            commit(4, &[]),
        ];
        history[0].carries.push(id(3));
        check(&history, Some(id(1)));

        let places = plot(&history, Some(id(1)), None);
        let link = places[0]
            .segments
            .iter()
            .find(|s| s.y1 == 1 && s.x2 != places[0].lane)
            .expect("a line leaves the squash for the branch it squashed");
        assert!(link.dashed);
        // Broken all the way down: past the row between, and into the tip.
        assert!(places[1]
            .segments
            .iter()
            .filter(|s| s.x1 == link.x2)
            .all(|s| s.dashed));
        assert_eq!(places[2].lane, link.x2);
        assert!(places[2]
            .segments
            .iter()
            .filter(|s| s.y2 == 1)
            .all(|s| s.dashed));
        // The branch's own history below its tip is history, and solid.
        assert!(places[2]
            .segments
            .iter()
            .filter(|s| s.y1 == 1)
            .all(|s| !s.dashed));
    }

    #[test]
    fn what_is_still_to_pull_is_held_back_down_to_the_commit_you_have() {
        // origin/main is 1 and main is 3: 1 and 2 are not pulled yet.
        let mut history = vec![
            commit(1, &[2]),
            commit(2, &[3]),
            commit(3, &[4]),
            commit(4, &[]),
        ];
        history[0].unpulled = true;
        history[1].unpulled = true;
        check(&history, Some(id(1)));

        let places = plot(&history, Some(id(1)), Some(id(3)));
        // Straight above, in the trunk's own column: not a branch beside it.
        assert!(places.iter().all(|p| p.lane == 0));
        assert!(places[0].segments.iter().all(|s| s.faint));
        assert!(places[1].segments.iter().all(|s| s.faint));
        // Held back into your node, and plain — and yours — out of it.
        assert!(places[2]
            .segments
            .iter()
            .filter(|s| s.y2 == 1)
            .all(|s| s.faint));
        assert!(places[2]
            .segments
            .iter()
            .filter(|s| s.y1 == 1)
            .all(|s| !s.faint && s.current));
        assert!(places[3].segments.iter().all(|s| !s.faint));
    }

    #[test]
    fn draws_several_branches_at_once() {
        // Two branches off the trunk, one merged back, one not, with the trunk
        // carrying on underneath both.
        let history = [
            commit(1, &[2]),
            commit(2, &[3, 6]),
            commit(3, &[4]),
            commit(4, &[5]),
            commit(5, &[7]),
            commit(6, &[7]),
            commit(7, &[8]),
            commit(8, &[]),
        ];
        check(&history, Some(id(1)));
    }
}
