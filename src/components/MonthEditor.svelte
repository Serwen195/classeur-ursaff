<script lang="ts">
  import { untrack } from 'svelte';
  import { backend } from '../lib/api';
  import { app } from '../lib/store.svelte';
  import { STATUSES, STATUS_LABELS, formatBytes, monthLabel, todayIso } from '../lib/months';
  import { parseEuros, toInputEuros } from '../lib/money';
  import type { Attachment, Status } from '../lib/types';
  import Modal from './Modal.svelte';

  interface Props { year: number; month: number }
  let { year, month }: Props = $props();

  const rec = $derived(app.recordOf(year, month));
  const initial = untrack(() => app.recordOf(year, month));

  let amount = $state(toInputEuros(initial?.amountCents ?? null));
  let declaredOn = $state(initial?.declaredOn ?? '');
  let status = $state<Status>(initial?.status ?? 'to_declare');
  let notes = $state(initial?.notes ?? '');
  let amountError = $state<string | null>(null);
  let saving = $state(false);
  let preview = $state<{ name: string; url: string } | null>(null);

  const dirty = $derived(
    amount.trim() !== toInputEuros(initial?.amountCents ?? null) ||
      declaredOn !== (initial?.declaredOn ?? '') ||
      status !== (initial?.status ?? 'to_declare') ||
      notes !== (initial?.notes ?? ''),
  );

  function onStatusChange() {
    // Un mois déclaré ou payé a une date de déclaration : on propose celle du jour.
    if (status !== 'to_declare' && declaredOn === '') declaredOn = todayIso();
  }

  async function save(e?: Event) {
    e?.preventDefault();
    const parsed = parseEuros(amount);
    if (!parsed.ok) {
      amountError = parsed.error;
      return;
    }
    amountError = null;
    saving = true;
    const ok = await app.saveMonth({ year, month, amountCents: parsed.cents, declaredOn: declaredOn || null, status, notes });
    saving = false;
    if (ok) app.closeEditor();
  }

  async function close() {
    if (dirty) {
      const ok = await app.ask({
        title: 'Abandonner les modifications ?',
        message: 'Les changements saisis dans ce mois ne seront pas enregistrés.',
        confirmLabel: 'Abandonner',
        danger: true,
      });
      if (!ok) return;
    }
    app.closeEditor();
  }

  async function clearMonth() {
    const n = rec?.attachments.length ?? 0;
    const ok = await app.ask({
      title: `Vider ${monthLabel(year, month)} ?`,
      message: `Le montant, la date, le statut, les notes${n > 0 ? ` et les ${n} pièce(s) jointe(s)` : ''} de ce mois seront supprimés.`,
      confirmLabel: 'Vider ce mois',
      danger: true,
    });
    if (ok && (await app.deleteMonth(year, month))) app.closeEditor();
  }

  async function removeAttachment(a: Attachment) {
    const ok = await app.ask({
      title: 'Supprimer la pièce jointe ?',
      message: `« ${a.name} » sera retirée de ce mois.`,
      confirmLabel: 'Supprimer',
      danger: true,
    });
    if (ok) await app.removeAttachment(year, month, a.id);
  }

  const previewable = (a: Attachment) => /^image\/(png|jpeg|gif|webp|bmp)$/.test(a.mime);

  async function showPreview(a: Attachment) {
    try {
      const bytes = await (await backend()).attachmentBytes(year, month, a.id);
      closePreview();
      preview = { name: a.name, url: URL.createObjectURL(new Blob([bytes], { type: a.mime })) };
    } catch (e) {
      app.notice = { kind: 'error', text: e instanceof Error ? e.message : 'Aperçu impossible.' };
    }
  }

  function closePreview() {
    if (preview) URL.revokeObjectURL(preview.url);
    preview = null;
  }

  // Glisser-déposer de fichiers dans la fenêtre pendant que le mois est ouvert.
  $effect(() => {
    let cancelled = false;
    let off: (() => void) | undefined;
    void backend()
      .then((b) => b.onFilesDropped((paths) => void app.addAttachments(year, month, paths)))
      .then((u) => {
        if (cancelled) u();
        else off = u;
      });
    return () => {
      cancelled = true;
      off?.();
    };
  });

  $effect(() => () => closePreview());
</script>

