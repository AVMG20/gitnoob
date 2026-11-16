use std::path::PathBuf;

/// The bundled `magequery` (tauri.conf.json `bundle.externalBin`) has to be
/// there for tauri-build to copy it, or nothing compiles. It is fetched by
/// `scripts/magequery.mjs`, which `npm run app`, `npm run app:build` and CI
/// run first. A debug build (`cargo test`, clippy, a fresh checkout) gets an
/// empty placeholder instead, which the app treats as "not bundled". A
/// release build refuses to go on without the real one.
fn sidecar() {
    let target = std::env::var("TARGET").unwrap_or_default();
    println!("cargo:rustc-env=GITNOOB_TARGET={target}");
    let windows = target.contains("windows");
    let name = format!("magequery-{target}{}", if windows { ".exe" } else { "" });
    let path = PathBuf::from("binaries").join(&name);
    println!("cargo:rerun-if-changed={}", path.display());
    let size = std::fs::metadata(&path).map(|meta| meta.len()).ok();
    let release = std::env::var("PROFILE").is_ok_and(|profile| profile == "release");
    match size {
        Some(size) if size > 0 => {}
        _ if release => panic!(
            "src-tauri/binaries/{name} is missing or empty. Run `node scripts/magequery.mjs` first."
        ),
        Some(_) => {}
        None => {
            let _ = std::fs::create_dir_all("binaries");
            std::fs::write(&path, b"").expect("could not write the magequery placeholder");
        }
    }
}

fn main() {
    sidecar();
    tauri_build::build()
}
