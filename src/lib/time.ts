/** « à l'instant », « il y a 5 min », « il y a 3 h », « le 05/03/2025 ». */
export function relativeTime(ts: number | null, now: number = Date.now()): string {
  if (ts === null) return 'jamais';
  const s = Math.max(0, Math.round((now - ts) / 1000));
  if (s < 45) return "à l'instant";
  const min = Math.round(s / 60);
  if (min < 60) return `il y a ${min} min`;
  const h = Math.round(min / 60);
  if (h < 24) return `il y a ${h} h`;
  const d = new Date(ts);
  const p = (n: number) => String(n).padStart(2, '0');
  return `le ${p(d.getDate())}/${p(d.getMonth() + 1)}/${d.getFullYear()}`;
}
