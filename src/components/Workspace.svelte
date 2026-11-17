<script lang="ts">
  import { ACTIONS } from '../lib/actions';
  import { settings } from '../lib/settings.svelte';
  import { chordFor, formatChord } from '../lib/shortcuts';
  import { tabs } from '../lib/tabs.svelte';
  import Editor from './Editor.svelte';
  import Icon from './Icon.svelte';
  import RowLimitMenu from './RowLimitMenu.svelte';
  import TabBar from './TabBar.svelte';

  const newTab = ACTIONS.find((a) => a.id === 'tab.new')!;
  const newTabChord = $derived(
    formatChord(chordFor(newTab, settings.value?.keyboard.overrides ?? {})),
  );

  // Editor / results split, as a fraction of the available height.
  let split = $state(0.62);
  let body = $state<HTMLDivElement>();

  function startDrag(event: PointerEvent) {
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const move = (e: PointerEvent) => {
      if (!body) return;
      const box = body.getBoundingClientRect();
      split = Math.min(0.85, Math.max(0.2, (e.clientY - box.top) / box.height));
    };
    const stop = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', stop);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', stop);
  }

  function nudge(event: KeyboardEvent) {
    if (event.key === 'ArrowUp') split = Math.max(0.2, split - 0.04);
    else if (event.key === 'ArrowDown') split = Math.min(0.85, split + 0.04);
  }
</script>

<section class="workspace">
  <TabBar />

  {#if tabs.active}
    {@const tab = tabs.active}
    <div class="body" bind:this={body}>
      <div class="pane" style:flex-basis="{split * 100}%">
        <div class="toolbar">
          <button
            class="btn primary"
            disabled
            title="Running queries needs a connection, which arrives in the next milestone (M1)"
          >
            <Icon name="play" size={14} />
            Run
          </button>
          <span class="muted hint">No connection</span>
        </div>
        <Editor />
      </div>

      <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
      <div
        class="splitter"
        role="separator"
        aria-orientation="horizontal"
        aria-label="Resize editor and results"
        aria-valuenow={Math.round(split * 100)}
        tabindex="0"
        onpointerdown={startDrag}
        onkeydown={nudge}
      ></div>

      <div class="pane results">
        <div class="toolbar">
          <span class="section">Results</span>
          <span class="spacer"></span>
          <RowLimitMenu value={tab.rowLimit} onchange={(limit) => (tab.rowLimit = limit)} />
        </div>
        <div class="placeholder muted">
          <Icon name="database" size={28} />
          <p>Results will appear here once you run a query.</p>
        </div>
      </div>
    </div>
  {:else}
    <div class="placeholder muted">
      <p>No open tabs.</p>
      <button class="btn" onclick={() => newTab.run()}>
        <Icon name="plus" /> New query tab <kbd>{newTabChord}</kbd>
      </button>
    </div>
  {/if}
</section>

<style>
  .workspace {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
  .body {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
  }
  .pane {
    display: flex;
    flex-direction: column;
    min-height: 0;
    flex: 0 0 auto;
  }
  .pane.results {
    flex: 1 1 0;
  }
  .toolbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 6px 10px;
    flex: none;
    border-bottom: 1px solid var(--border);
  }
  .hint {
    font-size: 12px;
  }
  .section {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .spacer {
    flex: 1;
  }
  .splitter {
    flex: none;
    height: 5px;
    margin: -2px 0;
    z-index: 5;
    cursor: row-resize;
    border-top: 2px solid transparent;
    border-bottom: 2px solid transparent;
    background: var(--border);
    background-clip: padding-box;
  }
  .splitter:hover,
  .splitter:focus-visible {
    background: var(--focus);
    outline: none;
  }
  .placeholder {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    min-height: 0;
    text-align: center;
  }
  .placeholder p {
    margin: 0;
  }
</style>
