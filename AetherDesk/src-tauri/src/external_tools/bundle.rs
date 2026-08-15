//! Locazione e installazione idempotente dei bundle dei tool esterni.
//!
//! Generalizzazione del locator di Steamless (`steamless/tool_locator.rs`):
//! un tool esterno viene cercato prima nella sua directory installata, poi
//! copiato dal bundle vendored nella repo/risorse. Ogni tool fornisce il
//! proprio validatore (es. "esiste Steamless.CLI.exe + Plugins/"), così
//! questa classe non sa nulla dei singoli tool (basso accoppiamento).

use std::fs;
use std::path::{Path, PathBuf};
use tauri::Manager;

/// Un bundle di tool esterno localizzato e pronto all'uso.
#[derive(Debug, Clone)]
pub struct ToolBundle {
    /// Directory radice del tool (es. `<install>/ExternalTools/Steamless`).
    pub dir: PathBuf,
}

/// Localizza (e se necessario installa) il bundle di un external tool.
pub struct ToolBundleLocator {
    app: tauri::AppHandle,
    /// Sottocartella delle risorse bundled, es. `"ExternalTools/Steamless"`.
    resource_subdir: &'static str,
    /// Directory installata di destinazione (tipicamente da `LocalAppPaths`).
    installed_dir: PathBuf,
    /// Predicato che stabilisce se una directory contiene un bundle valido.
    validator: Box<dyn Fn(&Path) -> bool>,
}

impl ToolBundleLocator {
    pub fn new(
        app: tauri::AppHandle,
        resource_subdir: &'static str,
        installed_dir: PathBuf,
        validator: impl Fn(&Path) -> bool + 'static,
    ) -> Self {
        Self {
            app,
            resource_subdir,
            installed_dir,
            validator: Box::new(validator),
        }
    }

    /// Restituisce un bundle valido, installandolo dal bundled source se
    /// la directory installata non esiste o non è valida.
    pub fn locate(&self) -> Result<ToolBundle, String> {
        if self.is_valid(&self.installed_dir) {
            return Ok(ToolBundle {
                dir: self.installed_dir.clone(),
            });
        }

        if let Some(source_dir) = self.bundled_source_dir() {
            self.copy_bundle(&source_dir, &self.installed_dir)?;
            if self.is_valid(&self.installed_dir) {
                return Ok(ToolBundle {
                    dir: self.installed_dir.clone(),
                });
            }
        }

        Err(format!(
            "Tool bundle was not found. Expected installed directory: {}",
            self.installed_dir.display()
        ))
    }

    fn is_valid(&self, dir: &Path) -> bool {
        (self.validator)(dir)
    }

    fn bundled_source_dir(&self) -> Option<PathBuf> {
        let mut candidates = Vec::new();

        if let Ok(resource_dir) = self.app.path().resource_dir() {
            candidates.push(resource_dir.join(self.resource_subdir));
        }

        candidates.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(self.resource_subdir));

        candidates
            .into_iter()
            .find(|candidate| self.is_valid(candidate))
    }

    /// Copia il bundle bundled nella directory installata. Idempotente ma
    /// distruttivo: una directory installata incompleta/invalida viene
    /// rimossa e ricreata dal source (stesso comportamento del vecchio
    /// locator Steamless).
    fn copy_bundle(&self, source: &Path, destination: &Path) -> Result<(), String> {
        if destination.exists() {
            let _ = fs::remove_dir_all(destination);
        }

        copy_dir_all(source, destination).map_err(|error| {
            format!(
                "Failed to install bundled tool from {} to {}: {}",
                source.display(),
                destination.display(),
                error
            )
        })
    }
}

fn copy_dir_all(source: &Path, destination: &Path) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;

    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let target = destination.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_all(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), target)?;
        }
    }

    Ok(())
}
