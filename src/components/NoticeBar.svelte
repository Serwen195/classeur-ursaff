<script lang="ts">
  import { app } from '../lib/store.svelte';
</script>

{#if app.notice}
  <div class="toast {app.notice.kind}" role={app.notice.kind === 'error' ? 'alert' : 'status'}>
    <span class="text">{app.notice.text}</span>
    <button class="btn ghost small" onclick={() => app.dismissNotice()} aria-label="Fermer le message">✕</button>
  </div>
{/if}

<style>
  .toast {
    position: fixed; left: 50%; bottom: 1.2rem; transform: translateX(-50%); z-index: 1000;
    max-width: min(680px, calc(100vw - 2rem)); display: flex; gap: 0.8rem; align-items: flex-start;
    padding: 0.7rem 0.9rem; border-radius: 10px; box-shadow: var(--shadow); border: 1px solid var(--border);
    background: var(--surface);
  }
  .text { white-space: pre-line; }
  .toast.error { border-color: var(--danger); background: var(--danger-soft); color: var(--danger); }
  .toast.info { border-color: var(--info); background: var(--info-soft); color: var(--info); }
</style>
