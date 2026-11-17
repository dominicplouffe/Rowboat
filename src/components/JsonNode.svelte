<script lang="ts">
  // One value in the JSON tree; renders itself for children.
  import { untrack } from 'svelte';
  import JsonNode from './JsonNode.svelte';

  let {
    name,
    value,
    path,
    depth,
    search,
    onpick,
  }: {
    name: string | null;
    value: unknown;
    /** JSON path to this value, e.g. `$.items[0].id`. */
    path: string;
    depth: number;
    search: string;
    onpick: (path: string, value: unknown) => void;
  } = $props();

  const isArray = $derived(Array.isArray(value));
  const isObject = $derived(value !== null && typeof value === 'object');
  const entries = $derived(
    isObject
      ? isArray
        ? (value as unknown[]).map((v, i) => [String(i), v] as const)
        : Object.entries(value as Record<string, unknown>)
      : [],
  );
  // Deep levels start collapsed so large documents stay readable.
  let open = $state(untrack(() => depth < 2));

  const childPath = (key: string) =>
    isArray
      ? `${path}[${key}]`
      : /^[A-Za-z_]\w*$/.test(key)
        ? `${path}.${key}`
        : `${path}[${JSON.stringify(key)}]`;

  const text = $derived(isObject ? '' : JSON.stringify(value));
  const matches = $derived(
    !!search &&
      ((name ?? '').toLowerCase().includes(search) ||
        (!isObject && text.toLowerCase().includes(search))),
  );
  /** True when something below this node matches the search. */
  const containsMatch = $derived(
    !!search && isObject && JSON.stringify(value).toLowerCase().includes(search),
  );
  $effect(() => {
    if (containsMatch) open = true;
  });
</script>

<div class="node" style:padding-left="{depth * 14}px">
  {#if isObject}
    <button
      class="toggle"
      aria-expanded={open}
      aria-label={open ? 'Collapse' : 'Expand'}
      onclick={() => (open = !open)}
    >
      {open ? '▾' : '▸'}
    </button>
  {:else}
    <span class="toggle"></span>
  {/if}
  <button
    class="line"
    class:match={matches}
    title="Click to copy this value's path"
    onclick={() => onpick(path, value)}
  >
    {#if name !== null}<span class="key">{name}</span><span class="colon">: </span>{/if}
    {#if isObject}
      <span class="muted">
        {isArray ? `[${entries.length}]` : `{${entries.length}}`}
      </span>
    {:else}
      <span class={value === null ? 'null' : typeof value}>{text}</span>
    {/if}
  </button>
</div>
{#if isObject && open}
  {#each entries as [key, child] (key)}
    <JsonNode name={key} value={child} path={childPath(key)} depth={depth + 1} {search} {onpick} />
  {/each}
{/if}

<style>
  .node {
    display: flex;
    align-items: flex-start;
    font: 12px/1.6 var(--editor-font);
  }
  .toggle {
    width: 14px;
    flex: none;
    padding: 0;
    border: none;
    background: none;
    color: var(--text-faint);
    font: inherit;
  }
  .line {
    flex: 1;
    min-width: 0;
    padding: 0 4px;
    border: none;
    border-radius: 3px;
    background: none;
    text-align: left;
    font: inherit;
    white-space: pre-wrap;
    word-break: break-word;
    user-select: text;
    -webkit-user-select: text;
  }
  .line:hover {
    background: var(--bg-hover);
  }
  .line.match {
    background: rgba(232, 163, 61, 0.35);
  }
  .key {
    color: var(--syn-keyword);
  }
  .colon,
  .muted {
    color: var(--text-faint);
  }
  .string {
    color: var(--syn-string);
  }
  .number,
  .boolean {
    color: var(--syn-number);
  }
  .null {
    color: var(--text-faint);
    font-style: italic;
  }
</style>
