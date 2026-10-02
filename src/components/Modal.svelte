<script lang="ts">
  import type { Snippet } from 'svelte';

  interface Props {
    title: string;
    onclose: () => void;
    wide?: boolean;
    children: Snippet;
    footer?: Snippet;
  }
  let { title, onclose, wide = false, children, footer }: Props = $props();

  let dialog: HTMLDialogElement;
  $effect(() => {
    if (!dialog.open) dialog.showModal();
  });
</script>

<dialog
  bind:this={dialog}
  class:wide
  aria-labelledby="modal-title"
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
  onclick={(e) => {
    if (e.target === dialog) onclose();
  }}
>
  <div class="panel">
    <header>
      <h2 id="modal-title">{title}</h2>
      <button class="btn ghost small" onclick={onclose} aria-label="Fermer">✕</button>
    </header>
    <div class="body">{@render children()}</div>
    {#if footer}<footer>{@render footer()}</footer>{/if}
  </div>
</dialog>

<style>
  dialog {
    border: none; padding: 0; background: transparent; color: inherit;
    width: min(560px, calc(100vw - 2rem)); max-height: calc(100vh - 2rem); overflow: visible;
  }
  dialog.wide { width: min(780px, calc(100vw - 2rem)); }
  dialog::backdrop { background: rgba(8, 12, 30, 0.55); }
  .panel {
    background: var(--surface); border: 1px solid var(--border); border-radius: 14px; box-shadow: var(--shadow);
    display: flex; flex-direction: column; max-height: calc(100vh - 2rem);
  }
  header { display: flex; align-items: center; justify-content: space-between; padding: 1rem 1.2rem 0.4rem; }
  header h2 { margin: 0; }
  .body { padding: 0.6rem 1.2rem 1rem; overflow-y: auto; }
  footer {
    padding: 0.8rem 1.2rem; border-top: 1px solid var(--border); display: flex; gap: 0.6rem; justify-content: flex-end; flex-wrap: wrap;
    background: var(--surface-2); border-radius: 0 0 14px 14px;
  }
</style>
