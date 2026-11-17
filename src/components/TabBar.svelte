<script lang="ts">
  import { ACTIONS } from '../lib/actions';
  import { settings } from '../lib/settings.svelte';
  import { chordFor, formatChord } from '../lib/shortcuts';
  import { tabs } from '../lib/tabs.svelte';
  import { ui } from '../lib/ui.svelte';
  import Icon from './Icon.svelte';

  const chord = (id: string) =>
    formatChord(
      chordFor(
        ACTIONS.find((a) => a.id === id)!,
        settings.value?.keyboard.overrides ?? {},
      ),
    );
  const run = (id: string) => ACTIONS.find((a) => a.id === id)!.run();
</script>

<div class="tabbar">
  <button
    class="btn ghost icon"
    onclick={() => run('sidebar.toggle')}
    title={`${ui.sidebarVisible ? 'Hide' : 'Show'} sidebar (${chord('sidebar.toggle')})`}
    aria-label="Toggle sidebar"
    aria-pressed={ui.sidebarVisible}
  >
    <Icon name="sidebar" />
  </button>

  <div class="tabs" role="tablist" aria-label="Query tabs">
    {#each tabs.list as tab (tab.id)}
      <div class="tab" class:active={tab.id === tabs.activeId}>
        <button
          class="label"
          role="tab"
          aria-selected={tab.id === tabs.activeId}
          onclick={() => (tabs.activeId = tab.id)}
          onauxclick={(e) => e.button === 1 && tabs.close(tab.id)}
        >
          {tab.title}
        </button>
        <button
          class="close"
          onclick={() => tabs.close(tab.id)}
          title={`Close tab (${chord('tab.close')})`}
          aria-label={`Close ${tab.title}`}
        >
          <Icon name="close" size={12} />
        </button>
      </div>
    {/each}
  </div>

  <button
    class="btn ghost icon"
    onclick={() => run('tab.new')}
    title={`New query tab (${chord('tab.new')})`}
    aria-label="New query tab"
  >
    <Icon name="plus" />
  </button>
</div>

<style>
  .tabbar {
    display: flex;
    align-items: center;
    gap: 2px;
    height: 38px;
    padding: 0 6px;
    background: var(--bg-subtle);
    border-bottom: 1px solid var(--border);
    flex: none;
  }
  .tabs {
    display: flex;
    align-items: flex-end;
    align-self: stretch;
    gap: 2px;
    min-width: 0;
    overflow: hidden;
    padding-top: 5px;
  }
  .tab {
    display: flex;
    align-items: center;
    height: 100%;
    min-width: 0;
    border: 1px solid transparent;
    border-bottom: none;
    border-radius: 6px 6px 0 0;
    color: var(--text-muted);
  }
  .tab:hover {
    background: var(--bg-hover);
  }
  .tab.active {
    background: var(--bg);
    border-color: var(--border);
    color: var(--text);
    /* Sit on top of the tab bar's bottom border. */
    margin-bottom: -1px;
    padding-bottom: 1px;
  }
  .label {
    height: 100%;
    padding: 0 4px 0 12px;
    border: none;
    background: none;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .close {
    display: grid;
    place-items: center;
    width: 18px;
    height: 18px;
    margin-right: 6px;
    border: none;
    border-radius: 4px;
    background: none;
    color: var(--text-faint);
    flex: none;
  }
  .close:hover {
    background: var(--bg-hover);
    color: var(--text);
  }
</style>
