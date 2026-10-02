import { describe, expect, it } from 'vitest';
import { relativeTime } from './time';

describe('relativeTime', () => {
  const now = new Date(2025, 5, 15, 12, 0, 0).getTime();
  it('formule en français', () => {
    expect(relativeTime(null, now)).toBe('jamais');
    expect(relativeTime(now - 10_000, now)).toBe("à l'instant");
    expect(relativeTime(now + 5_000, now)).toBe("à l'instant"); // horloge légèrement en avance
    expect(relativeTime(now - 5 * 60_000, now)).toBe('il y a 5 min');
    expect(relativeTime(now - 3 * 3_600_000, now)).toBe('il y a 3 h');
    expect(relativeTime(new Date(2025, 5, 10, 8, 0, 0).getTime(), now)).toBe('le 10/06/2025');
  });
});
