//! In-process-only per-(canonical Steam root, AppID) commit ownership.
//! No lock files: separate Desk processes are NOT coordinated here. Preparation (network)
//! is optimistic: any intervening Desk mutation or external Lua/ACF edit
//! invalidates the plan. Never hold a synchronous mutex across an await.
//! Lock order: game -> depotcache/config. Nested game acquisition is forbidden.
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, OnceLock, Weak,
    },
    time::Instant,
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};
#[derive(Hash, PartialEq, Eq, Clone)]
struct Key {
    root: String,
    app_id: u32,
}
struct Slot {
    gate: Arc<AsyncMutex<()>>,
    generation: AtomicU64,
}
fn slots() -> &'static Mutex<HashMap<Key, Weak<Slot>>> {
    static SLOTS: OnceLock<Mutex<HashMap<Key, Weak<Slot>>>> = OnceLock::new();
    SLOTS.get_or_init(|| Mutex::new(HashMap::new()))
}
fn root_key(root: &Path) -> Result<String, String> {
    let path = root
        .canonicalize()
        .map_err(|e| format!("Cannot resolve Steam mutation root: {e}"))?;
    let key = path.to_string_lossy().to_string();
    #[cfg(windows)]
    // Keep the extended prefix: stripping it breaks canonical UNC paths.
    let key = key.replace('/', "\\").to_lowercase();
    Ok(key)
}
fn slot(root: &Path, app_id: u32) -> Result<(Key, Arc<Slot>), String> {
    if app_id == 0 {
        return Err("A valid AppID is required for a mutation".into());
    }
    let key = Key {
        root: root_key(root)?,
        app_id,
    };
    let mut map = slots()
        .lock()
        .map_err(|_| "Mutation registry unavailable")?;
    map.retain(|_, value| value.strong_count() > 0);
    let value = map.get(&key).and_then(Weak::upgrade).unwrap_or_else(|| {
        let value = Arc::new(Slot {
            gate: Arc::new(AsyncMutex::new(())),
            generation: AtomicU64::new(0),
        });
        map.insert(key.clone(), Arc::downgrade(&value));
        value
    });
    Ok((key, value))
}

pub struct MutationGuard {
    _lock: OwnedMutexGuard<()>,
    slot: Arc<Slot>,
    key: Key,
    operation: &'static str,
    id: u64,
    started: Instant,
}
impl MutationGuard {
    fn new(
        lock: OwnedMutexGuard<()>,
        slot: Arc<Slot>,
        key: Key,
        operation: &'static str,
    ) -> Result<Self, String> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        crate::desk_log_info!(
            "mutations",
            "begin id={} operation={} app_id={} root={}",
            id,
            operation,
            key.app_id,
            key.root
        );
        Ok(Self {
            _lock: lock,
            slot,
            key,
            operation,
            id,
            started: Instant::now(),
        })
    }
}
impl Drop for MutationGuard {
    fn drop(&mut self) {
        // Also invalidate plans after partial failures; release != success.
        self.slot.generation.fetch_add(1, Ordering::SeqCst);
        crate::desk_log_info!(
            "mutations",
            "release id={} operation={} app_id={} elapsed_ms={} (outcome in caller log)",
            self.id,
            self.operation,
            self.key.app_id,
            self.started.elapsed().as_millis()
        );
    }
}
pub async fn acquire(
    root: &Path,
    app_id: u32,
    operation: &'static str,
) -> Result<MutationGuard, String> {
    let (key, slot) = slot(root, app_id)?;
    crate::desk_log_debug!(
        "mutations",
        "waiting operation={} app_id={} root={}",
        operation,
        app_id,
        key.root
    );
    let lock = slot.gate.clone().lock_owned().await;
    MutationGuard::new(lock, slot, key, operation)
}
/// Synchronous command/worker entry: fail busy, never block the UI/runtime or
/// deadlock against an async job. The monitor's existing retry handles busy.
pub fn try_acquire(
    root: &Path,
    app_id: u32,
    operation: &'static str,
) -> Result<MutationGuard, String> {
    let (key, slot) = slot(root, app_id)?;
    let lock = slot.gate.clone().try_lock_owned().map_err(|_| {
        crate::desk_log_warn!(
            "mutations",
            "busy operation={} app_id={}",
            operation,
            app_id
        );
        format!(
            "GAME_BUSY: another mutation is committing for app {app_id}; retry when it finishes"
        )
    })?;
    MutationGuard::new(lock, slot, key, operation)
}

