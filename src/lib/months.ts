import type { Status } from './types';

export const MONTHS_FR = [
  'janvier',
  'février',
  'mars',
  'avril',
  'mai',
  'juin',
  'juillet',
  'août',
  'septembre',
  'octobre',
  'novembre',
  'décembre',
] as const;

export const STATUS_LABELS: Record<Status, string> = {
  to_declare: 'À déclarer',
  declared: 'Déclaré',
  paid: 'Payé',
};

export const STATUSES: Status[] = ['to_declare', 'declared', 'paid'];

export const MIN_YEAR = 2000;
export const MAX_YEAR = 2100;

export function monthName(month: number): string {
  return MONTHS_FR[month - 1] ?? '';
}

export function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/** « Mars 2025 » */
export function monthLabel(year: number, month: number): string {
  return `${capitalize(monthName(month))} ${year}`;
}

export function clampYear(y: number): number {
  return Math.min(MAX_YEAR, Math.max(MIN_YEAR, Math.trunc(y)));
}

/** Date du jour au format AAAA-MM-JJ, dans le fuseau local. */
export function todayIso(now: Date = new Date()): string {
  const p = (n: number) => String(n).padStart(2, '0');
  return `${now.getFullYear()}-${p(now.getMonth() + 1)}-${p(now.getDate())}`;
}

/** « 2025-03-05 » → « 05/03/2025 ». Renvoie la valeur d'origine si elle n'est pas une date ISO. */
export function formatDate(iso: string | null): string {
  if (!iso) return '';
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(iso);
  return m ? `${m[3]}/${m[2]}/${m[1]}` : iso;
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} o`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(n < 10 * 1024 ? 1 : 0).replace('.', ',')} Ko`;
  return `${(n / 1024 / 1024).toFixed(1).replace('.', ',')} Mo`;
}
