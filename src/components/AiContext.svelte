<script lang="ts">
  // "View context": the exact payload that was sent to the provider.
  import type { AiEvent } from '../lib/types';

  let { context }: { context: Extract<AiEvent, { type: 'context' }> } = $props();
</script>

<details class="context">
  <summary>
    View context
    <span class="muted">
      — sent to {context.provider} ({context.request.model}){#if context.tablesTotal > 0}, {context.tablesSent}
        of {context.tablesTotal} tables described{/if}
    </span>
  </summary>
  <p class="muted">This is everything the request contained. Result rows are never part of it.</p>
  <h4>System</h4>
  <pre>{context.request.system}</pre>
  {#each context.request.messages as message, i (i)}
    <h4>{message.role === 'user' ? 'You' : 'Model'}</h4>
    <pre>{message.content}</pre>
  {/each}
</details>

<style>
  .context {
    font-size: 12px;
  }
  summary {
    cursor: pointer;
    color: var(--text-muted);
  }
  h4 {
    margin: 8px 0 2px;
    font-size: 11px;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--text-faint);
  }
  pre {
    margin: 0;
    padding: 8px;
    max-height: 220px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-inset);
    font-family: var(--editor-font);
    font-size: 12px;
    white-space: pre-wrap;
    user-select: text;
    -webkit-user-select: text;
  }
  p {
    margin: 6px 0 0;
  }
</style>
