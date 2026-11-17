<script lang="ts">
  import { formatRowLimit, NO_LIMIT, parseRowLimit, ROW_LIMIT_PRESETS } from '../lib/rowLimit';
  import Icon from './Icon.svelte';

  let { value, onchange }: { value: number; onchange: (limit: number) => void } = $props();

  let open = $state(false);
  let custom = $state('');
  let root: HTMLDivElement;

  const customValue = $derived(parseRowLimit(custom));

  function pick(limit: number) {
    onchange(limit);
    open = false;
    custom = '';
  }

  function onWindowPointerDown(event: PointerEvent) {
    if (open && !root.contains(event.target as Node)) open = false;
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} />

<div class="limit" bind:this={root}>
  <button
    class="btn"
    aria-haspopup="menu"
    aria-expanded={open}
    title="Rows fetched per page. Applied when fetching; your SQL is never rewritten."
    onclick={() => (open = !open)}
    onkeydown={(e) => e.key === 'Escape' && (open = false)}
  >
    <span class="muted">Limit</span>
    {formatRowLimit(value)}
    <Icon name="chevron" size={12} />
  </button>

  {#if open}
    <div
      class="menu"
      role="menu"
      tabindex="-1"
      onkeydown={(e) => e.key === 'Escape' && (open = false)}
    >
      {#each [...ROW_LIMIT_PRESETS, NO_LIMIT] as preset (preset)}
        <button role="menuitemradio" aria-checked={value === preset} onclick={() => pick(preset)}>
          <span class="tick">
            {#if value === preset}<Icon name="check" size={12} />{/if}
          </span>
          {formatRowLimit(preset)}
        </button>
      {/each}
      <form
        onsubmit={(e) => {
          e.preventDefault();
          if (customValue !== null) pick(customValue);
        }}
      >
        <input
          class="input"
          inputmode="numeric"
          placeholder="Custom…"
          aria-label="Custom row limit"
          bind:value={custom}
        />
        <button class="btn" type="submit" disabled={customValue === null}>Set</button>
      </form>
    </div>
  {/if}
</div>

<style>
  .limit {
    position: relative;
  }
  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    z-index: 20;
    min-width: 180px;
    padding: 4px;
    background: var(--bg);
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    box-shadow: var(--shadow);
  }
  .menu > button {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    height: 26px;
    padding: 0 8px 0 4px;
    border: none;
    border-radius: 4px;
    background: none;
    text-align: left;
  }
  .menu > button:hover {
    background: var(--bg-hover);
  }
  .tick {
    display: grid;
    place-items: center;
    width: 16px;
    color: var(--accent);
  }
  form {
    display: flex;
    gap: 4px;
    margin-top: 4px;
    padding-top: 6px;
    border-top: 1px solid var(--border);
  }
  form .input {
    flex: 1;
    width: 0;
  }
</style>
