//! Times the backend reads that a repository open or refresh is made of.
//!
//! `cargo run --release --example profile -- /path/to/repo [limit]`

use std::path::PathBuf;
use std::time::Instant;

use gitnoob_lib::state::AppState;
use gitnoob_lib::{graph, refs, remote, work};

fn time<T>(label: &str, runs: usize, mut f: impl FnMut() -> T) -> T {
    let mut last = None;
    let mut best = f64::MAX;
    let mut total = 0.0;
    for _ in 0..runs {
        let start = Instant::now();
        last = Some(f());
        let ms = start.elapsed().as_secs_f64() * 1000.0;
        best = best.min(ms);
        total += ms;
    }
    println!(
        "{label:<28} best {best:>9.2} ms   avg {:>9.2} ms",
        total / runs as f64
    );
    last.unwrap()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = PathBuf::from(args.next().expect("repository path"));
    let limit: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(200);
    let runs: usize = args.next().map(|s| s.parse().unwrap()).unwrap_or(3);

    let state = AppState::new(std::env::temp_dir().join("gitnoob-profile"));
    state.set_path(path.clone());

    println!("repo {}   limit {limit}   runs {runs}\n", path.display());

    time("Repository::open", runs, || {
        state.repo().map(|_| ()).unwrap()
    });
    let info = time("repo_info", runs, || refs::describe(&state).unwrap());
    println!("   head {} state {}", info.head, info.state);

    let tree = time("ref_tree", runs, || refs::tree(&state).unwrap());
    println!(
        "   locals {} remotes {} tags {} stashes {}",
        tree.locals.len(),
        tree.remotes.len(),
        tree.tags.len(),
        tree.stashes.len()
    );

    let status = time("working_status", runs, || refs::status(&state).unwrap());
    println!(
        "   staged {} unstaged {} conflicted {}",
        status.staged.len(),
        status.unstaged.len(),
        status.conflicted.len()
    );

    let page = time("commit_graph", runs, || {
        graph::build(&state, limit).unwrap()
    });
    let json = serde_json::to_string(&page.rows).unwrap();
    println!("   rows {}  json {} KB", page.rows.len(), json.len() / 1024);

    time("stash_list", runs, || work::stash_list(&state).unwrap());
    time("in_progress", runs, || remote::in_progress(&state).unwrap());

    // The pieces inside ref_tree and commit_graph, so the cost has a home.
    let repo = state.repo().unwrap();
    time("  branches(Local)+ahead", runs, || {
        let mut n = 0;
        for b in repo
            .branches(Some(git2::BranchType::Local))
            .unwrap()
            .flatten()
        {
            let (b, _) = b;
            let Some(oid) = b.get().target() else {
                continue;
            };
            if let Ok(up) = b.upstream() {
                if let Some(u) = up.get().target() {
                    let _ = repo.graph_ahead_behind(oid, u);
                }
            }
            n += 1;
        }
        n
    });
    time("  branches(Remote)", runs, || {
        repo.branches(Some(git2::BranchType::Remote))
            .unwrap()
            .flatten()
            .count()
    });
    time("  references()", runs, || {
        repo.references().unwrap().flatten().count()
    });
    time("  labels_by_oid", runs, || refs::labels_by_oid(&repo).len());
    time("  revwalk all refs", runs, || {
        let mut walk = repo.revwalk().unwrap();
        walk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::TIME)
            .unwrap();
        walk.push_glob("refs/heads/*").unwrap();
        let _ = walk.push_glob("refs/remotes/*");
        let _ = walk.push_glob("refs/tags/*");
        let _ = walk.push_head();
        let mut n = 0;
        for oid in walk {
            let oid = oid.unwrap();
            let c = repo.find_commit(oid).unwrap();
            let _ = c.summary();
            n += 1;
            if n >= limit {
                break;
            }
        }
        n
    });
}
