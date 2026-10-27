import React from 'react';
import { invoke } from '@tauri-apps/api/core';
import { ClickablePath } from '../../ui/ClickablePath';
import { openInFileManager } from '../../util/paths';
import type { AppearanceAssets } from './types';

interface SettingsAppearanceSectionProps {
  appearanceAssets: AppearanceAssets;
  customCssEnabled: boolean;
  setCustomCssEnabled: (v: boolean) => void;
  onCustomCssChange: (enabled: boolean) => void;
  handlePickTheme: () => void;
  isPicking: 'theme' | 'wallpaper' | 'icon' | null;
  personalWallpaperEnabled: boolean;
  setPersonalWallpaperEnabled: (v: boolean) => void;
  onPreviewPersonalWallpaper: (enabled: boolean, opacity: number) => void;
  handlePickWallpaper: () => void;
  personalWallpaperOpacity: number;
  setPersonalWallpaperOpacity: (v: number) => void;
  customIconEnabled: boolean;
  setCustomIconEnabled: (v: boolean) => void;
  handlePickIcon: () => void;
  useAlternativeGameCards: boolean;
  setUseAlternativeGameCards: (v: boolean) => void;
  alternativeCardsOpacity: number;
  setAlternativeCardsOpacity: (v: number) => void;
  alternativeCardsFade: number;
  setAlternativeCardsFade: (v: number) => void;
  onPreviewAlternativeCards: (opacity: number, fade: number) => void;
  setIconSelectedFile: (v: string) => void;
  showStatus: (message: string, kind: 'success' | 'error') => void;
  appearancePickBtn: (
    label: string,
    onClick: () => void,
    disabled: boolean,
    isCurrentPicking: boolean
  ) => React.ReactNode;
}

const clamp0to100 = (value: number) => Math.max(0, Math.min(100, Number.isFinite(value) ? value : 0));

