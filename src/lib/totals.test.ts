import { describe, expect, it } from 'vitest';
import { knownYears, recordFor, upsertRecord, yearTotals } from './totals';
import type { MonthRecord, Status } from './types';

function rec(year: number, month: number, amountCents: number | null, status: Status): MonthRecord {
  return { year, month, amountCents, declaredOn: null, status, notes: '', attachments: [], updatedAt: 1 };
}

describe('yearTotals', () => {
  it('additionne les mois déclarés et payés, sans cotisations', () => {
    const rs = [
      rec(2025, 1, 100_010, 'paid'),
      rec(2025, 2, 200_020, 'declared'),
      rec(2025, 3, 300_030, 'to_declare'),
      rec(2024, 12, 999_999, 'paid'),
    ];
    expect(yearTotals(rs, 2025)).toEqual({
      declaredCents: 300_030,
      notYetDeclaredCents: 300_030,
      declaredMonths: 2,
      paidMonths: 1,
    });
    expect(yearTotals(rs, 2024).declaredCents).toBe(999_999);
  });

  it('est exact au centime : 0,10 € × 3 = 0,30 €', () => {
    const rs = [rec(2025, 1, 10, 'declared'), rec(2025, 2, 10, 'declared'), rec(2025, 3, 10, 'declared')];
    expect(yearTotals(rs, 2025).declaredCents).toBe(30);
  });

  it('gère les mois sans montant et les années vides', () => {
    expect(yearTotals([rec(2025, 1, null, 'declared')], 2025)).toEqual({
      declaredCents: 0,
      notYetDeclaredCents: 0,
      declaredMonths: 1,
      paidMonths: 0,
    });
    expect(yearTotals([], 2025).declaredCents).toBe(0);
  });
});

describe('listes', () => {
  it('knownYears inclut l\'année en cours et reste trié', () => {
    expect(knownYears([rec(2023, 1, 1, 'paid'), rec(2025, 1, 1, 'paid')], 2026)).toEqual([2023, 2025, 2026]);
    expect(knownYears([], 2026, [2030])).toEqual([2026, 2030]);
  });

  it('upsertRecord remplace sans doublon et trie', () => {
    let rs = [rec(2025, 3, 1, 'paid'), rec(2025, 1, 1, 'paid')];
    rs = upsertRecord(rs, rec(2025, 2, 5, 'declared'));
    rs = upsertRecord(rs, rec(2025, 3, 9, 'declared'));
    expect(rs.map((r) => [r.month, r.amountCents])).toEqual([[1, 1], [2, 5], [3, 9]]);
    expect(recordFor(rs, 2025, 3)?.amountCents).toBe(9);
    expect(recordFor(rs, 2025, 4)).toBeUndefined();
  });
});
