use std::collections::HashSet;

use git2::{Oid, Sort};
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
#[derive(Serialize)]
pub struct Segment {
    pub x1: usize,
    pub y1: u8,
    pub x2: usize,
    pub y2: u8,
    pub color: usize,
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
}

#[derive(Serialize)]
pub struct GraphPage {
    pub rows: Vec<GraphRow>,
    /// True when the walk stopped at `limit` and older commits remain.
    pub has_more: bool,
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
    let unpushed = unpushed_commits(&repo, limit);

    let mut walk = repo.revwalk().map_err(err)?;
    // Topological order keeps a branch's commits contiguous; the time secondary
    // sort keeps the result close to what the user expects to read.
    walk.set_sorting(Sort::TOPOLOGICAL | Sort::TIME).map_err(err)?;
    // Push every ref, not just HEAD, so unmerged branches are visible.
    walk.push_glob("refs/heads/*").map_err(err)?;
    let _ = walk.push_glob("refs/remotes/*");
    let _ = walk.push_glob("refs/tags/*");
    let _ = walk.push_head();

    let mut lanes: Vec<Option<Oid>> = Vec::new();
    let mut colors: Vec<usize> = Vec::new();
    let mut next_color = 0usize;
    let mut rows: Vec<GraphRow> = Vec::with_capacity(limit.min(4096));
    let mut has_more = false;

    // The line the user is standing on takes the leftmost lane, whatever order
    // the walk happens to reach it in. Left to first come first served, a branch
    // whose tip is merely newer than HEAD takes the trunk's column, and the
    // trunk is then drawn stepping sideways around it on its way down — which
    // reads as the branch and the trunk swapping places rather than as a branch
    // leaving. The lane is only reserved: nothing is drawn in it until the
    // commit itself turns up, since above that row there is no line to draw.
    let mut reserved = None;
    if let Some(head) = repo.head().ok().and_then(|reference| reference.target()) {
        lanes.push(Some(head));
        colors.push(next_color);
        next_color += 1;
        reserved = Some(0);
    }

    for oid in walk {
        let oid = oid.map_err(err)?;
        if rows.len() >= limit {
            has_more = true;
            break;
        }
        let commit = repo.find_commit(oid).map_err(err)?;

        let lanes_before = lanes.clone();
        let colors_before = colors.clone();
        let reserved_before = reserved;

        // 1. Claim a lane.
        let lane = match lanes.iter().position(|l| *l == Some(oid)) {
            Some(i) => i,
            None => {
                let i = alloc(&mut lanes, &mut colors);
                lanes[i] = Some(oid);
                colors[i] = next_color;
                next_color += 1;
                i
            }
        };
        let color = colors[lane];
        if reserved == Some(lane) {
            reserved = None;
        }

        // 2. Release any other lane that was also waiting for this commit —
        //    several children merging back into one line.
        for i in 0..lanes.len() {
            if i != lane && lanes[i] == Some(oid) {
                lanes[i] = None;
            }
        }

        // 3. Hand the lane to the first parent, and give every other parent a
        //    lane of its own unless it is already tracked.
        let parents: Vec<Oid> = commit.parent_ids().collect();
        match parents.split_first() {
            None => lanes[lane] = None,
            Some((first, rest)) => {
                // The lane carries on to the first parent even when another
                // lane is already waiting for that same commit. Both hold it
                // until the row it lands on, where they meet at its node —
                // which is the row the branch actually rejoined. Collapsing the
                // duplicate here instead would move the join a row early and
                // draw the branch sliding into its neighbour's column before
                // there is anything there to join.
                lanes[lane] = Some(*first);
                for parent in rest {
                    if lanes.iter().any(|l| *l == Some(*parent)) {
                        continue;
                    }
                    let i = alloc(&mut lanes, &mut colors);
                    lanes[i] = Some(*parent);
                    colors[i] = next_color;
                    next_color += 1;
                }
            }
        }

        trim(&mut lanes, &mut colors);
        let lanes_after = lanes.clone();

        // 4. Turn the before/after tables into segments.
        // Where a line ends up, preferring the lane it is already in. Two lines
        // waiting for the same commit both hold it, so a plain search finds
        // whichever comes first and would draw a line that is staying put as
        // stepping into its neighbour.
        let find = |want: &Oid, prefer: usize| -> Option<usize> {
            if lanes_after.get(prefer).and_then(|slot| slot.as_ref()) == Some(want) {
                return Some(prefer);
            }
            lanes_after.iter().position(|slot| slot.as_ref() == Some(want))
        };

        let mut segments = Vec::new();
        for (x, slot) in lanes_before.iter().enumerate() {
            // A lane still only reserved holds a commit the walk has not
            // reached, so there is no line above this row to come down from.
            if reserved_before == Some(x) {
                continue;
            }
            let Some(waiting) = slot else { continue };
            if *waiting == oid {
                // Incoming line ends at this row's node.
                segments.push(Segment {
                    x1: x,
                    y1: 0,
                    x2: lane,
                    y2: 1,
                    color: colors_before[x],
                });
            } else if let Some(to) = find(waiting, x) {
                // A line that just passes this row by.
                segments.push(Segment {
                    x1: x,
                    y1: 0,
                    x2: to,
                    y2: 2,
                    color: colors_before[x],
                });
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
                // colour; a merge's other parents belong to the line they join.
                color: if i == 0 { color } else { colors[to] },
            });
        }

