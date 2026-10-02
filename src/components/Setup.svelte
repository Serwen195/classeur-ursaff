<script lang="ts">
  import { app } from '../lib/store.svelte';

  const MIN_LEN = 12;

  let url = $state('');
  let token = $state('');
  let busy = $state(false);

  let pass = $state('');
  let pass2 = $state('');
  let show = $state(false);
  let understood = $state(false);

  const passLen = $derived([...pass].length);
  const mismatch = $derived(pass2.length > 0 && pass !== pass2);
  const canCreate = $derived(passLen >= MIN_LEN && pass === pass2 && understood && !busy);

  async function connect(e: Event) {
    e.preventDefault();
    busy = true;
    await app.setupRemote(url, token);
    busy = false;
    if (app.phase !== 'not_configured') token = '';
  }

  async function create(e: Event) {
    e.preventDefault();
    if (!canCreate) return;
    busy = true;
    const ok = await app.createVault(pass);
    busy = false;
    if (ok) {
      pass = '';
      pass2 = '';
    }
  }
</script>

<main class="wrap">
  <div class="card panel">
    <div class="brand"><img src="/icon.png" alt="" width="56" height="56" /><div><h1>Classeur URSSAF</h1><p class="muted">Vos déclarations, rangées et chiffrées.</p></div></div>

    {#if app.phase === 'not_configured'}
      <h2>1. Connecter votre dépôt de données</h2>
      <p>
        Vos données sont chiffrées sur ce poste, puis stockées dans un dépôt Git <strong>privé</strong> que vous contrôlez, pour les retrouver sur tous vos appareils.
      </p>
      <details>
        <summary>Comment préparer le dépôt ?</summary>
        <ol>
          <li>Sur GitHub, créez un dépôt <strong>privé et vide</strong> (sans README), par exemple <code>classeur-urssaf-data</code>.</li>
          <li>
            Créez un <em>jeton d'accès à granularité fine</em> (Paramètres → Developer settings → Fine-grained tokens), limité à ce seul dépôt, avec la permission
            <strong>Contents : Read and write</strong>.
          </li>
          <li>Collez l'adresse du dépôt et le jeton ci-dessous. Le jeton est rangé dans le trousseau de votre système, jamais dans un fichier.</li>
        </ol>
      </details>

      <form onsubmit={connect}>
        <div class="field">
          <label for="url">Adresse du dépôt</label>
          <input id="url" type="text" bind:value={url} placeholder="https://github.com/votre-compte/classeur-urssaf-data" autocomplete="off" spellcheck="false" required />
        </div>
        <div class="field">
          <label for="token">Jeton d'accès</label>
          <input id="token" type="password" bind:value={token} placeholder="github_pat_…" autocomplete="off" spellcheck="false" required />
          <span class="hint">Sera stocké dans le trousseau de votre système.</span>
        </div>
        <button class="btn primary" type="submit" disabled={busy || !url.trim() || !token.trim()}>
          {busy ? 'Connexion au dépôt…' : 'Se connecter'}
        </button>
      </form>
    {:else}
      <h2>2. Choisir votre phrase secrète</h2>
      <div class="warning" role="note">
        <strong>Cette phrase chiffre toutes vos données. Elle n'est enregistrée nulle part.</strong>
        Si vous la perdez, vos déclarations seront <strong>définitivement irrécupérables</strong> : personne ne pourra les restaurer.
        Choisissez plusieurs mots (par exemple quatre mots au hasard) et notez-la dans un endroit sûr.
      </div>
      <form onsubmit={create}>
        <div class="field">
          <label for="p1">Phrase secrète</label>
          <input id="p1" type={show ? 'text' : 'password'} bind:value={pass} autocomplete="new-password" spellcheck="false" />
          <span class="hint" class:ok={passLen >= MIN_LEN}>{passLen} / {MIN_LEN} caractères minimum</span>
        </div>
        <div class="field">
          <label for="p2">Confirmer la phrase secrète</label>
          <input id="p2" type={show ? 'text' : 'password'} bind:value={pass2} autocomplete="new-password" spellcheck="false" />
          {#if mismatch}<span class="error-text">Les deux phrases ne correspondent pas.</span>{/if}
        </div>
        <label class="check"><input type="checkbox" bind:checked={show} /> Afficher la phrase</label>
        <label class="check"><input type="checkbox" bind:checked={understood} /> Je comprends que sans cette phrase mes données sont irrécupérables.</label>
        <div class="actions">
          <button class="btn primary" type="submit" disabled={!canCreate}>{busy ? 'Création…' : 'Créer le classeur'}</button>
        </div>
      </form>
    {/if}
  </div>
</main>

<style>
  .wrap { min-height: 100vh; display: grid; place-items: center; padding: 1.5rem; }
  .panel { width: min(640px, 100%); padding: 1.8rem; }
  .brand { display: flex; gap: 1rem; align-items: center; margin-bottom: 1.2rem; }
  .brand h1 { margin: 0; }
  .brand p { margin: 0; }
  details { margin: 0 0 1.2rem; padding: 0.6rem 0.9rem; background: var(--surface-2); border-radius: 8px; }
  summary { cursor: pointer; font-weight: 600; }
  ol { padding-left: 1.2rem; margin: 0.6rem 0 0; }
  li { margin-bottom: 0.4rem; }
  code { background: var(--surface-3); padding: 0.05rem 0.35rem; border-radius: 4px; font-size: 0.9em; }
  .warning { background: var(--warn-soft); color: var(--warn); padding: 0.8rem 1rem; border-radius: 8px; margin-bottom: 1.1rem; }
  .check { display: flex; gap: 0.5rem; align-items: flex-start; margin: 0 0 0.7rem; }
  .check input { margin-top: 0.25rem; }
  .hint.ok { color: var(--ok); }
  .actions { margin-top: 1rem; }
</style>
