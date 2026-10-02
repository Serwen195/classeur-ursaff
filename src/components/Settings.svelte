<script lang="ts">
  import { app } from '../lib/store.svelte';
  import { relativeTime } from '../lib/time';
  import Modal from './Modal.svelte';

  const AUTO_LOCK_CHOICES = [
    { value: 1, label: '1 minute' },
    { value: 5, label: '5 minutes' },
    { value: 10, label: '10 minutes' },
    { value: 15, label: '15 minutes' },
    { value: 30, label: '30 minutes' },
    { value: 60, label: '1 heure' },
    { value: 0, label: 'Jamais' },
  ];

  let token = $state('');
  let tokenBusy = $state(false);
  let oldP = $state('');
  let newP = $state('');
  let newP2 = $state('');
  let passError = $state<string | null>(null);
  let passBusy = $state(false);

  const passMismatch = $derived(newP2.length > 0 && newP !== newP2);
  const canChange = $derived(oldP.length > 0 && [...newP].length >= 12 && newP === newP2 && !passBusy);

  async function saveToken(e: Event) {
    e.preventDefault();
    tokenBusy = true;
    if (await app.setToken(token)) token = '';
    tokenBusy = false;
  }

  async function changePass(e: Event) {
    e.preventDefault();
    if (!canChange) return;
    passBusy = true;
    passError = await app.changePassphrase(oldP, newP);
    passBusy = false;
    if (!passError) {
      oldP = '';
      newP = '';
      newP2 = '';
    }
  }

  async function forget() {
    const ok = await app.ask({
      title: 'Oublier ce poste ?',
      message:
        "La copie locale du classeur et le jeton du trousseau seront supprimés de cet ordinateur. Le dépôt distant n'est pas modifié.",
      confirmLabel: 'Oublier ce poste',
      danger: true,
    });
    if (ok) await app.resetLocal();
  }

  const updateText = $derived.by(() => {
    const u = app.update;
    if (u.kind === 'checking') return 'Recherche en cours…';
    if (u.kind === 'uptodate') return 'Vous utilisez la dernière version.';
    if (u.kind === 'available') return `La version ${u.version} est disponible.`;
    if (u.kind === 'error') return `Vérification impossible : ${u.message}`;
    return '';
  });
</script>

<Modal title="Réglages" onclose={() => (app.settingsOpen = false)} wide>
  <section>
    <h3>Synchronisation</h3>
    <p class="small muted">Dépôt : <code>{app.remoteUrl}</code></p>
    <p class="small">
      {#if app.sync.status === 'syncing'}Synchronisation en cours…
      {:else}Dernière synchronisation : {relativeTime(app.sync.lastSyncAt)}{app.sync.pending ? ' · des modifications attendent d’être envoyées' : ''}.{/if}
    </p>
    {#if app.sync.message && app.sync.status !== 'idle' && app.sync.status !== 'syncing'}
      <p class="error-text" role="status">{app.sync.message}</p>
    {/if}
    {#if app.sync.ignoredFiles.length > 0}
      <p class="small warn">Fichiers non reconnus ignorés (jamais envoyés) : {app.sync.ignoredFiles.join(', ')}</p>
    {/if}
    <button class="btn" onclick={() => app.syncNow()} disabled={app.sync.status === 'syncing'}>Synchroniser maintenant</button>

    <form class="inline" onsubmit={saveToken}>
      <div class="field">
        <label for="new-token">Remplacer le jeton d'accès GitHub</label>
        <div class="row">
          <input id="new-token" type="password" bind:value={token} placeholder="github_pat_…" autocomplete="off" spellcheck="false" />
          <button class="btn" type="submit" disabled={tokenBusy || !token.trim()}>Enregistrer</button>
        </div>
        <span class="hint">À utiliser si le jeton a expiré ou été révoqué. Il est rangé dans le trousseau du système.</span>
      </div>
    </form>
  </section>

  <section>
    <h3>Sécurité</h3>
    <div class="field">
      <label for="autolock">Verrouillage automatique après inactivité</label>
      <select id="autolock" value={app.autoLockMinutes} onchange={(e) => app.setAutoLock(Number(e.currentTarget.value))}>
        {#each AUTO_LOCK_CHOICES as c (c.value)}<option value={c.value}>{c.label}</option>{/each}
      </select>
    </div>

    <form onsubmit={changePass}>
      <h4>Changer la phrase secrète</h4>
      <div class="field"><label for="old-pass">Phrase actuelle</label><input id="old-pass" type="password" bind:value={oldP} autocomplete="current-password" /></div>
      <div class="field"><label for="new-pass">Nouvelle phrase (12 caractères minimum)</label><input id="new-pass" type="password" bind:value={newP} autocomplete="new-password" /></div>
      <div class="field">
        <label for="new-pass2">Confirmer la nouvelle phrase</label>
        <input id="new-pass2" type="password" bind:value={newP2} autocomplete="new-password" />
        {#if passMismatch}<span class="error-text">Les deux phrases ne correspondent pas.</span>{/if}
      </div>
      {#if passError}<p class="error-text" role="alert">{passError}</p>{/if}
      <button class="btn" type="submit" disabled={!canChange}>Changer la phrase</button>
      <p class="hint muted small">Les données ne sont pas rechiffrées : seule la clé qui les protège est ré-enveloppée. Les autres appareils utiliseront la nouvelle phrase après leur prochaine synchronisation.</p>
    </form>
  </section>

  <section>
    <h3>Application</h3>
    <p class="small">Version {app.version}</p>
    <button class="btn" onclick={() => app.checkUpdate(true)} disabled={app.update.kind === 'checking' || app.update.kind === 'installing'}>Rechercher une mise à jour</button>
    {#if updateText}<p class="small" role="status">{updateText}</p>{/if}
    {#if app.update.kind === 'available'}<button class="btn primary" onclick={() => app.installUpdate()}>Installer et redémarrer</button>{/if}
  </section>

  <section class="danger-zone">
    <h3>Ce poste</h3>
    <p class="small muted">Supprime la copie locale et le jeton de cet ordinateur. Le dépôt distant et vos autres appareils ne sont pas touchés.</p>
    <button class="btn danger" onclick={forget}>Oublier ce poste</button>
  </section>

  {#snippet footer()}
    <button class="btn primary" onclick={() => (app.settingsOpen = false)}>Fermer</button>
  {/snippet}
</Modal>

<style>
  section { padding: 0.4rem 0 1rem; border-bottom: 1px solid var(--border); margin-bottom: 0.8rem; }
  section:last-of-type { border-bottom: none; margin-bottom: 0; }
  h4 { margin: 1rem 0 0.5rem; }
  code { background: var(--surface-3); padding: 0.05rem 0.35rem; border-radius: 4px; word-break: break-all; }
  .row { display: flex; gap: 0.5rem; }
  .inline { margin-top: 1rem; }
  .warn { color: var(--warn); }
  .hint { margin-top: 0.5rem; }
</style>
