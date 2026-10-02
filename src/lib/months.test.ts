import { describe, expect, it } from 'vitest';
import { clampYear, formatBytes, formatDate, monthLabel, todayIso } from './months';

describe('months', () => {
  it('libellés', () => {
    expect(monthLabel(2025, 3)).toBe('Mars 2025');
    expect(monthLabel(2025, 12)).toBe('Décembre 2025');
    expect(monthLabel(2025, 2)).toBe('Février 2025');
  });

  it('dates', () => {
    expect(formatDate('2025-03-05')).toBe('05/03/2025');
    expect(formatDate(null)).toBe('');
    expect(formatDate('n\'importe quoi')).toBe('n\'importe quoi');
    expect(todayIso(new Date(2025, 0, 9, 23, 59))).toBe('2025-01-09');
  });

  it('années bornées comme côté Rust', () => {
    expect(clampYear(1500)).toBe(2000);
    expect(clampYear(3000)).toBe(2100);
    expect(clampYear(2025.7)).toBe(2025);
  });

  it('tailles', () => {
    expect(formatBytes(512)).toBe('512 o');
    expect(formatBytes(2048)).toBe('2,0 Ko');
    expect(formatBytes(50 * 1024)).toBe('50 Ko');
    expect(formatBytes(3.5 * 1024 * 1024)).toBe('3,5 Mo');
  });
});
