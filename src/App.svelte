<script lang="ts">
  import { onMount } from 'svelte';
  import Sidebar from './components/Sidebar.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import Workspace from './components/Workspace.svelte';
  import { ACTIONS } from './lib/actions';
  import { connections } from './lib/connections.svelte';
  import { dialogs } from './lib/dialogs.svelte';
  import { prompts } from './lib/runner';
  import { ipc } from './lib/ipc';
  import { reportReady } from './lib/ready';
  import { settings } from './lib/settings.svelte';
  import { chordFromEvent, findAction } from './lib/shortcuts';
  import { tabs } from './lib/tabs.svelte';
  import type { AppInfo } from './lib/types';
  import { ui } from './lib/ui.svelte';

  let info = $state<AppInfo | null>(null);
  let loadError = $state<string | null>(null);

  // Prompts raised by the core's safety checks are answered here.
  prompts.confirmDestructive = (sql, reason, connectionName) =>
    dialogs.confirm({
      title: `Run this on ${connectionName}?`,
      message: reason,
      detail: sql.length > 1200 ? `${sql.slice(0, 1200)}…` : sql,
      confirmLabel: 'Run it',
      danger: true,
      prod: connections.list.find((c) => c.name === connectionName)?.environment === 'prod',
    });
  connections.askHostKey = (prompt) =>
    dialogs.confirm({
      title: 'Trust this SSH server?',
      message: `${prompt.message} Check the fingerprint with whoever runs the server. If you trust it, it is added to known_hosts.`,
      detail: `${prompt.keyType}\n${prompt.fingerprint}`,
      confirmLabel: 'Trust and connect',
    });

  const WELCOME = `-- Welcome to Rowboat.
-- Add a connection in the sidebar, then run a statement with Ctrl/⌘+Enter.

SELECT 1;
`;

  onMount(async () => {
    try {
      await Promise.all([settings.load(), connections.load()]);
      tabs.open(
        settings.value!.execution.defaultRowLimit,
        connections.list.length === 0 ? WELCOME : '',
        {
          // With a single connection there is nothing to choose.
          connectionId: connections.list.length === 1 ? connections.list[0].id : null,
        },
      );
    } catch (e) {
      loadError = String(e);
    }
    // Without settings there is no editor to wait for.
    if (loadError) reportReady();
    ipc.appInfo().then(
      (i) => (info = i),
      () => {},
    );
  });

  function onKeydown(event: KeyboardEvent) {
    if (ui.settingsOpen || dialogs.anyOpen) return; // Dialogs handle their own keys.
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
  {#if dialogs.connection}
    {#await import('./components/ConnectionDialog.svelte') then { default: ConnectionDialog }}
      <ConnectionDialog />
    {/await}
  {/if}
  {#if dialogs.confirmRequest}
    {#await import('./components/ConfirmDialog.svelte') then { default: ConfirmDialog }}
      {#key dialogs.confirmRequest}<ConfirmDialog />{/key}
    {/await}
  {/if}
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
