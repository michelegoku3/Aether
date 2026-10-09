import { memo, type FormEvent } from 'react';
import { OstWarningModal } from '../modals/OstWarningModal';
import { HubcapUpdateWarningModal } from '../modals/HubcapUpdateWarningModal';
import { SettingsAetherSection } from './settings/SettingsAetherSection';
import { SettingsProvidersSection } from './settings/SettingsProvidersSection';
import { SettingsStoreSection } from './settings/SettingsStoreSection';
import { SettingsAppearanceSection } from './settings/SettingsAppearanceSection';
import { LuaToolsLoginModal } from './settings/LuaToolsLoginModal';
import type { SettingsGuard } from './settings/types';
import { useSettingsController } from './settings/useSettingsController';
export type { SettingsGuard } from './settings/types';

interface SettingsViewProps {
  hubcapUsage: { usage: number; limit: number; hasKey: boolean };
  onRefreshUsage: (forcedKey?: string) => Promise<void>;
  onRefreshCustomCss: () => Promise<void>;
  onCustomCssChange: (enabled: boolean) => void;
  onPreviewPersonalWallpaper: (enabled: boolean, opacity: number) => void;
  onPreviewAlternativeCards: (opacity: number, fade: number) => void;
  onMissingSteamPath: () => void;
  guardRef: { current: SettingsGuard | null };
}

