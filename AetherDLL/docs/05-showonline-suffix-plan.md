# Piano `-showonline` definitivo — appid esatto via `game_extra_info` (versione "suffix")

> Unisce il lavoro delle due fasi precedenti (misurazioni in `04-showonline-plan.md`)
> con il trasporto deciso dopo il test live del 24/08. Sostituisce e rende obsoleto
> il canale rich-presence `aether_appid` (mai rilanciato dal server).

---

## 1. Fatti misurati (basi del design)

1. **Il CM non diffonde appid non licenziati.** Qualunque sia il `game_id` inviato — tipo
   App, GameMod, Shortcut o P2P — il server valida l'appid nei 24 bit bassi e scarta
   l'intero `ClientGamesPlayed` se non c'è licenza (Fase 0/1).
2. **Con la maschera 480 (ovvero `-onlinefix`) la presenza viaggia** e il testo
   `game_extra_info` che spediamo torna agli amici dentro `Friend.game_name`
   (misurato: `app=480 gameid=480 name='The Stanley Parable: Ultra Deluxe'`).
3. **Le rich-presence KV non sono un canale affidabile**: la prova del 24/08 (KV
   `aether_appid` staged e "delivered" lato mittente, mai vista lato amico) chiude
   quella strada. `game_extra_info`, invece, è l'unico dato che sappiamo arrivare
   sempre.
4. **L'icona si ricostruisce sul PC di chi guarda**: la foto storica (icona CK3) era
   prodotta dal client dell'amico, non dal server. Il metodo funzionante dell'altra
   build — reverse lookup titolo → appid sulla libreria Lua locale + patch della entry
   persona — è confermato dall'utente: con il gioco presente (o .lua) su entrambi i PC,
   nome e immagine compaiono.
5. **Riscrivere la propria entry uccide la sessione Spacewar** (regressione misurata):
   il patch degli amici salta sempre `friendid == selfId`.
6. **I metadati PICS non sono soggetti a licenza**: `ClientPICSProductInfoRequest`
   (8903) riceve risposta per qualsiasi appid; le risposte 8904 scorrono già sul filo.
   Serve a riempire la cache AppInfo del PC che guarda quando l'app non è mai stata
   vista (altrimenti l'icona non ha da cosa rendersi).

## 2. Canale definitivo: suffisso appid in `game_extra_info`

Per ogni sessione mascherata (`-onlinefix`, oppure `-showonline` — dove il processo
resta reale e solo il frame di presenza viene riscritto), il mittente scrive:

```
game_extra_info = "<nome mostrabile> | <appid decimale>"
```

- Il server lo ricicla in `Friend.game_name`: il PC amico lo legge **sotto forma di
  testo garantito**, non di KV (cfr. fatto 2 e 3).
- Il nome resta leggibile per gli amici vanilla (vedono tutta la stringa: compromesso
  accettato, il nome è la parte importante e arriva per primo).
- `presenceCustomGameName` continua a valere come override del nome; il suffisso
  appid viene comunque accodato.
- Separatore `" | "` con parsing **dall'ultima occorrenza**: i due punti e i pipe
  interni ai titoli non rompono nulla (testato in harness: 14/14 casi).

### Lato che guarda (`PersonaInject`, entry con `game_played_app_id == 480`)

Catena di risoluzione, in ordine:

1. **Suffisso** `" | <appid>"` da `game_extra_info`/KV o da `Friend.game_name`
   (esatto, indipendente dalla lingua, nessuna collisione).
2. **Reverse lookup per titolo** (`friend_appid_from_name`, metodo esistente,
   solo libreria configurata: case/space-insensitive, cache con negativi) —
   copre mittenti senza suffisso.
3. **Legacy locale** (`onlinefix_persona_patch` + `onlineFixRealAppId`) —
   comportamento preesistente, invariato.

Applicato il patch (`game_played_app_id`, `gameid`, `game_name` = titolo del cache
AppInfo locale, altrimenti il testo pulito del suffisso, altrimenti campo svuotato
perché la UI stock risolva da sola). Se il cache AppInfo locale **non sa nemmeno
nominare** l'app → viene originata una `ClientPICSProductInfoRequest` (8903, una
sola volta per appid per processo) tramite il nuovo `PacketRouter::SendClientFrame`,
così la risposta riempie il cache standard e l'icona può comparire.

