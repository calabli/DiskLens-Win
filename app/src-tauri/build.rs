fn main() {
    // 发布版本以管理员身份运行，才能直接读取 MFT。
    // 开发版和测试保持默认清单，否则在普通终端里无法启动（错误 740）。
    let attrs = if std::env::var("PROFILE").as_deref() == Ok("release") {
        let windows = tauri_build::WindowsAttributes::new()
            .app_manifest(include_str!("windows-app-manifest.xml"));
        tauri_build::Attributes::new().windows_attributes(windows)
    } else {
        tauri_build::Attributes::new()
    };
    tauri_build::try_build(attrs).expect("failed to run tauri-build");
}
