export type StatusType = 'info' | 'success' | 'error';

export interface StatusMessage {
  text: string;
  type: StatusType;
}

export const emptyStatus = (): StatusMessage => ({ text: '', type: 'info' });
