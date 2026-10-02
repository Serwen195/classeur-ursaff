<script lang="ts">
  import { app } from '../lib/store.svelte';
</script>

{#if app.update.kind === 'available'}
  <div class="banner info" role="status">
    <span>
      <strong>Nouvelle version {app.update.version} disponible.</strong>
      Vos données ne sont pas touchées par la mise à jour.
    </span>
    <button class="btn primary small" onclick={() => app.installUpdate()}>Installer et redémarrer</button>
  </div>
{:else if app.update.kind === 'installing'}
  <div class="banner info" role="status">
    <span>
      Téléchargement de la version {app.update.version}…
      {#if app.update.percent !== null}{app.update.percent} %{/if}
    </span>
    <progress max="100" value={app.update.percent ?? undefined} aria-label="Progression du téléchargement"></progress>
  </div>
{/if}
