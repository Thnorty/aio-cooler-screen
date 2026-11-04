fn main() {
    // Turkish strings for @tr live in translations/tr/LC_MESSAGES/aio-screen.po.
    let config = slint_build::CompilerConfiguration::new()
        .with_style("fluent".into())
        .with_bundled_translations("translations");
    println!("cargo:rerun-if-changed=translations");
    slint_build::compile_with_config("ui/app.slint", config).expect("compile ui/app.slint");

    // Embed the exe icon and version info (assets/icon.ico is produced by `aio-screen make-icon assets`).
    println!("cargo:rerun-if-changed=assets/icon.ico");
    let mut res = winresource::WindowsResource::new();
    if std::path::Path::new("assets/icon.ico").exists() {
        res.set_icon("assets/icon.ico");
    }
    res.set("FileDescription", "AIO Cooler Screen");
    res.set("ProductName", "AIO Cooler Screen");
    res.set("CompanyName", "AIO Cooler Screen contributors");
    res.set("LegalCopyright", "Copyright (c) 2026 Thnorty. MIT License.");
    res.set("OriginalFilename", "aio-screen.exe");
    res.set("InternalName", "aio-screen");
    if let Err(e) = res.compile() {
        println!("cargo:warning=could not embed Windows resources: {e}");
    }
}