#[derive(PartialEq, Eq)]
struct Fingerprint {
    path: PathBuf,
    digest: Option<[u8; 32]>,
}
fn fingerprint(path: PathBuf) -> Result<Fingerprint, String> {
    let digest = match std::fs::read(&path) {
        Ok(bytes) => Some(Sha256::digest(bytes).into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("Cannot snapshot {}: {e}", path.display())),
    };
    Ok(Fingerprint { path, digest })
}
fn snapshot(root: &Path, app_id: u32) -> Result<Vec<Fingerprint>, String> {
    let mut paths = vec![
        root.join("config/stplug-in").join(format!("{app_id}.lua")),
        root.join("steamapps/libraryfolders.vdf"),
    ];
    for library in crate::steam::library::SteamLibraryScanner::new(root).discover_library_paths() {
        paths.push(
            library
                .join("steamapps")
                .join(format!("appmanifest_{app_id}.acf")),
        );
    }
    paths.push(
        root.join("steamapps")
            .join(format!("appmanifest_{app_id}.acf")),
    );
    paths.sort();
    paths.dedup();
    paths.into_iter().map(fingerprint).collect()
}

pub struct MutationPlan {
    root: PathBuf,
    key: Key,
    slot: Arc<Slot>,
    generation: u64,
    files: Vec<Fingerprint>,
    operation: &'static str,
}
impl MutationPlan {
    pub fn prepare(root: &Path, app_id: u32, operation: &'static str) -> Result<Self, String> {
        let (key, slot) = slot(root, app_id)?;
        let generation = slot.generation.load(Ordering::SeqCst);
        let files = snapshot(root, app_id)?;
        Ok(Self {
            root: root.into(),
            key,
            slot,
            generation,
            files,
            operation,
        })
    }
    pub async fn commit(self) -> Result<MutationGuard, String> {
        let lock = self.slot.gate.clone().lock_owned().await;
        let guard = MutationGuard::new(lock, self.slot.clone(), self.key.clone(), self.operation)?;
        if self.slot.generation.load(Ordering::SeqCst) != self.generation
            || snapshot(&self.root, self.key.app_id)? != self.files
        {
            crate::desk_log_warn!(
                "mutations",
                "conflict operation={} app_id={} (prepared state changed; no Lua/ACF commit)",
                self.operation,
                self.key.app_id
            );
            return Err(format!("GAME_STATE_CONFLICT: app {} changed during preparation; retry on the current state", self.key.app_id));
        }
        Ok(guard)
    }
    pub async fn commit_for(self, app: &tauri::AppHandle) -> Result<MutationGuard, String> {
        let root = self.key.root.clone();
        let guard = self.commit().await?;
        ensure_current_root(app, Path::new(&root))?;
        Ok(guard)
    }
}

