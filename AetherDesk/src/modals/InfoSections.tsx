import type { ReactNode } from 'react';
import { NA } from './gameInfoFormat';

export const InfoRow = ({
  label,
  value,
  clamp = false,
}: {
  label: string;
  value?: string | number | null;
  clamp?: boolean;
}) => (
  <div className="info-row">
    <span className="info-label">{label}</span>
    <span className={`info-value${clamp ? ' info-value-clamp' : ''}`}>
      {value === undefined || value === null || value === '' ? NA : value}
    </span>
  </div>
);

export const InfoSection = ({
  title,
  children,
  className = '',
}: {
  title: string;
  children: ReactNode;
  className?: string;
}) => (
  <section className={`info-section ${className}`.trim()}>
    <h3 className="info-section-title">{title}</h3>
    <div className="info-section-body">{children}</div>
  </section>
);
