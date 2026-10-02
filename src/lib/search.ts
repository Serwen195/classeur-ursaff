import { formatDate, monthName, STATUS_LABELS } from './months';
import { formatEuros, toInputEuros } from './money';
import type { MonthRecord } from './types';

/** Minuscules, sans accents, espaces normalisés : « Février » ~ « fevrier ». */
export function normalize(s: string): string {
  return s
    .normalize('NFD')
    .replace(/\p{Diacritic}/gu, '')
    .toLowerCase()
    .replace(/[\s  ]+/g, ' ')
    .trim();
}

function haystack(r: MonthRecord): string {
  const mm = String(r.month).padStart(2, '0');
  const parts = [
    monthName(r.month),
    String(r.year),
    `${r.year}-${mm}`,
    `${mm}/${r.year}`,
    STATUS_LABELS[r.status],
    r.notes,
    r.declaredOn ?? '',
    formatDate(r.declaredOn),
    r.amountCents === null ? '' : `${toInputEuros(r.amountCents)} ${formatEuros(r.amountCents)} ${(r.amountCents / 100).toFixed(2)}`,
    ...r.attachments.map((a) => a.name),
  ];
  return normalize(parts.join(' \n '));
}

/** Tous les mots saisis doivent apparaître (ET). Résultats du plus récent au plus ancien. */
export function searchRecords(records: MonthRecord[], query: string): MonthRecord[] {
  const terms = normalize(query).split(' ').filter(Boolean);
  if (terms.length === 0) return [];
  return records
    .filter((r) => {
      const h = haystack(r);
      return terms.every((t) => h.includes(t));
    })
    .sort((a, b) => b.year - a.year || b.month - a.month);
}

/** Extrait de texte autour de la première occurrence, pour l'afficher dans les résultats. */
export function snippet(text: string, query: string, radius = 40): string {
  const flat = text.replace(/\s+/g, ' ').trim();
  if (flat.length <= radius * 2) return flat;
  const nt = normalize(flat);
  const first = normalize(query).split(' ').filter(Boolean).find((t) => nt.includes(t));
  const at = first ? nt.indexOf(first) : 0;
  const start = Math.max(0, at - radius);
  const end = Math.min(flat.length, at + radius);
  return `${start > 0 ? '…' : ''}${flat.slice(start, end)}${end < flat.length ? '…' : ''}`;
}
