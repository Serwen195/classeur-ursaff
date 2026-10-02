import { describe, expect, it } from 'vitest';
import { normalize, searchRecords, snippet } from './search';
import type { MonthRecord } from './types';

const base = { declaredOn: null, attachments: [], updatedAt: 1, notes: '' };
const records: MonthRecord[] = [
  { ...base, year: 2025, month: 2, amountCents: 123456, status: 'declared', declaredOn: '2025-03-05', notes: 'Déclaré avec retard, pénalité évitée' },
  { ...base, year: 2025, month: 3, amountCents: 50000, status: 'paid', attachments: [{ id: 'a'.repeat(32), name: 'Accusé réception mars.pdf', mime: 'application/pdf', size: 10, addedAt: 1 }] },
  { ...base, year: 2024, month: 12, amountCents: null, status: 'to_declare' },
];

const keys = (rs: MonthRecord[]) => rs.map((r) => `${r.year}-${r.month}`);

describe('searchRecords', () => {
  it('ignore la casse et les accents', () => {
    expect(keys(searchRecords(records, 'FEVRIER'))).toEqual(['2025-2']);
    expect(keys(searchRecords(records, 'penalite'))).toEqual(['2025-2']);
    expect(keys(searchRecords(records, 'accuse reception'))).toEqual(['2025-3']);
  });

  it('combine les mots en ET', () => {
    expect(keys(searchRecords(records, '2025 payé'))).toEqual(['2025-3']);
    expect(keys(searchRecords(records, '2025'))).toEqual(['2025-3', '2025-2']);
    expect(searchRecords(records, 'mars février')).toEqual([]);
  });

  it('trouve par montant, date et statut', () => {
    expect(keys(searchRecords(records, '1234,56'))).toEqual(['2025-2']);
    expect(keys(searchRecords(records, '1234.56'))).toEqual(['2025-2']);
    expect(keys(searchRecords(records, '05/03/2025'))).toEqual(['2025-2']);
    expect(keys(searchRecords(records, '2025-03-05'))).toEqual(['2025-2']);
    expect(keys(searchRecords(records, 'à déclarer'))).toEqual(['2024-12']);
    expect(keys(searchRecords(records, '2025-03'))).toEqual(['2025-3', '2025-2']); // « 2025-03-05 » contient aussi 2025-03
  });

  it('requête vide = aucun résultat', () => {
    expect(searchRecords(records, '')).toEqual([]);
    expect(searchRecords(records, '   ')).toEqual([]);
  });
});

describe('helpers', () => {
  it('normalize', () => {
    expect(normalize('  Été  Déjà Là ')).toBe('ete deja la');
  });

  it('snippet reste court et centré sur la recherche', () => {
    const long = `${'a '.repeat(60)}cible${' b'.repeat(60)}`;
    const s = snippet(long, 'cible');
    expect(s.length).toBeLessThan(120);
    expect(s).toContain('cible');
    expect(s.startsWith('…') && s.endsWith('…')).toBe(true);
    expect(snippet('court', 'x')).toBe('court');
  });
});
