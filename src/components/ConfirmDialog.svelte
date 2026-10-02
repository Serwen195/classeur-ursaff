<script lang="ts">
  import { app } from '../lib/store.svelte';
  import Modal from './Modal.svelte';
</script>

{#if app.confirmRequest}
  {@const req = app.confirmRequest}
  <Modal title={req.title} onclose={() => app.answerConfirm(false)}>
    <p>{req.message}</p>
    {#snippet footer()}
      <button class="btn" onclick={() => app.answerConfirm(false)}>Annuler</button>
      <button class="btn {req.danger ? 'danger' : 'primary'}" onclick={() => app.answerConfirm(true)}>{req.confirmLabel}</button>
    {/snippet}
  </Modal>
{/if}
