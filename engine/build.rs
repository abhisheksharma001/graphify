//! Embeds `ui/dist` into the binary, when there is one.
//!
//! A release binary is meant to be the whole product: no static directory to ship beside
//! it, nothing to point a web server at. So the built dashboard goes inside it.
//!
//! Done here rather than with `rust-embed` because that crate costs fourteen new
//! dependencies — a second copy of sha2, a mime table, a directory walker — to do what a
//! recursive `read_dir` and `include_bytes!` do in thirty lines. It also cannot express
//! the case this has to handle: `ui/dist` is absent in a fresh checkout, and `rust-embed`
//! needs its folder to exist at compile time. An empty table is the honest answer, and
//! `src/ui.rs` turns it into a page that says the UI has not been built yet.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let dist = PathBuf::from("../ui/dist");
    // Vite rewrites the whole folder on every build, so watching the folder catches a new
    // one appearing. While it is absent cargo re-runs this script on every build, which is
    // what we want: that is exactly the moment the answer is about to change. Changes to
    // files already embedded are caught by rustc instead, through `include_bytes!`.
    println!("cargo::rerun-if-changed={}", dist.display());

    let mut files = Vec::new();
    collect(&dist, &dist, &mut files);
    // Sorted so a rebuild with the same folder writes the same file, byte for byte.
    files.sort();

    let mut out = String::from(
        "/// Every file in `ui/dist` as (url path, bytes). Empty when there was no build.\n\
         static ASSETS: &[(&str, &[u8])] = &[\n",
    );
    for (rel, abs) in &files {
        writeln!(out, "    ({rel:?}, include_bytes!({abs:?})),").unwrap();
    }
    out.push_str("];\n");

    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("assets.rs");
    std::fs::write(dest, out).unwrap();
}

/// Every file under `dir`, as the path a browser will ask for and the path to read it from.
fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(root, &path, out);
        } else if let (Ok(rel), Ok(abs)) = (path.strip_prefix(root), std::fs::canonicalize(&path)) {
            // Absolute, because `include_bytes!` resolves against the generated file, which
            // lives in `OUT_DIR` and has no idea where `ui/dist` is. Forward slashes on the
            // other half: past this point it is a URL path, not a filesystem path.
            let rel = rel.to_string_lossy().replace('\\', "/");
            out.push((rel, abs.to_string_lossy().into_owned()));
        }
    }
}
