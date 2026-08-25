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

    // Read the commits first, then work out the picture. The two are separate
    // jobs, and only the second one is hard enough to be worth testing on
    // histories that would be a chore to build a repository for.
    let mut commits: Vec<Commit> = Vec::with_capacity(limit.min(4096));
    let mut has_more = false;
    for oid in walk {
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
        });
    }

    let head = repo.head().ok().and_then(|reference| reference.target());
    let plotted = plot(&commits, head);

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
            unpushed: unpushed.contains(&commit.oid.to_string()),
        })
        .collect();

    Ok(GraphPage { rows, has_more })
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
}

/// Where one row's node sits and what lines cross it.
pub struct Place {
    pub lane: usize,
    pub color: usize,
    pub width: usize,
    pub segments: Vec<Segment>,
}

/// Turns a list of commits, newest first, into the lines to draw.
fn plot(commits: &[Commit], head: Option<Oid>) -> Vec<Place> {
    let mut lanes: Vec<Option<Oid>> = Vec::new();
    let mut colors: Vec<usize> = Vec::new();
    let mut next_color = 0usize;
    let mut places: Vec<Place> = Vec::with_capacity(commits.len());

    // The line the user is standing on takes the leftmost lane, whatever order
    // the walk happens to reach it in. Left to first come first served, a branch
    // whose tip is merely newer than HEAD takes the trunk's column, and the
    // trunk is then drawn stepping sideways around it on its way down — which
    // reads as the branch and the trunk swapping places rather than as a branch
    // leaving. The lane is only reserved: nothing is drawn in it until the
    // commit itself turns up, since above that row there is no line to draw.
    let mut reserved = None;
    if let Some(head) = head {
        lanes.push(Some(head));
        colors.push(next_color);
        next_color += 1;
        reserved = Some(0);
    }

    for commit in commits {
        let oid = commit.oid;

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
        let parents = &commit.parents;
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

        let width = lanes_before.len().max(lanes_after.len()).max(lane + 1);

        places.push(Place {
            lane,
            color,
            width,
            segments,
        });
    }

    places
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

#[cfg(test)]
mod tests {
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
        }
    }

    /// Every complaint the picture could make about itself.
    ///
    /// A line is drawn one row at a time, so the only thing holding it together
    /// is that what leaves the bottom of a row arrives at the top of the next
    /// one, in the same lane and the same colour. Where that fails the user
    /// sees exactly what a broken graph looks like: a line that stops in
    /// mid-air, or a stub that starts from nothing.
    fn faults(commits: &[Commit], head: Option<Oid>) -> Vec<String> {
        let places = plot(commits, head);
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
                    out.push(format!("row {row}: a line is drawn outside the row's width"));
                }
            }
            // A commit with parents keeps its line going; only a root ends one.
            let leaving = place.segments.iter().filter(|s| s.y1 == 1).count();
            if leaving != commits[row].parents.len() {
                out.push(format!(
                    "row {row}: {} parents but {leaving} lines leaving the node",
                    commits[row].parents.len()
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

    fn check(commits: &[Commit], head: Option<Oid>) {
        let faults = faults(commits, head);
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

        let places = plot(&history, Some(id(5)));
        for row in 1..=3 {
            assert!(
                places[row].segments.iter().any(|s| s.x1 == 0 && s.y1 == 0),
                "row {row} drops the line the merge put into HEAD's lane"
            );
        }
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
                history.push(commit(n, &parents));
            }

            // Sometimes the walk stopped at a limit, leaving lines in flight on
            // the last row, and sometimes HEAD is an older commit than the tip.
            let cut = history.len() - random(3) as usize;
            let history = &history[..cut];
            let head = history.get(random(history.len() as u32) as usize).map(|c| c.oid);

            let faults = faults(history, head);
            assert!(
                faults.is_empty(),
                "round {round}:\n{}\nhistory: {:?}",
                faults.join("\n"),
                history
                    .iter()
                    .map(|c| (
                        c.summary.clone(),
                        c.parents.iter().map(|p| p.to_string()[38..].to_string()).collect::<Vec<_>>()
                    ))
                    .collect::<Vec<_>>()
            );
        }
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
