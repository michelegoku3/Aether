import type { CSSProperties } from 'react';
import { StatusMessage } from '../../types/ui';

interface StatusAlertProps {
  status: StatusMessage;
  className?: string;
  style?: CSSProperties;
}

export const StatusAlert = ({ status, className = '', style }: StatusAlertProps) => {
  if (!status.text) return null;

  return (
    <div className={`settings-alert ${status.type} ${className}`.trim()} style={style}>
      {status.text}
    </div>
  );
};
