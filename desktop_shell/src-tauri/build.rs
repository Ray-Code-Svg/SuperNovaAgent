fn main() {
    println!("cargo:rerun-if-env-changed=SUPERNOVA_WINDOWS_ICON_PATH");
    let current_dir = std::env::current_dir().expect("failed to resolve build.rs current_dir");
    let window_icon_path = std::env::var_os("SUPERNOVA_WINDOWS_ICON_PATH")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| current_dir.join("icons").join("icon.ico"));
    let windows = tauri_build::WindowsAttributes::new().window_icon_path(window_icon_path);
    let attributes = tauri_build::Attributes::new().windows_attributes(windows);
    tauri_build::try_build(attributes).expect("failed to run tauri build script");
}
