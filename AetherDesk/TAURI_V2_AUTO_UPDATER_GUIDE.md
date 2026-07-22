# Guida all'Auto-Updater di Tauri v2 per AetherDesk

Questa guida spiega in modo approfondito e con standard di livello enterprise come gestire il rilascio, la firma crittografica e l'aggiornamento automatico della GUI di **AetherDesk** utilizzando **GitHub Releases** come server di aggiornamento gratuito, senza bisogno di database o server API custom!

---

## 🏗️ Come funziona l'Auto-Updater di Tauri v2?

Il meccanismo di aggiornamento di Tauri v2 è stateless, sicuro ed estremamente leggero. Segue questo flusso di lavoro:

1. **Il Controllo**: All'avvio dell'applicazione (o tramite un pulsante "Check Updates"), il client Tauri effettua una richiesta HTTP GET a un endpoint JSON pubblico configurato (chiamato solitamente **`latest.json`**).
2. **Il Confronto**: Il client confronta la versione definita nel suo `tauri.conf.json` locale con la chiave `"version"` presente nel file `latest.json` scaricato.
3. **La Validazione (Sicurezza)**: Se nel JSON è presente una versione maggiore, Tauri verifica la **firma crittografica (`.sig`)** dell'installer prima di eseguirlo. Se la firma non corrisponde alla chiave pubblica incorporata nell'applicazione, l'aggiornamento viene abortito per proteggere l'utente da malware o manomissioni!
4. **L'Installazione Silenziosa**: Tauri scarica l'eseguibile di installazione di Windows (NSIS `.exe` o `.msi`), lo lancia in background in modalità silenziosa (`/S` per NSIS), sovrascrive i vecchi file ed esegue il riavvio immediato dell'applicazione con la nuova versione!

---

## 🔑 Step 1: Generare la Coppia di Chiavi Crittografiche (Firma)

Per ragioni di sicurezza, Tauri v2 **rifiuta tassativamente** di applicare aggiornamenti che non siano firmati crittograficamente con algoritmo Ed25519.

Esegui questo comando nel terminale del tuo PC Windows all'interno di `AetherDesk`:

```cmd
npx tauri signer generate
```

### Cosa ti restituirà questo comando?
Ti genererà due chiavi:
1. **La Chiave Pubblica (Public Key)**: Una stringa pubblica che devi copiare e incollare nel tuo file `tauri.conf.json` per permettere all'app di validare gli aggiornamenti.
2. **La Chiave Privata (Private Key)**: Una chiave segreta che devi tenere al sicuro sul tuo PC (o nei segreti di GitHub Actions) per firmare i binari durante la build.

---

## 🛠️ Step 2: Configurare `tauri.conf.json` per l'Updater

Nel tuo `tauri.conf.json`, aggiungi la configurazione del plugin dell'updater e dichiara la tua chiave pubblica. 

Ecco lo schema v2 ufficiale che devi inserire sotto la sezione `"plugins"`:

```json
{
  "bundle": {
    "active": true,
    "targets": ["nsis"],
    "createUpdaterArtifacts": true // <-- Forza la generazione dei file .sig e del latest.json durante la build!
  },
  "plugins": {
    "updater": {
      "endpoints": [
        // Sfrutta il download diretto della release più recente su GitHub!
        "https://github.com/michelegoku3/Aether/releases/latest/download/latest.json"
      ],
      "pubkey": "INCOLLA_QUI_LA_TUA_CHIAVE_PUBBLICA_GENERATA"
    }
  }
}
```

E nel tuo file dei permessi delle capacità (`src-tauri/capabilities/default.json` o simili), assicurati di abilitare i permessi per l'updater:
```json
"permissions": [
  "core:default",
  "updater:default"
]
```

---

## 📦 Step 3: Compilare e Firmare l'Aggiornamento

Quando sei pronto a rilasciare una nuova versione di AetherDesk (ad esempio porti `"version": "1.0.1"` nel tuo `tauri.conf.json`):

1. **Imposta la tua chiave privata nel terminale di Windows** (sostituisci con la tua chiave privata generata):
   ```cmd
   set TAURI_SIGNING_PRIVATE_KEY="LA_TUA_CHIAVE_PRIVATA_Ed25519"
   ```
   *(Opzionale: se hai impostato una password per la chiave, inserisci anche `set TAURI_SIGNING_PRIVATE_KEY_PASSWORD="password"`)*

2. **Avvia la build di produzione**:
   ```cmd
   npm run tauri build
   ```

### Cosa genererà questo comando nella cartella `target/release/bundle/nsis/`?
* **`AetherDesk_1.0.1_x64-setup.exe`**: Il tuo nuovo installer Windows.
* **`AetherDesk_1.0.1_x64-setup.exe.sig`**: Un piccolissimo file di testo contenente la firma crittografica dell'installer appena compilato!

---

## 🌐 Step 4: Come strutturare e compilare il file `latest.json`

Tauri si aspetta che tu carichi online un file chiamato **`latest.json`** che descrive l'aggiornamento. Ecco lo schema ufficiale per Tauri v2:

```json
{
  "version": "1.0.1",
  "notes": "Bug fixes, performance improvements and new Aether tab layout!",
  "pub_date": "2026-07-21T18:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "signature": "CONTENUTO_DI_AETHERDESK_1.0.1_X64-SETUP.EXE.SIG",
      "url": "https://github.com/michelegoku3/Aether/releases/download/v1.0.1/AetherDesk_1.0.1_x64-setup.exe"
    }
  }
}
```

### Cosa devi inserire in ciascun campo?
* **`signature`**: Apri il file `.sig` generato sul tuo PC con Blocco Note, copia la stringa di testo crittografata al suo interno e incollala qui.
* **`url`**: L'URL di download diretto dell'installer `.exe` che caricherai sulla tua release di GitHub.

---

## 🚀 Step 5: Pubblicare la Release su GitHub (Zero Manutenzione!)

Per completare il ciclo e rendere l'aggiornamento attivo per tutti i tuoi utenti:

1. Vai su GitHub e crea una nuova Release con il tag corrispondente (es. **`v1.0.1`**).
2. Carica tre file all'interno degli asset della Release:
   * **`AetherDesk_1.0.1_x64-setup.exe`** (L'installer reale)
   * **`AetherDesk_1.0.1_x64-setup.exe.sig`** (La firma)
   * **`latest.json`** (Il file manifest di aggiornamento descritto sopra)
3. Pubblica la Release.

### Cosa succede ora al vecchio programma già installato?
1. L'utente avvia l'AetherDesk v1.0.0 già installato sul proprio PC.
2. L'app interroga l'indirizzo `https://github.com/michelegoku3/Aether/releases/latest/download/latest.json`.
3. GitHub reindirizza automaticamente la richiesta all'ultima release attiva, servendo il file `latest.json` di v1.0.1.
4. L'app rileva che `1.0.1` > `1.0.0`, scarica l'installer, verifica la firma e **si aggiorna da sola all'istante!**
