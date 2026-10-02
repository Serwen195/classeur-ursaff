import { describe, expect, it } from 'vitest';
import { formatEuros, parseEuros, toInputEuros } from './money';

const cents = (s: string) => {
  const r = parseEuros(s);
  if (!r.ok) throw new Error(`refusé : ${s} (${r.error})`);
  return r.cents;
};

describe('parseEuros', () => {
  it('lit les formats français courants', () => {
    expect(cents('1234')).toBe(123400);
    expect(cents('1234,56')).toBe(123456);
    expect(cents('1 234,56')).toBe(123456);
    expect(cents('1 234,56')).toBe(123456);
    expect(cents('1 234,56 €')).toBe(123456);
    expect(cents('1.234,56')).toBe(123456);
    expect(cents('1.234.567,8')).toBe(123456780);
    expect(cents('12 €')).toBe(1200);
    expect(cents('12 EUR')).toBe(1200);
    expect(cents(',5')).toBe(50);
    expect(cents('0,05')).toBe(5);
  });

  it('lit le point décimal anglo-saxon quand il est sans ambiguïté', () => {
    expect(cents('1234.5')).toBe(123450);
    expect(cents('1234.56')).toBe(123456);
    expect(cents('0.99')).toBe(99);
  });

  it('traite « 1.234 » et « 12.345.678 » comme des milliers, pas des décimales', () => {
    expect(cents('1.234')).toBe(123400);
    expect(cents('12.345.678')).toBe(1234567800);
  });

  it('champ vide = aucun montant', () => {
    expect(cents('')).toBeNull();
    expect(cents('   ')).toBeNull();
  });

  it('refuse plutôt que d\'arrondir ou de deviner', () => {
    for (const bad of ['12,345', '1,2,3', 'abc', '12a', '-5', '+5', '1..2', '1.2.3', '12.3456', '1e5', '1 000 000 000 000']) {
      expect(parseEuros(bad).ok, bad).toBe(false);
    }
  });

  it('ne perd pas de centime sur les valeurs piégeuses en flottant', () => {
    expect(cents('0,1')).toBe(10);
    expect(cents('0,29')).toBe(29); // 0.29 * 100 = 28.999… en flottant
    expect(cents('1,15')).toBe(115); // 1.15 * 100 = 114.99999… en flottant
    expect(cents('4,35')).toBe(435);
    expect(cents('8,2')).toBe(820);
  });
});

describe('formatage', () => {
  it('formate en euros à la française', () => {
    expect(formatEuros(123456).replace(/[\s  ]/g, ' ')).toBe('1 234,56 €');
    expect(formatEuros(0).replace(/[\s  ]/g, ' ')).toBe('0,00 €');
    expect(formatEuros(null)).toBe('—');
  });

  it('toInputEuros est l\'inverse de parseEuros', () => {
    for (const c of [0, 1, 5, 10, 99, 100, 123456, 100000000]) {
      expect(cents(toInputEuros(c))).toBe(c);
    }
    expect(toInputEuros(null)).toBe('');
    expect(toInputEuros(123450)).toBe('1234,50');
    expect(toInputEuros(123400)).toBe('1234');
  });
});
