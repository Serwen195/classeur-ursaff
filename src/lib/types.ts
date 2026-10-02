// Miroir des types sérialisés par la coque Rust (camelCase côté JSON).

export type Status = 'to_declare' | 'declared' | 'paid';

export interface Attachment {
  id: string;
  name: string;
  mime: string;
  size: number;
  addedAt: number;
}

export interface MonthRecord {
  year: number;
  /** 1 à 12 */
  month: number;
  /** Centimes d'euro. */
  amountCents: number | null;
  /** AAAA-MM-JJ */
  declaredOn: string | null;
  status: Status;
  notes: string;
  attachments: Attachment[];
  updatedAt: number;
}

export interface MonthInput {
  year: number;
  month: number;
  amountCents: number | null;
  declaredOn: string | null;
  status: Status;
  notes: string;
}

export type Phase = 'not_configured' | 'needs_vault_creation' | 'locked' | 'unlocked';

export type SyncStatus = 'idle' | 'syncing' | 'offline' | 'auth_error' | 'error' | 'deferred';

export interface ConflictInfo {
  year: number;
  month: number;
}

export interface SyncState {
  status: SyncStatus;
  message: string | null;
  lastSyncAt: number | null;
  pending: boolean;
  conflicts: ConflictInfo[];
  ignoredFiles: string[];
}

export interface AppStatus {
  phase: Phase;
  remoteUrl: string | null;
  version: string;
  autoLockMinutes: number;
  sync: SyncState;
}

export interface UnlockResult {
  records: MonthRecord[];
  skipped: string[];
}

export interface AddAttachmentsResult {
  record: MonthRecord | null;
  errors: string[];
}

export type LockReason = 'manual' | 'inactivity' | 'reset';