La propria entry viene **sempre saltata** (fatto 5).

## 3. Invarianti protette

- La voce **self** della presenza locale resta gestita da `presenceInjectLocal`
  (gioco reale in vista locale), mai dal friend-patch.
- `-onlinefix` (processo mascherato + payload multiplayer) **vince** su `-showonline`
  se entrambi i flag sono presenti.
- `-showonline` continua a NON toccare il processo: achievement/DLC/cloud/screenshot/
  overlay restano identici a un avvio senza flag.
- Giochi **posseduti**: nessun masking, nessun suffisso — il CM trasmette già
  correttamente (fatto 1), toccare qualcosa sarebbe regressione.

## 4. File toccati (rispetto a `5dc0f2f`)

| file | contenuto |
|------|-----------|
| `hooks/wire/GamesPlayedModule.cpp` | rewrite `-showonline` inline (game_id → 480 + extra_info con suffisso); suffisso anche per le entry 480 di `-onlinefix`; `[DIAG] TX` change-triggered |
| `hooks/wire/PersonaInject.cpp` | catena suffisso → by-name → legacy; skip self; `EnsureAppInfo` (PICS); `[DIAG] SERVER self-push` + dump KV self/friend |
| `hooks/wire/PacketRouter.{h,cpp}` | `SendClientFrame` (frame originati con header clonato e job id azzerati); cattura connessione; `[DIAG] SharedLibrary`; timbro `[DIAG] BUILD showonline-suffix` |
| `utils/GameNameResolver.{h,cpp}` | `ResolveAppIdByName` (lookup inverso libreria configurata, cache ±) |
| `core/Settings.{h,cpp}` | `presenceShowOnlineBroadcast`, `presenceFriendAppIdFromName` (default ON) |
| `core/Constants.h` | `kShowOnlineFlag`, `kExtraInfoAppIdSep` |
| `core/AetherCoreState.h` | `showOnlineAppId` |
| `hooks/steamclient/OnlineFixHooks.{h,cpp}` | rilevamento flag `-showonline` (niente maschera) |
| `diagnostics/StatusWriter.cpp` | dump `showonline_appid` + toggle |
| `proto/steam_messages.proto` | `process_id=9`, `game_flags=11`, `owner_id=12` (numeri verificati su SteamKit; owner_id riservato alla via Family Sharing) |
| `config/aethercore.example.toml` | documentazione delle due chiavi |
| `AetherDesk` (`steam.rs`, `main.rs`, `LibraryGameActionsModal.tsx`) | azione "Show Online" + mutua esclusione con `-onlinefix` (round 1, invariata) |
| `docs/04-showonline-plan.md` | le misurazioni dell'altra fase (riferimento) |

Rimossi tutti i componenti del canale KV `aether_appid` (modulo ShowOnline, merge
7501, codec KV1 dedicato, contatori di broadcast) e l'esperimento CGameID-tipizzato
(falsificato: non riproporlo).

## 5. Test da eseguire (log da mandare se fallisce qualcosa)

Setup: `[log] level = "debug"` (o `info` per il minimo). Log `<Steam>\aethercore\main.log`.