pub fn ensure_current_root(app: &tauri::AppHandle, root: &Path) -> Result<(), String> {
    let current = super::settings::SettingsManager::new(app)
        .try_load()?
        .steam_path;
    if root_key(Path::new(&current))? != root_key(root)? {
        crate::desk_log_warn!(
            "mutations",
            "Steam root changed during preparation; commit cancelled"
        );
        return Err("GAME_STATE_CONFLICT: Steam root changed during preparation; retry".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn same_resource_excluded_different_app_and_root_independent() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let guard = acquire(a.path(), 10, "test").await.unwrap();
        assert!(try_acquire(a.path(), 10, "test").is_err());
        assert!(try_acquire(&a.path().join("."), 10, "alias").is_err());
        assert!(try_acquire(a.path(), 11, "test").is_ok());
        assert!(try_acquire(b.path(), 10, "test").is_ok());
        drop(guard);
        assert!(try_acquire(a.path(), 10, "test").is_ok());
    }
    #[tokio::test]
    async fn stale_plan_rejected_after_even_noop_commit() {
        let d = tempfile::tempdir().unwrap();
        let p = MutationPlan::prepare(d.path(), 1, "test").unwrap();
        drop(acquire(d.path(), 1, "other").await.unwrap());
        assert!(p.commit().await.is_err());
        assert!(MutationPlan::prepare(d.path(), 1, "fresh")
            .unwrap()
            .commit()
            .await
            .is_ok());
    }
    #[tokio::test]
    async fn external_lua_change_rejected() {
        let d = tempfile::tempdir().unwrap();
        let p = MutationPlan::prepare(d.path(), 1, "test").unwrap();
        std::fs::create_dir_all(d.path().join("config/stplug-in")).unwrap();
        std::fs::write(d.path().join("config/stplug-in/1.lua"), b"external").unwrap();
        assert!(p.commit().await.is_err());
    }
    #[tokio::test]
    async fn cancelled_waiter_does_not_strand_lock() {
        let d = tempfile::tempdir().unwrap();
        let guard = acquire(d.path(), 1, "leader").await.unwrap();
        let root = d.path().to_owned();
        let waiter = tokio::spawn(async move { acquire(&root, 1, "waiter").await });
        tokio::task::yield_now().await;
        waiter.abort();
        let _ = waiter.await;
        drop(guard);
        assert!(try_acquire(d.path(), 1, "after-cancel").is_ok());
    }
    #[tokio::test]
    async fn external_acf_change_rejected() {
        let d = tempfile::tempdir().unwrap();
        let p = MutationPlan::prepare(d.path(), 1, "test").unwrap();
        std::fs::create_dir_all(d.path().join("steamapps")).unwrap();
        std::fs::write(
            d.path().join("steamapps/appmanifest_1.acf"),
            b"external steam update",
        )
        .unwrap();
        assert!(p.commit().await.is_err());
    }
    #[tokio::test]
    async fn blocking_worker_keeps_ownership_after_waiter_abort() {
        let d = tempfile::tempdir().unwrap();
        let guard = acquire(d.path(), 1, "worker").await.unwrap();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        let worker = tokio::task::spawn_blocking(move || {
            let _guard = guard;
            let _ = started_tx.send(());
            let _ = finish_rx.recv();
        });
        started_rx.await.unwrap();
        worker.abort(); // A running blocking worker cannot be cancelled.
        assert!(try_acquire(d.path(), 1, "other").is_err());
        finish_tx.send(()).unwrap();
        worker.await.unwrap();
        assert!(try_acquire(d.path(), 1, "after").is_ok());
    }
    #[test]
    fn game_coordination_creates_no_files() {
        let root = tempfile::tempdir().unwrap();
        let guard = try_acquire(root.path(), 1, "first").unwrap();
        assert!(try_acquire(root.path(), 1, "same-game").is_err());
        assert!(try_acquire(root.path(), 2, "other-game").is_ok());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
        drop(guard);
        assert!(try_acquire(root.path(), 1, "released").is_ok());
        assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    }
    #[tokio::test]
    async fn toggle_invalidates_remote_plan_without_losing_new_lua() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("config/stplug-in")).unwrap();
        let lua = dir.path().join("config/stplug-in/10.lua");
        std::fs::write(&lua, "addappid(10)\nsetManifestid(10, \"123\")\n").unwrap();
        let remote = MutationPlan::prepare(dir.path(), 10, "remote-version").unwrap();
        {
            let _toggle = try_acquire(dir.path(), 10, "toggle").unwrap();
            crate::manifest::pins::LuaManifestPins::new(dir.path(), 10)
                .set_updates_enabled(true)
                .unwrap();
        }
        let after_toggle = std::fs::read(&lua).unwrap();
        assert!(remote.commit().await.is_err());
        assert_eq!(std::fs::read(&lua).unwrap(), after_toggle);
        assert!(crate::manifest::pins::LuaManifestPins::new(dir.path(), 10)
            .updates_are_enabled()
            .unwrap());
    }
}
