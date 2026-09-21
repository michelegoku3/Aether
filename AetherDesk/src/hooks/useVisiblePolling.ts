import { useCallback, useEffect, useRef } from 'react';

export interface VisiblePollingOptions {
  /** Cadenza mentre il documento è visibile. */
  intervalMs: number;
  /** Cadenza ridotta mentre il documento è nascosto (finestra minimizzata,
   *  altro tab del SO attivo). Il polling non si ferma del tutto: tornando
   *  alla vista i dati non devono risultare troppo vecchi. */
  backoffMs: number;
  /** `false` ferma completamente il loop (es. pannello chiuso). */
  enabled?: boolean;
  /** Esegue un tick immediato all'attivazione invece di attendere il primo
   *  intervallo. Default `true`. */
  immediate?: boolean;
  /** Cambiare questa valore riavvia il loop (e, con `immediate`, esegue subito
   *  un tick). Serve ai consumatori la cui query dipende da un input UI — per
   *  esempio la sorgente log selezionata — e che quindi devono ricaricare i
   *  dati al cambio, non al prossimo tick. */
  resetKey?: unknown;
}

/**
 * Polling "visibility-aware" condiviso.
 *
 * Regole (le stesse che `LogView` applicava a mano e che l'audit chiedeva di
 * estendere a `SyncStatusModal`):
 *
 * 1. **setTimeout auto-rischedulante, mai setInterval**: il tick successivo
 *    parte DOPO il completamento della chiamata, quindi una IPC lenta non può
 *    accumulare richieste concorrenti.
 * 2. **Backoff quando il documento è nascosto**: `backoffMs` invece di
 *    `intervalMs`, rivalutato ad ogni tick (l'utente può minimizzare in
 *    qualsiasi momento).
 * 3. **Ripresa immediata al ritorno visibile**: `visibilitychange` → un tick
 *    subito, poi la catena riparte.
 * 4. **Una sola catena alla volta**: ogni catena ha un id; un tick in volo al
 *    momento del cleanup (o di un `resetKey`) non può rischedulare nulla.
 *
 * Il `tick` è letto da un ref: il consumatore può passare una funzione creata
 * ad ogni render (che chiude sopra il proprio stato) senza riavviare il loop.
 */
export const useVisiblePolling = (
  tick: () => void | Promise<void>,
  { intervalMs, backoffMs, enabled = true, immediate = true, resetKey }: VisiblePollingOptions,
): void => {
  const tickRef = useRef(tick);

  // Aggiornato dopo ogni render (non durante: scrivere un ref in fase di
  // render non è sicuro con il concurrent rendering).
  useEffect(() => {
    tickRef.current = tick;
  });

  const runOnce = useCallback(async () => {
    try {
      await tickRef.current();
    } catch {
      // Il tick è responsabile del proprio stato di errore (es. mostrarlo in
      // UI). Qui si ingoia l'eccezione solo per non interrompere la catena.
    }
  }, []);

  useEffect(() => {
    if (!enabled) return;

    let timer = 0;
    let stopped = false;
    let chain = 0;

    // Function declarations (hoisted): `schedule` e `startChain` si chiamano a
    // vicenda, e nessuna delle due viene eseguita prima che entrambe esistano.
    function schedule(owner: number) {
      if (stopped || owner !== chain) return;
      const hidden = document.visibilityState !== 'visible';
      timer = window.setTimeout(() => startChain(), hidden ? backoffMs : intervalMs);
    }

    function startChain() {
      if (stopped) return;
      const owner = ++chain;
      void runOnce().then(() => schedule(owner));
    }

    const onVisibilityChange = () => {
      if (document.visibilityState !== 'visible') return;
      // Annulla il timer (eventualmente in backoff) e riparti subito: chi
      // torna sulla finestra non deve aspettare il residuo del backoff.
      window.clearTimeout(timer);
      startChain();
    };

    if (immediate) {
      startChain();
    } else {
      const owner = ++chain;
      schedule(owner);
    }

    document.addEventListener('visibilitychange', onVisibilityChange);
    return () => {
      stopped = true;
      chain += 1;
      window.clearTimeout(timer);
      document.removeEventListener('visibilitychange', onVisibilityChange);
    };
    // `resetKey` è deliberatamente tra le dipendenze: è il segnale con cui il
    // consumatore chiede di riavviare il loop (e ricaricare subito i dati).
  }, [enabled, intervalMs, backoffMs, immediate, resetKey, runOnce]);
};
