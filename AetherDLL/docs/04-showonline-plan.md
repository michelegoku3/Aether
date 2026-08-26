# Piano `-showonline` — mostrare il gioco reale agli amici (senza online)

> Repo `michelegoku3/Aether` @ `5dc0f2f`. Aggiornato dopo la Fase 0 (log `all.log_14-15-54.txt`)
> e la foto della lista amici di SyntaxCode15.
> **Stato: vincolo misurato, meccanismo del ricordo identificato, esperimento Fase 1 pronto in tree.**

---

## 1. Misura Fase 0 — il CM valida la licenza (per il tipo `App`)

| # | Gioco | Licenza | Inviato (`TX`) | **Risposta del server** |
|---|---|---|---|---|
| 1 | Rivals of Aether `383980` | posseduto | `game_id=383980` | ✅ `app=383980` dopo **0,32 s** |
| 2 | Stanley `1703340` | **non posseduto**, nessun flag | `game_id=1703340` | ❌ **nessun push per 20 s** |
| 3 | Stanley `1703340` | non posseduto, `-aetheronline` | `game_id=480` | ✅ `app=480 name='The Stanley Parable: Ultra Deluxe'` dopo **0,46 s** |

Il PersonaState che il CM rimanda per il nostro SteamID è ciò che riceve la lista amici. Nei casi 1 e 3
risponde in meno di mezzo secondo, nel caso 2 tace. Non è latenza: **è un rifiuto**.

**Conclusione:** con `CGameID` di **tipo `App`** e un appid senza licenza sull'account, il server non
diffonde nulla. Lo spoof di ownership di Aether è client-side e il CM non lo vede.

**Effetto collaterale utile (run 3):** il campo `game_extra_info` che spediamo torna indietro come
`Friend.game_name` nel PersonaState diffuso. Il titolo reale arriva già agli amici anche sotto mask 480.

---

## 2. La foto: cosa dimostra davvero

Lista amici di SyntaxCode15, riga `michelegoku3`:

```
[icona rossa CK3]  [avatar "?"]  michelegoku3
                                 Crusader Kings III
```

Lettura corretta (confermata dall'utente): **il quadratino a sinistra è l'icona del gioco**, ed è la
banda rossa di Crusader Kings III. Il "?" centrale è l'avatar di default (michelegoku3 non ha immagine
profilo). Confronto con la riga `Dreadstew`: icona KH + avatar in bianco e nero.

Questo è dirimente: **il client dell'amico ha risolto l'appid `1158310`**. Con `app=480` sarebbe
impossibile — Spacewar su SteamDB ha `icon · empty string`, non c'è nulla da disegnare.

Configurazione dell'epoca (confermata): **nessuna licenza reale** su CK3, solo Lua, e l'**aetheronline
esterno "buggato" che mandava l'appid del gioco**.

### Il conflitto apparente, e la sua soluzione
Run 2 dice «appid reale non posseduto → il server tace». La foto dice «l'appid reale è arrivato
all'amico». Entrambe le cose sono vere **se il tipo del CGameID era diverso da `App`**.

```
CGameID = appid:24 | type:8 | modid:32          (Valve, steamclientpublic.h)
  type 0 = App        -> il CM verifica la licenza      [MISURATO: scartato]
  type 1 = GameMod    -> non e' un claim di ownership
  type 2 = Shortcut   -> idem
  type 3 = P2P
```

Con `type != 0` il messaggio **non è** l'affermazione "sto giocando all'app X che possiedo", quindi il
controllo di licenza non si applica — ma **l'appid reale resta nei 24 bit bassi**, e il client
dell'amico lo usa per risolvere icona e nome. Risultato: icona CK3 + titolo reale, esattamente la foto.

E spiega anche il resto del tuo ricordo: il **"mezzo e mezzo" in cui DLC e workshop non funzionavano**
è precisamente questo, perché quell'aetheronline cambiava l'identità **a livello di processo**, rompendo
DLC e workshop lato client.

---

## 3. Fase 1 — ESEGUITA, ipotesi FALSIFICATA

Log `all.log_18-09-37.txt` (23/08, 18:04–18:09). Build verificata dal timbro
`[Wire] [DIAG] BUILD showonline-fase1` a riga 150; hot-reload del TOML confermato
(la riga `[Settings] [DIAG] presence:` cambia a ogni giro). Sei run valide.

### Senza `-aetheronline` — appid reale non posseduto (Stanley 1703340)

