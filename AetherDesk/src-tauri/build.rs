fn main() {
    #[cfg(target_os = "windows")]
    {
        let mut windows = tauri_build::WindowsAttributes::new();
        windows = windows.app_manifest(include_str!("windows/aetherdesk-admin.manifest"));
        let attrs = tauri_build::Attributes::new().windows_attributes(windows);
        tauri_build::try_build(attrs)
            .expect("failed to build Tauri app with administrator manifest");
    }

    #[cfg(not(target_os = "windows"))]
    tauri_build::build();
}
