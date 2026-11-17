<script lang="ts">
  import { onMount, tick } from 'svelte';
  import Sidebar from './components/Sidebar.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import Workspace from './components/Workspace.svelte';
  import { ACTIONS } from './lib/actions';
  import { ipc } from './lib/ipc';
  import { settings } from './lib/settings.svelte';
  import { chordFromEvent, findAction } from './lib/shortcuts';
  import { tabs } from './lib/tabs.svelte';
  import type { AppInfo } from './lib/types';
  import { ui } from './lib/ui.svelte';

  let info = $state<AppInfo | null>(null);
  let loadError = $state<string | null>(null);

  const WELCOME = `-- Welcome to Rowboat.
-- Connections arrive in the next milestone; for now, try the editor and Settings.

SELECT o.id, c.name, o.total
FROM orders AS o
JOIN customers AS c ON c.id = o.customer_id
WHERE o.created_at >= now() - interval '7 days'
ORDER BY o.total DESC;
`;

  onMount(async () => {
    try {
      await settings.load();
      tabs.open(settings.value!.execution.defaultRowLimit, WELCOME);
    } catch (e) {
      loadError = String(e);
    }
    // The window is usable from here; everything below is off the hot path.
    await tick();
    requestAnimationFrame(() => void ipc.appReady().catch(() => {}));
    ipc.appInfo().then(
      (i) => (info = i),
      () => {},
    );
  });

  function onKeydown(event: KeyboardEvent) {
    if (ui.settingsOpen) return; // The dialog handles its own keys.
    const chord = chordFromEvent(event);
    if (!chord) return;
    const action = findAction(ACTIONS, settings.value?.keyboard.overrides ?? {}, chord);
    if (action) {
      event.preventDefault();
      action.run();
    }
  }

  // The webview's own context menu (Reload, Back…) makes no sense in an app;
  // keep it only where it offers cut/copy/paste.
  function onContextMenu(event: MouseEvent) {
    const target = event.target as Element | null;
    if (!target?.closest('input, textarea, .cm-content')) event.preventDefault();
  }
</script>

<svelte:window onkeydown={onKeydown} />
<svelte:document oncontextmenu={onContextMenu} />

{#if loadError}
  <main class="fatal" role="alert">
    <h1>Rowboat could not start</h1>
    <p>{loadError}</p>
  </main>
{:else if settings.value}
  <div class="app">
    <div class="main">
      {#if ui.sidebarVisible}<Sidebar />{/if}
      <Workspace />
    </div>
    <StatusBar {info} />
  </div>
  {#if ui.settingsOpen}
    {#await import('./components/SettingsDialog.svelte') then { default: SettingsDialog }}
      <SettingsDialog {info} />
    {/await}
  {/if}
{/if}

<style>
  .app {
    display: flex;
    flex-direction: column;
    height: 100%;
  }
  .main {
    display: flex;
    flex: 1;
    min-height: 0;
  }
  .fatal {
    padding: 48px;
    user-select: text;
    -webkit-user-select: text;
  }
</style>
