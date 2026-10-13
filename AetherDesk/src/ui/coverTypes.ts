export type CoverFit = 'portrait' | 'landscape';

export interface ResolvedCover {
  url: string;
  fit: CoverFit;
}

export interface PersistedCoverEnvelope {
  v: 4;
  url: string;
  fit: CoverFit;
  /** Epoch ms at which this entry was written. */
  cachedAt: number;
}
