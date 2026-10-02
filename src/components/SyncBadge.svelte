<script lang="ts">
  import { app } from '../lib/store.svelte';
  import { relativeTime } from '../lib/time';

  interface Props { onclick?: () => void }
  let { onclick }: Props = $props();

  // Rafraîchit « il y a 5 min » sans dépendre d'un événement.
  let now = $state(Date.now());
  $effect(() => {
    const id = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(id);
  });

  const view = $derived.by(() => {
    const s = app.sync;
    switch (s.status) {
      case 'syncing':
        return { cls: 'busy', icon: '⟳', text: 'Synchronisation…' };
      case 'offline':
        return { cls: 'warn', icon: '⚠', text: s.pending ? 'Hors ligne — modifications en attente' : 'Hors ligne' };
      case 'auth_error':
        return { cls: 'bad', icon: '⚠', text: 'Accès refusé — vérifiez le jeton' };
      case 'error':
        return { cls: 'bad', icon: '⚠', text: 'Erreur de synchronisation' };
      case 'deferred':
        return { cls: 'warn', icon: '⟳', text: 'Fusion à faire après déverrouillage' };
      default:
        return s.pending
          ? { cls: 'warn', icon: '●', text: 'Modifications à envoyer' }
          : s.lastSyncAt === null
            ? { cls: 'neutral', icon: '…', text: 'Pas encore synchronisé' }
            : { cls: 'ok', icon: '✓', text: `Synchronisé ${relativeTime(s.lastSyncAt, now)}` };
    }
  });
</script>

<button class="badge {view.cls}" {onclick} title={app.sync.message ?? 'Ouvrir les réglages de synchronisation'} aria-live="polite">
  <span class="icon" aria-hidden="true">{view.icon}</span>
  <span>{view.text}</span>
</button>

<style>
  .badge {
    display: inline-flex; align-items: center; gap: 0.4rem; padding: 0.3rem 0.7rem; border-radius: 999px;
    border: 1px solid var(--border); background: var(--surface); cursor: pointer; font-size: 0.84rem;
  }
  .badge.ok { color: var(--ok); }
  .badge.warn { color: var(--warn); background: var(--warn-soft); border-color: transparent; }
  .badge.bad { color: var(--danger); background: var(--danger-soft); border-color: transparent; }
  .badge.neutral { color: var(--muted); }
  .badge.busy { color: var(--info); }
  .badge.busy .icon { display: inline-block; animation: spin 1.2s linear infinite; }
  @keyframes spin { to { transform: rotate(360deg); } }
</style>
