<script lang="ts">
  import { onMount } from 'svelte';
  import Sidebar from './components/Sidebar.svelte';
  import StatusBar from './components/StatusBar.svelte';
  import Workspace from './components/Workspace.svelte';
  import { ACTIONS } from './lib/actions';
  import { ai } from './lib/ai.svelte';
  import { connections } from './lib/connections.svelte';
  import { dialogs } from './lib/dialogs.svelte';
  import { prompts } from './lib/runner';
  import { loadSaved, restore, saveSoon } from './lib/session';
  import { ipc } from './lib/ipc';
  import { reportReady } from './lib/ready';
  import { settings } from './lib/settings.svelte';
  import { chordFromEvent, findAction } from './lib/shortcuts';
  import { tabs } from './lib/tabs.svelte';
  import type { AppInfo } from './lib/types';
  import { ui } from './lib/ui.svelte';

  let info = $state<AppInfo | null>(null);
  let loadError = $state<string | null>(null);
  let sessionReady = $state(false);

  // Save open tabs and their text shortly after every change.
  $effect(() => {
    if (!sessionReady) return;
    for (const tab of tabs.list)
      void [tab.title, tab.text, tab.connectionId, tab.rowLimit, tab.path];
    void tabs.activeId;
    saveSoon();
  });

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
  // Failed statements go to the copilot, which decides whether a fix may be suggested.
  prompts.failed = (tab, statement) => void ai.autoFix(tab, statement);
  // Index a schema as soon as its connection opens, but only once AI is set up.
  connections.opened = (id) => {
    if (ai.provider) void ai.reindex(id, true);
  };
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
      // One round of requests, not three in a row: this is on the startup path.
      const [, , saved] = await Promise.all([settings.load(), connections.load(), loadSaved()]);
      const known = new Set(connections.list.map((c) => c.id));
      const restored = settings.value!.general.restoreTabs && restore(known, saved);
      if (!restored) {
        tabs.open(
          settings.value!.execution.defaultRowLimit,
          connections.list.length === 0 ? WELCOME : '',
          {
            // With a single connection there is nothing to choose.
            connectionId: connections.list.length === 1 ? connections.list[0].id : null,
          },
        );
      }
      sessionReady = true;
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
    // Dialogs handle their own keys.
    if (ui.settingsOpen || ui.historyOpen || dialogs.anyOpen || ai.open) return;
    if (!settings.value?.general.onboardingCompleted) return;
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
  {#if ai.open}
    {#await import('./components/AskPanel.svelte') then { default: AskPanel }}
      <AskPanel />
    {/await}
  {/if}
  {#if !settings.value.general.onboardingCompleted}
    {#await import('./components/Onboarding.svelte') then { default: Onboarding }}
      <Onboarding {info} />
    {/await}
  {/if}
  {#if ui.historyOpen}
    {#await import('./components/HistoryDialog.svelte') then { default: HistoryDialog }}
      <HistoryDialog />
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
