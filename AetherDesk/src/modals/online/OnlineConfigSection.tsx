import React from 'react';
import type { OnlineEnableRequest, OnlinePlan } from '../../types/online';

interface OnlineConfigSectionProps {
  plan: OnlinePlan;
  request: OnlineEnableRequest;
  enabled: boolean;
  setRequest: React.Dispatch<React.SetStateAction<OnlineEnableRequest>>;
}

export const OnlineConfigSection: React.FC<OnlineConfigSectionProps> = ({
  plan,
  request,
  enabled,
  setRequest,
}) => {
  const set = <K extends keyof OnlineEnableRequest>(key: K, value: OnlineEnableRequest[K]) =>
    setRequest((prev) => ({ ...prev, [key]: value }));

  const setPhoton = (key: 'realtimeGuid' | 'voiceGuid' | 'fusionGuid', value: string) =>
    setRequest((prev) => ({ ...prev, photon: { ...prev.photon, [key]: value } }));

  const setEos = (
    key: 'productId' | 'sandboxId' | 'deploymentId' | 'clientId' | 'clientSecret',
    value: string
  ) => setRequest((prev) => ({ ...prev, eos: { ...prev.eos, [key]: value } }));

  const checkboxRow = (
    checked: boolean,
    disabled: boolean,
    onChange: (v: boolean) => void,
    description: string
  ) => (
    <div className="op-checkbox-row">
      <span className="op-checkbox-desc">{description}</span>
      <label className="crack-checkbox-label op-checkbox-label">
        <input
          type="checkbox"
          className="crack-checkbox-input"
          checked={checked}
          onChange={(e) => onChange(e.target.checked)}
          disabled={disabled}
        />
        <span className="crack-checkbox-box"></span>
      </label>
    </div>
  );

  return (
    <>
      {/* AppID */}
      <div className="op-box">
        <div className="op-section-title">AppID</div>
        <div className="op-row">
          <span className="op-label">Spoof</span>
          <input
            className="op-input"
            type="text"
            inputMode="numeric"
            pattern="[0-9]*"
            value={request.spoofAppId}
            onChange={(e) => set('spoofAppId', Number(e.target.value) || 480)}
            disabled={enabled}
          />
        </div>
        <div className="op-row">
          <span className="op-label">ogAppID</span>
          <input
            className="op-input"
            type="text"
            inputMode="numeric"
            pattern="[0-9]*"
            value={request.ogAppId}
            onChange={(e) => set('ogAppId', Number(e.target.value) || 0)}
            disabled={enabled}
          />
        </div>
        <div className="op-row">
          <span className="op-label">Client (old-SDK)</span>
          <input
            className="op-input"
            type="text"
            value={request.client}
            onChange={(e) => set('client', e.target.value)}
            disabled={enabled}
            placeholder="017"
          />
        </div>
        <div className="op-note-warn">
          ⚠ Write 017 for old-SDK games that crash on startup (e.g. SpeedRunners).
        </div>
      </div>

      {/* Photon */}
      {plan.detection.backends.photon !== 'none' && (
        <div className="op-box">
          <div className="op-section-title">Photon</div>
          {checkboxRow(
            request.deployPhoton,
            enabled,
            (v) => set('deployPhoton', v),
            'Deploy Photon plugin'
          )}
          {request.deployPhoton && (
            <div className="op-sub">
              {plan.detection.backends.photon === 'fusion' ? (
                <div className="op-row">
                  <span className="op-label">Fusion App GUID</span>
                  <input
                    className="op-input"
                    value={request.photon.fusionGuid}
                    onChange={(e) => setPhoton('fusionGuid', e.target.value)}
                    disabled={enabled}
                    placeholder="app-id-xxxx"
                  />
                </div>
              ) : (
                <>
                  <div className="op-row">
                    <span className="op-label">Realtime App GUID</span>
                    <input
                      className="op-input"
                      value={request.photon.realtimeGuid}
                      onChange={(e) => setPhoton('realtimeGuid', e.target.value)}
                      disabled={enabled}
                      placeholder="app-id-xxxx"
                    />
                  </div>
                  {plan.detection.backends.photonVoice && (
                    <div className="op-row">
                      <span className="op-label">Voice App GUID</span>
                      <input
                        className="op-input"
                        value={request.photon.voiceGuid}
                        onChange={(e) => setPhoton('voiceGuid', e.target.value)}
                        disabled={enabled}
                        placeholder="app-id-xxxx"
                      />
                    </div>
                  )}
                </>
              )}
            </div>
          )}
        </div>
      )}

      {/* EOS */}
      {plan.detection.backends.eos && (
        <div className="op-box">
          <div className="op-section-title">Epic Online Services</div>
          {checkboxRow(
            request.deployEosCustom,
            enabled,
            (v) => set('deployEosCustom', v),
            'Deploy EOS_custom'
          )}
          {request.deployEosCustom && (
            <div className="op-sub">
              {(
                ['productId', 'sandboxId', 'deploymentId', 'clientId', 'clientSecret'] as const
              ).map((key) => (
                <div className="op-row" key={key}>
                  <span className="op-label">{key}</span>
                  <input
                    className="op-input"
                    value={request.eos[key]}
                    onChange={(e) => setEos(key, e.target.value)}
                    disabled={enabled}
                  />
                </div>
              ))}
            </div>
          )}
        </div>
      )}

      {/* PlayFab */}
      {plan.detection.backends.playfab && (
        <div className="op-box">
          <div className="op-section-title">PlayFab</div>
          <div className="op-row">
            <span className="op-label">TitleId (yours)</span>
            <input
              className="op-input"
              value={request.playfab.titleId}
              onChange={(e) => set('playfab', { ...request.playfab, titleId: e.target.value })}
              disabled={enabled || request.playfab.useShared}
              placeholder="XXXXX or SHARED (empty = inert plugin)"
            />
          </div>
          {checkboxRow(
            request.playfab.useShared,
            enabled,
            (v) => set('playfab', { ...request.playfab, useShared: v }),
            'Use the SHARED community TitleId (1D861F — everyone must match)'
          )}
        </div>
      )}

      {/* coherence */}
      {plan.detection.backends.coherence && (
        <div className="op-box">
          <div className="op-section-title">coherence</div>
          <div className="op-row">
            <span className="op-label">Runtime key</span>
            <input
              className="op-input"
              value={request.coherence.runtimeKey}
              onChange={(e) =>
                set('coherence', { ...request.coherence, runtimeKey: e.target.value })
              }
              disabled={enabled || request.coherence.useShared}
              placeholder="your project (schema uploaded)"
            />
          </div>
          {checkboxRow(
            request.coherence.useShared,
            enabled,
            (v) => set('coherence', { ...request.coherence, useShared: v }),
            'Use the SHARED community project (no account, availability not guaranteed)'
          )}
        </div>
      )}

      {/* Settings */}
      <div className="op-box">
        <div className="op-section-title">Settings</div>
        {checkboxRow(request.verboseLog, enabled, (v) => set('verboseLog', v), 'Verbose UCO2 logs')}
        {checkboxRow(request.emulateTicket, enabled, (v) => set('emulateTicket', v), 'Emulate auth ticket')}
        {checkboxRow(
          request.warnOverlayDisabled,
          enabled,
          (v) => set('warnOverlayDisabled', v),
          'Warn when Steam overlay is disabled'
        )}
        {checkboxRow(
          request.loadOverlay,
          enabled,
          (v) => set('loadOverlay', v),
          'Load Steam overlay renderer (LoadOverlay)'
        )}
        {checkboxRow(
          request.logOverlay,
          enabled,
          (v) => set('logOverlay', v),
          'Write steam_overlay.log (LogOverlay)'
        )}
        {checkboxRow(
          request.getStubbedLol,
          enabled,
          (v) => set('getStubbedLol', v),
          'SteamStub runtime hook (GetStubbedLol)'
        )}
        {checkboxRow(request.unlockAllDlc, enabled, (v) => set('unlockAllDlc', v), 'Unlock all DLC')}
        {checkboxRow(request.sdr, enabled, (v) => set('sdr', v), 'Steam Datagram Relay')}
        {plan.detection.engine !== 'generic' && plan.detection.arch === 'x64' &&
          checkboxRow(
            request.deployOverlayProxy,
            enabled,
            (v) => set('deployOverlayProxy', v),
            plan.detection.engine === 'unity'
              ? 'Early overlay proxy (version.dll)'
              : 'Early overlay proxy (XINPUT1_3.dll)'
          )}
      </div>
    </>
  );
};
