fn main() {
    #[cfg(target_os = "windows")]
    {
        // AetherDesk writes inside the Steam folder (localconfig.vdf,
        // depotcache, stplug-in) and installs a DLL: the release exe requests
        // requireAdministrator.
        //
        // That manifest, however, gets embedded in EVERY artifact produced by
        // this build script — including the harness that `cargo test` compiles
        // and RUNS. Result: on Windows every `cargo test` popped a UAC prompt
        // (or failed with ERROR_ELEVATION_REQUIRED in a non-interactive
        // context), so tests ran neither in dev nor in CI.
        //
        // Way out: `AETHERDESK_NO_ADMIN_MANIFEST=1` produces an asInvoker exe.
        // `build.cmd` (test step) and CI set it. Release builds do NOT set it,
        // so the published exe stays identical to before — elevation is a
        // product property, not a test property.
        //
        // Note: the variable is read here and not through a feature flag because
        // `cargo test` cannot disable a default package feature without
        // `--no-default-features`, which would in turn disable everything
        // else. The env var is the channel Cargo already propagates to build
        // scripts (`rerun-if-env-changed` below).
        let no_admin_manifest = std::env::var("AETHERDESK_NO_ADMIN_MANIFEST")
            .map(|value| {
                let normalized = value.trim().to_ascii_lowercase();
                !normalized.is_empty() && normalized != "0" && normalized != "false"
            })
            .unwrap_or(false);

        println!("cargo:rerun-if-env-changed=AETHERDESK_NO_ADMIN_MANIFEST");

        let mut attrs = tauri_build::Attributes::new();
        if no_admin_manifest {
            println!("cargo:warning=Building WITHOUT the administrator manifest (test/non-elevated target).");
        } else {
            let windows = tauri_build::WindowsAttributes::new()
                .app_manifest(include_str!("windows/aetherdesk-admin.manifest"));
            attrs = attrs.windows_attributes(windows);
        }

        tauri_build::try_build(attrs).expect("failed to build Tauri app");
    }

    #[cfg(not(target_os = "windows"))]
    tauri_build::build();
}