- **T0 build check**: in main.log deve comparire `[DIAG] BUILD showonline-suffix | ...`
  — se manca, la DLL caricata NON è questa build (l'updater di DESK dice 0.9.10 per
  entrambe le build: non fidarsi di quel numero, **non lanciare l'auto-update**).
- **T1 mittente (`-showonline`)**: `ShowOnline session for app <id>: process NOT masked`;
  `[DIAG] TX[0] game_id=... (app=480) extra='<Nome> | <id>'`;
  `showonline: games_played <id> -> 480 (extra_info ...)`. Achievement/DLC/cloud intonsi.
- **T2 mittente (`-onlinefix`)**: come prima + `[DIAG] TX` con `extra='<Nome> | <id>'`;
  regressione multiplayer/DLC nulla.
- **T3 amico (due account amici, lista amici aperta)**: sul PC amico cerca
  `Patched friend <steamid>: 480 -> <id> (extra_info)`; lista amici con nome reale e
  icona. Se il gioco non è mai stato visto dal PC amico, atteso anche
  `[DIAG] app <id> missing from local AppInfo cache; requesting PICS product info` +
  `SendClientFrame eMsg=8903 ... -> ok`.
- **T4 amico senza suffisso** (mittente con build vecchia): deve ancora valere il
  metodo per titolo (`Patched friend ... (by name)`) quando la libreria Lua dell'amico
  contiene il gioco.
- **T5 stabilità sessione**: con `-onlinefix`, Spacewar **non deve cadere**; se cade,
  nel log cercare `[DIAG] SharedLibrary msg` (è il CM a ordinare lo stop) vs
  `[DIAG] self entry arrived as 480` (finestra di desync, non deve toccare self).

## 6. Limiti accettati (da testare sul campo)

1. **Icona per app mai viste dal PC amico**: dipende dal riempimento PICS — è la
   parte da validare con T3. Se 8903 viene risposto ma il client non processa la
   risposta (job id azzerato), l'icona resta assente mentre nome/appid funzionano:
   segno che serve una seconda iterazione lato ricevitore.
2. **Vanilla friends** vedono `Nome | appid` per intero (informazione in più, non
   rotta).
3. **Zero push persona se non "osservati"**: le delta 766 arrivano con lista amici/
   chat aperta e account effettivamente amici (senza frame, niente patch — non è bug).
4. **Titoli con `" | "` finale + cifre**: combacerebbero col parser solo in casi
   patologici (nome che termina esattamente così); il rischio è trascurabile e
   coperto dal limite 24 bit / cifre.
5. **Playtime server-side** resta attribuito a 480 (come da sempre con `-onlinefix`).

## 7. Crash del client amico — scenario "gioco mai avuto" (in diagnosi, 2026-08-24)

**Sintomo**: con il mittente in `-showonline` su un gioco che il client amico possiede
(installato o .lua) tutto funziona; se il mittente gioca un titolo **che l'amico non ha
mai avuto**, lo Steam dell'amico **crasha** ~30–40 s dopo ogni consegna della presenza.

**Fatti misurati** (log 12:12–12:15, due macchine):

1. Il CM diffonde l'appid **reale** (es. 1703340) per giochi `-showonline` posseduti
   dal mittente: nessuna entry 480 arriva all'amico → la catena di recovery e il PICS
   (8903) **non vengono mai eseguiti** sul client che crasha. Il crash non è nel
   codice appena aggiunto.
2. Cronologia portatile: crash n.1 ~12:13:40 (PC avvia Stanley 1703340); riavvio
   12:14:10 (build stamp `showonline-suffix` presente → build corretta); login
   completato 12:14:34; ultima traccia DLL 12:14:36.731; crash n.2 entro ~30 s.
   Il mittente **non** crasha mai (log continuo oltre 12:15:38).
3. Logger: flush immediato per riga → il log è completo, nulla è perso in coda.
4. Ipotesi aperte: (a) percorso vanilla di Steam (richiesta appinfo/icona per app
   sconosciuta al client) detonato dallo scenario; (b) un frame sul trasporto (Multi
   eMsg=1, jobs 146/147) che non era ancora tracciato.

### 7.1 Flight recorder (build `showonline-suffix+flightrec`)

Logging diagnostico aggiunto, **nessuna logica toccata**:

- `hooks/wire/PacketRouter.cpp`: trace per-frame bidirezionale
  (`send eMsg=… hLen=… bLen=… / recv eMsg=…`, una riga a frame), firma hex dei primi
  8 byte del body dei frame Multi (`eMsg=1 (Multi) bLen=… head8=…`, prima erano
  silenti), job tracker jobid→nome servizio (`[DIAG] service recv name='…' jobid=…
  eresult=… bLen=…`), build stamp aggiornato a `showonline-suffix+flightrec`.
- `hooks/wire/PersonaInject.cpp`: per ogni frame persona, una riga `INFO` per ogni
  friend che gioca (`[DIAG] PERSONA friend <steamid> (SELF|FRIEND): app=… gid=…
  name='…' kvs=…`), deduplicata solo al cambio di stato → dice *quando* la voce
  amico (es. 1703340 reale) raggiunge la macchina. Più `TRACE [DIAG] PERSONA frame:
  friends=N bLen=…`.

Default `[log] level = "trace"` → attivo senza modifiche config.

### 7.2 Procedura di test (crash)

1. Compilare e mettere questa build su **entrambe** le macchine.
2. PC: Steam attivo; lanciare con `-showonline` un gioco posseduto dal PC e **assente**
   dal portatile (es. Stanley 1703340).
3. Portatile: tenere Steam aperto, amici che lo osservano, fino al crash.
4. Per OGNI macchina allegare **entrambi** `<Steam>\aethercore\main.log` **e**
   `<Steam>\aethercore\main.log.last` (rotate al riavvio: la sessione crashata finisce
   in `.last`!) — più il `DESK` session log se disponibile.
5. Dal **portatile**, catturare anche: Event Viewer → Windows Logs → Application →
   voci "Application Error"/"Windows Error Reporting" nell'istante del crash; il
   contenuto di `Steam\dumps\` (minidump con timestamp del crash); in Main.log di
   Steam (`Steam\logs\`), le righe dell'ultimo minuto.

### 7.3 Lettura attesa

- Se prima del crash compare `recv eMsg=…` nuovo e inatteso o un `service recv` con
  eresult ≠ 1 → colpevole lato wire, si guarda quel frame.
- Se compare `PERSONA friend … 1703340` → la voce è consegnata via 766 (non Multi);
  il crash segue entro pochi secondi → probabile consumatore vanilla (appinfo/icona).
- Se il crash abbatte il trace senza frame sospetti → fallimento fuori dal wire
  (thread UI): i dump/Event Viewer indicano il modulo reale; si rimuove il flight
  recorder dopo la diagnosi.

### 7.4 Colpevole del crash — TROVATO E FIXATO (2026-08-24, build `+flightrec`)

Log nuovi: catena recovery **funzionante** su Gamblers Table (`Patched friend
76561199876393402: 480 -> 3618390 (extra_info)`), crash immediato su Stanley:

- 12:57:21.863 persona delta friend: `app=480 name='The Stanley Parable: Ultra
  Deluxe | 1703340'` (PC aveva trasmesso 480+suffisso alle 12:57:20.089).
- **Nessuna** riga successiva: morto dentro `gamename::ForApp(1703340)`, ovvero
  dentro la chiamata raw `CAppInfoCache::GetAppDataFromAppInfo("common/name")`
  su un appid **senza record locale**: su questa build di Steam quella sonda fa
  fault e ammazza il processo senza eccezione intercettabile dal flush.
- Le entry di app note alla cache (es. 3618390 installato) sondano bene → per
  questo la catena funzionava con i giochi presenti su entrambe le macchine.

Fix applicato:

1. **`PersonaInject`**: il nome display prende il testo del suffisso (coniato dal
   mittente dalla SUA AppInfo cache — fresco e localizzato). `ForApp` resta solo
   per la sorgente "local session" (app installata in loco → sonda provata safe).
   Mai più probe live per app esterne al percorso recovery.
2. **`GameNameResolver::ForApp` (SEH-guard)**: la sonda raw è avvolta in
   `__try/__except` (MSVC): un fault diventa "nome sconosciuto" con log
   `appinfo probe ... raised SEH 0x...` invece del crash — serve anche da verità
   definitiva se l'ipotesi fosse sbagliata.
3. **PICS 8903 su worker thread detached** (+50 ms, parentesi TRACE
   `PICS appinfo send begin/done`): niente ri-entro di `BBuildAndAsyncSendFrame`
   dentro lo stack di RecvPkt.

Atteso al retest T3 (Stanley): `Patched friend ... (extra_info)`, poi
`[DIAG] app 1703340 missing from local AppInfo cache; requesting PICS`, poi
`[DIAG] SendClientFrame eMsg=8903 ... -> ok`; icona che si riempie a cache fatta.

**CONFERMA FIX (log 13:19–13:27, build `+fix2`)**: portatile patchea e richiede PICS
per tutte le app sconosciute senza crash — `Patched friend ... -> 3618390/1703340/1398210
(extra_info)` + `SendClientFrame eMsg=8903 ... -> ok`; Steam vivo fino all'export.

## 8. Crash del GIOCO con `-showonline` su parser argv rigidi (2026-08-24, `+fix3`)

**Sintomo**: "Selene ~Apoptosis~" (1398210) esce ~3–4 s dopo ogni avvio con
`-showonline` (3 lanci tra 13:21:31 e 13:21:57, vite 4.6 s / 4.2 s), mentre il lancio
senza flag vive 28 s. Gamblers Table e Stanley tollerano il flag — Selene no.

**Causa**: con `-showonline` il processo NON è mascherato, NON riceve payload né env
patch — l'**unica** differenza visibile al figlio rispetto al lancio plain è il token
`-showonline` lasciato nella riga di comando (SpawnProcess passa `cmdLine` invariata).
I giochi con parser argv rigido (o che trattano argomenti ignoti come path) crashano.

**Fix** (`hooks/steamclient/OnlineFixHooks.cpp`): `h_SpawnProcess` ora toglie i token
Aether (`-onlinefix`, `-showonline`) dalla cmdline prima di chiamare
`o_SpawnProcess` → il figlio riceve argv pulito (log: `Stripped Aether launch flags
from child cmdline (app <id>, was '...').`). Bonus: stessa protezione per `-onlinefix`.
Tokenizzazione di `HasFlagArg` resa coerente (spazio+tab). Test harness 14/14.

## 9. Canale invisibile per il suffisso (build `+fix4`, 2026-08-24)

**Motivazione**: con il suffisso ASCII `"Nome | appid"`, gli amici SENZA Aether vedono
l'intera stringa. Richiesta: mostrare solo il nome ai vanilla, senza perdere la ricezione
deterministica dell'appid.

**Idea proposta (braille): scartata** — i caratteri braille (U+2800–U+28FF) sono VISIBILI
(puntini) nei font Windows. Soluzione adottata: **caratteri format Default-Ignorable**,
invisibili per definizione Unicode in ogni renderer conforme (la friends UI CEF inclusa):
tag = `U+200B` ZERO WIDTH SPACE (E2 80 8B) + esattamente 6 **Variation Selectors**
U+FE00..U+FE0F (EE B8 8n), un nibble ciascuno, 24 bit big-endian. U+200B stacca la catena
VS dall'ultimo carattere visibile (niente restyle di cifre/®/© col FE0F). Overhead 21 byte.

| Lato | File | Comportamento |
|---|---|---|
| Mittente | `GamesPlayedModule::WithAppIdSuffix` | `presenceSuffixInvisible` (default **true**, toml `suffix_invisible`) → forma invisibile; `false` → ASCII legacy |
| Ricevitore | `PersonaInject::AppIdFromSuffix` | prova ASCII **poi** canale invisibile → entrambe le build vecchie e nuove si intendono |
| Config | `Settings.h/cpp`, `aethercore.example.toml` | nuovo flag `suffix_invisible` |

Round-trip testati in harness (`/tmp` 22 test, ALL PASS): nomi unicode/CJK, trailing ® e
cifre, appid con zeri iniziali, 480/0 rifiutati, tail malformato rifiutato, back-compat ASCII.

**Limited residui**: (1) ricevitori Aether < `+fix4` non leggono il canale invisibile
(fallback: match per titolo, come prima); (2) se il CM mai normalizzasse/ripulisse i DICP
dal game_name, il suffisso invisibile svanirebbe → fallback by-name: da verificare sul campo (T3).

---

## 10. Canale `game_data_blob` (fix5) — appid invisibile ai vanilla, senza nessun testo

### Misura sul campo che chiude §9
Il canale a caratteri-invisibili FALLISCE live: la UI amici di Steam rasterizza
U+FE00-FE0F come rettangoli tofu (il font usato per la riga del gioco non ha
quei glyph — misura 2026-08-24, `all.log` + conferma utente). Qualsiasi
encoding **testuale** dipende dal font del renderer: soluzione definitiva =
canale **non-testuale**.

### Il campo
Il set protobuf Steam completo (verificato su LumaCore/source/proto) definisce
| messaggio | campo | n. | tipo |
|---|---|---|---|
| `CMsgClientGamesPlayed.GamePlayed` | `game_data_blob` | 8 | bytes |
| `CMsgClientPersonaState.Friend` | `game_data_blob` | 60 | bytes |

Byte grezzi, MAI renderizzati da nessuna UI client (Steam stabile/beta/mobile).
Come `game_extra_info`→`game_name`, il CM recicla `game_data_blob` nelle frame
Persona degli amici → canale speculare ma invisibile al 100%.

### Formato (9 byte)
`"AETR"` (4) + versione `0x01` (1) + appid LE (4). Decoder rigoroso: lunghezza,
magic, versione, 0 < appid ≤ 0xFFFFFF, appid ≠ 480. Blob estranei (altri
giochi/emulatori che lo usano davvero) vengono scartati senza effetti.

### Flusso fix5
- **TX** (GamePlayed): `AnnotateMaskedEntry()` — con `appid_blob=true`
  `game_extra_info` = **solo nome piano** + `game_data_blob` = blob; entrambi
  i path (-showonline rewrite e -onlinefix annotate).
- **RX** (PersonaState): dopo i parse suffix, prima dei fallback by-title:
  `AppIdFromBlob(f.game_data_blob)` → `source="blob"`, displayName = testo
  extra pulito. I vaniglia vedono **unicamente il nome**: la richiesta utente
  ("niente `" | appid"`, niente tofu") è rispettata per definizione (il CM
  sanitizza `extra_info` con ESATTAMENTE il nome dell'app → nessuna leak
  testuale possibile).
- **Policy default**: `appid_blob=true`, `suffix_invisible=false` (ritirato:
  tofu misurato), suffix ASCII disponibile solo con `appid_blob=false` per
  receiver legacy (<fix3 non ha nemmeno quello).

### Test sul campo richiesto (punto di prova unico)
Il comportamento del CM su `game_data_blob` è NON documentato: serve la prova
che venga riciclato come `game_extra_info`. Build fix5, ricerca nel log RX del
portatile:
- `[DIAG] BUILD showonline-suffix+fix5 ... appid_blob=1` (stamp avvio)
- riga `Patched friend <name>: gameid -> <appid> (blob)` o diagnostica
  "blob" nella fonte (aggiunta `source="blob"`) → CM **inoltre** il blob: caso
  OK. Il portatile vede l'appid reale.
- **Se invece** il CM scarta il blob (presenza ricaduta su by-title/by-name):
  si passa al piano B (`Friend.gameid` field 56 fixed64 — bit 32-63 liberi
  quando type=mod: impacchettare appid lì; i vanilla vedono Spacewar, Aether
  decodifica gli high bit; collisione accettabile solo con mod Spacewar
  autentiche).

### Edge cases considerati
- **AM-sanitizzazione extra_info**: irrilevante per l'appid (non viaggia più
  nel testo); il nome pulito arriva comunque (CM lo ricicla come game_name).
