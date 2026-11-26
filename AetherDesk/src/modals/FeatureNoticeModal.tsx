import type { ReactNode } from 'react';
import { ModalShell } from '../ui/ModalShell';

interface FeatureNoticeModalProps {
  title: string;
  lead: string;
  onClose: () => void;
  children: ReactNode;
}

const FeatureNoticeModal = ({ title, lead, onClose, children }: FeatureNoticeModalProps) => (
  <ModalShell title={title} onClose={onClose} containerClassName="uninstall-modal" bodyClassName="uninstall-modal-body">
    <p className="uninstall-modal-lead">{lead}</p>
    {children}
    <div className="uninstall-modal-actions">
      <button type="button" className="uninstall-btn uninstall-btn-primary" onClick={onClose}>
        Close
      </button>
    </div>
  </ModalShell>
);

export interface NoticeModalProps {
  onClose: () => void;
}

/**
 * Placeholder notice for the Backup sidebar entry: explains what will ship
 * later and where AetherDesk already stores automatic per-game backups.
 */
export const BackupNoticeModal = ({ onClose }: NoticeModalProps) => (
  <FeatureNoticeModal title="Backup" lead="The Backup view is not available yet." onClose={onClose}>
    <p className="uninstall-modal-copy">
      Upcoming updates will introduce local and cloud backups for game saves and more.
    </p>
    <p className="uninstall-modal-copy">
      For now, even though it is not shown in the UI yet, AetherDesk already backs up your .lua/.manifest files,
      achievements, and playtime automatically.
    </p>
    <p className="uninstall-modal-copy">
      You can find these backups in <code>{'AetherDesk/AetherData/backup/<appid>'}</code>.
    </p>
  </FeatureNoticeModal>
);

/**
 * Placeholder notice for ADD DLCs on Home: explains the upcoming Add DLC flow,
 * the current Store base-game Lua workaround, and provider/API limits.
 */
export const AddDlcNoticeModal = ({ onClose }: NoticeModalProps) => (
  <FeatureNoticeModal title="Add DLCs" lead="Full Add DLC support will be added in upcoming updates." onClose={onClose}>
    <p className="uninstall-modal-copy">
      For now, you can still add DLCs to games you own by downloading the base game&apos;s .lua from the Store. Download
      the game&apos;s .lua, not the DLC&apos;s .lua.
    </p>
    <p className="uninstall-modal-copy">
      This works for most games, while others require CreamAPI or SmokeAPI and will not unlock via Lua alone.
    </p>
    <p className="uninstall-modal-copy">
      Which DLCs are included depends on the provider rather than Aether, non-owned Lua games already include DLCs if
      the providers included them.
    </p>
    <p className="uninstall-modal-copy">
      You can verify installed DLCs in-game or in the game&apos;s Steam Properties.
    </p>
  </FeatureNoticeModal>
);
