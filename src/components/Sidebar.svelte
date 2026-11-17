<script lang="ts">
  import { ai } from '../lib/ai.svelte';
  import { ACTIONS } from '../lib/actions';
  import { connections, KIND_LABELS } from '../lib/connections.svelte';
  import { dialogs } from '../lib/dialogs.svelte';
  import { asDbError, ipc } from '../lib/ipc';
  import { run } from '../lib/runner';
  import { insertTemplate, qualifiedName, schema, selectTemplate } from '../lib/schema.svelte';
  import { settings } from '../lib/settings.svelte';
  import { chordFor, formatChord } from '../lib/shortcuts';
  import { tabs } from '../lib/tabs.svelte';
  import type { ConnectionView, SchemaNode } from '../lib/types';
  import { ui } from '../lib/ui.svelte';
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';
  import SchemaNodes from './SchemaNodes.svelte';

  const settingsAction = ACTIONS.find((a) => a.id === 'settings.open')!;
  const settingsChord = $derived(
    formatChord(chordFor(settingsAction, settings.value?.keyboard.overrides ?? {})),
  );

  let filterText = $state('');
  const filter = $derived(filterText.trim().toLowerCase());

  interface MenuItem {
    label: string;
    run: () => void | Promise<void>;
  }
  let menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  let notice = $state<string | null>(null);

  // Connections grouped by folder; unfiled ones come first.
  const groups = $derived.by(() => {
    const byFolder: Record<string, ConnectionView[]> = {};
    for (const connection of connections.list)
      (byFolder[connection.folder] ??= []).push(connection);
    return Object.keys(byFolder)
      .sort((a, b) => (a === '' ? -1 : b === '' ? 1 : a.localeCompare(b)))
      .map((folder) => ({ folder, items: byFolder[folder] }));
  });
  let collapsed = $state<Record<string, boolean>>({});

  const rowLimit = () => settings.value?.execution.defaultRowLimit ?? 1000;

  function showMenu(event: MouseEvent, items: MenuItem[]) {
    const target = event.currentTarget as HTMLElement;
    const box = target.getBoundingClientRect();
    const fromButton = event.type === 'click';
    menu = {
      x: Math.min(fromButton ? box.left : event.clientX, window.innerWidth - 230),
      y: Math.min(
        fromButton ? box.bottom + 2 : event.clientY,
        window.innerHeight - items.length * 28 - 16,
      ),
      items,
    };
  }

  async function toggleConnection(connection: ConnectionView) {
    // Selecting a connection makes it the active tab's connection.
    const tab = tabs.active;
    if (tab && !tab.running && !tab.generated) tab.connectionId = connection.id;
    if (!connection.connected) {
      if (!(await connections.open(connection.id))) return;
      schema.expanded[schema.key(connection.id, [])] = true;
      await schema.load(connection.id, []);
      return;
    }
    await schema.toggle(connection.id, []);
  }

  async function disconnect(connection: ConnectionView) {
    if (connection.inTransaction) {
      const ok = await dialogs.confirm({
        title: 'Disconnect with an open transaction?',
        message: `“${connection.name}” has uncommitted changes. Disconnecting rolls them back.`,
        confirmLabel: 'Roll back and disconnect',
        danger: true,
      });
      if (!ok) return;
    }
    await connections.close(connection.id);
    schema.forget(connection.id);
  }

  function connectionMenu(event: MouseEvent, connection: ConnectionView) {
    showMenu(event, [
      connection.connected
        ? { label: 'Disconnect', run: () => disconnect(connection) }
        : { label: 'Connect', run: () => toggleConnection(connection) },
      {
        label: 'New query tab',
        run: () => void tabs.open(rowLimit(), '', { connectionId: connection.id }),
      },
      ...(connection.connected
        ? [{ label: 'Refresh', run: () => schema.refresh(connection.id) }]
        : []),
      ...(connection.connected && connection.aiEnabled !== false
        ? [{ label: 'Re-index schema for AI', run: () => void ai.reindex(connection.id, true) }]
        : []),
      { label: 'Edit…', run: () => void (dialogs.connection = { editing: connection }) },
    ]);
  }

  async function columnsOf(connection: ConnectionView, path: string[]): Promise<string[]> {
    if (!schema.branch(connection.id, path)) await schema.load(connection.id, path);
    return schema.branch(connection.id, path)?.nodes.map((n) => n.name) ?? [];
  }

  async function selectRows(connection: ConnectionView, path: string[]) {
    // Plain SELECT *: the tab's row limit bounds the fetch, not a LIMIT clause.
    const sql = `SELECT * FROM ${qualifiedName(connection.kind, path)};\n`;
    const tab = tabs.open(rowLimit(), sql, {
      connectionId: connection.id,
      title: path[path.length - 1],
    });
    await run(tab, { from: 0, to: 0 }, 'script');
  }

  async function showDdl(connection: ConnectionView, path: string[]) {
    try {
      const ddl = await ipc.schemaDdl(connection.id, path);
      tabs.open(rowLimit(), ddl + '\n', {
        connectionId: connection.id,
        title: `${path[path.length - 1]} (DDL)`,
        generated: true,
      });
    } catch (e) {
      flash(asDbError(e).message);
    }
  }

  async function copyName(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      flash(`Copied ${text}`);
    } catch {
      flash('Could not copy to the clipboard.');
    }
  }

  function flash(text: string) {
    notice = text;
    setTimeout(() => notice === text && (notice = null), 2500);
  }

  function nodeMenu(
    event: MouseEvent,
    connection: ConnectionView,
    node: SchemaNode,
    path: string[],
  ) {
    const relation = ['table', 'view', 'matview'].includes(node.kind);
    const items: MenuItem[] = [];
    if (relation) {
      items.push({
        label: `Select rows (limit ${rowLimit() || 'none'})`,
        run: () => selectRows(connection, path),
      });
    }
    if (node.kind !== 'column')
      items.push({ label: 'Show DDL', run: () => showDdl(connection, path) });
    items.push({
      label: 'Copy name',
      run: () => copyName(relation ? qualifiedName(connection.kind, path) : node.name),
    });
    if (relation) {
      items.push({
        label: 'Generate SELECT',
        run: async () =>
          void tabs.open(
            rowLimit(),
            selectTemplate(connection.kind, path, await columnsOf(connection, path)),
            {
              connectionId: connection.id,
            },
          ),
      });
    }
    if (node.kind === 'table') {
      items.push({
        label: 'Generate INSERT',
        run: async () =>
          void tabs.open(
            rowLimit(),
            insertTemplate(connection.kind, path, await columnsOf(connection, path)),
            {
              connectionId: connection.id,
            },
          ),
      });
    }
    if (node.hasChildren) {
      items.push({ label: 'Refresh', run: () => schema.refresh(connection.id, path) });
    }
    showMenu(event, items);
  }

  function activateNode(connection: ConnectionView, node: SchemaNode, path: string[]) {
    if (['table', 'view', 'matview'].includes(node.kind)) void selectRows(connection, path);
  }

  const matches = (connection: ConnectionView) =>
    !filter ||
    connection.name.toLowerCase().includes(filter) ||
    // Keep connections whose loaded tree might hold a match.
    connection.connected;
