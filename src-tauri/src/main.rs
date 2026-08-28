// Prevents an additional console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Git runs `GIT_SEQUENCE_EDITOR <todo-file>`, and for an interactive
    // rebase this app puts itself in that slot: the plan the window built is
    // copied over the list git offered, and nothing is opened. It has to be
    // answered before the window is built, because git is waiting on it.
    let args: Vec<String> = std::env::args().collect();
    if let [_, flag, from, to, ..] = args.as_slice() {
        if flag == "--write-todo" {
            match std::fs::copy(from, to) {
                Ok(_) => std::process::exit(0),
                Err(error) => {
                    eprintln!("gitnoob could not write the rebase plan: {error}");
                    std::process::exit(1)
                }
            }
        }
    }

    gitnoob_lib::run()
}
