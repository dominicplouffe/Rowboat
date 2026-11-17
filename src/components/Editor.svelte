<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import type { SqlEditor } from '../editor/sqlEditor';
  import { reportReady } from '../lib/ready';
  import { settings } from '../lib/settings.svelte';
  import { tabs } from '../lib/tabs.svelte';

  let host: HTMLDivElement;
  let editor = $state<SqlEditor | null>(null);
  let failed = $state(false);

  onMount(() => {
    let destroyed = false;
    // CodeMirror is its own chunk: it loads after the window is on screen.
    import('../editor/sqlEditor')
      .then(({ SqlEditor }) => {
        if (destroyed || !settings.value) return;
        editor = new SqlEditor(host, { ...settings.value.editor }, (tabId, text) => {
          const tab = tabs.list.find((t) => t.id === tabId);
          if (tab) tab.text = text;
        });
      })
      .catch(() => (failed = true))
      // Usable means you can type SQL: only now is the app "started".
      .finally(reportReady);
    return () => {
      destroyed = true;
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
    const options = settings.value?.editor;
    if (editor && options) editor.configure({ ...options });
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
