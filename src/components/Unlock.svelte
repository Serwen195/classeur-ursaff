<script lang="ts">
  import { app } from '../lib/store.svelte';
  import SyncBadge from './SyncBadge.svelte';

  let pass = $state('');
  let show = $state(false);
  let error = $state<string | null>(null);
  let busy = $state(false);
  let input: HTMLInputElement;

  $effect(() => {
    input?.focus();
  });

  const repoName = $derived(app.remoteUrl?.replace(/^https:\/\//, '') ?? '');

  async function submit(e: Event) {
    e.preventDefault();
    if (!pass || busy) return;
    busy = true;
    error = null;
    const err = await app.unlock(pass);
    busy = false;
    if (err) {
      error = err;
      pass = '';
      input?.focus();
    } else {
      pass = '';
    }
  }

  async function forget() {
    const ok = await app.ask({
      title: 'Oublier ce poste ?',
      message:
        "La copie locale du classeur et le jeton enregistré dans le trousseau seront supprimés de cet ordinateur. Le dépôt distant n'est pas modifié : vos données restent disponibles et vous pourrez reconnecter ce poste.",
      confirmLabel: 'Oublier ce poste',
      danger: true,
    });
    if (ok) await app.resetLocal();
  }
</script>

<main class="wrap">
  <form class="card panel" onsubmit={submit}>
    <img src="/icon.png" alt="" width="72" height="72" />
    <h1>Classeur URSSAF</h1>
    <p class="muted repo">{repoName}</p>
    {#if app.lockReason}<p class="note" role="status">{app.lockReason}</p>{/if}

    <div class="field">
      <label for="pass">Phrase secrète</label>
      <div class="row">
        <input id="pass" bind:this={input} type={show ? 'text' : 'password'} bind:value={pass} autocomplete="current-password" spellcheck="false" aria-invalid={error ? 'true' : undefined} aria-describedby={error ? 'unlock-error' : undefined} />
        <button type="button" class="btn" onclick={() => (show = !show)} aria-pressed={show}>{show ? 'Masquer' : 'Afficher'}</button>
      </div>
      {#if error}<span id="unlock-error" class="error-text" role="alert">{error}</span>{/if}
    </div>
    <button class="btn primary full" type="submit" disabled={!pass || busy}>{busy ? 'Déverrouillage…' : 'Déverrouiller'}</button>

    <div class="foot">
      <SyncBadge />
      <button type="button" class="btn ghost small" onclick={forget}>Oublier ce poste</button>
    </div>
  </form>
</main>

<style>
  .wrap { min-height: 100vh; display: grid; place-items: center; padding: 1.5rem; }
  .panel { width: min(420px, 100%); padding: 2rem; text-align: center; }
  .panel h1 { margin: 0.6rem 0 0.1rem; }
  .repo { word-break: break-all; font-size: 0.85rem; margin-bottom: 1.2rem; }
  .field { text-align: left; }
  .row { display: flex; gap: 0.5rem; }
  .full { width: 100%; }
  .note { background: var(--info-soft); color: var(--info); padding: 0.5rem 0.8rem; border-radius: 8px; font-size: 0.9rem; }
  .foot { margin-top: 1.4rem; display: flex; align-items: center; justify-content: space-between; gap: 0.5rem; flex-wrap: wrap; }
</style>
