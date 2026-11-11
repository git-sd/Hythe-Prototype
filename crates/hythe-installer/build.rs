//! Embeds the programs to install. `HYTHE_PAYLOAD` is the folder holding the release
//! builds (see `installer/build-installer.ps1`); without it the installer is built
//! empty so `cargo build --workspace` and `cargo test --workspace` still work.

use std::path::{Path, PathBuf};
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-env-changed=HYTHE_PAYLOAD");
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let dir = env::var("HYTHE_PAYLOAD").ok();
    for file in ["hythe.exe", "hythe-agent.exe"] {
        let dst = out.join(file);
        match &dir {
            Some(d) => {
                let src = Path::new(d).join(file);
                println!("cargo:rerun-if-changed={}", src.display());
                fs::copy(&src, &dst).unwrap_or_else(|e| panic!("cannot read {}: {e}", src.display()));
            }
            None => fs::write(&dst, b"").expect("write placeholder"),
        }
    }
    println!("cargo:rustc-env=PAYLOAD_DIR={}", out.display());
}