- **Merge su persona frame**: `FILL` spec per infuse frames (proprie+amici)
  rispetta unknown→f.relay via proto field 60 dedicato: il codice aggregatore
  non tocca `game_nome`, il decoder legge il campo diretto.
- **Versione futura**: byte 4 riservato; bump a `AETR\x02`→ decoder legacy
  rifiuta e cade su by-title (graceful).
- **Log DIAG RX**: `Patched friend ... (blob)` codifica la prova sul campo;
  `showonline` TX logga `channel=blob`.

### Misura sul campo n.2 (build fix5, 2026-08-24 16:38) — il CM non inoltra il blob
`pc_all.log_16-42-51.txt` (TX, fix5, blob attivo su ogni gioco) e
`portatile_all.log_16-42-27.txt` (RX, fix5): il ricevitore vede la mask
(`PERSONA friend ...: app=480 gid=480 name='DAVE THE DIVER'`) ma il recovery
fallisce su tutta la catena (blob incluso) → **il CM non ricicla
`game_data_blob` nelle frame Persona**. Il canale extra_info (testo) resta
l'unico effettivamente riciclato; da qui la vanilla-friendly update: i vanilla
vedono il nome pulito ✓, ma gli amici Aether non recuperano più l'appid.

### Piano B attivato (fix6): appid nei bit 32-63 di game_id
- **TX**: sul path -showonline `AnnotateMaskedEntry(packGameIdHighBits=true)`
  scrive `game_id = (480 | typeBits salvati) | (appid << 32)`. Il path
  -onlinefix NON tocca i bit alti del gid (rischio per la discovery OF).
  Invariato per i vanilla: la UI chiave sui bit 0-23 (480) + testo
  extra_info; i mod bit non sono mai renderizzati.
