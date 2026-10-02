// Faux backend en mémoire pour développer l'interface dans un navigateur ordinaire et pour les
// tests e2e. Reproduit les règles du vrai backend (validations, codes d'erreur) mais ne chiffre
// rien et ne parle à aucun serveur. Chargé uniquement en mode développement.
//
// Paramètres d'URL : ?scenario=setup|vault|locked|unlocked   ?offline=1   ?update=1   ?slow=1

import { AppError, type Backend, type UpdateInfo } from './api';
import type {
  AddAttachmentsResult,
  AppStatus,
  Attachment,
  LockReason,
  MonthInput,
  MonthRecord,
  Phase,
  SyncState,
  UnlockResult,
} from './types';

export const MOCK_PASSPHRASE = 'phrase secrete de test';
const MIN_PASSPHRASE = 12;
const MAX_ATTACHMENT = 20 * 1024 * 1024;
const MIME: Record<string, string> = {
  pdf: 'application/pdf',
  png: 'image/png',
  jpg: 'image/jpeg',
  jpeg: 'image/jpeg',
  gif: 'image/gif',
  webp: 'image/webp',
  bmp: 'image/bmp',
  tif: 'image/tiff',
  tiff: 'image/tiff',
  heic: 'image/heic',
  heif: 'image/heif',
};

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));
const rid = () => Array.from(crypto.getRandomValues(new Uint8Array(16)), (b) => b.toString(16).padStart(2, '0')).join('');
const key = (y: number, m: number) => `${y}-${m}`;

function seed(): MonthRecord[] {
  const y = new Date().getFullYear();
  const mk = (year: number, month: number, cents: number | null, status: MonthRecord['status'], day: number | null, notes = ''): MonthRecord => ({
    year,
    month,
    amountCents: cents,
    declaredOn: day ? `${month === 12 ? year + 1 : year}-${String(month === 12 ? 1 : month + 1).padStart(2, '0')}-${String(day).padStart(2, '0')}` : null,
    status,
    notes,
    attachments: [],
    updatedAt: Date.now() - (13 - month) * 86_400_000,
  });
  const withPdf = (r: MonthRecord, name: string): MonthRecord => ({
    ...r,
    attachments: [{ id: rid(), name, mime: 'application/pdf', size: 48_213, addedAt: Date.now() }],
  });
  return [
    mk(y - 1, 11, 184_250, 'paid', 8, 'Gros client en novembre.'),
    mk(y - 1, 12, 312_000, 'paid', 10, 'Soldes d\'hiver.'),
    withPdf(mk(y, 1, 250_075, 'paid', 9), 'Accusé de déclaration janvier.pdf'),
    withPdf(mk(y, 2, 198_000, 'paid', 12, 'Déclaré un peu en retard.'), 'Justificatif de paiement février.pdf'),
    mk(y, 3, 421_990, 'declared', 7),
    mk(y, 4, 87_500, 'to_declare', null, 'À vérifier : facture fournisseur manquante.'),
  ];
}

