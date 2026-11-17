<script lang="ts">
  import { onMount } from 'svelte';
  import { dialogs } from '../lib/dialogs.svelte';

  const request = $derived(dialogs.confirmRequest!);
  let cancelButton: HTMLButtonElement;

  // Focus starts on Cancel: Enter must never confirm something destructive.
  onMount(() => cancelButton.focus());

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      dialogs.answer(false);
    }
  }
</script>

<svelte:window onkeydowncapture={onKeydown} />

<div class="backdrop" role="presentation">
  <div
    class="dialog"
    class:prod={request.prod}
    role="alertdialog"
    aria-modal="true"
    aria-labelledby="confirm-title"
  >
    <h2 id="confirm-title">{request.title}</h2>
    <p>{request.message}</p>
    {#if request.detail}<pre>{request.detail}</pre>{/if}
    <div class="buttons">
      <button class="btn" bind:this={cancelButton} onclick={() => dialogs.answer(false)}
        >Cancel</button
      >
      <button
        class="btn"
        class:danger={request.danger}
        class:primary={!request.danger}
        onclick={() => dialogs.answer(true)}
      >
        {request.confirmLabel}
      </button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 200;
    display: grid;
    place-items: center;
    background: rgba(8, 20, 34, 0.5);
  }
  .dialog {
    width: min(520px, calc(100vw - 48px));
    padding: 20px 22px 18px;
    background: var(--bg);
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    box-shadow: var(--shadow);
  }
  .dialog.prod {
    border-top: 4px solid var(--danger);
  }
  h2 {
    margin: 0 0 8px;
    font-size: 15px;
  }
  p {
    margin: 0 0 10px;
    line-height: 1.5;
  }
  pre {
    margin: 0 0 14px;
    padding: 10px 12px;
    max-height: 180px;
    overflow: auto;
    border-radius: var(--radius);
    background: var(--bg-inset);
    font: 12px/1.5 var(--editor-font);
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    -webkit-user-select: text;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
  }
  .btn.danger {
    background: var(--danger);
    border-color: var(--danger);
    color: #fff;
    font-weight: 600;
  }
</style>