- **RX**: dopo il blob, prima di by-title: `gidHi = gameid >> 32` validato
  (0 < gidHi ≤ 0xFFFFFF, ≠ 480) → `source="gameid"`.
- **DIAG definitiva RX** (una riga per amico maskato):
  `[DIAG] mask friend <id>: gid=<gid> gidHi=<hi> bloblen=<n> blobhead=<hex> extra='...'`
  — decide senza ambiguità quali canali il CM inoltra realmente.
- Rischio residuo: il CM potrebbe azzerare i mod bit nel relay
  (`PERSONA friend ... gid=480` del test fix5 NON prova nulla: i bit alti nel
  TX fix5 erano 0 per costruzione). Se gidHi arriva, il canale è definitivo;
  in caso contrario la catena ricade su by-title (comportamento noto pre-fix1).
- Harness: `/tmp/gid_tests.cpp` (8/8: rewrite 480 preserva low, hi decode,
  rifiuto hi=0/spacewar/>24-bit).

---

## 11. Crash class: i giochi che muoiono sugli argomenti di avvio (marker `showonline_apps`, fix7)

### Perché Selene e Z.A.T.O. crashavano
`-showonline` era scritto nelle **Launch Options di Steam** (permanente), quindi
presente in argv ad ogni avvio.
- **Selene ~Apoptosis~**: parser argv rigido — usciva 3-4 s dopo TUTTI i
  lanci con il flag (misura 2026-08-24, tre run). Fix B: strip del token in
  `SpawnProcess` (`StripAetherFlagArgs`).
