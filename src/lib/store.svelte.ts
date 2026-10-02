// État global de l'interface (Svelte 5, runes). Aucune logique de chiffrement ni de Git ici :
// tout passe par le backend.

import { AppError, backend, toAppError, type UpdateInfo } from './api';
import { clampYear, monthLabel } from './months';
import { recordFor, upsertRecord } from './totals';
import type { AppStatus, MonthInput, MonthRecord, Phase, SyncState } from './types';

export type ViewPhase = Phase | 'loading' | 'fatal';

export interface Notice {
  kind: 'error' | 'info';
  text: string;
}

export interface ConfirmRequest {
  title: string;
  message: string;
  confirmLabel: string;
  danger?: boolean;
  resolve: (ok: boolean) => void;
}

export type UpdateState =
  | { kind: 'idle' }
  | { kind: 'checking' }
  | { kind: 'uptodate' }
  | { kind: 'available'; version: string; notes: string | null }
  | { kind: 'installing'; version: string; percent: number | null }
  | { kind: 'error'; message: string };

const EMPTY_SYNC: SyncState = { status: 'idle', message: null, lastSyncAt: null, pending: false, conflicts: [], ignoredFiles: [] };

class AppStore {
  phase = $state<ViewPhase>('loading');
  fatal = $state<string | null>(null);
  remoteUrl = $state<string | null>(null);
  version = $state('');
  autoLockMinutes = $state(10);
  sync = $state<SyncState>(EMPTY_SYNC);

  records = $state<MonthRecord[]>([]);
  skipped = $state<string[]>([]);
  year = $state(new Date().getFullYear());
  query = $state('');
  editing = $state<{ year: number; month: number } | null>(null);
  settingsOpen = $state(false);

  notice = $state<Notice | null>(null);
  confirmRequest = $state<ConfirmRequest | null>(null);
  update = $state<UpdateState>({ kind: 'idle' });
  /** Raison du dernier verrouillage, affichée sur l'écran de déverrouillage. */
  lockReason = $state<string | null>(null);
  /** Conflits résolus par la dernière synchronisation (bandeau fermable). */
  conflictsDismissed = $state(false);

  private gotSyncEvent = false;
  private pendingUpdate: UpdateInfo | null = null;
  private lastTouch = 0;
  private started = false;

  // ───────── démarrage ─────────

  async init(): Promise<void> {
    if (this.started) return;
    this.started = true;
    try {
      const b = await backend();
      // S'abonner AVANT de lire l'état : la synchronisation du lancement démarre côté Rust avant que
      // la page soit prête, et sa fin ne doit pas pouvoir passer entre les deux.
      await b.onSyncState((s) => {
        this.gotSyncEvent = true;
        this.setSync(s);
      });
      await b.onLocked((reason) => void this.onLocked(reason));
      await b.onRecordsChanged(() => void this.reload());
      this.apply(await b.getStatus());
      this.trackActivity();
      // Vérifie les mises à jour peu après le lancement, sans gêner l'affichage.
      setTimeout(() => void this.checkUpdate(false), 2500);
      if (this.phase === 'unlocked') await this.reload();
    } catch (e) {
      this.phase = 'fatal';
      this.fatal = toAppError(e).message;
    }
  }

  private apply(s: AppStatus): void {
    this.phase = s.phase;
    this.remoteUrl = s.remoteUrl;
    this.version = s.version;
    this.autoLockMinutes = s.autoLockMinutes;
    // Un événement plus récent que cet instantané a peut-être déjà été reçu : il prévaut.
    if (!this.gotSyncEvent) this.setSync(s.sync);
  }

  private setSync(s: SyncState): void {
    if (s.conflicts.length > 0 && s.conflicts.length !== this.sync.conflicts.length) this.conflictsDismissed = false;
    this.sync = s;
    // Un autre appareil a pu créer le coffre pendant la mise à jour du lancement.
    if (s.status === 'idle' && this.phase === 'needs_vault_creation') void this.refreshStatus();
  }

  private async refreshStatus(): Promise<void> {
    try {
      const st = await (await backend()).getStatus();
      if (st.phase !== this.phase && this.phase === 'needs_vault_creation') this.phase = st.phase;
    } catch {
      /* sans conséquence : le prochain événement réessaiera */
    }
  }