| type | modid | game_id inviato | risposta del CM |
|------|-------|-----------------|-----------------|
| 1 GameMod  | 0x80000001 | 9223372041168223660 | **nessun push, 28 s** |
| 2 Shortcut | 0x80000002 | 9223372045479968172 | **nessun push, 19 s** |
| 3 P2P      | 0x80000003 | 9223372049791712684 | **nessun push, 20 s** |

La riscrittura funziona: `[DIAG] ShowOnline: game_id 1703340 -> ...` compare a ogni
run, il frame parte (`bodyLen` cresce da 86 a 94/97). Il CM semplicemente **non
risponde**.

Prova ulteriore, decisiva: i quattro push `app=0` (fine sessione) cadono solo a
18:04:28, 18:08:31, 18:09:06, 18:09:32 — cioè **solo attorno alle run con 480**.
Dopo le uscite dalle run 1/2/3 non c'è nessun push di "clear": il CM non aveva
niente da azzerare perché **non aveva mai accettato il messaggio**.

### Con `-aetheronline` — game_id 480 (i tre tipi, per controllo)

Il blocco ShowOnline salta di proposito le entry 480, quindi il comportamento è
identico in tutti e tre i giri: push del CM dopo ~0,2–0,3 s con
`app=480 gameid=480 name='The Stanley Parable: Ultra Deluxe'`. Nome sì, icona no.

### Conclusione

**I bit `type` del CGameID non aggirano nulla.** Il CM valida l'appid nei 24 bit
bassi e scarta l'intero messaggio se non c'è licenza, qualunque sia il tipo.
La strada "CGameID tipizzato" è chiusa: non riproporla.

Insieme alla Fase 0 questo chiude *tutte* le vie server-side: non esiste alcun
`ClientGamesPlayed` che convinca il CM a trasmettere agli amici un appid non
posseduto.

---

## 4. Fase 2 — la pista giusta: la riscrittura avviene sul PC di CHI GUARDA

Se il server non può trasmettere l'appid reale, allora un mese fa **non l'ha mai
trasmesso**. L'icona di CK3 nella foto è stata ricostruita **localmente dal client
di SyntaxCode15**. Il codice per farlo esiste già in repo — è
`PersonaInject.cpp:229`:

```cpp
// AetherOnline: patch any friend entry still showing Spacewar (local view).
if (ofPersonaPatch && ofReal != 0 && luadata::IsConfigured(ofReal)) {
    ...
    if (f->game_played_app_id() != kSpacewarAppId) continue;
    f->set_game_played_app_id(ofReal);
    f->set_gameid(ofReal);
    f->set_game_name(name);
```

Riscrive **qualsiasi** voce amico che mostri 480, sostituendovi `ofReal`. È
esattamente la riga che nei nostri log produce
`Patched friend 76561199876393402: 480 -> 1703340`.

### Perché oggi non basta

Due vincoli la rendono inerte nel caso normale:

1. `ofReal != 0` — si attiva **solo mentre anche tu stai giocando** qualcosa con
   `-aetheronline`. Se guardi la lista amici senza giocare, `ofReal` è 0 e non fa nulla.
2. Usa **il tuo** appid per la voce **dell'amico**. Funziona solo nel caso
   simmetrico: due PC che giocano lo stesso gioco. Che è, con ogni probabilità,
   proprio la configurazione di un mese fa.

### Il design corretto: risoluzione per nome

La Fase 0 ha misurato che il CM ricicla `game_extra_info` dentro
`Friend.game_name`. Quindi nel messaggio in arrivo il **nome reale c'è già**,
anche quando l'appid è 480:

```
app=480 gameid=480 name='The Stanley Parable: Ultra Deluxe'
```

Il client che guarda ha tutto il necessario. Serve solo invertire la mappa:

```
per ogni friend f con f.game_played_app_id == 480 e f.game_name non vuoto:
    real = ResolveAppIdByName(f.game_name)
    se real != 0:
        f.game_played_app_id = real
        f.gameid             = real
```

Niente `ofReal`, niente vincolo di simmetria, funziona anche a gioco spento.
`ResolveAppIdByName` si costruisce dalla `appinfo.vdf` locale (contiene nome +
`clienticon` di ogni app che il client ha visto), con cache in memoria.

### Conferma dall'utente

Un mese fa: due PC, due account diversi, **stesso programma e stesso gioco su
entrambi**. E' il caso simmetrico esatto in cui il blocco `ofReal` esistente si
attiva. Il meccanismo e' quindi identificato: nessuna magia server-side, solo
riscrittura locale sul PC di chi guardava.

### Il limite da accettare

L'icona la vedono **solo gli amici che eseguono Aether**. Chiunque altro continua
a vedere Spacewar. Non è una regressione: è il massimo ottenibile, e coincide con
ciò che è realmente successo un mese fa.

