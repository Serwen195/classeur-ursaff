<script lang="ts">
  import { app } from '../lib/store.svelte';
  import { MAX_YEAR, MIN_YEAR, STATUS_LABELS, capitalize, formatDate, monthName } from '../lib/months';
  import { formatEuros } from '../lib/money';
  import { knownYears, yearTotals } from '../lib/totals';

  const currentYear = new Date().getFullYear();
  const currentMonth = new Date().getMonth() + 1;

  const years = $derived(knownYears(app.records, currentYear, [app.year]));
  const totals = $derived(yearTotals(app.records, app.year));
  const rows = $derived(
    Array.from({ length: 12 }, (_, i) => ({ month: i + 1, rec: app.recordOf(app.year, i + 1) })),
  );
  const isFuture = (m: number) => app.year > currentYear || (app.year === currentYear && m > currentMonth);

  function firstLine(s: string): string {
    return s.split('\n')[0] ?? '';
  }
</script>

<section aria-labelledby="year-title">
  <div class="top">
    <div class="yearnav">
      <button class="btn" onclick={() => app.setYear(app.year - 1)} disabled={app.year <= MIN_YEAR} aria-label="Année précédente">◀</button>
      <label class="sr-only" for="year-select">Année</label>
      <select id="year-select" value={app.year} onchange={(e) => app.setYear(Number(e.currentTarget.value))}>
        {#each years as y (y)}<option value={y}>{y}</option>{/each}
      </select>
      <button class="btn" onclick={() => app.setYear(app.year + 1)} disabled={app.year >= MAX_YEAR} aria-label="Année suivante">▶</button>
      {#if app.year !== currentYear}<button class="btn ghost small" onclick={() => app.setYear(currentYear)}>Année en cours</button>{/if}
    </div>
    <h1 id="year-title" class="sr-only">Déclarations {app.year}</h1>

    <div class="totals card" aria-label="Totaux {app.year}">
      <div>
        <div class="label">CA déclaré en {app.year}</div>
        <div class="big" data-testid="total-declared">{formatEuros(totals.declaredCents)}</div>
      </div>
      <div class="side">
        <div>{totals.declaredMonths} mois déclaré{totals.declaredMonths > 1 ? 's' : ''} sur 12 · {totals.paidMonths} payé{totals.paidMonths > 1 ? 's' : ''}</div>
        {#if totals.notYetDeclaredCents > 0}
          <div class="warn" data-testid="total-pending">+ {formatEuros(totals.notYetDeclaredCents)} saisi, pas encore déclaré (non inclus)</div>
        {/if}
      </div>
    </div>
  </div>

  <div class="card tablewrap">
    <table>
      <caption class="sr-only">Les douze mois de {app.year}. Sélectionnez un mois pour le modifier.</caption>
      <thead>
        <tr><th>Mois</th><th class="num">CA déclaré</th><th>Déclaré le</th><th>Statut</th><th>PJ</th><th>Notes</th></tr>
      </thead>
      <tbody>
        {#each rows as { month, rec } (month)}
          <tr class:future={isFuture(month) && !rec} onclick={() => app.openMonth(app.year, month)}>
            <th scope="row">
              <button class="monthbtn" onclick={(e) => { e.stopPropagation(); app.openMonth(app.year, month); }} aria-label="Modifier {monthName(month)} {app.year}">{capitalize(monthName(month))}</button>
            </th>
            <td class="num">{rec ? formatEuros(rec.amountCents) : '—'}</td>
            <td>{rec?.declaredOn ? formatDate(rec.declaredOn) : '—'}</td>
            <td>
              {#if rec}<span class="pill {rec.status}">{STATUS_LABELS[rec.status]}</span>{:else}<span class="pill empty">Vide</span>{/if}
            </td>
            <td>{#if rec && rec.attachments.length > 0}<span title="{rec.attachments.length} pièce(s) jointe(s)">📎 {rec.attachments.length}</span>{:else}<span class="muted">—</span>{/if}</td>
            <td class="notes" title={rec?.notes ?? ''}>{rec ? firstLine(rec.notes) : ''}</td>
          </tr>
        {/each}
      </tbody>
      <tfoot>
        <tr><th scope="row">Total {app.year}</th><td class="num strong">{formatEuros(totals.declaredCents)}</td><td colspan="4" class="muted small">Somme des mois « Déclaré » et « Payé »</td></tr>
      </tfoot>
    </table>
  </div>
</section>

<style>
  .top { display: flex; gap: 1rem; align-items: stretch; justify-content: space-between; flex-wrap: wrap; margin-bottom: 1rem; }
  .yearnav { display: flex; gap: 0.4rem; align-items: center; align-self: flex-start; }
  .yearnav select { width: auto; font-size: 1.3rem; font-weight: 700; padding: 0.3rem 0.6rem; }
  .totals { display: flex; gap: 1.5rem; align-items: center; padding: 0.8rem 1.2rem; flex: 1; justify-content: space-between; min-width: 320px; }
  .label { color: var(--muted); font-size: 0.85rem; }
  .big { font-size: 1.7rem; font-weight: 700; font-variant-numeric: tabular-nums; }
  .side { text-align: right; font-size: 0.88rem; color: var(--muted); }
  .side .warn { color: var(--warn); }
  .tablewrap { overflow: hidden; }
  table { width: 100%; border-collapse: collapse; }
  th, td { padding: 0.6rem 0.9rem; text-align: left; border-bottom: 1px solid var(--border); vertical-align: middle; }
  thead th { background: var(--surface-2); font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.04em; color: var(--muted); }
  tbody tr { cursor: pointer; }
  tbody tr:hover { background: var(--primary-soft); }
  tbody tr.future { opacity: 0.6; }
  tbody th { font-weight: 600; }
  tfoot th, tfoot td { border-bottom: none; background: var(--surface-2); }
  .num { text-align: right; font-variant-numeric: tabular-nums; white-space: nowrap; }
  .strong { font-weight: 700; }
  .notes { max-width: 260px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--muted); }
  .monthbtn { all: unset; cursor: pointer; font-weight: 600; padding: 0.1rem 0.2rem; border-radius: 4px; }
  .monthbtn:focus-visible { box-shadow: var(--focus); }
</style>
