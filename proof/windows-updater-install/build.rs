fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let manifest = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
            .join("as-invoker.manifest");
        println!("cargo:rerun-if-changed=as-invoker.manifest");
        println!("cargo:rustc-link-arg-bin=tauri-updater-install-proof=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg-bin=tauri-updater-install-proof=/MANIFESTINPUT:{}",
            manifest.display()
        );
    }
}
