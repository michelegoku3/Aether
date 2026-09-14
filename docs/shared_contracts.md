# Shared contracts — AetherDesk ↔ AetherDLL

Specifica unica delle logiche **gemellate** tra AetherDesk (Rust/Tauri) e
AetherDLL (C++, in-process in Steam). Le due parti girano in processi diversi,
in momenti diversi (Desk può essere chiuso; il DLL vive dentro Steam) e devono
comunque produrre **lo stesso albero di backup, la stessa cache e lo stesso
consumo di quota**. Questa pagina è la fonte di verità della *policy*; il codice
la implementa due volte, i test di contratto (§6) la proteggono.

> **Regola di manutenzione**: se cambi un valore o una regola qui, devi
> allineare ENTRAMBE le implementazioni e aggiornare questa tabella nella
> stessa commit. Ogni sezione elenca i due "gemelli" con path precisi.

---

## 1. Layout su disco (single source of truth)

| Percorso | Contenuto | Scrittori |
|---|---|---|
| `<Steam>\depotcache\<depot>_<gid>.manifest` | cache primaria di Steam | Steam, Desk (resolver/workshop/store), DLL (generazione live, restore) |
| `<Steam>\config\depotcache\` | cache secondaria di Steam (sola lettura per noi) | Steam |
| `<AetherData>\backup\<app_id>\lua\<app_id>.lua` | Lua **originale** (pristino dalle fonti) | Desk (`store_original`) |
| `<AetherData>\backup\<app_id>\lua\history\<app>-<millis>[-n].lua` | versioni modificate, deduplicate per contenuto | Desk (`archive_lua_history`), DLL (solo copia "as-missing") |
| `<AetherData>\backup\<app_id>\lua\<depot>_<gid>.manifest` | manifest referenziati (pin attivi E commentati) + manifest Workshop | Desk e DLL (contratto §2) |
| `<AetherData>\state\hubcap_generation_quota.json` (+ `.lock` sibling) | budget giornaliero condiviso | Desk e DLL (contratto §4) |
| `<AetherData>\temp\hubcap\workshop\<item>.manifest` | cache Workshop keyed per item (solo Desk) | Desk |
| `<AetherData>\state\hubcap_game_updates.json` | checkpoint del monitor (fingerprint, `contents_checked`, …) | solo Desk |

Il nome file `<depot>_<gid>.manifest` è l'**identità completa** del manifest:
due byte diversi con lo stesso nome non esistono in pratica; per questo il
contratto di copia non sovrascrive mai (§2).

## 2. Contratto di backup/restore dei manifest

Gemelli:
- Desk: `AetherDesk/src-tauri/src/core/backup.rs` → `backup_referenced_manifests`,
  `backup_manifest_bytes`, `sync_lua_backups_from_stplug_in`.
- DLL: `AetherDLL/AetherCore/hooks/wire/ManifestRestore.cpp` →
  `CopyAtomicallyIfMissing`, `BackupReferencedManifestsAtStartup`,
  `BackupManifestAfterGeneration`, `RestoreLuaDirectory`/`RestoreAppOnce`.

Regole (identiche nei due processi):
1. si copiano **solo** i manifest referenziati dal Lua corrente (pin attivi o
   commentati — la regex non è ancorata a inizio riga, quindi `--setManifestid`
   conta come referenza); i manifest Workshop hanno un canale dedicato
   (`backup_manifest_bytes` lato Desk, keyed sull'app proprietaria);
2. una destinazione **non vuota non viene mai sovrascritta**;
3. una destinazione da **0 byte è un placeholder**: va rimossa e riscritta;
4. la copia è **atomica** (temp + rename/`AtomicReplace`) e **verificata per
   size**;
5. una copia fallita **ricontrolla la destinazione**: se l'altro processo ha
   completato nella finestra di race, il file conta come presente, non come
   errore.

Ordinamento di ripristino (local-first, identico nei due):
`depotcache` → `config\depotcache` → `backup\<app_id>\lua`.
Il DLL, quando trova il file nel backup, lo **pubblica** in depotcache prima di
rispondere hit; il Desk (`manifest/resolver.rs`) fa lo stesso restore-then-gate.
Un file vuoto non è MAI un hit locale.

Backup del **Lua**: l'originale è la versione pristina (primo Lua visto o
ultimo scaricato dalle fonti); ogni contenuto diverso va in `history/`,
deduplicato per byte. Gli archivi nella stessa millisecondo prendono un
suffisso `-n` (i pipeline pin archiviano pre+post back-to-back). Chiunque
riscriva un Lua deve archiviare **pre E post** (vedi §5, matrice nel report §12).

## 3. Contratto di commit "manifest prima, Lua ultimo"

Il Lua in `config\stplug-in\` è il **grilletto** dell'hot-reload del DLL: nel
momento in cui appare, ogni manifest che referenzia deve essere già visibile in
depotcache. Implementazioni:
- Desk: `steam/compat.rs` → `install_lua_and_manifest_files` (staging → remove
  vecchi target → rename manifest → verifica → rename Lua → verifica).
- Desk: `commands/manifests.rs` → `refresh_game_pins_from_hubcap` (gate di
  completezza PRIMA di `realign_commented_pins`).
- DLL: la generazione live scrive in depotcache (temp + `AtomicReplace` +
  verifica size) senza mai toccare il Lua.

## 4. Contratto di quota e generazione Hubcap

Gemelli: `AetherDesk/src-tauri/src/providers/hubcap_generation.rs` ↔
`AetherDLL/AetherCore/network/HubcapQuota.cpp`.

- Budget giornaliero: **game 1500/giorno**, **workshop 500/giorno**; reset a
  **mezzanotte EST fissa** (come il provider), non mezzanotte locale.
- Un solo file di stato + `.lock` sibling: ogni ciclo read-modify-write avviene
  sotto lock, quindi i contatori restano coerenti qualunque processo scriva.
- **reserve → request → release-on-failure**: un tentativo fallito restituisce
  l'unità riservata (nessun errore transitorio brucia quota).
- Il DLL riserva solo generazioni *game* (il workshop è solo Desk).
- Endpoint di generazione game: `GET /generate/manifest?depot_id=&manifest_id=`;
  la risposta va validata per identità (depot+gid nel protobuf) prima
  dell'install.

Policy di traffico (valori attuali — allineare qui se cambiano):

| Parametro | Desk | DLL |
|---|---|---|
| Serializzazione generazioni | scheduler di processo, intervallo min **750 ms** | worker serializzato (coda), niente retry lunghi in-pass |
| Tentativi per generazione | 1 (il retry è a livello lane) | **2** × timeout **6 s**, sleep 750 ms tra tentativi |
| Backoff su fallimento | lane monitor: **30 s × 2^n**, cap **30 min** | proactive: **{30, 60, 120, 300, 600, 900} s**, max **6** retry |
| Finestra bridge (recv CM) | — | `manifestBridgeWaitMs` default **2500 ms** (clamp 0–10000), poi passthrough + continuazione background |
| Cap per passata | workshop: **5** generazioni remote/run (deferred → re-run dopo **90 s**); pin_refresh: 1 app/poll, intervallo **15 min** | proactive queue: 1 chiave (gid,depot) alla volta, dedup per chiave |
| Poll/cadenza | monitor: start delay **8 s**, poll **20 s** | DirWatch/event-driven + coda proactive |

Le asimmetrie sono **intenzionali**: il DLL sta dentro il thread di rete di
Steam (deve rispondere in ~ms e non può accodare work infinito), il Desk è un
orchestratore UI (può permettersi lane periodiche e batch). I valori però
devono restare compatibili: es. la finestra del bridge (2,5 s) ≥ p95 della
generazione live (~1 s); il backoff DLL (max 15 min) ≈ cap backoff Desk (30 min).

## 5. Contratto dei pin Lua

- Pin **attivo** (`setManifestid(depot,"gid")`) = version-lock: Steam deve
  restare su quel gid; i pipeline automatici non lo toccano MAI.
- Pin **commentato** = informativo: il gioco è in modalità update; il valore
  commentato deve puntare all'ultima versione nota perché "Disable updates"
  lo riattiva.
- I **GID non sono ordinati cronologicamente**: "più nuovo" non esiste; il
  segnale è "diverso dal pin", con priorità all'evidenza locale
  (`latest_installed_gid`, mtime su depotcache) rispetto alla vista Hubcap
  (`/manifest/{appid}/contents`, endpoint gratuito).
- Ogni riscrittura di Lua (realign, toggle, policy, versione, edits) deve
  archiviare pre+post in `history/` e backuppare i manifest referenziati
  (matrice completa: report §12).

## 6. Test di contratto

- **Desk** (`cargo test` in `AetherDesk/src-tauri`):
  `core/backup.rs::tests` — regole 2/3/4 del contratto di copia
  (`backup_manifest_bytes`), sostituzione placeholder 0-byte, payload vuoto
  rifiutato, `store_original` (Created/Updated/Unchanged), dedup di
  `store_history_version`, unicità degli archivi same-millisecond.
- **DLL**: nessuna harness nel repo (build Windows). Checklist manuale quando
  si tocca `ManifestRestore.cpp`/`ManifestFetch.cpp`:
  1. copia con destinazione non vuota → invariata; 2. destinazione 0 byte →
  sostituita; 3. size verificata post-copy; 4. race Desk/DLL → nessun errore
  spurio (regola 5); 5. `BackupManifestAfterGeneration` intercetta anche i pin
  commentati; 6. quota: release su fallimento.
- Se una regola di §2–§5 cambia: aggiornare entrambi i gemelli, questa pagina
  e i test nella stessa commit.
