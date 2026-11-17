<script lang="ts">
  // One level of the schema navigator; renders itself for expanded children.
  import { schema } from '../lib/schema.svelte';
  import type { SchemaNode } from '../lib/types';
  import Icon from './Icon.svelte';
  import SchemaNodes from './SchemaNodes.svelte';

  let {
    connectionId,
    path,
    depth,
    filter,
    onmenu,
    onactivate,
  }: {
    connectionId: string;
    path: string[];
    depth: number;
    filter: string;
    onmenu: (event: MouseEvent, node: SchemaNode, path: string[]) => void;
    onactivate: (node: SchemaNode, path: string[]) => void;
  } = $props();

  const branch = $derived(schema.branch(connectionId, path));

  /** With a filter, a node shows if it or any loaded descendant matches. */
  function visible(node: SchemaNode, nodePath: string[]): boolean {
    if (!filter) return true;
    if (node.name.toLowerCase().includes(filter)) return true;
    const children = schema.branch(connectionId, nodePath);
    return children?.nodes.some((child) => visible(child, [...nodePath, child.name])) ?? false;
  }

  const GLYPH: Record<string, string> = {
    table: 'T',
    view: 'V',
    matview: 'M',
    function: 'ƒ',
    sequence: '#',
    type: 'τ',
    dictionary: 'D',
    index: 'I',
    trigger: '⚡',
  };
</script>

{#if branch?.status === 'failed'}
  <p class="state error" style:padding-left="{depth * 14 + 26}px">{branch.error}</p>
{:else if branch?.status === 'loading' && branch.nodes.length === 0}
  <p class="state muted" style:padding-left="{depth * 14 + 26}px">Loading…</p>
{:else if branch?.status === 'ready' && branch.nodes.length === 0}
  <p class="state muted" style:padding-left="{depth * 14 + 26}px">Empty</p>
{/if}

{#each branch?.nodes ?? [] as node (node.name)}
  {@const nodePath = [...path, node.name]}
  {@const open = schema.isExpanded(connectionId, nodePath)}
  {#if visible(node, nodePath)}
    <div
      class="node {node.kind}"
      role="treeitem"
      aria-expanded={node.hasChildren ? open : undefined}
      aria-selected="false"
      tabindex="-1"
      style:padding-left="{depth * 14 + 8}px"
      onclick={() => node.hasChildren && schema.toggle(connectionId, nodePath)}
      ondblclick={() => onactivate(node, nodePath)}
      oncontextmenu={(e) => {
        e.preventDefault();
        onmenu(e, node, nodePath);
      }}
      onkeydown={(e) => {
        if (e.key === 'Enter') onactivate(node, nodePath);
        else if (e.key === ' ' && node.hasChildren) void schema.toggle(connectionId, nodePath);
      }}
    >
      <span class="chevron" class:open>
        {#if node.hasChildren}<Icon name="chevron" size={10} />{/if}
      </span>
      {#if GLYPH[node.kind]}<span class="glyph" aria-hidden="true">{GLYPH[node.kind]}</span>{/if}
      <span class="name">{node.name}</span>
      {#if node.detail}<span class="detail" title={node.detail}>{node.detail}</span>{/if}
      {#if node.kind !== 'group' && node.kind !== 'schema' && node.kind !== 'database'}
        <button
          class="more"
          aria-label="Actions for {node.name}"
          title="Actions"
          onclick={(e) => {
            e.stopPropagation();
            onmenu(e, node, nodePath);
          }}>⋯</button
        >
      {/if}
    </div>
    {#if node.hasChildren && open}
      <SchemaNodes
        {connectionId}
        path={nodePath}
        depth={depth + 1}
        {filter}
        {onmenu}
        {onactivate}
      />
    {/if}
  {/if}
{/each}

<style>
  .node {
    display: flex;
    align-items: center;
    gap: 5px;
    height: 24px;
    padding-right: 4px;
    white-space: nowrap;
    outline-offset: -2px;
  }
  .node:hover {
    background: var(--bg-hover);
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
  .glyph {
    width: 14px;
    flex: none;
    text-align: center;
    font-size: 10px;
    font-weight: 700;
    color: var(--accent);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .group > .name {
    color: var(--text-muted);
  }
  .column > .name {
    font-family: var(--editor-font);
    font-size: 12px;
  }
  .detail {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: 11px;
    color: var(--text-faint);
  }
  .more {
    margin-left: auto;
    width: 20px;
    height: 20px;
    padding: 0;
    border: none;
    border-radius: 4px;
    background: none;
    color: var(--text-muted);
    visibility: hidden;
    flex: none;
  }
  .node:hover .more,
  .more:focus-visible {
    visibility: visible;
  }
  .more:hover {
    background: var(--bg-hover);
  }
  .state {
    margin: 0;
    height: 22px;
    line-height: 22px;
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .state.error {
    color: var(--danger);
  }
</style>