</script>

<svelte:window
  onpointerdown={(e) => {
    if (menu && !(e.target as Element).closest('.menu')) menu = null;
  }}
  onkeydown={(e) => e.key === 'Escape' && (menu = null)}
/>

<aside class="sidebar" aria-label="Connections">
  <header>
    <span class="title">Connections</span>
    <button
      class="btn ghost icon"
      onclick={() => (dialogs.connection = { editing: null })}
      title="New connection"
      aria-label="New connection"
    >
      <Icon name="plus" />
    </button>
  </header>

  {#if connections.list.length === 0}
    <div class="empty">
      <Logo width={96} />
      <p class="lead">No connections yet</p>
      <p class="muted">Add a PostgreSQL, ClickHouse or SQLite connection to start querying.</p>
      <button class="btn primary" onclick={() => (dialogs.connection = { editing: null })}>
        <Icon name="plus" /> New connection
      </button>
    </div>
  {:else}
    <label class="filter">
      <Icon name="search" size={13} />
      <input
        type="search"
        placeholder="Filter"
        aria-label="Filter connections and objects"
        spellcheck="false"
        bind:value={filterText}
      />
    </label>

    <div class="tree" role="tree">
      {#each groups as group (group.folder)}
        {#if group.folder}
          <button
            class="folder"
            aria-expanded={!collapsed[group.folder]}
            onclick={() => (collapsed[group.folder] = !collapsed[group.folder])}
          >
            <span class="chevron" class:open={!collapsed[group.folder]}
              ><Icon name="chevron" size={10} /></span
            >
            {group.folder}
          </button>
        {/if}
        {#if !collapsed[group.folder]}
          {#each group.items.filter(matches) as connection (connection.id)}
            {@const expanded = connection.connected && schema.isExpanded(connection.id, [])}
            <div
              class="connection"
              class:prod={connection.environment === 'prod'}
              class:current={tabs.active?.connectionId === connection.id}
              class:indented={!!group.folder}
              role="treeitem"
              aria-expanded={expanded}
              aria-selected={tabs.active?.connectionId === connection.id}
              tabindex="0"
              style:--accent-color={connection.environment === 'prod'
                ? 'var(--danger)'
                : connection.color || 'transparent'}
              onclick={() => toggleConnection(connection)}
              oncontextmenu={(e) => {
                e.preventDefault();
                connectionMenu(e, connection);
              }}
              onkeydown={(e) =>
                (e.key === 'Enter' || e.key === ' ') && toggleConnection(connection)}
            >
              <span class="chevron" class:open={expanded}><Icon name="chevron" size={10} /></span>
              <span
                class="dot"
                class:on={connection.connected}
                title={connection.connected ? 'Connected' : 'Not connected'}
              ></span>
              <span class="name">{connection.name}</span>
              {#if connection.environment !== 'none'}
                <span class="env {connection.environment}">{connection.environment}</span>
              {/if}
              {#if connection.readOnly}<span class="env" title="Read-only">ro</span>{/if}
              <span class="kind">{KIND_LABELS[connection.kind]}</span>
              <button
                class="more"
                aria-label="Actions for {connection.name}"
                title="Actions"
                onclick={(e) => {
                  e.stopPropagation();
                  connectionMenu(e, connection);
                }}>⋯</button
              >
            </div>
            {#if connections.opening[connection.id]}
              <p class="state muted">Connecting…</p>
            {:else if connections.errors[connection.id]}
              <p class="state error" title={connections.errors[connection.id].message}>
                {connections.errors[connection.id].message}
                {#if connections.errors[connection.id].hint}<br />{connections.errors[connection.id]
                    .hint}{/if}
              </p>
            {/if}
            {#if expanded}
              <SchemaNodes
                connectionId={connection.id}
                path={[]}
                depth={group.folder ? 2 : 1}
                {filter}
                onmenu={(event, node, path) => nodeMenu(event, connection, node, path)}
                onactivate={(node, path) => activateNode(connection, node, path)}
              />
            {/if}
          {/each}
        {/if}
      {/each}
    </div>
  {/if}

  {#if notice}<p class="notice" role="status">{notice}</p>{/if}

  <footer>
    <button class="btn ghost" onclick={() => ui.openSettings()}>
      <Icon name="settings" />
      Settings
      <kbd>{settingsChord}</kbd>
    </button>
  </footer>
</aside>

{#if menu}
  <div class="menu" role="menu" style:left="{menu.x}px" style:top="{menu.y}px">
    {#each menu.items as item (item.label)}
      <button
        role="menuitem"
        onclick={() => {
          menu = null;
          void item.run();
        }}>{item.label}</button
      >
    {/each}
  </div>
{/if}

<style>
  .sidebar {
    display: flex;
    flex-direction: column;
    width: 264px;
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
    gap: 6px;
    padding: 0 24px 40px;
    text-align: center;
    min-height: 0;
  }
  .lead {
    margin: 6px 0 0;
    font-weight: 600;
  }
  .empty .muted {
    margin: 0 0 8px;
    font-size: 12px;
    line-height: 1.5;
  }
  .filter {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 26px;
    margin: 0 8px 6px;
    padding: 0 8px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--text-muted);
    flex: none;
  }
  .filter:focus-within {
    outline: 2px solid var(--focus);
    outline-offset: -1px;
  }
  .filter input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: none;
    appearance: none;
    -webkit-appearance: none;
    user-select: text;
    -webkit-user-select: text;
  }
  .tree {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding-bottom: 8px;
  }
  .folder {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    height: 24px;
    padding: 0 8px;
    border: none;
    background: none;
    text-align: left;
    font-weight: 600;
    color: var(--text-muted);
  }
  .folder:hover {
    background: var(--bg-hover);
  }
  .connection {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 4px 0 5px;
    border-left: 3px solid var(--accent-color);
    white-space: nowrap;
    outline-offset: -2px;
  }
  .connection.indented {
    padding-left: 19px;
  }
  .connection:hover {
    background: var(--bg-hover);
  }
  .connection.current {
    background: var(--bg-active);
  }
  .chevron {
    display: grid;
    place-items: center;
    width: 12px;
    flex: none;
    color: var(--text-faint);
    transform: rotate(-90deg);
  }
  .chevron.open {
    transform: none;
  }
  .dot {
    width: 7px;
    height: 7px;
    flex: none;
    border-radius: 50%;
    border: 1px solid var(--text-faint);
  }
  .dot.on {
    background: #27ae60;
    border-color: #27ae60;
  }
  .name {
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .env {
    padding: 0 5px;
    border-radius: 3px;
    font-size: 10px;
    font-weight: 700;
    text-transform: uppercase;
    background: var(--bg-inset);
    color: var(--text-muted);
    flex: none;
  }
  .env.prod {
    background: var(--danger);
    color: #fff;
  }
  .kind {
    margin-left: auto;
    font-size: 11px;
    color: var(--text-faint);
    flex: none;
  }
  .more {
    width: 20px;
    height: 20px;
    padding: 0;
    border: none;
    border-radius: 4px;
    background: none;
    color: var(--text-muted);
    flex: none;
  }
  .more:hover {
    background: var(--bg-hover);
  }
  .state {
    margin: 0;
    padding: 2px 10px 4px 26px;
    font-size: 12px;
    line-height: 1.4;
    user-select: text;
    -webkit-user-select: text;
  }
  .state.error {
    color: var(--danger);
    word-break: break-word;
  }
  .notice {
    margin: 0;
    padding: 6px 12px;
    font-size: 12px;
    border-top: 1px solid var(--border);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  footer {
    padding: 8px;
    border-top: 1px solid var(--border);
    flex: none;
  }
  footer .btn {
    width: 100%;
  }
  footer kbd {
    margin-left: auto;
  }
  .menu {
    position: fixed;
    z-index: 50;
    min-width: 200px;
    padding: 4px;
    background: var(--bg);
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    box-shadow: var(--shadow);
  }
  .menu button {
    display: block;
    width: 100%;
    height: 26px;
    padding: 0 10px;
    border: none;
    border-radius: 4px;
    background: none;
    text-align: left;
    white-space: nowrap;
  }
  .menu button:hover,
  .menu button:focus-visible {
    background: var(--bg-hover);
  }
</style>
