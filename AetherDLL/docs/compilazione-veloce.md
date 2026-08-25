# Compilazione veloce di AetherDLL

Guida al workflow di sviluppo con agenti AI nel browser: come ridurre i tempi di
compilazione da ~576 step ("Ricompila tutto") a pochi secondi.

## Il problema

1. I file scaricati da GitHub hanno come timestamp l'ora del commit; il
   copia-incolla di Windows la conserva. Ninja confronta i timestamp: se il
   sorgente sembra "più vecchio" degli `.obj` già compilati, **Compila tutto
   non ricompila nulla** anche se il contenuto è diverso.
2. Per questo si era costretti a usare **Ricompila tutto**, che però pulisce e
   ricompila anche protobuf/abseil/lua/minhook (~500 step su 576).

## Il nuovo workflow (3 passi)

1. **Sostituisci i file** scaricati dentro `AetherDLL\` con il copia-incolla
   **senza cancellare la cartella** (la `out\` con la cache di build deve
   sopravvivere).
2. **Esegui `Tools\sync_aetherdll.cmd`** (doppio click). Confronta gli hash dei
   contenuti e aggiorna il timestamp **solo dei file davvero cambiati**.
3. In Visual Studio: **Genera → Compila tutto** (MAI "Ricompila tutto").
   Ninja ricompila solo i file toccati + link: in genere **10-60 secondi**.

## Protobuf pre-generato (default ON)

`proto/generated/steam_messages.pb.{h,cc}` sono **generati una volta con
protoc 25.3 e committati nel repo**. Così la build non compila più
`protoc` + `libprotoc` (~120 step) né il `libprotobuf` completo (~70 step):
AetherCore linka solo `libprotobuf-lite`, e con `EXCLUDE_FROM_ALL` ninja
costruisce soltanto le librerie realmente linkate.

- **Se modifichi `proto/steam_messages.proto`**: esegui
  `Tools\regen_proto.cmd` per rigenerare i file in `proto/generated/` e
  committali insieme al `.proto`. (Richiede protoc 25.3; se non c'è, lo script
  spiega come ottenerlo.)
- **Escape hatch**: configura con `-DAETHER_PROTO_PREGEN=OFF` per tornare al
  comportamento vecchio (protoc compilato e rigenerazione automatica a ogni
  modifica del `.proto`).

Verificato: i file committati compilano e serializzano correttamente contro il
runtime protobuf-lite v25.3 (round-trip test eseguito).

## ccache / sccache (opzionale, consigliato)

Un *compiler launcher* mette in cache ogni oggetto compilato: anche un
"Ricompila tutto" diventa quasi istantaneo sulle parti invariate (è la tecnica
standard usata in CI e nei team C++).

1. Scarica ccache per Windows: <https://github.com/ccache/ccache/releases>
   (zip `windows-x86_64`), estrai e metti la cartella nel `PATH`.
2. Imposta una volta le variabili d'ambiente (richieste con MSVC + PCH):
   ```
   setx CCACHE_SLOPPINESS "pch_defines,time_macros"
   setx CCACHE_DIR "%LOCALAPPDATA%\ccache"
   ```
3. Riavvia Visual Studio: CMake rileva ccache automaticamente
   (`[AetherDLL] Compiler cache enabled: ...` nell'output di configure).
   Per disattivarlo: variabile cache `AETHER_COMPILER_CACHE=OFF`.

## Numeri attesi

| Scenario | Step di compilazione |
|---|---|
| Prima (Ricompila tutto) | 576 |
| Full rebuild senza protoc/libprotoc/libprotobuf | ~330-390 |
| Full rebuild **con ccache** (cache calda) | ~330-390 ma quasi tutti da cache |
| **Workflow normale: sync + Compila tutto** | **solo i file cambiati + link** |

## Nota per AetherDesk (Tauri/Rust)

- La UI: usa `npm run build:fast` (solo `vite build`) durante lo sviluppo;
  `tsc` (typecheck completo, sempre più lento al crescere del codice) resta in
  `npm run build` per il packaging e in `npm run typecheck` quando vuoi
  verificarlo.
- Il backend Rust è **un solo crate**: ogni modifica ricompila tutto il crate e
  rilinka ~546 dipendenze. Per le build di prova locali valuta
  `[profile.release] incremental = true` in `Cargo.toml` (binari un po' meno
  ottimizzati, ricompilazioni molto più rapide; toglilo per i rilasci).
- Tieni `src-tauri\target\` e `node_modules\` **fuori** dalle cartelle che
  sostituisci, altrimenti ogni sostituzione riparte da zero (10-20 minuti).
- Aggiungi un'esclusione di Windows Defender per la cartella del progetto e per
  `target\`: su Windows è uno dei rallentamenti più comuni.
