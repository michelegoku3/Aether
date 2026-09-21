fn main() {
    #[cfg(target_os = "windows")]
    {
        // AetherDesk scrive dentro la cartella di Steam (localconfig.vdf,
        // depotcache, stplug-in) e installa una DLL: l'exe di release chiede
        // requireAdministrator.
        //
        // Quel manifest viene però embeddato in OGNI artefatto prodotto da
        // questo build script — incluso l'harness che `cargo test` compila ed
        // ESEGUE. Risultato: su Windows ogni `cargo test` apriva un prompt UAC
        // (o falliva con ERROR_ELEVATION_REQUIRED in un contesto non
        // interattivo), quindi i test non venivano lanciati né in dev né in CI.
        //
        // Via d'uscita: `AETHERDESK_NO_ADMIN_MANIFEST=1` produce un exe
        // asInvoker. Lo impostano `build.cmd` (step di test) e la CI. Il build
        // di release NON lo imposta, quindi l'exe pubblicato resta identico a
        // prima — l'elevazione è una proprietà del prodotto, non dei test.
        //
        // Nota: la variabile è letta qui e non tramite un feature flag perché
        // `cargo test` non può disattivare una feature di default del package
        // senza `--no-default-features`, che a sua volta disattiverebbe tutto
        // il resto. L'env var è il canale che Cargo già propaga ai build
        // script (`rerun-if-env-changed` sotto).
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