- **Z.A.T.O.** (app 4122860): lo strip AVVIENE correttamente (log
  `pc_all.log_17-05-07.txt` r.1219: `Stripped Aether launch flags ... ZATO.exe
  -showonline`), il figlio riceve `'"ZATO.exe"'` pulito, eppure muore ~2,5 s
  dopo il lancio. Conclusione: la superficie argv non era la (sola) via —
  il gioco legge i launch options anche attraverso un'altra superficie
  (p.es. `ISteamApps::GetLaunchCommandLine`, che Steam alimenta dalla launch
  option REGISTRATA, non dalla argv del figlio — il nostro strip non la tocca).

### Prevenzione strutturale (fix7): niente marker sulla riga di comando. MAI.
L'unica difesa affidabile contro la CLASSE intera è non toccare nessuna
superficie visibile al gioco:
- **AetherDesk**: il toggle showonline scrive l'appid in
  `[presence] showonline_apps` in **entrambe** le copie di aethercore.toml
  (config dir locale + `<Steam>/aethercore/aethercore.toml`). Niente più
  `-showonline` nelle Launch Options; i token legacy vengono rimossi al
  primo toggle (migrazione in `set_aether_showonline`), e il reader
  (`get_aether_showonline`) riconosce marker O token legacy per compat.
  Attivare il marker rimuove anche `-onlinefix` (mutua esclusione, invariato).