---

## 5. Fase 2 — implementata

| file | modifica |
|------|----------|
| `utils/GameNameResolver.{h,cpp}` | `ResolveAppIdByName(name)`: lookup inverso sui soli `luadata::LibraryAppIds()`, confronto case/space-insensitive, cache con negativi |
| `hooks/wire/PersonaInject.cpp` | nuovo blocco *Friend icon recovery by title*, prima del fallback `ofReal` |
| `core/Settings.{h,cpp}` | chiave `[presence] friend_appid_from_name` (default `true`, hot-reload) |
| `config/aethercore.example.toml`, `AetherDesk/.../aethercore.toml` | chiave documentata |

Il lookup inverso non enumera `appinfo.vdf` (100+ MB, formato binario
versionato): usa la libreria configurata in Lua, che nel nostro scenario
contiene per costruzione il gioco da riconoscere. Costo: una scansione sola per
titolo, poi cache.

Il blocco ShowOnline in uscita resta in tree come strumento di misura ma e'
**disattivo per default** (`show_online_gameid_type = 0`) e il commento in cima
riporta l'esito negativo, per non farlo riprovare in futuro.

### Come verificare

Sul PC che *guarda* (non serve giocare):

1. `[Wire] [DIAG] BUILD showonline-fase1 | ... friend_appid_from_name=1`
2. l'amico lancia il gioco con `-aetheronline`
3. attesa: `[GameName] Reverse lookup 'Crusader Kings III' -> app 1158310.`
   e `[Wire.PersonaInject] [DIAG] Friend ...: 480 -> 1158310 by name '...'`
4. nella lista amici deve comparire l'icona del gioco al posto di Spacewar

Se compare (3) ma non l'icona, il client non ha in cache l'appinfo dell'app;
se non compare (3), il titolo non e' nella libreria Lua locale.

---

## 6. Fase 2 — funziona; regressione aperta: la sessione Spacewar cade

Esito riportato dall'utente: **l'icona compare**. Ma poco dopo Spacewar si chiude
e si perde lo stato online di `-aetheronline`.

### Sospetto

Entrambi i blocchi in `PersonaInject` riscrivevano **anche la nostra stessa
voce** (nei log: `Patched friend 76561199876393402: 480 -> 1703340`, che e'
l'utente stesso). Il client Steam usa il proprio stato di sessione per sapere
quale app sta girando: se la persona che il server ci rimanda per noi stessi
dice "app 1158310" mentre il session manager locale ha viva la sessione 480,
la riconciliazione puo' concludere che la sessione 480 e' stanca e chiuderla.
Con `-aetheronline` la sessione 480 *e'* lo stato online.

Il tutto e' inutile ai fini della feature: la voce che conta e' quella che
l'**amico** riscrive sul **suo** PC. Riscrivere la propria e' rischio senza
beneficio; la vista locale di se stessi appartiene gia' a `presenceInjectLocal`.

### Correzione applicata

Entrambi i blocchi ora saltano l'entry con `friendid == selfId`. Il vecchio
blocco `ofReal` logga una riga `[DIAG] self entry arrived as 480 while playing
%u; left untouched`, che dice se la finestra di desync esiste davvero.

### Come confermare la causalita'

`friend_appid_from_name` ha hot-reload: si puo' fare A/B senza riavviare Steam.

- con `false` Spacewar **non** cade  -> la causa e' la nostra riscrittura
- con `false` Spacewar cade lo stesso -> comportamento preesistente di
  `-aetheronline`, indipendente dalla Fase 2

Nel log servono, attorno al momento della chiusura: `[DIAG] TX: games_played
vuoto`, `Playing app -> 0`, e le righe `Masked AppId` / `AetherOnline`.

---

## 7. Stato delle patch in tree

| File | Contenuto |
|---|---|
| `docs/showonline-fase0.patch` | diagnostica: `[DIAG] TX` + `[DIAG] SERVER self-push` |
| `docs/showonline-fase1.patch` | diagnostica **+** riscrittura CGameID + settings + TOML |

Aggiunte al `.proto` (`process_id=9`, `game_flags=11`, `owner_id=12`, verificate su SteamKit):
**tienile**, servono alla via Family Sharing.
Il log `SERVER self-push` è lo strumento di misura di questa feature: tienilo finché non è validata,
poi degradalo a `AC_LOG_DEBUG_ONCE`.

⚠️ La tua DLL installata è **0.9.12**, la repo è a **0.9.10**: applica le patch al tuo albero locale.

---

## 8. Nota storica

Storia pre-Aether persa (squash `56bccf1`, 19/07/2026). Restano tre riferimenti a codice assente: il
commit `9aa4a76`, il piano `docs/03-presence-identity-plan.md` mai importato, e la **regressione
"Meccha"** in `AetherOnlineHooks.cpp:209` — *"leaking real identity into multiplayer routing / friends
presence"*. La "doppia strada" che ricordavi esiste ancora ed è `h_BuildSpawnEnvBlock` (CGameID di
processo 480 per il routing, `SteamOverlayGameId` reale per DLC e overlay).