<Modal title={monthLabel(year, month)} onclose={close} wide>
  <form id="month-form" onsubmit={save}>
    <div class="grid">
      <div class="field">
        <label for="amount">CA déclaré (€)</label>
        <input id="amount" type="text" inputmode="decimal" bind:value={amount} placeholder="ex. 1 234,56" autocomplete="off" aria-invalid={amountError ? 'true' : undefined} />
        {#if amountError}<span class="error-text" role="alert">{amountError}</span>{/if}
      </div>
      <div class="field">
        <label for="declared-on">Date de déclaration</label>
        <input id="declared-on" type="date" bind:value={declaredOn} />
      </div>
    </div>

    <fieldset class="field status">
      <legend class="label">Statut</legend>
      <div class="seg">
        {#each STATUSES as s (s)}
          <label class="opt {s}" class:on={status === s}>
            <input type="radio" name="status" value={s} bind:group={status} onchange={onStatusChange} />
            {STATUS_LABELS[s]}
          </label>
        {/each}
      </div>
    </fieldset>

    <div class="field">
      <label for="notes">Notes</label>
      <textarea id="notes" bind:value={notes} rows="4" placeholder="Remarques libres sur cette déclaration…"></textarea>
    </div>
  </form>

  <section class="att" aria-labelledby="att-title">
    <h3 id="att-title">Pièces jointes</h3>
    {#if rec && rec.attachments.length > 0}
      <ul>
        {#each rec.attachments as a (a.id)}
          <li>
            <span class="icon" aria-hidden="true">{a.mime === 'application/pdf' ? '📄' : '🖼️'}</span>
            <span class="name" title={a.name}>{a.name}</span>
            <span class="muted small">{formatBytes(a.size)}</span>
            <span class="acts">
              {#if previewable(a)}<button class="btn small" onclick={() => showPreview(a)}>Aperçu</button>{/if}
              <button class="btn small" onclick={() => app.openAttachment(year, month, a.id)}>Ouvrir</button>
              <button class="btn small" onclick={() => app.saveAttachmentAs(year, month, a.id)}>Enregistrer sous…</button>
              <button class="btn small danger" onclick={() => removeAttachment(a)} aria-label="Supprimer {a.name}">Supprimer</button>
            </span>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="muted">Aucune pièce jointe (accusé de déclaration, justificatif de paiement…).</p>
    {/if}
    <button class="btn" type="button" onclick={() => app.pickAndAddAttachments(year, month)}>Ajouter des fichiers…</button>
    <p class="hint muted small">PDF ou images, 20 Mo maximum chacun. Chiffrés avant d'être stockés. Vous pouvez aussi les glisser dans cette fenêtre.</p>

    {#if preview}
      <div class="preview">
        <div class="phead"><strong>{preview.name}</strong><button class="btn small" onclick={closePreview}>Fermer l'aperçu</button></div>
        <img src={preview.url} alt="Aperçu de {preview.name}" />
      </div>
    {/if}
  </section>

  {#snippet footer()}
    {#if rec}<button class="btn danger left" type="button" onclick={clearMonth}>Vider ce mois</button>{/if}
    <button class="btn" type="button" onclick={close}>Annuler</button>
    <button class="btn primary" type="submit" form="month-form" disabled={saving}>{saving ? 'Enregistrement…' : 'Enregistrer'}</button>
  {/snippet}
</Modal>

<style>
  .grid { display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; }
  fieldset { border: none; padding: 0; margin: 0 0 0.9rem; }
  .seg { display: flex; gap: 0.5rem; flex-wrap: wrap; }
  .opt {
    position: relative; padding: 0.45rem 0.9rem; border: 1px solid var(--border); border-radius: 999px; cursor: pointer;
    font-weight: 600; background: var(--surface);
  }
  .opt input { position: absolute; opacity: 0; inset: 0; cursor: pointer; }
  .opt:has(input:focus-visible) { box-shadow: var(--focus); }
  .opt.on.to_declare { background: var(--warn-soft); color: var(--warn); border-color: var(--warn); }
  .opt.on.declared { background: var(--info-soft); color: var(--info); border-color: var(--info); }
  .opt.on.paid { background: var(--ok-soft); color: var(--ok); border-color: var(--ok); }
  .att { border-top: 1px solid var(--border); padding-top: 0.8rem; }
  ul { list-style: none; margin: 0 0 0.8rem; padding: 0; }
  li { display: flex; align-items: center; gap: 0.6rem; padding: 0.45rem 0; border-bottom: 1px solid var(--border); flex-wrap: wrap; }
  .name { flex: 1; min-width: 8rem; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-weight: 500; }
  .acts { display: flex; gap: 0.35rem; flex-wrap: wrap; }
  .hint { margin: 0.5rem 0 0; }
  .preview { margin-top: 0.8rem; background: var(--surface-2); border-radius: 8px; padding: 0.6rem; }
  .phead { display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.5rem; gap: 0.5rem; }
  .preview img { max-width: 100%; max-height: 50vh; display: block; margin: 0 auto; border-radius: 4px; }
  :global(dialog footer .left) { margin-right: auto; }
  @media (max-width: 640px) { .grid { grid-template-columns: 1fr; } }
</style>