- **AetherDLL**: `h_SpawnProcess` chiama `Settings::ReloadIfModified` (mtime;
  nessun riavvio di Steam) e attiva la sessione se `realApp` è nella lista.
  Env block, blob, argv, workdir restano byte-identici a un lancio normale:
  il crash "da argomenti" diventa impossibile per costruzione.
- Log spawn indicante la sorgente: `(source: showonline_apps marker (clean
  argv))` vs `(source: launch arg (legacy; strip applied below))`.

### Verifica sul campo prevista
Abilitare showonline su Z.A.T.O. dalla UI AetherDesk (marker), lanciare: attesa
vita > 2,5 s (no crash). Se crasha ANCHE senza alcun argomento nelle launch
options Steam, il problema è interno al gioco/ambiente e fuori dalla portata
di Aether (test: mettere `-test123` qualsiasi nelle launch options SENZA
Aether attivo — se crasha, il gioco è semplicemente fragile ai launch
arguments, cosa nota per lui, e il marker resta la sola via sana).

---

## 12. Config centralizzata nel TOML (fix8): policy + overrides, zero argv

### Il modello professionale adottato
- **TOML come unica fonte di verità** (`[presence]`): i tre array
  `showonline_apps`, `onlinefix_apps`, `exclude_apps` + la policy
  `default_mode = "none" | "showonline"`.
