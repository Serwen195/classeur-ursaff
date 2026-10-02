// Les montants sont manipulés en centimes entiers : jamais de flottants dans les additions.

export type ParsedAmount = { ok: true; cents: number | null } | { ok: false; error: string };

const MAX_EUROS = 1_000_000_000_000;

/**
 * Lit un montant saisi à la française : « 1234 », « 1 234,56 », « 1.234,56 », « 1234.5 », « 12 € ».
 * Champ vide → `cents: null`. Plus de deux décimales → refus (on n'arrondit jamais en silence).
 */
export function parseEuros(input: string): ParsedAmount {
  let s = input
    .replace(/[\s  ]/g, '')
    .replace(/€|eur(os?)?/gi, '');
  if (s === '') return { ok: true, cents: null };
  if (s.startsWith('-') || s.startsWith('+')) {
    return { ok: false, error: 'Le montant ne peut pas être négatif.' };
  }
  if (!/^[0-9.,]+$/.test(s)) return { ok: false, error: 'Montant invalide : utilisez des chiffres, par exemple 1 234,56.' };

  const lastComma = s.lastIndexOf(',');
  const lastDot = s.lastIndexOf('.');
  let intPart: string;
  let decPart = '';
  if (lastComma >= 0) {
    // La virgule est le séparateur décimal ; les points qui la précèdent séparent les milliers.
    intPart = s.slice(0, lastComma);
    decPart = s.slice(lastComma + 1);
    if (/,/.test(intPart)) return { ok: false, error: 'Montant invalide : une seule virgule est permise.' };
    intPart = intPart.replace(/\./g, '');
  } else if (lastDot >= 0) {
    const after = s.slice(lastDot + 1);
    const dotCount = s.split('.').length - 1;
    if (dotCount === 1 && after.length >= 1 && after.length <= 2) {
      intPart = s.slice(0, lastDot);
      decPart = after;
    } else if (/^\d{1,3}(\.\d{3})+$/.test(s)) {
      intPart = s.replace(/\./g, ''); // 1.234.567 → séparateurs de milliers
    } else {
      return { ok: false, error: 'Montant invalide.' };
    }
  } else {
    intPart = s;
  }
  if (intPart === '') intPart = '0';
  if (!/^\d+$/.test(intPart) || !/^\d*$/.test(decPart)) return { ok: false, error: 'Montant invalide.' };
  if (decPart.length > 2) {
    return { ok: false, error: 'Deux décimales au maximum (centimes).' };
  }
  const euros = Number(intPart);
  if (!Number.isSafeInteger(euros) || euros >= MAX_EUROS) return { ok: false, error: 'Montant trop élevé.' };
  const cents = euros * 100 + Number(decPart.padEnd(2, '0') || '0');
  return { ok: true, cents };
}

const euroFormat = new Intl.NumberFormat('fr-FR', { style: 'currency', currency: 'EUR' });

/** Affichage : « 1 234,56 € ». */
export function formatEuros(cents: number | null): string {
  if (cents === null) return '—';
  return euroFormat.format(cents / 100);
}

/** Valeur à remettre dans un champ de saisie : « 1234,56 » (sans séparateur de milliers). */
export function toInputEuros(cents: number | null): string {
  if (cents === null) return '';
  const euros = Math.trunc(cents / 100);
  const rest = cents % 100;
  return rest === 0 ? String(euros) : `${euros},${String(rest).padStart(2, '0')}`;
}
