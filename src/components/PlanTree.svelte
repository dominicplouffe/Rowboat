<script lang="ts">
  // Shows an EXPLAIN result as a tree. PostgreSQL returns JSON, SQLite returns
  // rows with parent ids, ClickHouse returns indented text.
  import type { DbKind, RunResult } from '../lib/types';

  let { kind, result }: { kind: DbKind; result: RunResult } = $props();

  interface PlanNode {
    title: string;
    detail: string;
    children: PlanNode[];
  }

  interface PgNode {
    'Node Type': string;
    'Relation Name'?: string;
    Alias?: string;
    'Index Name'?: string;
    'Join Type'?: string;
    'Startup Cost'?: number;
    'Total Cost'?: number;
    'Plan Rows'?: number;
    Filter?: string;
    'Index Cond'?: string;
    'Hash Cond'?: string;
    Plans?: PgNode[];
  }

  function fromPostgres(node: PgNode): PlanNode {
    const subject = [
      node['Join Type'] && `${node['Join Type']} join`,
      node['Index Name'] && `using ${node['Index Name']}`,
      node['Relation Name'] &&
        `on ${node['Relation Name']}${node.Alias && node.Alias !== node['Relation Name'] ? ` ${node.Alias}` : ''}`,
    ].filter(Boolean);
    const facts = [
      node['Total Cost'] !== undefined && `cost ${node['Startup Cost']}..${node['Total Cost']}`,
      node['Plan Rows'] !== undefined && `~${node['Plan Rows'].toLocaleString('en-US')} rows`,
      node['Index Cond'] && `index cond ${node['Index Cond']}`,
      node['Hash Cond'] && `hash cond ${node['Hash Cond']}`,
      node.Filter && `filter ${node.Filter}`,
    ].filter(Boolean);
    return {
      title: [node['Node Type'], ...subject].join(' '),
      detail: facts.join(' · '),
      children: (node.Plans ?? []).map(fromPostgres),
    };
  }

  const tree = $derived.by((): PlanNode[] | null => {
    try {
      if (kind === 'postgres') {
        const plans = JSON.parse(result.rows[0]?.[0] ?? '') as { Plan: PgNode }[];
        return plans.map((p) => fromPostgres(p.Plan));
      }
      if (kind === 'sqlite') {
        // Columns: id, parent, notused, detail.
        // Local to this computation; nothing reactive reads it.
        // eslint-disable-next-line svelte/prefer-svelte-reactivity
        const nodes = new Map<string, PlanNode>();
        const roots: PlanNode[] = [];
        for (const row of result.rows) {
          const node: PlanNode = { title: row[3] ?? '', detail: '', children: [] };
          nodes.set(row[0] ?? '', node);
          const parent = nodes.get(row[1] ?? '');
          if (parent) parent.children.push(node);
          else roots.push(node);
        }
        return roots;
      }
    } catch {
      // Fall through to plain text.
    }
    return null;
  });

  const text = $derived(result.rows.map((row) => row[0] ?? '').join('\n'));
</script>

{#snippet branch(nodes: PlanNode[], depth: number)}
  {#each nodes as node, i (i)}
    <div class="node" style:padding-left="{depth * 20 + 12}px">
      <span class="title">{depth > 0 ? '└ ' : ''}{node.title}</span>
      {#if node.detail}<span class="detail">{node.detail}</span>{/if}
    </div>
    {@render branch(node.children, depth + 1)}
  {/each}
{/snippet}

<div class="plan" role="tree" aria-label="Query plan">
  {#if tree}
    {@render branch(tree, 0)}
  {:else}
    <pre>{text}</pre>
  {/if}
</div>

<style>
  .plan {
    flex: 1;
    min-height: 0;
    overflow: auto;
    padding: 10px 0;
    user-select: text;
    -webkit-user-select: text;
  }
  .node {
    display: flex;
    flex-wrap: wrap;
    gap: 2px 12px;
    padding-top: 3px;
    padding-bottom: 3px;
    font: 12px/1.5 var(--editor-font);
  }
  .node:hover {
    background: var(--bg-hover);
  }
  .title {
    font-weight: 700;
    white-space: pre;
  }
  .detail {
    color: var(--text-muted);
  }
  pre {
    margin: 0;
    padding: 0 14px;
    font: 12px/1.5 var(--editor-font);
  }
</style>
