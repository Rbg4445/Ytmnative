// Embed the app icon and version info into ytm-native.exe (Explorer, taskbar, installer shortcuts).
fn main() {
    println!("cargo:rerun-if-changed=../icons/icon.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../icons/icon.ico");
        res.set("ProductName", "YTM Native");
        res.set("FileDescription", "YTM Native — YouTube Music desktop client");
        res.set("CompanyName", "Rbg4445");
        res.set("LegalCopyright", "Copyright (c) 2026 Rbg4445 (MIT License)");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed Windows resources: {e}");
        }
    }
}
