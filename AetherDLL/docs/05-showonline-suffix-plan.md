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

### 7.4 Cache-causa del crash — TROVATA E FIXATA (2026-08-24, build `+flightrec`)

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