export function createMockBackend(): Backend {
  const q = new URLSearchParams(location.search);
  const scenario = q.get('scenario') ?? 'setup';
  const slow = q.get('slow') === '1';
  let offline = q.get('offline') === '1';
  window.__mockOffline = (v: boolean) => {
    offline = v;
  };

  let phase: Phase =
    scenario === 'vault' ? 'needs_vault_creation' : scenario === 'locked' || scenario === 'unlocked' || scenario === 'skipped' ? 'locked' : 'not_configured';
  let remoteUrl: string | null = phase === 'not_configured' ? null : 'https://github.com/demo/classeur-urssaf-data';
  let passphrase = MOCK_PASSPHRASE;
  let autoLockMinutes = 10;
  let records = new Map<string, MonthRecord>();
  const files = new Map<string, Uint8Array>();
  const picked = new Map<string, File>();
  let sync: SyncState = { status: 'idle', message: null, lastSyncAt: Date.now() - 120_000, pending: false, conflicts: [], ignoredFiles: [] };
  const syncListeners = new Set<(s: SyncState) => void>();
  const lockListeners = new Set<(r: LockReason) => void>();
  const changedListeners = new Set<() => void>();

  if (phase === 'locked') for (const r of seed()) records.set(key(r.year, r.month), r);

  const status = (): AppStatus => ({ phase, remoteUrl, version: '0.1.0-mock', autoLockMinutes, sync });
  const setSync = (s: Partial<SyncState>) => {
    sync = { ...sync, ...s };
    syncListeners.forEach((l) => l(sync));
  };
  const runSync = async () => {
    setSync({ status: 'syncing', message: null });
    await delay(slow ? 1500 : 350);
    if (offline) {
      setSync({ status: 'offline', pending: true, message: 'Hors ligne : vos modifications sont conservées et seront envoyées dès que possible.' });
    } else {
      setSync({ status: 'idle', pending: false, message: null, lastSyncAt: Date.now() });
    }
  };
  const requireUnlocked = () => {
    if (phase !== 'unlocked') throw new AppError('locked', 'Le classeur est verrouillé.');
  };
  const sorted = () => [...records.values()].sort((a, b) => a.year - b.year || a.month - b.month);
  const checkYM = (y: number, m: number) => {
    if (y < 2000 || y > 2100) throw new AppError('invalid', 'Année hors limites (2000–2100).');
    if (m < 1 || m > 12) throw new AppError('invalid', 'Le mois doit être compris entre 1 et 12.');
  };
  const touchRecord = (r: MonthRecord): MonthRecord => {
    const next = { ...r, updatedAt: Date.now() };
    records.set(key(r.year, r.month), next);
    return next;
  };
  const blank = (year: number, month: number): MonthRecord => ({
    year,
    month,
    amountCents: null,
    declaredOn: null,
    status: 'to_declare',
    notes: '',
    attachments: [],
    updatedAt: 0,
  });
  const unlockResult = (): UnlockResult => ({ records: sorted(), skipped: scenario === 'skipped' ? ['abc.enc'] : [] });

  async function addAttachments(year: number, month: number, paths: string[]): Promise<AddAttachmentsResult> {
    requireUnlocked();
    checkYM(year, month);
    const errors: string[] = [];
    let record: MonthRecord | null = null;
    for (const p of paths) {
      const f = picked.get(p);
      const name = f?.name ?? p.split(/[\\/]/).pop() ?? p;
      const ext = name.includes('.') ? name.split('.').pop()!.toLowerCase() : '';
      if (!f) {
        errors.push(`${name} : fichier introuvable.`);
        continue;
      }
      if (!MIME[ext]) {
        errors.push(`${name} : Seuls les PDF et les images (PNG, JPEG, GIF, WebP, BMP, TIFF, HEIC) sont acceptés.`);
        continue;
      }
      if (f.size > MAX_ATTACHMENT) {
        errors.push(`${name} : Fichier trop volumineux (maximum 20 Mo).`);
        continue;
      }
      const id = rid();
      files.set(id, new Uint8Array(await f.arrayBuffer()));
      const cur = records.get(key(year, month)) ?? blank(year, month);
      const att: Attachment = { id, name, mime: MIME[ext], size: f.size, addedAt: Date.now() };
      record = touchRecord({ ...cur, attachments: [...cur.attachments, att] });
    }
    void runSync();
    return { record, errors };
  }

  if (scenario === 'unlocked') {
    // Raccourci pour les captures d'écran : déjà déverrouillé.
    phase = 'unlocked';
  }

  return {
    async getStatus() {
      return status();
    },
    async touch() {},
    async setupRemote(url, token) {
      await delay(slow ? 1200 : 300);
      if (!/^https:\/\/[^/@\s]+\/[^/\s]+\/[^/\s]+/.test(url.trim())) {
        throw new AppError('invalid', 'Adresse incomplète. Exemple : https://github.com/votre-compte/classeur-urssaf-data');
      }
      if (/@/.test(url)) throw new AppError('invalid', "N'indiquez pas d'identifiant ni de jeton dans l'adresse.");
      if (token.trim() === '') throw new AppError('invalid', "Le jeton d'accès GitHub est requis.");
      if (token.trim() === 'mauvais') {
        throw new AppError('auth', "Authentification refusée par le dépôt distant (jeton invalide, expiré ou sans droit d'écriture).");
      }
      remoteUrl = url.trim();
      phase = url.includes('existant') ? 'locked' : 'needs_vault_creation';
      if (phase === 'locked') for (const r of seed()) records.set(key(r.year, r.month), r);
      return status();
    },
    async createVault(p) {
      await delay(300);
      if (p.length < MIN_PASSPHRASE) throw new AppError('invalid', `La phrase secrète doit contenir au moins ${MIN_PASSPHRASE} caractères.`);
      passphrase = p;
      phase = 'unlocked';
      records = new Map();
      void runSync();
      return unlockResult();
    },
    async unlock(p) {
      await delay(slow ? 1200 : 250);
      if (p !== passphrase) throw new AppError('wrong_passphrase', 'Phrase secrète incorrecte.');
      phase = 'unlocked';
      void runSync();
      return unlockResult();
    },
    async lock() {
      phase = 'locked';
      lockListeners.forEach((l) => l('manual'));
    },
    async listRecords() {
      requireUnlocked();
      return unlockResult();
    },
    async saveMonth(input: MonthInput) {
      requireUnlocked();
      checkYM(input.year, input.month);
      if (input.amountCents !== null && input.amountCents < 0) throw new AppError('invalid', 'Montant invalide.');
      if (input.declaredOn !== null && !/^\d{4}-\d{2}-\d{2}$/.test(input.declaredOn)) {
        throw new AppError('invalid', 'Date invalide (format attendu : AAAA-MM-JJ).');
      }
      if ([...input.notes].length > 10_000) throw new AppError('invalid', 'Les notes sont limitées à 10000 caractères.');
      await delay(60);
      const cur = records.get(key(input.year, input.month)) ?? blank(input.year, input.month);
      const saved = touchRecord({ ...cur, ...input, attachments: cur.attachments });
      void runSync();
      return saved;
    },
    async deleteMonth(year, month) {
      requireUnlocked();
      const cur = records.get(key(year, month));
      cur?.attachments.forEach((a) => files.delete(a.id));
      records.delete(key(year, month));
      void runSync();
    },
    addAttachments,
    async saveAttachmentAs() {
      requireUnlocked();
      return '/mock/export/piece-jointe';
    },
    async removeAttachment(year, month, id) {
      requireUnlocked();
      const cur = records.get(key(year, month));
      if (!cur || !cur.attachments.some((a) => a.id === id)) throw new AppError('invalid', 'Pièce jointe introuvable dans ce mois.');
      files.delete(id);
      void runSync();
      return touchRecord({ ...cur, attachments: cur.attachments.filter((a) => a.id !== id) });
    },
    async attachmentBytes(year, month, id) {
      requireUnlocked();
      const att = records.get(key(year, month))?.attachments.find((a) => a.id === id);
      if (!att) throw new AppError('invalid', 'Pièce jointe introuvable dans ce mois.');
      const bytes = files.get(id) ?? new Uint8Array();
      return bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength) as ArrayBuffer;
    },
    async openAttachment() {
      requireUnlocked();
    },
    async syncNow() {
      void runSync();
    },
    async setToken(token) {
      if (!token.trim()) throw new AppError('invalid', "Le jeton d'accès GitHub est requis.");
      void runSync();
    },
    async setAutoLock(minutes) {
      autoLockMinutes = minutes;
    },
    async changePassphrase(oldP, newP) {
      requireUnlocked();
      if (oldP !== passphrase) throw new AppError('wrong_passphrase', 'Phrase secrète incorrecte.');
      if (newP.length < MIN_PASSPHRASE) throw new AppError('invalid', `La phrase secrète doit contenir au moins ${MIN_PASSPHRASE} caractères.`);
      passphrase = newP;
    },
    async resetLocal(force) {
      if (!force && sync.pending) {
        throw new AppError('unpushed', "Des modifications n'ont pas encore été envoyées au dépôt : elles seraient perdues.");
      }
      phase = 'not_configured';
      remoteUrl = null;
      records = new Map();
      lockListeners.forEach((l) => l('reset'));
      return status();
    },

    async pickAttachments(year, month) {
      const files = await new Promise<File[]>((resolve) => {
        const input = document.createElement('input');
        input.type = 'file';
        input.multiple = true;
        input.onchange = () => resolve(Array.from(input.files ?? []));
        input.oncancel = () => resolve([]);
        input.click();
      });
      if (files.length === 0) return null;
      const paths = files.map((f) => {
        const p = `/mock/${rid().slice(0, 6)}/${f.name}`;
        picked.set(p, f);
        return p;
      });
      return addAttachments(year, month, paths);
    },
    async onSyncState(cb) {
      syncListeners.add(cb);
      return () => syncListeners.delete(cb);
    },
    async onLocked(cb) {
      lockListeners.add(cb);
      return () => lockListeners.delete(cb);
    },
    async onRecordsChanged(cb) {
      changedListeners.add(cb);
      return () => changedListeners.delete(cb);
    },
    async onFilesDropped() {
      return () => {};
    },
    async checkForUpdate(): Promise<UpdateInfo | null> {
      await delay(200);
      if (q.get('update') !== '1') return null;
      return {
        version: '9.9.9',
        notes: 'Correctifs et améliorations.',
        async install(onProgress) {
          if (q.get('updateFail') === '1') {
            await delay(150);
            throw new AppError('other', 'signature de la mise à jour invalide');
          }
          for (let i = 1; i <= 5; i++) {
            await delay(80);
            onProgress(i * 200, 1000);
          }
        },
      };
    },
  };
}

// Permet aux tests e2e de basculer l'état hors ligne.
declare global {
  interface Window {
    __mockOffline?: (v: boolean) => void;
  }
}
