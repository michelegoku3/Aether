import { useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

const STYLE_ID = 'aether-custom-css';

/**
 * Injects `AetherData/config/custom.css` as a `<style>` tag when `enabled` is true.
 * - When `false`, any existing tag is removed (so toggling OFF instantly reverts the theme).
 * - When `true`, fetches the file via Tauri (`get_custom_css`) and injects it as
 *   the *last* child of `<head>` so it wins by cascade order over `style.css`.
 * - Fail-open: empty file, missing file, or read error → no tag, no crash, just a console.warn.
 *
 * This hook is intentionally tiny (SRP) and has no knowledge of settings persistence.
 * The caller (App.tsx) owns the `enabled` boolean via `get_settings`.
 */
export function useCustomCss(enabled: boolean) {
  useEffect(() => {
    const existing = document.getElementById(STYLE_ID) as HTMLStyleElement | null;

    if (!enabled) {
      existing?.remove();
      return;
    }

    let cancelled = false;

    invoke<string>('get_custom_css')
      .then((css) => {
        if (cancelled) return;
        const trimmed = css.trim();
        if (!trimmed) {
          // File missing or empty → ensure no stale tag remains.
          document.getElementById(STYLE_ID)?.remove();
          return;
        }
        let el = document.getElementById(STYLE_ID) as HTMLStyleElement | null;
        if (!el) {
          el = document.createElement('style');
          el.id = STYLE_ID;
          // Append as last child of <head> so it overrides :root vars.
          document.head.appendChild(el);
        }
        el.textContent = css;
      })
      .catch((err) => {
        // Network or file error → fail-open, just warn.
        console.warn('[custom-css] failed to load:', err);
        document.getElementById(STYLE_ID)?.remove();
      });

    return () => {
      cancelled = true;
    };
  }, [enabled]);
}