  private trackActivity(): void {
    const bump = () => {
      const now = Date.now();
      if (this.phase !== 'unlocked' || now - this.lastTouch < 15_000) return;
      this.lastTouch = now;
      void backend().then((b) => b.touch());
    };
    for (const ev of ['pointerdown', 'keydown', 'wheel', 'pointermove'] as const) {
      window.addEventListener(ev, bump, { passive: true });
    }
  }

  // ───────── erreurs & confirmations ─────────

  private fail(e: unknown): AppError {
    const err = toAppError(e);
    if (err.code === 'locked') {
      void this.onLocked('inactivity');
    } else {
      this.notice = { kind: 'error', text: err.message };
    }
    return err;
  }

  info(text: string): void {
    this.notice = { kind: 'info', text };
  }

  dismissNotice(): void {
    this.notice = null;
  }

  ask(req: Omit<ConfirmRequest, 'resolve'>): Promise<boolean> {
    return new Promise((resolve) => {
      this.confirmRequest = { ...req, resolve };
    });
  }

  answerConfirm(ok: boolean): void {
    const r = this.confirmRequest;
    this.confirmRequest = null;
    r?.resolve(ok);
  }

  // ───────── installation et verrouillage ─────────

  async setupRemote(url: string, token: string): Promise<boolean> {
    try {
      this.apply(await (await backend()).setupRemote(url, token));
      this.notice = null;
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  private clearSensitive(): void {
    this.records = [];
    this.skipped = [];
    this.editing = null;
    this.query = '';
    this.settingsOpen = false;
  }

  private async afterUnlock(r: { records: MonthRecord[]; skipped: string[] }): Promise<void> {
    this.records = r.records;
    this.skipped = r.skipped;
    this.phase = 'unlocked';
    this.lockReason = null;
    this.notice = null;
    this.year = new Date().getFullYear();
    this.lastTouch = 0;
  }

  async createVault(passphrase: string): Promise<boolean> {
    try {
      await this.afterUnlock(await (await backend()).createVault(passphrase));
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  /** Renvoie `null` si OK, sinon le message d'erreur à afficher sous le champ. */
  async unlock(passphrase: string): Promise<string | null> {
    try {
      await this.afterUnlock(await (await backend()).unlock(passphrase));
      return null;
    } catch (e) {
      return toAppError(e).message;
    }
  }

  async lock(): Promise<void> {
    await (await backend()).lock();
    // L'événement « locked » met l'interface à jour ; on le fait aussi ici si l'événement tarde.
    await this.onLocked('manual');
  }

  private async onLocked(reason: string): Promise<void> {
    this.clearSensitive();
    if (reason === 'reset') {
      this.apply(await (await backend()).getStatus());
      return;
    }
    this.phase = 'locked';
    this.lockReason = reason === 'inactivity' ? 'Verrouillé automatiquement après une période d’inactivité.' : null;
  }

  async reload(): Promise<void> {
    try {
      const r = await (await backend()).listRecords();
      this.records = r.records;
      this.skipped = r.skipped;
    } catch (e) {
      this.fail(e);
    }
  }

  // ───────── données ─────────

  get currentRecords(): MonthRecord[] {
    return this.records.filter((r) => r.year === this.year);
  }

  recordOf(year: number, month: number): MonthRecord | undefined {
    return recordFor(this.records, year, month);
  }

  setYear(y: number): void {
    this.year = clampYear(y);
  }

  openMonth(year: number, month: number): void {
    this.year = year;
    this.editing = { year, month };
  }

  closeEditor(): void {
    this.editing = null;
  }

  async saveMonth(input: MonthInput): Promise<boolean> {
    try {
      const rec = await (await backend()).saveMonth(input);
      this.records = upsertRecord(this.records, rec);
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  async deleteMonth(year: number, month: number): Promise<boolean> {
    try {
      await (await backend()).deleteMonth(year, month);
      this.records = this.records.filter((r) => !(r.year === year && r.month === month));
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  async addAttachments(year: number, month: number, paths: string[]): Promise<void> {
    if (paths.length === 0) return;
    try {
      const r = await (await backend()).addAttachments(year, month, paths);
      if (r.record) this.records = upsertRecord(this.records, r.record);
      if (r.errors.length > 0) this.notice = { kind: 'error', text: r.errors.join('\n') };
    } catch (e) {
      this.fail(e);
    }
  }

  async pickAndAddAttachments(year: number, month: number): Promise<void> {
    try {
      const r = await (await backend()).pickAttachments(year, month);
      if (!r) return;
      if (r.record) this.records = upsertRecord(this.records, r.record);
      if (r.errors.length > 0) this.notice = { kind: 'error', text: r.errors.join('\n') };
    } catch (e) {
      this.fail(e);
    }
  }

  async removeAttachment(year: number, month: number, id: string): Promise<void> {
    try {
      const rec = await (await backend()).removeAttachment(year, month, id);
      this.records = upsertRecord(this.records, rec);
    } catch (e) {
      this.fail(e);
    }
  }

  async openAttachment(year: number, month: number, id: string): Promise<void> {
    try {
      await (await backend()).openAttachment(year, month, id);
    } catch (e) {
      this.fail(e);
    }
  }

  async saveAttachmentAs(year: number, month: number, id: string): Promise<void> {
    try {
      const dest = await (await backend()).saveAttachmentAs(year, month, id);
      if (dest) this.info(`Pièce jointe enregistrée : ${dest}`);
    } catch (e) {
      this.fail(e);
    }
  }

  // ───────── réglages ─────────

  async syncNow(): Promise<void> {
    try {
      await (await backend()).syncNow();
    } catch (e) {
      this.fail(e);
    }
  }

  async setToken(token: string): Promise<boolean> {
    try {
      await (await backend()).setToken(token);
      this.info('Jeton mis à jour. Synchronisation en cours…');
      return true;
    } catch (e) {
      this.fail(e);
      return false;
    }
  }

  async setAutoLock(minutes: number): Promise<void> {
    try {
      await (await backend()).setAutoLock(minutes);
      this.autoLockMinutes = minutes;
    } catch (e) {
      this.fail(e);
    }
  }

  async changePassphrase(oldP: string, newP: string): Promise<string | null> {
    try {
      await (await backend()).changePassphrase(oldP, newP);
      this.info('Phrase secrète modifiée. Les autres appareils devront utiliser la nouvelle phrase après leur prochaine synchronisation.');
      return null;
    } catch (e) {
      return toAppError(e).message;
    }
  }

  async resetLocal(): Promise<void> {
    const b = await backend();
    try {
      this.apply(await b.resetLocal(false));
    } catch (e) {
      const err = toAppError(e);
      if (err.code !== 'unpushed') {
        this.fail(err);
        return;
      }
      const ok = await this.ask({
        title: 'Modifications non envoyées',
        message: `${err.message} Continuer quand même ?`,
        confirmLabel: 'Oublier ce poste quand même',
        danger: true,
      });
      if (!ok) return;
      try {
        this.apply(await b.resetLocal(true));
      } catch (e2) {
        this.fail(e2);
      }
    }
    this.clearSensitive();
  }

  // ───────── mises à jour de l'application ─────────

  async checkUpdate(manual: boolean): Promise<void> {
    if (this.update.kind === 'installing') return;
    if (manual) this.update = { kind: 'checking' };
    try {
      const info = await (await backend()).checkForUpdate();
      this.pendingUpdate = info;
      if (info) this.update = { kind: 'available', version: info.version, notes: info.notes };
      else this.update = manual ? { kind: 'uptodate' } : { kind: 'idle' };
    } catch (e) {
      // Au lancement, une vérification qui échoue (hors ligne…) ne doit pas déranger.
      this.update = manual ? { kind: 'error', message: toAppError(e).message } : { kind: 'idle' };
    }
  }

  async installUpdate(): Promise<void> {
    const info = this.pendingUpdate;
    if (!info) return;
    this.update = { kind: 'installing', version: info.version, percent: null };
    try {
      await info.install((done, total) => {
        this.update = { kind: 'installing', version: info.version, percent: total ? Math.min(100, Math.round((done / total) * 100)) : null };
      });
    } catch (e) {
      // Échec (signature invalide, coupure réseau…) : on le dit et on permet de réessayer.
      const message = toAppError(e).message;
      this.update = { kind: 'available', version: info.version, notes: info.notes };
      this.notice = { kind: 'error', text: `La mise à jour n'a pas pu être installée : ${message}` };
    }
  }
}

export const app = new AppStore();

/** « Mars 2025, Avril 2025 » pour le bandeau de conflits. */
export function conflictLabels(s: SyncState): string {
  return s.conflicts.map((c) => monthLabel(c.year, c.month)).join(', ');
}
