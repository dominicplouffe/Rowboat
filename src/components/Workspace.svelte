<script lang="ts">
  import { ACTIONS } from '../lib/actions';
  import { connections, KIND_LABELS } from '../lib/connections.svelte';
  import { dialogs } from '../lib/dialogs.svelte';
  import { asDbError } from '../lib/ipc';
  import { endTransaction } from '../lib/runner';
  import { settings } from '../lib/settings.svelte';
  import { chordFor, formatChord } from '../lib/shortcuts';
  import { tabs } from '../lib/tabs.svelte';
  import Editor from './Editor.svelte';
  import Icon from './Icon.svelte';
  import ResultsPane from './ResultsPane.svelte';
  import TabBar from './TabBar.svelte';

  const action = (id: string) => ACTIONS.find((a) => a.id === id)!;
  const chord = (id: string) =>
    formatChord(chordFor(action(id), settings.value?.keyboard.overrides ?? {}));

  const connection = $derived(connections.byId(tabs.active?.connectionId));
  let transactionError = $state<string | null>(null);

  // Editor / results split, as a fraction of the available height.
  let split = $state(0.55);
  let body = $state<HTMLDivElement>();

  function startDrag(event: PointerEvent) {
    const handle = event.currentTarget as HTMLElement;
    handle.setPointerCapture(event.pointerId);
    const move = (e: PointerEvent) => {
      if (!body) return;
      const box = body.getBoundingClientRect();
      split = Math.min(0.85, Math.max(0.15, (e.clientY - box.top) / box.height));
    };
    const stop = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', stop);
      handle.removeEventListener('pointercancel', stop);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', stop);
    handle.addEventListener('pointercancel', stop);
  }

  function nudge(event: KeyboardEvent) {
    if (event.key === 'ArrowUp') split = Math.max(0.15, split - 0.04);
    else if (event.key === 'ArrowDown') split = Math.min(0.85, split + 0.04);
  }

  async function finish(commit: boolean) {
    if (!connection) return;
    transactionError = null;
    try {
      await endTransaction(connection.id, commit);
    } catch (e) {
      transactionError = asDbError(e).message;
    }
  }
</script>

<section class="workspace">
  <TabBar />

  {#if tabs.active}
    {@const tab = tabs.active}
    <div class="body" bind:this={body}>
      <div class="pane" style:flex-basis="{split * 100}%">
        <div class="toolbar" class:prod={connection?.environment === 'prod'}>
          {#if tab.generated}
            <span class="muted">Generated definition (read-only)</span>
          {:else if tab.running}
            <button
              class="btn"
              onclick={() => action('query.cancel').run()}
              title="Cancel ({chord('query.cancel')})"
            >
              <Icon name="close" size={14} /> Cancel
            </button>
          {:else}
            <button
              class="btn primary"
              disabled={!connection}
              onclick={() => action('query.run').run()}
              title="Run the statement under the cursor, or the selection ({chord('query.run')})"
            >
              <Icon name="play" size={14} /> Run
            </button>
            <button
              class="btn"
              disabled={!connection}
              onclick={() => action('query.runScript').run()}
              title="Run every statement ({chord('query.runScript')})"
            >
              Run script
            </button>
          {/if}

          {#if connections.list.length === 0}
            <button class="btn" onclick={() => (dialogs.connection = { editing: null })}>
              <Icon name="plus" /> New connection
            </button>
          {:else if !tab.generated}
            <select
              class="input picker"
              aria-label="Connection for this tab"
              disabled={tab.running}
              value={tab.connectionId ?? ''}
              onchange={(e) => (tab.connectionId = e.currentTarget.value || null)}
            >
              <option value="">No connection</option>
              {#each connections.list as option (option.id)}
                <option value={option.id}>{option.name} — {KIND_LABELS[option.kind]}</option>
              {/each}
            </select>
          {/if}

          {#if connection?.environment === 'prod'}<span class="badge prod">PROD</span>{/if}
          {#if connection?.readOnly}<span class="badge">read-only</span>{/if}

          <span class="spacer"></span>

          {#if connection?.inTransaction}
            <span class="tx" role="status">Transaction open</span>
            <button class="btn" disabled={tab.running} onclick={() => finish(true)}>Commit</button>
            <button class="btn" disabled={tab.running} onclick={() => finish(false)}
              >Rollback</button
            >
          {:else if connection?.supportsTransactions && connection.connected && !tab.generated}
            <label
              class="check"
              title="For this connection only. Off: statements run in a transaction until you commit or roll back."
            >
              <input
                type="checkbox"
                checked={connection.autoCommit}
                onchange={(e) => connections.setAutoCommit(connection.id, e.currentTarget.checked)}
              />
              Auto-commit
            </label>
          {/if}
        </div>
        {#if transactionError}<p class="tx-error" role="alert">{transactionError}</p>{/if}
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
        <ResultsPane {tab} />
      </div>
    </div>
  {:else}
    <div class="placeholder muted">
      <p>No open tabs.</p>
      <button class="btn" onclick={() => action('tab.new').run()}>
        <Icon name="plus" /> New query tab <kbd>{chord('tab.new')}</kbd>
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
    border-top: 2px solid transparent;
  }
  .toolbar.prod {
    border-top-color: var(--danger);
  }
  .picker {
    max-width: 280px;
  }
  .badge {
    padding: 1px 7px;
    border-radius: 4px;
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    background: var(--bg-inset);
    color: var(--text-muted);
  }
  .badge.prod {
    background: var(--danger);
    color: #fff;
  }
  .spacer {
    flex: 1;
  }
  .tx {
    font-size: 12px;
    font-weight: 600;
    color: var(--wood, #b97a16);
  }
  .tx-error {
    margin: 0;
    padding: 5px 12px;
    font-size: 12px;
    color: var(--danger);
    border-bottom: 1px solid var(--border);
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    font-size: 12px;
    color: var(--text-muted);
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
