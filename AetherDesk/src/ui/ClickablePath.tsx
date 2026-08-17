import { openInFileManager, shortenPath } from '../util/paths';

interface ClickablePathProps {
  path: string;
  onError?: (message: string) => void;
}

/**
 * Reusable clickable path: opens the folder in the system file manager when
 * clicked, and shows at most 80 characters (77 + "..."). Used for the themes,
 * wallpapers and icons folders in Settings, and anywhere a folder path needs
 * to be opened from the UI.
 */
export const ClickablePath = ({ path, onError }: ClickablePathProps) => {
  const handleClick = async () => {
    try {
      await openInFileManager(path);
    } catch (err) {
      onError?.(`Could not open the folder: ${err}`);
    }
  };

  return (
    <code
      className="settings-path settings-path-clickable"
      title={path}
      onClick={handleClick}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === 'Enter' || e.key === ' ') {
          e.preventDefault();
          void handleClick();
        }
      }}
    >
      {shortenPath(path)}
    </code>
  );
};
