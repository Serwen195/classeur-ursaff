import type { MonthRecord } from './types';

export interface YearTotals {
  /** Somme des montants des mois « Déclaré » ou « Payé ». Simple addition, aucun calcul de cotisations. */
  declaredCents: number;
  /** Montants saisis pour des mois encore « À déclarer » (non inclus dans le total déclaré). */
  notYetDeclaredCents: number;
  declaredMonths: number;
  paidMonths: number;
}

export function yearTotals(records: MonthRecord[], year: number): YearTotals {
  const t: YearTotals = { declaredCents: 0, notYetDeclaredCents: 0, declaredMonths: 0, paidMonths: 0 };
  for (const r of records) {
    if (r.year !== year) continue;
    if (r.status === 'to_declare') {
      t.notYetDeclaredCents += r.amountCents ?? 0;
      continue;
    }
    t.declaredCents += r.amountCents ?? 0;
    t.declaredMonths += 1;
    if (r.status === 'paid') t.paidMonths += 1;
  }
  return t;
}

/** Années présentes dans les données, plus l'année en cours, triées. */
export function knownYears(records: MonthRecord[], currentYear: number, extra: number[] = []): number[] {
  const set = new Set<number>([currentYear, ...extra]);
  for (const r of records) set.add(r.year);
  return [...set].sort((a, b) => a - b);
}

export function recordFor(records: MonthRecord[], year: number, month: number): MonthRecord | undefined {
  return records.find((r) => r.year === year && r.month === month);
}

/** Remplace ou insère un enregistrement, en gardant la liste triée. */
export function upsertRecord(records: MonthRecord[], rec: MonthRecord): MonthRecord[] {
  const rest = records.filter((r) => !(r.year === rec.year && r.month === rec.month));
  rest.push(rec);
  return rest.sort((a, b) => a.year - b.year || a.month - b.month);
}