- **Policy + overrides, NON enumerazione**: "ogni gioco senza configurazione
  va in presenza" = UNA riga `default_mode = "showonline"` invece di
  materializzare l'intera libreria Steam nell'array (anti-pattern: dati
  derivati scritti come intenti, file inutilmente enorme, writer multipli,
  install/disinstall lasciano spazzatura).
- **Precedenza fissa e documentata**: `exclude > onlinefix > showonline >
  default_mode`. Legata in `ResolveLaunchMode()` (h_SpawnProcess); harness
  C++ 12/12 (include: exclude batte perfino un token legacy dimenticato).
- **`exclude_apps` = hard opt-out**: la DLL ignora l'app interamente; i token
  residui in argv vengono comunque strappati (strip).
- **Mutua esclusione lato scrittura**: ogni comando Desk rimuove l'app da
  tutti gli array e la aggiunge al massimo a uno → configurazioni sporche
  "app in due liste" non si creano; se scritta a mano, l'ordine di precedenza
  decide (deterministico).

### AetherDesk (fix8)
- I quattro comandi esistenti mantengono nomi e firme → la UI non cambia.
  `set_aether_showonline` / `set_aether_onlinefix` ora scrivono negli array
  del toml (entrambe le copie) e **rimuovono sempre i token legacy** dalle
  Launch Options (migrazione progressiva).
- Nuovi comandi registrati: `get/set_aether_excluded` (opt-out) e
  `get/set_presence_default_mode` (policy globale showonline default).
- Editor TOML generalizzato: upsert canonico delle tre chiavi sotto
  `[presence]`, creazione sezione se assente, preservazione di tutto il resto;
  test mirror Python 11/11 (idempotenza, canonical order, dedupe, append).
- La DLL fa ReloadIfModified in SpawnProcess → **zero riavvi di Steam**.

### Risposta alla domanda sulle crack (online-fix.me / UCO2)
Osservazione utente: `-showonline` ha funzionato con crack che PENSANO di
essere Spacewar. Motivo architetturale: showonline non tocca il processo —
riscrive solo le frame di presenza in uscita — quindi convive con qualunque
masking locale. Ma:
- **-showonline + -onlinefix sullo STESSO gioco non va combinato** e non
  serve: la modalità OnlineFix (mask 480 reale) è un superset funzionale —
  la presenza verso gli amici è inclusa nel suo percorso di annotazione.
  La mutua esclusione (un solo array per app) è la forma corretta.
- Con una crack che si maska DA SOLA (senza `-onlinefix` di Aether): il
  cablaggio di annotazione showonline non si attiva (require
  `luadata::IsConfigured`/depot): gli amici vedono semplicemente "Spacewar".
  Se vuoi nome+appid agli amici su quel gioco, usa `onlinefix_apps`.
- "La pipeline del multiplayer non dovrebbe essere occupata": corretto per
  UCO2 — showonline non riserva nulla lato processo; l'unica zona condivisa è
  il canale wire di presenza, gestito dal CM.

### Procedura utente dopo l'update (migrazione)
1. Ricompilare DLL + Desk.
2. In AetherDesk: ri-togleggere (off→on) ogni gioco precedentemente marcato →
   i token spariscono dalle Launch Options e compaiono gli array nel toml.
3. Per il plug-and-play globale: `set_presence_default_mode(true)` (futuro
   toggle UI) o a mano `default_mode = "showonline"` nel toml.
4. Verifica log: stamp `showonline-suffix+fix8`; alla spawn riga con
   `(source: showonline_apps|onlinefix_apps|default_mode=showonline|...)`.
