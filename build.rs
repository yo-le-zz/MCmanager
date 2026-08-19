fn main() {
    // Only relevant when actually building for Windows; embeds assets/icon.ico
    // into the .exe so it shows up in Explorer, the taskbar, and Alt-Tab
    // instead of the generic Rust binary icon. Requires a windres (provided
    // by mingw-w64 when cross-compiling from Linux, or by MSVC/Windows SDK
    // when building natively on Windows).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "MCManager");
        res.set("FileDescription", "MCManager - gestionnaire de serveurs Minecraft");
        res.set("LegalCopyright", "MIT License - yolezz");
        if let Err(e) = res.compile() {
            println!("cargo:warning=impossible d'embarquer l'icone Windows: {e}");
        }
    }
}
