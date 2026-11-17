<script lang="ts">
  import { connections } from '../lib/connections.svelte';
  import { ipc } from '../lib/ipc';
  import { formatRowLimit } from '../lib/rowLimit';
  import { settings } from '../lib/settings.svelte';
  import { tabs } from '../lib/tabs.svelte';
  import type { AppInfo } from '../lib/types';
  import { ui } from '../lib/ui.svelte';

  let { info }: { info: AppInfo | null } = $props();

  const connection = $derived(connections.byId(tabs.active?.connectionId));
</script>

<footer class="status">
  {#if settings.value}
    <button onclick={() => ui.openSettings('general')} title="Your Client ID. Click to change it.">
      Client ID <strong>{settings.value.general.clientId}</strong>
    </button>
    <button
      onclick={() => ui.openSettings('execution')}
      title="Default row limit for new tabs. Click to change it."
    >
      Default limit <strong>{formatRowLimit(settings.value.execution.defaultRowLimit)}</strong>
    </button>
  {/if}
  {#if connection?.connected && connection.serverVersion}
    <span class="server">{connection.name} · {connection.serverVersion}</span>
  {/if}
  <span class="spacer"></span>
  {#if info && !info.secretsPersistent}
    <span
      class="warning"
      title="No system keychain was found, so passwords cannot be stored. They are kept in memory and forgotten when Rowboat quits."
    >
      No keychain: passwords are not saved
    </span>
  {/if}
  {#if settings.error}
    <span class="error" role="alert">Could not save settings: {settings.error}</span>
  {/if}
  {#if ui.update}
    <button
      class="update"
      onclick={() => ipc.updateOpen(ui.update!.url)}
      title="Open the release page to download it"
    >
      Rowboat {ui.update.version} is available
    </button>
  {/if}
  {#if info}
    <span>Rowboat {info.version}</span>
  {/if}
</footer>

<style>
  .update {
    color: var(--accent);
    font-weight: 600;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 24px;
    padding: 0 8px;
    flex: none;
    font-size: 11px;
    color: var(--text-muted);
    background: var(--bg-subtle);
    border-top: 1px solid var(--border);
  }
  button {
    height: 20px;
    padding: 0 6px;
    border: none;
    border-radius: 4px;
    background: none;
    color: inherit;
    font-size: inherit;
  }
  button:hover {
    background: var(--bg-hover);
  }
  strong {
    color: var(--text);
    font-weight: 600;
  }
  .spacer {
    flex: 1;
  }
  .server {
    padding: 0 6px;
  }
  .warning {
    margin-right: 8px;
    color: var(--danger);
  }
  .error {
    color: var(--danger);
    margin-right: 8px;
  }
</style>
