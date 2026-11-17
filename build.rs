//! Gives hythe.exe the Hythe star as its icon (Explorer, taskbar, Start menu, Settings > Apps).

fn main() {
    println!("cargo:rerun-if-changed=assets/hythe.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/hythe.ico");
        res.set("ProductName", "Hythe");
        res.set("FileDescription", "Hythe");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed the Hythe icon: {e}");
        }
    }
}
