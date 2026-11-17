<script lang="ts" module>
  export interface MenuItem {
    label: string;
    run: () => void | Promise<void>;
    /** Shows a tick; the menu stays open so several can be toggled. */
    checked?: boolean;
    disabled?: boolean;
    /** Draws a separator above this item. */
    separator?: boolean;
  }
</script>

<script lang="ts">
  import Icon from './Icon.svelte';

  let {
    label,
    items,
    title,
    disabled = false,
  }: { label: string; items: MenuItem[]; title?: string; disabled?: boolean } = $props();

  let open = $state(false);
  let root: HTMLDivElement;
</script>

<svelte:window
  onpointerdown={(e) => {
    if (open && !root.contains(e.target as Node)) open = false;
  }}
/>

<div class="menu-button" bind:this={root}>
  <button
    class="btn"
    {title}
    {disabled}
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={() => (open = !open)}
    onkeydown={(e) => e.key === 'Escape' && (open = false)}
  >
    {label}
    <Icon name="chevron" size={12} />
  </button>
  {#if open}
    <div
      class="menu"
      role="menu"
      tabindex="-1"
      onkeydown={(e) => e.key === 'Escape' && (open = false)}
    >
      {#each items as item (item.label)}
        <button
          class:separator={item.separator}
          role={item.checked === undefined ? 'menuitem' : 'menuitemcheckbox'}
          aria-checked={item.checked}
          disabled={item.disabled}
          onclick={() => {
            if (item.checked === undefined) open = false;
            void item.run();
          }}
        >
          {#if item.checked !== undefined}
            <span class="tick"
              >{#if item.checked}<Icon name="check" size={12} />{/if}</span
            >
          {/if}
          {item.label}
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .menu-button {
    position: relative;
  }
  .menu {
    position: absolute;
    right: 0;
    top: calc(100% + 4px);
    z-index: 30;
    min-width: 190px;
    max-height: 320px;
    overflow-y: auto;
    padding: 4px;
    background: var(--bg);
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    box-shadow: var(--shadow);
  }
  .menu button {
    display: flex;
    align-items: center;
    gap: 6px;
    width: 100%;
    min-height: 26px;
    padding: 0 10px 0 6px;
    border: none;
    border-radius: 4px;
    background: none;
    text-align: left;
    white-space: nowrap;
  }
  .menu button:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .menu button:disabled {
    color: var(--text-faint);
  }
  .menu button.separator {
    margin-top: 4px;
    border-top: 1px solid var(--border);
    border-radius: 0 0 4px 4px;
    padding-top: 4px;
  }
  .tick {
    display: grid;
    place-items: center;
    width: 14px;
    color: var(--accent);
  }
</style>