        let width = lanes_before.len().max(lanes_after.len()).max(lane + 1);
        let author = commit.author();

        rows.push(GraphRow {
            oid: oid.to_string(),
            short: oid.to_string()[..7].to_string(),
            summary: commit.summary().unwrap_or("").to_string(),
            author: author.name().unwrap_or("").to_string(),
            email: author.email().unwrap_or("").to_string(),
            time: commit.time().seconds(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            lane,
            color,
            width,
            segments,
            labels: labels
                .get(&oid.to_string())
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
            unpushed: unpushed.contains(&oid.to_string()),
        });
    }

    Ok(GraphPage { rows, has_more })
}

/// The commits a local branch has and its upstream does not.
///
/// An ahead count says how many there are; this says which, so the graph can
/// draw the boundary between what the remote knows about and what is still only
/// here. Branches with no upstream contribute nothing: everything on them is
/// unpushed in a sense, but there is nowhere it was meant to go.
fn unpushed_commits(repo: &git2::Repository, limit: usize) -> HashSet<String> {
    let mut out = HashSet::new();
    let Ok(branches) = repo.branches(Some(git2::BranchType::Local)) else {
        return out;
    };

    for branch in branches.flatten() {
        let (branch, _) = branch;
        let Some(local) = branch.get().target() else {
            continue;
        };
        let Some(upstream) = branch.upstream().ok().and_then(|u| u.get().target()) else {
            continue;
        };
        if local == upstream {
            continue;
        }

        let Ok(mut walk) = repo.revwalk() else { continue };
        if walk.push(local).is_err() || walk.hide(upstream).is_err() {
            continue;
        }
        for oid in walk.flatten().take(limit) {
            out.insert(oid.to_string());
        }
    }
    out
}

/// Returns the index of a reusable empty lane, appending one if none is free.
fn alloc(lanes: &mut Vec<Option<Oid>>, colors: &mut Vec<usize>) -> usize {
    match lanes.iter().position(|l| l.is_none()) {
        Some(i) => i,
        None => {
            lanes.push(None);
            colors.push(0);
            lanes.len() - 1
        }
    }
}

/// Drops trailing empty lanes so the graph column stays as narrow as the history
/// actually needs.
fn trim(lanes: &mut Vec<Option<Oid>>, colors: &mut Vec<usize>) {
    while lanes.last() == Some(&None) {
        lanes.pop();
        colors.pop();
    }
}

fn err(e: git2::Error) -> String {
    e.message().to_string()
}