---

## 9. `-showonline` come modalità di default — inventario e design

### 9.1 Cosa traduce `-aetheronline` oggi

Inventario completo dei punti 480↔reale già in tree:

| punto | direzione | a cosa serve |
|---|---|---|
| `AetherOnlineHooks::h_SpawnProcess` | reale → **480** | CGameID di process-tracking: è quello che vedono CM e routing multiplayer |
| `h_BuildSpawnEnvBlock` | 480 → **reale** | CGameID dell'overlay: DLC, metadati depot, identità overlay |
| `CmdUtils::GetAppID` (IPC) | 480 → **reale** | il gioco che chiama `IClientUtils::GetAppID` riceve l'appid vero |
| `h_GetAppIDForCurrentPipe`, solo *stats scope* | 480 → **reale** | stats e achievement salvati sotto l'app giusta |
| `OwnershipHooks::h_SendCallbackToPipe` | doppia consegna | callback achievement all'overlay (reale) **e** agli handler del gioco (480) |
| `AchievementModule` (wire) | 480 → **reale** | `ClientGetUserStatsResponse` |
| `GamesPlayedModule` + `PersonaInject` | annota | `game_extra_info` / `game_name` col titolo reale |
| `SteamCapture::CurrentRouteAppId` | reale | routing delle chiamate IPC |

**Quindi: l'idea descritta è già implementata — è esattamente il design di
`-aetheronline`.** Maschera 480 sul filo e ritraduce in locale dove serve.

Buco reale: **nessun hook UGC/Workshop esiste** (`grep -rn "UGC\|Workshop"` → 0
risultati). Workshop oggi funziona solo di riflesso, perché `GetAppID` e il
CGameID dell'overlay riportano l'appid vero. Non è mai stato verificato.

### 9.2 Cosa aggiunge davvero `-showonline`

Non un meccanismo nuovo: **una policy**. Stessa pipeline di appid, meno roba
attaccata. In concreto salta `OnlinePayload` (iniezione nel processo del gioco)
e tutto ciò che serve solo al matchmaking — peso e superficie di rischio che per
un singleplayer non ha motivo di esistere.

Implementazione onesta: `kShowOnlineFlag` accanto a `kAetherOnlineFlag`, che entra
nello stesso percorso di mascheramento con un booleano `wantsOnlinePayload` a
false. Nessun ramo duplicato: due percorsi paralleli che fanno il 90% delle
stesse cose divergerebbero entro un mese.

### 9.3 Il default non può essere "tutti i giochi"

Va ristretto, per un motivo misurato nella Fase 0: **un gioco che possiedi
davvero oggi viene già trasmesso correttamente dal CM**, con icona e nome, a
*tutti* gli amici, anche a chi non ha Aether. Mascherarlo come 480 sarebbe una
regressione netta.

Default corretto:

```
maschera come 480  <=>  luadata::HasDepot(app) && !luadata::IsOwned(app)
```

cioè solo i giochi gestiti da Lua e non posseduti — quelli che oggi il CM scarta
in silenzio, dove non c'è niente da perdere. Più la lista di esclusione per i
titoli con online reale, che restano su `-aetheronline`.

### 9.4 Ricostruzione quando l'amico NON ha il gioco

Oggi `ResolveAppIdByName` cerca solo in `luadata::LibraryAppIds()`: se l'amico
non ha quel gioco configurato, nessun match. Tre livelli per superarlo, in
ordine di robustezza crescente.

**(a) Cache nome → appid da Steam.** `ISteamApps/GetAppList/v2` restituisce
l'elenco completo (~250k voci, ~10 MB), oppure
`steamcommunity.com/actions/SearchApps/<nome>` per la singola query. Si salva su
disco e si aggiorna ogni tanto. Funziona, ma eredita due debolezze del match per
nome: **collisioni** (titoli identici) e soprattutto **localizzazione** — il
titolo che spediamo viene da `gamename::ForApp` nella lingua del *mittente*,
mentre la cache del ricevente è in inglese.

