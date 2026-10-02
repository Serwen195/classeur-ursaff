<script lang="ts">
  import { onMount } from 'svelte';
  import ConfirmDialog from './components/ConfirmDialog.svelte';
  import NoticeBar from './components/NoticeBar.svelte';
  import Setup from './components/Setup.svelte';
  import Shell from './components/Shell.svelte';
  import Unlock from './components/Unlock.svelte';
  import UpdateBanner from './components/UpdateBanner.svelte';
  import { app } from './lib/store.svelte';

  onMount(() => {
    void app.init();
  });
</script>

<UpdateBanner />

{#if app.phase === 'loading'}
  <div class="center" role="status">Chargement…</div>
{:else if app.phase === 'fatal'}
  <div class="center" role="alert">
    <h1>Classeur URSSAF</h1>
    <p>{app.fatal}</p>
  </div>
{:else if app.phase === 'not_configured' || app.phase === 'needs_vault_creation'}
  <Setup />
{:else if app.phase === 'locked'}
  <Unlock />
{:else}
  <Shell />
{/if}

<NoticeBar />
<ConfirmDialog />

<style>
  .center { min-height: 100vh; display: grid; place-content: center; text-align: center; padding: 2rem; color: var(--muted); }
</style>
