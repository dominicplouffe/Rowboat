<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { SqlEditor } from '../editor/sqlEditor';
  import { connections } from '../lib/connections.svelte';
  import { editorBridge } from '../lib/editorBridge';
  import { reportReady } from '../lib/ready';
  import { complete } from '../lib/complete';
  import { errorOffset } from '../lib/runner';
  import { schemaCache } from '../lib/schemaCache.svelte';
  import { settings } from '../lib/settings.svelte';
  import { tabs } from '../lib/tabs.svelte';

  let host: HTMLDivElement;
  let editor = $state<SqlEditor | null>(null);
  let failed = $state(false);

  /** Editor settings plus what depends on the active tab. */
  function options() {
    const base = settings.value?.editor;
    if (!base) return null;
    const tab = tabs.active;
    return {
      ...base,
      readOnly: tab?.generated ?? false,
      completeOnTyping: base.autocomplete === 'typing',
      dialect: connections.byId(tab?.connectionId)?.kind ?? null,
    };
  }

  onMount(() => {
    let destroyed = false;
    // CodeMirror is its own chunk: it loads after the window is on screen.
    import('../editor/sqlEditor')
      .then(({ SqlEditor }) => {
        if (destroyed || !settings.value) return;
        editor = new SqlEditor(
          host,
          options()!,
          (tabId, text) => {
            const tab = tabs.list.find((t) => t.id === tabId);
            if (tab) tab.text = text;
          },
          {
            // Served from the in-memory schema index: no database round trip.
            complete: (text, cursor) => {
              const index = schemaCache.get(tabs.active?.connectionId);
              return index ? complete(index, text, cursor, { usage: schemaCache.usage }) : null;
            },
            accepted: (usageKey) => schemaCache.used(usageKey),
          },
        );
        editorBridge.attach(editor);
      })
      .catch(() => (failed = true))
      // Usable means you can type SQL: only now is the app "started".
      .finally(reportReady);
    return () => {
      destroyed = true;
      editorBridge.attach(null);
      editor?.destroy();
    };
  });

  // Follow the active tab.
  // Depends only on which tab is active: reading the tab's text here would
  // re-run this (and refocus the editor) on every keystroke.
  $effect(() => {
    const id = tabs.activeId;
    if (!editor) return;
    const tab = untrack(() => tabs.active);
    if (!tab || tab.id !== id) return;
    editor.show(tab.id, tab.text);
    editor.focus();
  });

  // Follow editor settings.
  $effect(() => {
    const current = options();
    if (editor && current) editor.configure(current);
  });

  // Keep the schema index for the active tab's connection loaded.
  $effect(() => {
    const connection = connections.byId(tabs.active?.connectionId);
    if (connection?.connected) void schemaCache.ensure(connection.id, connection.kind);
  });

  // Underline where the database says the error is.
  $effect(() => {
    const tab = tabs.active;
    if (!editor || !tab || tab.running) return;
    const shown = tab.runs[tab.shown];
    editor.markError(shown?.status === 'failed' ? errorOffset(shown) : null);
  });

  // Free state for closed tabs.
  let known = new Set<number>();
  $effect(() => {
    const open = new Set(tabs.list.map((t) => t.id));
    for (const id of known) if (!open.has(id)) editor?.forget(id);
    known = open;
  });
</script>

<div class="editor" bind:this={host}>
  {#if failed}
    <p class="muted">The editor failed to load.</p>
  {/if}
</div>

<style>
  .editor {
    height: 100%;
    min-height: 0;
    overflow: hidden;
    user-select: text;
    -webkit-user-select: text;
    cursor: text;
  }
  .editor :global(.cm-editor) {
    height: 100%;
  }
  p {
    padding: 12px 16px;
  }
</style>
