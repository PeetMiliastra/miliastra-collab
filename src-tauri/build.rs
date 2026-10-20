fn main() {
    println!("cargo:rerun-if-changed=commands.generated.rs");
    println!("cargo:rerun-if-changed=commands.test.generated.rs");
    println!("cargo:rerun-if-env-changed=MCOLLAB_TEST_PROFILES");
    // Test builds (debug builds, or release builds with MCOLLAB_TEST_PROFILES=1) include
    // the built-in coordinator, the practice fixture, test profiles and their commands.
    println!("cargo:rustc-check-cfg=cfg(test_build)");
    // Cargo does not expose debug_assertions to build scripts; PROFILE is "debug" for dev/test builds.
    let test_build = std::env::var("PROFILE").is_ok_and(|p| p == "debug")
        || std::env::var("MCOLLAB_TEST_PROFILES").is_ok_and(|v| v == "1");
    if test_build {
        println!("cargo:rustc-cfg=test_build");
    }
    const NORMAL: &[&str] = include!("commands.generated.rs");
    const TEST: &[&str] = include!("commands.test.generated.rs");
    let commands: Vec<&str> = NORMAL
        .iter()
        .chain(if test_build { TEST } else { &[] })
        .copied()
        .collect();
    let handler = format!("tauri::generate_handler![{}]", commands.join(","));
    std::fs::write(
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("command-handler.rs"),
        handler,
    )
    .expect("Write native handler");
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(commands.leak())),
    )
    .expect("Tauri build failed");
}
