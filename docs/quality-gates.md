# Quality gates — cosa controlla il build e dove

Regola unica: **ogni controllo gira identico in locale e su GitHub**. Il
workflow di release (`.github/workflows/build.yml`) non aggiunge nulla che
`build.cmd` o "Compila tutto" in Visual Studio non facciano già; serve solo da
ultima barriera prima della pubblicazione.

## AetherDesk — `build.cmd` (e job `release-desk`)

| Step | Comando | Cosa blocca | Dove si configura |
|---|---|---|---|
| 3 | `npm audit --audit-level=high` | CVE high/critical nelle dipendenze npm, senza modificare lockfile o working tree | — (`/skipaudit` per saltare) |
| 4 | `npm run verify` → `check:tauri-versions` | disallineamento major/minor tra runtime Rust, API JS e CLI Tauri | `scripts/check_tauri_versions.mjs` |
| 4 | `npm run verify` → build | errori di tipo TS (`strict`, `noUnusedLocals/Parameters`) | `tsconfig.json` |
| 4 | `npm run verify` → `lint:ci` | regole ESLint a livello `error` (hook, import inutilizzati, fallthrough) | `eslint.config.js` |
| 4 | `npm run verify` → `test:ci` | regressioni React/TypeScript e soglie coverage | `vitest.config.ts` |
| 4 | `npm run verify` → `check:wiring` | file `.tsx/.ts` mai importati da `main.tsx` | `scripts/check_frontend_wiring.mjs` |
| 4 | `npm run verify` → `check:unused` (**knip**) | export/tipi mai importati, dipendenze npm non usate o non dichiarate | `knip.json`, `scripts/check_unused.mjs` |
| 5 | `cargo clippy --all-targets -- -D warnings` | ogni warning rustc/clippy: codice morto, import inutili, `unsafe` senza `// SAFETY:`, lock tenuti attraverso `.await`, clone ridondanti, troncamenti di cast | `Cargo.toml` → `[lints.rust]`, `[lints.clippy]` |
| 6 | `cargo audit` | CVE note nelle crate (database RustSec) | — (`/skipaudit`) |
| 6 | `cargo machete` | dipendenze in `Cargo.toml` mai usate | — |
| 7 | `cargo test` | unit test + test di contratto IPC | `src/tests/` (`/skiptests`) |

`cargo-audit` e `cargo-machete` vengono installati automaticamente la prima volta.

### Come leggere un fallimento

- **clippy**: il messaggio indica il lint (`clippy::xxx`) e un link. Se il lint
  è rumore sistematico, si disattiva **in `Cargo.toml` con un commento che dice
  perché**, non con `#[allow]` sparsi nel codice. Un `#[allow]` locale è
  accettabile solo con un commento sulla riga.
- **npm audit**: aggiornare esplicitamente la dipendenza diretta e rigenerare il
  lockfile; il build non esegue `npm audit fix`, perché un quality gate non deve
  mutare dipendenze o sorgenti mentre li verifica.
- **knip – unused export**: la funzione/tipo è esportata ma nessun altro file
  la importa → togliere `export` (se è usata solo lì) o cancellarla.
- **knip – unused dependency**: rimuoverla da `package.json`.
- **cargo audit**: aggiornare la crate (`cargo update -p <crate>`); se non c'è
  fix, `cargo audit --ignore RUSTSEC-XXXX` con motivazione in `build.cmd`.
- **cargo machete**: rimuovere la riga da `Cargo.toml`. Falsi positivi
  (crate usate solo via macro) si dichiarano in
  `[package.metadata.cargo-machete] ignored = ["..."]`.

## AetherDLL — CMake (Visual Studio "Compila tutto" e job `release-dll`)

Tutto vive in `AetherDLL/cmake/Quality.cmake` ed è applicato a ogni target con
`aether_apply_quality(<target>)`.

| Controllo | Cosa blocca | Escape hatch |
|---|---|---|
| `/W4 /permissive- /Zc:preprocessor` | compila a livello warning alto e C++ conforme (prima: default W1) | — |
| `/WX` | ogni warning MSVC è un errore | `-DAETHER_WARNINGS_AS_ERRORS=OFF` (stampa un WARNING a configure) |
| `/external:W0` | zero rumore da protobuf/abseil/lua/MinHook | — |
| **clang-tidy** | bug C++ statici: use-after-move, narrowing, copie evitabili, lock mancanti, UB (CERT) | `-DAETHER_CLANG_TIDY=OFF`; check in `AetherDLL/.clang-tidy` |
| `aether_check_sources` | `.cpp` nella cartella del target ma non nel target | — |
| `aether_check_headers` | `.h/.inl` che nessun file del progetto include | — |
| `aether_run_tests` (ALL) | `ctest` gira a ogni build; un test rosso = build rosso | `-DAETHER_BUILD_TESTS=OFF` |
| `verify_version.ps1` | DLL senza version resource | — |

clang-tidy si attiva da solo se trova il binario (Visual Studio Installer →
*Singoli componenti* → **"C++ Clang tools for Windows"**). Se manca, il
configure lo dice chiaramente e prosegue.

### Prima attivazione (una tantum)

`/W4 /WX` e clippy applicati a ~60k righe esistenti faranno emergere un
arretrato. Procedura consigliata, un componente alla volta:

```text
AetherDLL : cmake --preset x64-Release -DAETHER_WARNINGS_AS_ERRORS=OFF
            → leggi i warning, sistemali, poi togli il flag (torna ON)
AetherDesk: cd AetherDesk/src-tauri && cargo clippy --all-targets --fix --allow-dirty
            → sistema a mano ciò che resta, poi build.cmd
Frontend  : cd AetherDesk && npm install && npx knip
            → rimuovi export/dipendenze segnalati, poi build.cmd
```

Dopo questa pulizia il baseline è zero e ogni nuovo warning è attribuibile al
commit che lo introduce.
