import React from 'react';
import { EyeIcon, EyeOffIcon } from '../../ui/icons';
import type { LuaToolsAuthStatus } from './types';

interface SettingsProvidersSectionProps {
  hubcapUsage: { usage: number; limit: number; hasKey: boolean };
  apiKey: string;
  setApiKey: (v: string) => void;
  showApiKey: boolean;
  setShowApiKey: (v: boolean | ((prev: boolean) => boolean)) => void;
  ryuuKey: string;
  setRyuuKey: (v: string) => void;
  showRyuuKey: boolean;
  setShowRyuuKey: (v: boolean | ((prev: boolean) => boolean)) => void;
  luaToolsAuth: LuaToolsAuthStatus;
  isLuaToolsAuthBusy: boolean;
  isLuaToolsOAuthBusy: boolean;
  onLuaToolsSignOut: () => Promise<void>;
  onOpenLuaToolsLogin: () => void;
  onCancelLuaToolsOAuth: () => Promise<void>;
}

export const SettingsProvidersSection: React.FC<SettingsProvidersSectionProps> = ({
  hubcapUsage,
  apiKey,
  setApiKey,
  showApiKey,
  setShowApiKey,
  ryuuKey,
  setRyuuKey,
  showRyuuKey,
  setShowRyuuKey,
  luaToolsAuth,
  isLuaToolsAuthBusy,
  isLuaToolsOAuthBusy,
  onLuaToolsSignOut,
  onOpenLuaToolsLogin,
  onCancelLuaToolsOAuth,
}) => {
  return (
    <>
      {/* Hubcap API Key Section */}
      <div className="settings-group">
        <label className="settings-label">Hubcap API Key</label>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
          <p className="settings-desc">
            Enter your hubcapmanifest.com API key to unlock database lookups and downloads.
          </p>
          {hubcapUsage.hasKey && (
            <span
              style={{
                fontSize: '12px',
                color: '#8f8f9e',
                fontWeight: 'bold',
                marginLeft: '12px',
                whiteSpace: 'nowrap',
              }}
            >
              {hubcapUsage.usage}/{hubcapUsage.limit}
            </span>
          )}
        </div>
        <div className="settings-input-wrap">
          <input
            type={showApiKey ? 'text' : 'password'}
            placeholder="Enter API key (e.g. smm_...)"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            className="settings-input"
            autoComplete="off"
            spellCheck={false}
          />
          <button
            type="button"
            className="settings-input-eye"
            title={showApiKey ? 'Hide API key' : 'Show API key'}
            aria-label={showApiKey ? 'Hide API key' : 'Show API key'}
            onClick={() => setShowApiKey((v) => !v)}
          >
            {showApiKey ? <EyeOffIcon size={16} /> : <EyeIcon size={16} />}
          </button>
        </div>
      </div>

      {/* Ryuu API Key Section */}
      <div className="settings-group">
        <label className="settings-label">Ryuu API Key</label>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
          <p className="settings-desc">
            Enter your generator.ryuu.lol API key to unlock downloads via Ryuu.
          </p>
          {ryuuKey.trim() !== '' && (
            <span
              style={{
                fontSize: '12px',
                color: '#8f8f9e',
                fontWeight: 'bold',
                marginLeft: '12px',
                whiteSpace: 'nowrap',
              }}
            >
              50/day
            </span>
          )}
        </div>
        <div className="settings-input-wrap">
          <input
            type={showRyuuKey ? 'text' : 'password'}
            placeholder="Enter Ryuu key (e.g. V1nr...)"
            value={ryuuKey}
            onChange={(e) => setRyuuKey(e.target.value)}
            className="settings-input"
            autoComplete="off"
            spellCheck={false}
          />
          <button
            type="button"
            className="settings-input-eye"
            title={showRyuuKey ? 'Hide API key' : 'Show API key'}
            aria-label={showRyuuKey ? 'Hide API key' : 'Show API key'}
            onClick={() => setShowRyuuKey((v) => !v)}
          >
            {showRyuuKey ? <EyeOffIcon size={16} /> : <EyeIcon size={16} />}
          </button>
        </div>
      </div>

      {/* LuaTools account */}
      <div className="settings-group">
        <label className="settings-label">LuaTools Account</label>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
          <p className="settings-desc">
            Log in to your lua.tools account to download from its available manifest sources.
          </p>
          {luaToolsAuth.signedIn && (
            <span
              style={{
                fontSize: '12px',
                color: '#8f8f9e',
                fontWeight: 'bold',
                marginLeft: '12px',
                whiteSpace: 'nowrap',
              }}
            >
              25/day
            </span>
          )}
        </div>
        <div className="settings-toggle-row" style={{ padding: 0 }}>
          <span className="settings-toggle-text">
            {luaToolsAuth.signedIn
              ? `Connected${
                  luaToolsAuth.displayName
                    ? ` as ${luaToolsAuth.displayName}`
                    : luaToolsAuth.email
                    ? ` as ${luaToolsAuth.email}`
                    : ''
                }`
              : 'Not connected'}
          </span>
          <button
            type="button"
            className="settings-small-btn"
            style={{
              width: '96px',
              minWidth: '96px',
              maxWidth: '96px',
              height: '33px',
              padding: '0',
              display: 'inline-flex',
              alignItems: 'center',
              justifyContent: 'center',
              boxSizing: 'border-box',
            }}
            disabled={isLuaToolsAuthBusy && !isLuaToolsOAuthBusy}
            onClick={() =>
              void (isLuaToolsOAuthBusy
                ? onCancelLuaToolsOAuth()
                : luaToolsAuth.signedIn
                ? onLuaToolsSignOut()
                : onOpenLuaToolsLogin())
            }
          >
            {isLuaToolsOAuthBusy
              ? 'Cancel'
              : isLuaToolsAuthBusy
              ? '...'
              : luaToolsAuth.signedIn
              ? 'Logout'
              : 'Login'}
          </button>
        </div>
      </div>
    </>
  );
};