**(b) L'icona per un appid mai visto.** Serve l'appinfo (`clienticon`) nella
cache del client di chi guarda. Ma — differenza cruciale rispetto alla presence
— **i metadati PICS non sono soggetti a licenza**: il CM risponde a
`ClientPICSProductInfoRequest` per qualunque appid. Il router già gestisce quel
messaggio (`AccessToken` module), quindi la richiesta si può emettere. Da
verificare che il client scarichi poi l'icona dalla CDN.

**(c) Passare l'appid esplicitamente — la strada pulita.** Invece di indovinare
il titolo, spedirlo: `kClientRichPresenceUpload` (7501) è **già hookato** in
`GamesPlayedModule::HandleRichPresenceUpload`, e nei log del 23/08 compare tra i
frame in uscita. Se una KV tipo `aether_appid=1158310` sopravvive fino al
`PersonaState` degli amici (nel DIAG il campo si legge già: `rp_kvs=N`), allora
il ricevente ha l'appid **esatto**: niente collisioni, niente problemi di
lingua, nessuna cache da mantenere. Il match per nome resta come fallback.

**Da misurare per prima cosa:** le KV di rich presence arrivano agli amici
quando l'app dichiarata è 480? Il DIAG `SERVER self-push` stampa già `rp_kvs`,
quindi la risposta costa un giro di test.

---

## 10. Fase 3 — strumentazione per i due test aperti

Timbro di build aggiornato: cercare `[DIAG] BUILD showonline-fase3`.

### Log aggiunti

| dove | riga | risponde a |
|---|---|---|
| `PacketRouter::DispatchRecv` | `[DIAG] SharedLibrary msg eMsg=… ` | è il CM a ordinare lo stop, o muore il client? |
| `PersonaInject` (blocco `ofReal`) | `[DIAG] self entry arrived as 480 while playing …` | esiste la finestra di desync sospettata? |
| `PersonaInject` (self-push) | `[DIAG] SERVER rp kv: 'k' = 'v'` | le nostre KV tornano indietro dal server? |
| `PersonaInject` (amici) | `[DIAG] FRIEND … rp kv: … (app=…)` | le KV di un altro account arrivano fin qui? |
| `GamesPlayedModule` | `[DIAG] RP upload appid=… 'k' = 'v'` | il gioco spedisce rich presence di suo? |
| `GamesPlayedModule` | `[DIAG] RP probe: invio aether_appid=…` | la sonda è partita |
| `PacketRouter::SendClientFrame` | `[DIAG] SendClientFrame eMsg=… -> ok/FAILED` | il frame sintetico è stato accettato |

### `SendClientFrame` — nuovo strumento

`PacketRouter` ora sa **originare** un frame client→CM: cattura l'oggetto
connessione e il `CMsgProtoBufHeader` da un frame in uscita reale, azzera i job
id e rispedisce con l'eMsg che vogliamo. Serve perché i singleplayer non fanno
upload di rich presence da soli: senza sonda, `rp_kvs=0` non distingue "il
server le scarta" da "nessuno le ha mai spedite".

### Test 1 — Spacewar cade ancora? (1 PC)

`friend_appid_from_name` ha hot-reload: due run, `true` e `false`, stesso gioco
con `-aetheronline`. Nel log, al momento della caduta, guardare in quest'ordine:

1. `[DIAG] SharedLibrary msg` → **presente**: è il CM che ordina lo stop, la
   nostra riscrittura non c'entra
2. `[DIAG] TX: games_played vuoto` senza SharedLibrary → è il client locale a
   chiudere la sessione
3. `[DIAG] self entry arrived as 480 while playing N` → la finestra di desync
   esiste; con la correzione ora la voce resta intatta

### Test 2 — l'appid viaggia in rich presence? (1 PC)

`rp_probe = true`, lanciare un gioco con `-aetheronline`, attendere ~30 s.

- `[DIAG] RP probe: invio` + `SendClientFrame … -> ok` → sonda partita
- `[DIAG] SERVER rp kv: 'aether_appid' = '1703340'` → **ha fatto il giro**:
  l'appid si può trasmettere, il match per nome diventa solo un fallback
- nessuna riga `SERVER rp kv` per ~30 s → il CM scarta le KV con app 480, e
  resta il match per nome + cache

`SendClientFrame … -> FAILED` o `no captured connection` significa che la sonda
non è mai partita: il test è nullo, non è una risposta.

### Cosa NON si misura con un solo PC

La vista dell'amico. L'icona ricostruita e la propagazione delle KV a un altro
account richiedono il secondo PC — ma l'icona è già stata verificata, e il
punto 2 dà la risposta sul canale usando l'eco del server su di sé.