export const SettingsAppearanceSection: React.FC<SettingsAppearanceSectionProps> = ({
  appearanceAssets,
  customCssEnabled,
  setCustomCssEnabled,
  onCustomCssChange,
  handlePickTheme,
  isPicking,
  personalWallpaperEnabled,
  setPersonalWallpaperEnabled,
  onPreviewPersonalWallpaper,
  handlePickWallpaper,
  personalWallpaperOpacity,
  setPersonalWallpaperOpacity,
  customIconEnabled,
  setCustomIconEnabled,
  handlePickIcon,
  useAlternativeGameCards,
  setUseAlternativeGameCards,
  alternativeCardsOpacity,
  setAlternativeCardsOpacity,
  alternativeCardsFade,
  setAlternativeCardsFade,
  onPreviewAlternativeCards,
  setIconSelectedFile,
  showStatus,
  appearancePickBtn,
}) => {
  const handleOpenThemeKit = async () => {
    try {
      // Rebuild the kit on demand so the folder exists even when a previous
      // run removed it, then hand it to the file manager.
      await invoke('ensure_custom_css');
      await openInFileManager(appearanceAssets.themeKitDir);
    } catch (err) {
      showStatus(`Failed to open the theme kit: ${err}`, 'error');
    }
  };

  return (
    <div className="settings-group">
      <label className="settings-label">Appearance</label>

      {/* Enable theme */}
      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Enable custom theme</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={customCssEnabled}
            disabled={!appearanceAssets.themeExists}
            onChange={async (e) => {
              const next = e.target.checked;
              setCustomCssEnabled(next);
              onCustomCssChange(next);
              if (next) {
                try {
                  await invoke('ensure_custom_css');
                } catch {
                  /* best-effort: prepares the theme folder, the toggle carries on regardless */
                }
              }
            }}
          />
          <span></span>
        </label>
      </div>

      {!appearanceAssets.themeExists && (
        <p className="settings-desc settings-assets-missing">
          No theme found in AetherData/config/themes — the switch is disabled. Add a .css file to enable it.
        </p>
      )}

      {customCssEnabled && appearanceAssets.themeExists && (
        <div className="settings-appearance-sub">
          <div className="settings-appearance-row">
            <p className="settings-desc">
              The first .css file in{' '}
              <ClickablePath path={appearanceAssets.themesDir} onError={(msg) => showStatus(msg, 'error')} /> is applied
              automatically.{' '}
              {appearanceAssets.themeName ? (
                <>
                  Currently active: <strong>{appearanceAssets.themeName}</strong>.{' '}
                </>
              ) : (
                ''
              )}
              Use the button to pick a different theme.
            </p>
            {appearancePickBtn(
              'Theme',
              handlePickTheme,
              !appearanceAssets.themeExists && !customCssEnabled,
              isPicking === 'theme'
            )}
          </div>
        </div>
      )}

      {/* The kit is how a new theme starts, so it stays reachable while the
          custom theme is off. */}
      <div className="settings-appearance-sub">
        <div className="settings-appearance-row">
          <div className="settings-theme-kit-text">
            <span className="settings-toggle-text">Theme kit</span>
            <p className="settings-desc">
              default.css is the stylesheet AetherDesk uses. Copy it and give it to an AI to build your own theme.
            </p>
          </div>
          <button type="button" className="appearance-pick-btn" onClick={() => void handleOpenThemeKit()}>
            Kit
          </button>
        </div>
      </div>

      {/* Enable personal wallpaper */}
      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Enable custom wallpaper</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={personalWallpaperEnabled}
            disabled={!appearanceAssets.wallpaperExists}
            onChange={async (e) => {
              const next = e.target.checked;
              setPersonalWallpaperEnabled(next);
              onPreviewPersonalWallpaper(next, personalWallpaperOpacity);
              if (next) {
                try {
                  await invoke('ensure_custom_css');
                } catch {
                  /* best-effort: prepares the appearance folders, the toggle carries on regardless */
                }
              }
            }}
          />
          <span></span>
        </label>
      </div>

      {!appearanceAssets.wallpaperExists && (
        <p className="settings-desc settings-assets-missing">
          No wallpaper found in AetherData/config/wallpapers — the switch is disabled. Add an image to enable it.
        </p>
      )}

      {personalWallpaperEnabled && appearanceAssets.wallpaperExists && (
        <div className="settings-appearance-sub">
          <div className="settings-appearance-row">
            <p className="settings-desc">
              The first image in{' '}
              <ClickablePath path={appearanceAssets.wallpapersDir} onError={(msg) => showStatus(msg, 'error')} /> is
              used as the app background.{' '}
              {appearanceAssets.wallpaperName ? (
                <>
                  Currently active: <strong>{appearanceAssets.wallpaperName}</strong>.{' '}
                </>
              ) : (
                ''
              )}
              Use the button to pick a different wallpaper.
            </p>
            {appearancePickBtn(
              'Wallpaper',
              handlePickWallpaper,
              !appearanceAssets.wallpaperExists && !personalWallpaperEnabled,
              isPicking === 'wallpaper'
            )}
          </div>
          <div className="settings-toggle-row">
            <span className="settings-toggle-text">Wallpaper opacity</span>
            <input
              type="text"
              inputMode="numeric"
              pattern="[0-9]*"
              className="settings-number-input"
              value={personalWallpaperOpacity}
              onChange={(e) => {
                const numeric = e.target.value.replace(/\D/g, '');
                const next = clamp0to100(Number(numeric || 0));
                setPersonalWallpaperOpacity(next);
                onPreviewPersonalWallpaper(personalWallpaperEnabled, next);
              }}
            />
          </div>
        </div>
      )}

      {/* Enable custom icon */}
      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Enable custom icon</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={customIconEnabled}
            disabled={!appearanceAssets.iconExists}
            onChange={(e) => {
              const next = e.target.checked;
              setCustomIconEnabled(next);
              if (!next) setIconSelectedFile('');
            }}
          />
          <span></span>
        </label>
      </div>

      {!appearanceAssets.iconExists && (
        <p className="settings-desc settings-assets-missing">
          No icon found in AetherData/config/icons — the switch is disabled. Add an image or .ico to enable it.
        </p>
      )}

      {customIconEnabled && appearanceAssets.iconExists && (
        <div className="settings-appearance-sub">
          <div className="settings-appearance-row">
            <p className="settings-desc">
              The first icon in{' '}
              <ClickablePath path={appearanceAssets.iconsDir} onError={(msg) => showStatus(msg, 'error')} /> is used as
              the window icon.{' '}
              {appearanceAssets.iconName ? (
                <>
                  Currently active: <strong>{appearanceAssets.iconName}</strong>.{' '}
                </>
              ) : (
                ''
              )}
              Use the button to pick a different icon.
            </p>
            {appearancePickBtn(
              'Icon',
              handlePickIcon,
              !appearanceAssets.iconExists && !customIconEnabled,
              isPicking === 'icon'
            )}
          </div>
        </div>
      )}

      {/* Alternative game cards */}
      <div className="settings-toggle-row">
        <span className="settings-toggle-text">Use alternative game cards</span>
        <label className="version-switch">
          <input
            type="checkbox"
            checked={useAlternativeGameCards}
            onChange={(e) => setUseAlternativeGameCards(e.target.checked)}
          />
          <span></span>
        </label>
      </div>

      {useAlternativeGameCards && (
        <div className="settings-appearance-sub">
          <div className="settings-toggle-row">
            <span className="settings-toggle-text">Backdrop opacity</span>
            <input
              type="text"
              inputMode="numeric"
              pattern="[0-9]*"
              className="settings-number-input"
              value={alternativeCardsOpacity}
              onChange={(e) => {
                const numeric = e.target.value.replace(/\D/g, '');
                const next = clamp0to100(Number(numeric || 0));
                setAlternativeCardsOpacity(next);
                onPreviewAlternativeCards(next, alternativeCardsFade);
              }}
            />
          </div>
          <div className="settings-toggle-row">
            <span className="settings-toggle-text">Backdrop fade (bottom)</span>
            <input
              type="text"
              inputMode="numeric"
              pattern="[0-9]*"
              className="settings-number-input"
              value={alternativeCardsFade}
              onChange={(e) => {
                const numeric = e.target.value.replace(/\D/g, '');
                const next = clamp0to100(Number(numeric || 0));
                setAlternativeCardsFade(next);
                onPreviewAlternativeCards(alternativeCardsOpacity, next);
              }}
            />
          </div>
        </div>
      )}
    </div>
  );
};