export const SettingsView = memo(function SettingsView(props: SettingsViewProps) {
  const c = useSettingsController(props);
  const { form, appearance: a, luaTools: lua } = c;
  const pickButton = (label: string, onClick: () => void, disabled: boolean, busy: boolean) => (
    <button type="button" className="appearance-pick-btn" onClick={onClick} disabled={disabled || busy}>
      {busy ? '...' : label}
    </button>
  );

  return (
    <div className="settings-view">
      <div className="settings-header">
        <h1 className="settings-title">Settings</h1>
        <p className="settings-subtitle">Manage system configurations, API keys, Steam injection paths, and other settings.</p>
      </div>
      <div className="settings-separator"></div>
      {c.status.text && <div className={`settings-alert ${c.status.type}`}>{c.status.text}</div>}
      {c.settingsConflict && (
        <div className="settings-alert error">
          Settings changed elsewhere. Your unsaved edits have not been overwritten.
          <button type="button" className="action-btn" onClick={() => void c.reloadSaved()}>
            Reload saved settings (discard edits)
          </button>
        </div>
      )}

      <form className="settings-form" onSubmit={(event: FormEvent) => { event.preventDefault(); void c.save(); }}>
        <SettingsAetherSection
          enableWebviewDevtools={form.enableWebviewDevtools}
          setEnableWebviewDevtools={(value) => c.setField('enableWebviewDevtools', value)}
          enableTestUpdates={form.enableTestUpdates}
          setEnableTestUpdates={(value) => c.setField('enableTestUpdates', value)}
          useOstSource={c.useOstSource}
          setUseOstSource={c.setUseOstSource}
          ostWarningAcknowledged={form.ostWarningAcknowledged}
          setShowOstWarning={c.setShowOstWarning}
          manifestRestoreOnStartup={c.manifestRestoreOnStartup}
          setManifestRestoreOnStartup={c.setManifestRestoreOnStartup}
          presenceDefaultShowOnline={c.presenceDefaultShowOnline}
          setPresenceDefaultShowOnline={c.setPresenceDefaultShowOnline}
          customGameName={form.customGameName}
          setCustomGameName={(value) => c.setField('customGameName', value)}
          steamPath={form.steamPath}
          setSteamPath={(value) => c.setField('steamPath', value)}
          steamCheck={c.steamCheck}
          onBrowseSteamFolder={c.browseSteam}
          onDetectSteamPath={c.detectSteam}
          showStatus={c.showStatus}
        />
        <div className="settings-separator"></div>

        <SettingsProvidersSection
          hubcapUsage={props.hubcapUsage}
          apiKey={form.apiKey}
          setApiKey={(value) => c.setField('apiKey', value)}
          showApiKey={c.showApiKey}
          setShowApiKey={c.setShowApiKey}
          ryuuKey={form.ryuuKey}
          setRyuuKey={(value) => c.setField('ryuuKey', value)}
          showRyuuKey={c.showRyuuKey}
          setShowRyuuKey={c.setShowRyuuKey}
          luaToolsAuth={lua.auth}
          isLuaToolsAuthBusy={lua.busy}
          isLuaToolsOAuthBusy={lua.oauthBusy}
          onLuaToolsSignOut={lua.signOut}
          onOpenLuaToolsLogin={lua.open}
          onCancelLuaToolsOAuth={lua.cancelOAuth}
        />
        <div className="settings-separator"></div>

        <SettingsStoreSection
          showStoreDlcs={form.showStoreDlcs}
          setShowStoreDlcs={(value) => c.setField('showStoreDlcs', value)}
          showStoreDelisted={form.showStoreDelisted}
          setShowStoreDelisted={(value) => c.setField('showStoreDelisted', value)}
          showStoreNsfw={form.showStoreNsfw}
          setShowStoreNsfw={(value) => c.setField('showStoreNsfw', value)}
          showStoreFrontGames={form.showStoreFrontGames}
          setShowStoreFrontGames={(value) => c.setField('showStoreFrontGames', value)}
          storeFrontFilter={form.storeFrontFilter}
          setStoreFrontFilter={(value) => c.setField('storeFrontFilter', value)}
          storeCurrency={form.storeCurrency}
          setStoreCurrency={(value) => c.setField('storeCurrency', value)}
          downloadGamesWithUpdatesOn={form.downloadGamesWithUpdatesOn}
          onDownloadGamesWithUpdatesChange={c.changeDownloadUpdates}
          workshopAutoDownloadContent={form.workshopAutoDownloadContent}
          setWorkshopAutoDownloadContent={(value) => c.setField('workshopAutoDownloadContent', value)}
        />
        <div className="settings-separator"></div>

        <SettingsAppearanceSection
          appearanceAssets={a.assets}
          customCssEnabled={form.customCssEnabled}
          setCustomCssEnabled={(value) => c.setField('customCssEnabled', value)}
          onCustomCssChange={props.onCustomCssChange}
          handlePickTheme={() => void a.pickTheme()}
          isPicking={a.picking}
          personalWallpaperEnabled={form.personalWallpaperEnabled}
          setPersonalWallpaperEnabled={(value) => c.setField('personalWallpaperEnabled', value)}
          onPreviewPersonalWallpaper={props.onPreviewPersonalWallpaper}
          handlePickWallpaper={() => void a.pickWallpaper()}
          personalWallpaperOpacity={form.personalWallpaperOpacity}
          setPersonalWallpaperOpacity={(value) => c.setField('personalWallpaperOpacity', value)}
          customIconEnabled={form.customIconEnabled}
          setCustomIconEnabled={(value) => c.setField('customIconEnabled', value)}
          handlePickIcon={() => void a.pickIcon()}
          useAlternativeGameCards={form.useAlternativeGameCards}
          setUseAlternativeGameCards={(value) => c.setField('useAlternativeGameCards', value)}
          alternativeCardsOpacity={form.alternativeCardsOpacity}
          setAlternativeCardsOpacity={(value) => c.setField('alternativeCardsOpacity', value)}
          alternativeCardsFade={form.alternativeCardsFade}
          setAlternativeCardsFade={(value) => c.setField('alternativeCardsFade', value)}
          onPreviewAlternativeCards={props.onPreviewAlternativeCards}
          persistAppearanceSelection={c.persistAppearance}
          setIconSelectedFile={(value) => c.setField('iconSelectedFile', value)}
          showStatus={c.showStatus}
          appearancePickBtn={pickButton}
        />
        <div className="settings-separator"></div>

        <div className="form-actions" style={{ justifyContent: 'center', gap: '12px' }}>
          <button type="submit" className="save-settings-btn" style={{ flex: '1 1 0', maxWidth: '200px' }}>Save Settings</button>
          <button type="button" className="save-settings-btn" style={{ flex: '1 1 0', maxWidth: '200px', backgroundColor: '#1c1c21', border: '1px solid var(--border-color)' }} onClick={() => void c.reset()}>
            Reset Settings
          </button>
        </div>
      </form>

      {c.showHubcapWarning && <HubcapUpdateWarningModal onClose={() => c.setShowHubcapWarning(false)} />}
      {c.showOstWarning && <OstWarningModal onConfirm={() => void c.confirmOst()} onCancel={() => c.setShowOstWarning(false)} />}
      <LuaToolsLoginModal
        open={lua.modalOpen}
        busy={lua.busy}
        mode={lua.mode}
        code={lua.code}
        error={lua.error}
        onClose={lua.close}
        onSetMode={lua.changeMode}
        onCodeChange={lua.changeCode}
        onOAuthSignIn={() => void lua.signIn()}
        onCodeSignIn={() => void lua.codeSignIn()}
      />
    </div>
  );
});
