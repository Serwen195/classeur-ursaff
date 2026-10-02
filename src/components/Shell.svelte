<script lang="ts">
  import { app, conflictLabels } from '../lib/store.svelte';
  import MonthEditor from './MonthEditor.svelte';
  import SearchResults from './SearchResults.svelte';
  import Settings from './Settings.svelte';
  import SyncBadge from './SyncBadge.svelte';
  import YearView from './YearView.svelte';

  let search: HTMLInputElement;

  function onKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'f') {
      e.preventDefault();
      search?.focus();
      search?.select();
    }
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="shell">
  <header>
    <div class="brand"><img src="/icon.png" alt="" width="30" height="30" /><strong>Classeur URSSAF</strong></div>
    <div class="searchwrap">
      <label class="sr-only" for="search">Rechercher</label>
      <input
        id="search"
        bind:this={search}
        type="search"
        placeholder="Rechercher (mois, montant, note, pièce jointe…)"
        bind:value={app.query}
        onkeydown={(e) => {
          if (e.key === 'Escape') app.query = '';
        }}
        autocomplete="off"
        spellcheck="false"
      />
    </div>
    <SyncBadge onclick={() => (app.settingsOpen = true)} />
    <button class="btn" onclick={() => (app.settingsOpen = true)}>⚙ Réglages</button>
    <button class="btn primary" onclick={() => app.lock()}>🔒 Verrouiller</button>
  </header>

  {#if app.sync.conflicts.length > 0 && !app.conflictsDismissed}
    <div class="banner warn" role="status">
      <span>
        Deux appareils avaient modifié le même mois ({conflictLabels(app.sync)}) : la version la plus récente a été conservée.
        L'ancienne reste dans l'historique du dépôt.
      </span>
      <button class="btn small" onclick={() => (app.conflictsDismissed = true)}>Compris</button>
    </div>
  {/if}
  {#if app.skipped.length > 0}
    <div class="banner error" role="alert">
      {app.skipped.length} fichier{app.skipped.length > 1 ? 's' : ''} du classeur {app.skipped.length > 1 ? "n'ont" : "n'a"} pas pu être déchiffré{app.skipped.length > 1 ? 's' : ''} (altéré{app.skipped.length > 1 ? 's' : ''} ou chiffré{app.skipped.length > 1 ? 's' : ''} avec une autre phrase). Ils ne sont pas affichés.
    </div>
  {/if}

  <main>
    {#if app.query.trim() !== ''}
      <SearchResults />
    {:else}
      <YearView />
    {/if}
  </main>
</div>

{#if app.editing}
  {#key `${app.editing.year}-${app.editing.month}`}
    <MonthEditor year={app.editing.year} month={app.editing.month} />
  {/key}
{/if}
{#if app.settingsOpen}<Settings />{/if}

<style>
  header {
    position: sticky; top: 0; z-index: 10; display: flex; gap: 0.7rem; align-items: center; flex-wrap: wrap;
    padding: 0.7rem 1.2rem; background: var(--surface); border-bottom: 1px solid var(--border);
  }
  .brand { display: flex; gap: 0.5rem; align-items: center; margin-right: 0.4rem; }
  .searchwrap { flex: 1; min-width: 220px; }
  main { max-width: 1060px; margin: 0 auto; padding: 1.2rem; }
</style>
