import { useCallback, useEffect, useRef, useState } from 'react';
import { listen } from '@tauri-apps/api/event';
import { HUBCAP_KEY_STATUS_EVENT } from '../constants/providers';
import type { HubcapKeyState, HubcapUsage } from './types';

interface HubcapKeyStatusEvent {
  state: HubcapKeyState;
  reason?: string;
  /** Digest of the key the verdict belongs to. */
  fingerprint?: string;
}

/**
 * Tracks the saved Hubcap key verdict.
 *
 * The startup `get_hubcap_usage` answer seeds the state, and
 * `hubcap://key-status` covers failures that happen later (Store, Library,
 * Workshop, version switch). A later `active: true` clears a previous
 * dismissal; `unknown` never raises or clears the warning. A `quiet` answer
 * (the caller already reported the rejection) only feeds the badge.
 */
export function useHubcapKeyStatus(usage: HubcapUsage) {
  const [eventState, setEventState] = useState<HubcapKeyState>('unknown');
  const [eventReason, setEventReason] = useState<string | undefined>();
  const [dismissed, setDismissed] = useState(false);
  const usageRef = useRef(usage);

  useEffect(() => {
    usageRef.current = usage;
  }, [usage]);

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    void listen<HubcapKeyStatusEvent>(HUBCAP_KEY_STATUS_EVENT, (event) => {
      const { state, reason, fingerprint } = event.payload;
      // A request still in flight for the previous key must not raise or clear
      // the warning for the key shown now.
      const current = usageRef.current.keyFingerprint;
      if (fingerprint && current && fingerprint !== current) return;
      setEventState(state);
      setEventReason(reason);
    })
      .then((off) => {
        if (cancelled) off();
        else unlisten = off;
      })
      .catch((error) => console.warn('Hubcap key-status subscription failed:', error));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    if (usage.active === true) {
      setEventState('active');
      setEventReason(undefined);
      setDismissed(false);
    } else if (usage.active === false && !usage.quiet) {
      setEventState('inactive');
      if (usage.reason) setEventReason(usage.reason);
    }
  }, [usage.active, usage.reason, usage.quiet]);

  const dismiss = useCallback(() => setDismissed(true), []);
  // A quiet answer only feeds the badge: the caller already told the user, so
  // it must not raise (or clear) the global warning.
  const inactive = (usage.active === false && !usage.quiet) || eventState === 'inactive';

  return {
    state: inactive ? 'inactive' : eventState,
    reason: eventReason ?? usage.reason,
    inactive,
    dismissed,
    dismiss,
  } as const;
}
