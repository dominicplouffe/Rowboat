<script lang="ts">
  import { ACTIONS } from '../lib/actions';
  import { settings } from '../lib/settings.svelte';
  import { chordFor, formatChord } from '../lib/shortcuts';
  import { ui } from '../lib/ui.svelte';
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';

  const settingsAction = ACTIONS.find((a) => a.id === 'settings.open')!;
  const settingsChord = $derived(
    formatChord(chordFor(settingsAction, settings.value?.keyboard.overrides ?? {})),
  );
</script>

<aside class="sidebar" aria-label="Connections">
  <header>
    <span class="title">Connections</span>
    <button
      class="btn ghost icon"
      disabled
      title="Adding connections arrives in the next milestone (M1)"
      aria-label="Add connection"
    >
      <Icon name="plus" />
    </button>
  </header>

  <div class="empty">
    <Logo width={96} />
    <p class="lead">No connections yet</p>
    <p class="muted">
      PostgreSQL, ClickHouse and SQLite connections land in the next milestone. Until then you can
      write SQL and tune your settings.
    </p>
  </div>

  <footer>
    <button class="btn ghost" onclick={() => ui.openSettings()}>
      <Icon name="settings" />
      Settings
      <kbd>{settingsChord}</kbd>
    </button>
  </footer>
</aside>

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    width: 248px;
    flex: none;
    background: var(--bg-subtle);
    border-right: 1px solid var(--border);
    min-height: 0;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 38px;
    padding: 0 6px 0 14px;
    flex: none;
  }
  .title {
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.06em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  .empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    padding: 0 24px 40px;
    text-align: center;
    min-height: 0;
  }
  .empty :global(svg) {
    margin-bottom: 12px;
  }
  .lead {
    margin: 0 0 6px;
    font-weight: 600;
  }
  .empty .muted {
    margin: 0;
    font-size: 12px;
    line-height: 1.5;
  }
  footer {
    padding: 8px;
    border-top: 1px solid var(--border);
  }
  footer .btn {
    width: 100%;
  }
  footer kbd {
    margin-left: auto;
  }
</style>
