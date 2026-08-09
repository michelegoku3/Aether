import { useEffect, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';

const WALLPAPER_ID = 'aether-personal-wallpaper';
const STYLE_ID = 'aether-personal-wallpaper-style';

const clampOpacity = (value: number) => Math.max(0, Math.min(100, Number.isFinite(value) ? value : 35));

const removeWallpaper = () => {
  document.getElementById(WALLPAPER_ID)?.remove();
  document.getElementById(STYLE_ID)?.remove();
  document.body.classList.remove('aether-wallpaper-enabled');
};

const ensureWallpaperStyle = () => {
  let style = document.getElementById(STYLE_ID) as HTMLStyleElement | null;
  if (!style) {
    style = document.createElement('style');
    style.id = STYLE_ID;
    document.head.appendChild(style);
  }
  style.textContent = `
#aether-personal-wallpaper {
  position: fixed;
  top: 0;
  right: 0;
  bottom: 0;
  /* The wallpaper must cover only the content area, never the sidebar:
     the image is centered on the main column, so it cannot overflow
     underneath the sidebar when the window is resized. */
  left: var(--sidebar-width, 215px);
  z-index: 0;
  pointer-events: none;
  background-size: cover;
  background-position: center;
  background-repeat: no-repeat;
  transition: opacity 0.15s ease;
}
body.aether-wallpaper-enabled .app-container {
  position: relative;
  z-index: 1;
}
body.aether-wallpaper-enabled .main-content,
body.aether-wallpaper-enabled .store-view,
body.aether-wallpaper-enabled .settings-view,
body.aether-wallpaper-enabled .home-view,
body.aether-wallpaper-enabled .aether-view {
  background-color: transparent;
}
`;
};

export function usePersonalWallpaper(enabled: boolean, opacityPercent: number, revision = 0) {
  const latestOpacity = useRef(opacityPercent);

  useEffect(() => {
    latestOpacity.current = opacityPercent;
    const wallpaper = document.getElementById(WALLPAPER_ID) as HTMLDivElement | null;
    if (enabled && wallpaper) {
      wallpaper.style.opacity = String(clampOpacity(opacityPercent) / 100);
    }
  }, [enabled, opacityPercent]);

  useEffect(() => {
    if (!enabled) {
      removeWallpaper();
      return;
    }

    let cancelled = false;

    invoke<string>('get_personal_wallpaper_data_uri')
      .then((dataUri) => {
        if (cancelled) return;
        const trimmed = dataUri.trim();
        if (!trimmed) {
          removeWallpaper();
          return;
        }

        let wallpaper = document.getElementById(WALLPAPER_ID) as HTMLDivElement | null;
        if (!wallpaper) {
          wallpaper = document.createElement('div');
          wallpaper.id = WALLPAPER_ID;
          document.body.prepend(wallpaper);
        }
        wallpaper.style.opacity = String(clampOpacity(latestOpacity.current) / 100);
        wallpaper.style.backgroundImage = `url("${trimmed}")`;

        ensureWallpaperStyle();
        document.body.classList.add('aether-wallpaper-enabled');
      })
      .catch((err) => {
        console.warn('[personal-wallpaper] failed to load:', err);
        removeWallpaper();
      });

    return () => {
      cancelled = true;
    };
  }, [enabled, revision]);
}
