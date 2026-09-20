import React from 'react';
import type { OnlinePlan } from '../../types/online';
import { backendChips, conflictLabel, engineLabel } from '../../types/online';
import { folderOf, pathFromGameRoot, shortenPath } from '../../util/paths';

interface OnlineDetectionSectionProps {
  plan: OnlinePlan;
  onOpenFolder: (folder: string) => void;
}

export const OnlineDetectionSection: React.FC<OnlineDetectionSectionProps> = ({
  plan,
  onOpenFolder,
}) => {
  return (
    <>
      {/* Prerequisites errors */}
      {plan.prerequisites.errors.length > 0 && (
        <div className="op-box">
          {plan.prerequisites.errors.map((error, index) => (
            <div key={index} className="op-error">⚠ {error}</div>
          ))}
        </div>
      )}

      {/* Detection info box */}
      <div className="op-box">
        <div className="op-row">
          <span className="op-label">UCO2</span>
          <span className={plan.prerequisites.bundleOk ? 'op-ok' : 'op-error'}>
            {plan.prerequisites.bundleOk
              ? `${plan.prerequisites.bundleVersion ?? 'Available'} ✓`
              : 'N/A ✗'}
          </span>
        </div>
        <div className="op-row">
          <span className="op-label">Writable folder</span>
          <span className={plan.prerequisites.steamApiDirWritable ? 'op-ok' : 'op-error'}>
            {plan.prerequisites.steamApiDirWritable ? 'Yes ✓' : 'No ✗'}
          </span>
        </div>
        <div className="op-row">
          <span className="op-label">Engine</span>
          <span>
            {engineLabel(plan.detection.engine)} · {plan.detection.arch === 'x64' ? '64-bit' : '32-bit'}
          </span>
        </div>
        {plan.detection.gameExe && (
          <div className="op-row">
            <span className="op-label">Executable</span>
            <span
              className="settings-path settings-path--wide settings-path-clickable op-path-link"
              title={plan.detection.gameExe}
              onClick={() => onOpenFolder(folderOf(plan.detection.gameExe!))}
            >
              {shortenPath(pathFromGameRoot(plan.detection.gameRoot, plan.detection.gameExe))}
            </span>
          </div>
        )}
        {plan.detection.steamApiDir && (
          <div className="op-row">
            <span className="op-label">DLL location</span>
            <span
              className="settings-path settings-path--wide settings-path-clickable op-path-link"
              title={plan.detection.steamApiDir}
              onClick={() => onOpenFolder(plan.detection.steamApiDir!)}
            >
              {shortenPath(`${pathFromGameRoot(plan.detection.gameRoot, plan.detection.steamApiDir)}\\`)}
            </span>
          </div>
        )}
        <div className="op-row">
          <span className="op-label">Backend</span>
          <span>
            {backendChips(plan.detection.backends).map((chip) => (
              <span key={chip} className="op-chip op-chip--off">{chip}</span>
            ))}
          </span>
        </div>
        {plan.detection.conflicts.length > 0 && (
          <div className="op-row">
            <span className="op-label">Conflicts</span>
            <span className="op-warn">
              {plan.detection.conflicts.map((c) => conflictLabel(c.kind)).join(', ')}: will be neutralized (reversible)
            </span>
          </div>
        )}
        {/* Notices: always the last lines of this box */}
        {plan.notices.length > 0 && (
          <div className="op-notices">
            {plan.notices.map((notice, index) => (
              <div key={index} className="op-warn">⚠ {notice}</div>
            ))}
          </div>
        )}
      </div>
    </>
  );
};
