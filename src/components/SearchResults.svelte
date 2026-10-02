<script lang="ts">
  import { app } from '../lib/store.svelte';
  import { STATUS_LABELS, formatDate, monthLabel } from '../lib/months';
  import { formatEuros } from '../lib/money';
  import { searchRecords, snippet } from '../lib/search';
  import type { MonthRecord } from '../lib/types';

  const results = $derived(searchRecords(app.records, app.query));

  function detail(r: MonthRecord): string {
    const files = r.attachments.map((a) => a.name).join(', ');
    return snippet([r.notes, files].filter(Boolean).join(' · '), app.query);
  }

  function open(r: MonthRecord) {
    app.query = '';
    app.openMonth(r.year, r.month);
  }
</script>

<section aria-labelledby="search-title">
  <h1 id="search-title" class="title">
    {results.length === 0 ? 'Aucun résultat' : `${results.length} résultat${results.length > 1 ? 's' : ''}`}
    <span class="muted">pour « {app.query.trim()} »</span>
  </h1>
  {#if results.length === 0}
    <p class="muted">Essayez un mois (« mars »), une année, un montant, un statut, un mot des notes ou le nom d'une pièce jointe.</p>
  {:else}
    <ul class="card">
      {#each results as r (`${r.year}-${r.month}`)}
        <li>
          <button class="row" onclick={() => open(r)}>
            <span class="when">{monthLabel(r.year, r.month)}</span>
            <span class="amount">{formatEuros(r.amountCents)}</span>
            <span class="pill {r.status}">{STATUS_LABELS[r.status]}</span>
            <span class="muted small date">{r.declaredOn ? `déclaré le ${formatDate(r.declaredOn)}` : ''}</span>
            <span class="detail muted small">{detail(r)}</span>
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</section>

<style>
  .title { font-size: 1.2rem; }
  .title .muted { font-weight: 400; }
  ul { list-style: none; margin: 0; padding: 0; overflow: hidden; }
  li + li { border-top: 1px solid var(--border); }
  .row {
    all: unset; box-sizing: border-box; width: 100%; cursor: pointer; padding: 0.7rem 1rem;
    display: grid; grid-template-columns: 11rem 8rem 7rem 11rem 1fr; gap: 0.8rem; align-items: center;
  }
  .row:hover, .row:focus-visible { background: var(--primary-soft); }
  .when { font-weight: 600; }
  .amount { text-align: right; font-variant-numeric: tabular-nums; }
  .detail { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
