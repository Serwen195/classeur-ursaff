// Point d'entrée unique vers le backend. Dans l'application de bureau : commandes Tauri.
// En développement dans un simple navigateur (`npm run dev`) : un faux backend en mémoire
// (`mock.ts`), absent de la version finale grâce à `import.meta.env.DEV`.

import type {
  AddAttachmentsResult,
  AppStatus,
  MonthInput,
  MonthRecord,
  UnlockResult,
  SyncState,
  LockReason,
} from './types';

export class AppError extends Error {
  constructor(
    public code: string,
    message: string,
  ) {
    super(message);
  }
}

export interface UpdateInfo {
  version: string;
  notes: string | null;
  install(onProgress: (done: number, total: number | null) => void): Promise<void>;
}

/** Surface complète du backend : implémentée par Tauri (ci-dessous) et par le faux backend. */
export interface Backend {
  getStatus(): Promise<AppStatus>;
  touch(): Promise<void>;
  setupRemote(url: string, token: string): Promise<AppStatus>;
  createVault(passphrase: string): Promise<UnlockResult>;
  unlock(passphrase: string): Promise<UnlockResult>;
  lock(): Promise<void>;
  listRecords(): Promise<UnlockResult>;
  saveMonth(input: MonthInput): Promise<MonthRecord>;
  deleteMonth(year: number, month: number): Promise<void>;
  /** Fichiers déposés dans la fenêtre (chemins fournis par l'événement de glisser-déposer). */
  addAttachments(year: number, month: number, paths: string[]): Promise<AddAttachmentsResult>;
  /** Sélecteur de fichiers natif ouvert côté backend. `null` si l'utilisateur annule. */
  pickAttachments(year: number, month: number): Promise<AddAttachmentsResult | null>;
  removeAttachment(year: number, month: number, id: string): Promise<MonthRecord>;
  attachmentBytes(year: number, month: number, id: string): Promise<ArrayBuffer>;
  openAttachment(year: number, month: number, id: string): Promise<void>;
  /** Boîte « Enregistrer sous » native ouverte côté backend. Renvoie le chemin écrit, ou `null` si annulé. */
  saveAttachmentAs(year: number, month: number, id: string): Promise<string | null>;
  syncNow(): Promise<void>;
  setToken(token: string): Promise<void>;
  setAutoLock(minutes: number): Promise<void>;
  changePassphrase(oldPassphrase: string, newPassphrase: string): Promise<void>;
  resetLocal(force: boolean): Promise<AppStatus>;
  // Événements
  onSyncState(cb: (s: SyncState) => void): Promise<() => void>;
  onLocked(cb: (reason: LockReason) => void): Promise<() => void>;
  onRecordsChanged(cb: () => void): Promise<() => void>;
  onFilesDropped(cb: (paths: string[]) => void): Promise<() => void>;
  checkForUpdate(): Promise<UpdateInfo | null>;
}

function toAppError(e: unknown): AppError {
  if (e instanceof AppError) return e;
  if (e && typeof e === 'object' && 'code' in e && 'message' in e) {
    const o = e as { code: unknown; message: unknown };
    return new AppError(String(o.code), String(o.message));
  }
  return new AppError('other', typeof e === 'string' ? e : e instanceof Error ? e.message : 'Erreur inconnue.');
}

async function createTauriBackend(): Promise<Backend> {
  const [{ invoke }, { listen }, { getCurrentWebview }] = await Promise.all([
    import('@tauri-apps/api/core'),
    import('@tauri-apps/api/event'),
    import('@tauri-apps/api/webview'),
  ]);
  const call = async <T>(cmd: string, args?: Record<string, unknown>): Promise<T> => {
    try {
      return await invoke<T>(cmd, args);
    } catch (e) {
      throw toAppError(e);
    }
  };
  const on = async <T>(event: string, cb: (payload: T) => void) => listen<T>(event, (e) => cb(e.payload));

  return {
    getStatus: () => call('get_status'),
    touch: () => call('touch'),
    setupRemote: (url, token) => call('setup_remote', { url, token }),
    createVault: (passphrase) => call('create_vault', { passphrase }),
    unlock: (passphrase) => call('unlock', { passphrase }),
    lock: () => call('lock'),
    listRecords: () => call('list_records'),
    saveMonth: (input) => call('save_month', { input }),
    deleteMonth: (year, month) => call('delete_month', { year, month }),
    addAttachments: (year, month, paths) => call('add_attachments', { year, month, paths }),
    pickAttachments: (year, month) => call('pick_attachments', { year, month }),
    removeAttachment: (year, month, id) => call('remove_attachment', { year, month, id }),
    attachmentBytes: (year, month, id) => call('attachment_bytes', { year, month, id }),
    openAttachment: (year, month, id) => call('open_attachment', { year, month, id }),
    saveAttachmentAs: (year, month, id) => call('save_attachment_as', { year, month, id }),
    syncNow: () => call('sync_now'),
    setToken: (token) => call('set_token', { token }),
    setAutoLock: (minutes) => call('set_auto_lock', { minutes }),
    changePassphrase: (oldPassphrase, newPassphrase) => call('change_passphrase', { oldPassphrase, newPassphrase }),
    resetLocal: (force) => call('reset_local', { force }),

    onSyncState: (cb) => on<SyncState>('sync-state', cb),
    onLocked: (cb) => on<LockReason>('locked', cb),
    onRecordsChanged: (cb) => on<null>('records-changed', () => cb()),
    async onFilesDropped(cb) {
      return getCurrentWebview().onDragDropEvent((e) => {
        if (e.payload.type === 'drop') cb(e.payload.paths);
      });
    },
    async checkForUpdate() {
      const [{ check }, { relaunch }] = await Promise.all([
        import('@tauri-apps/plugin-updater'),
        import('@tauri-apps/plugin-process'),
      ]);
      const update = await check();
      if (!update) return null;
      return {
        version: update.version,
        notes: update.body ?? null,
        async install(onProgress) {
          let done = 0;
          let total: number | null = null;
          await update.downloadAndInstall((ev) => {
            if (ev.event === 'Started') total = ev.data.contentLength ?? null;
            if (ev.event === 'Progress') {
              done += ev.data.chunkLength;
              onProgress(done, total);
            }
          });
          await relaunch();
        },
      };
    },
  };
}

let backendPromise: Promise<Backend> | undefined;

export function backend(): Promise<Backend> {
  backendPromise ??= (async () => {
    const inTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
    if (inTauri) return createTauriBackend();
    if (import.meta.env.DEV) {
      const { createMockBackend } = await import('./mock');
      return createMockBackend();
    }
    throw new AppError('no_backend', "Cette page doit être ouverte depuis l'application Classeur URSSAF.");
  })();
  return backendPromise;
}

export { toAppError };
