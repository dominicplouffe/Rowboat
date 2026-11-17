<script lang="ts">
  // Ask (natural language → SQL) and Explain. The SQL is only ever shown and
  // inserted into the editor: running it is the user's decision.
  import { onMount } from 'svelte';
  import { ai } from '../lib/ai.svelte';
  import { streamingSql, tokenCount } from '../lib/aiText';
  import { connections } from '../lib/connections.svelte';
  import { tabs } from '../lib/tabs.svelte';
  import { ui } from '../lib/ui.svelte';
  import AiContext from './AiContext.svelte';

  let input = $state<HTMLTextAreaElement>();
  let copied = $state(false);

  const connection = $derived(connections.byId(tabs.active?.connectionId));
  const blocked = $derived(!ai.allowed(tabs.active?.connectionId ?? null));
  const live = $derived(streamingSql(ai.text));
  const sql = $derived(ai.outcome?.sql ?? live.sql);
  const explanation = $derived(ai.outcome ? ai.outcome.explanation : live.rest);
  const hasSelection = $derived(ai.target.from !== ai.target.to);

  onMount(() => {
    input?.focus();
    // Opened from a `-- comment`: the prompt is already there, so just ask.
    if (ai.task === 'ask' && ai.prompt && !blocked) void ai.submit();
  });

  function onKeydown(event: KeyboardEvent) {
    if (event.key === 'Escape') {
      event.preventDefault();
      event.stopPropagation();
      if (ai.busy) ai.cancel();
      else ai.close();
    }
  }

  function onPromptKey(event: KeyboardEvent) {
    if (event.key === 'Enter' && !event.shiftKey) {
      event.preventDefault();
      void ai.submit();
    }
  }

  async function copy() {
    if (!ai.outcome?.sql) return;
    await navigator.clipboard.writeText(ai.outcome.sql);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }

  function openAiSettings() {
    ai.close();
    ui.openSettings('ai');
  }
</script>

<svelte:window onkeydowncapture={onKeydown} />

<div
  class="backdrop"
  role="presentation"
  onpointerdown={(e) => e.target === e.currentTarget && ai.close()}
>
  <div class="panel" role="dialog" aria-modal="true" aria-labelledby="ask-title">
    <header>
      <h2 id="ask-title">{ai.task === 'ask' ? 'Ask AI' : 'Explain with AI'}</h2>
      <span class="muted">
        {#if connection}{connection.name}{:else}No connection: the AI will not know your tables{/if}
      </span>
      <span class="spacer"></span>
      <button class="btn ghost" onclick={() => ai.close()} aria-label="Close">Close</button>
    </header>

    {#if blocked}
      <p class="notice" role="alert">
        AI is turned off for this connection. Nothing is sent. You can change that in the
        connection's settings.
      </p>
    {:else if ai.problem && !ai.busy && !ai.outcome}
      <p class="notice" role="alert">
        {ai.problem}
        <button class="btn" onclick={openAiSettings}>Open AI settings</button>
      </p>
    {/if}

    {#if ai.task === 'ask'}
      <div class="ask">
        <textarea
          class="input"
          rows="2"
          bind:this={input}
          bind:value={ai.prompt}
          onkeydown={onPromptKey}
          disabled={blocked}
          placeholder="What do you want to query? For example: revenue per customer last month"
          aria-label="What do you want to query?"></textarea>
        {#if ai.busy}
          <button class="btn" onclick={() => ai.cancel()}>Cancel</button>
        {:else}
          <button
            class="btn primary"
            disabled={blocked || !ai.prompt.trim()}
            onclick={() => ai.submit()}
            title="Enter"
          >
            {ai.outcome ? 'Ask again' : 'Ask'}
          </button>
        {/if}
      </div>
    {:else if ai.subject}
      <pre class="subject">{ai.subject}</pre>
    {/if}

    <div class="answer" aria-live="polite">
      {#if ai.retrying}
        <p class="muted">The first answer did not parse ({ai.retrying}). Trying once more…</p>
      {/if}
      {#if ai.error}
        <p class="error" role="alert">{ai.error.message}</p>
      {/if}
      {#if sql}
        {#if ai.outcome?.destructive}
          <p class="badge-row">
            <span class="badge danger">Changes or removes data</span>
            {ai.outcome.destructive} Read it carefully before you run it.
          </p>
        {/if}
        {#if ai.outcome?.parseError}
          <p class="badge-row">
            <span class="badge danger">May not be valid SQL</span>
            Rowboat could not parse this, even after a retry: {ai.outcome.parseError}
          </p>
        {/if}
        <pre class="sql">{sql}</pre>
      {/if}
      {#if explanation}<p class="explanation">{explanation}</p>{/if}
      {#if ai.busy && !ai.text}
        <p class="muted">
          {ai.context ? 'Waiting for the model…' : 'Preparing the request…'}
          {#if ai.task === 'explain'}<button class="btn" onclick={() => ai.cancel()}>Cancel</button
            >{/if}
        </p>
      {/if}
    </div>

    {#if ai.outcome?.sql}
      <div class="actions">
        <button class="btn primary" onclick={() => ai.insert('insert')}>Insert into editor</button>
        <button
          class="btn"
          disabled={!hasSelection}
          title={hasSelection ? '' : 'Select text in the editor first'}
          onclick={() => ai.insert('replace')}
        >
          Replace selection
        </button>
        <button class="btn" onclick={copy}>{copied ? 'Copied' : 'Copy'}</button>
        <span class="muted">Nothing runs until you run it.</span>
      </div>
    {/if}

    <footer>
      {#if ai.context}<AiContext context={ai.context} />{/if}
      {#if ai.outcome}
        <span class="muted usage">
          {tokenCount(ai.outcome.usage.inputTokens)} tokens in · {tokenCount(
            ai.outcome.usage.outputTokens,
          )} out · {ai.outcome.model}
        </span>
      {/if}
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 150;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 9vh;
    background: rgba(8, 20, 34, 0.35);
  }
  .panel {
    display: flex;
    flex-direction: column;
    gap: 10px;
    width: min(720px, calc(100vw - 48px));
    max-height: 80vh;
    padding: 14px 16px;
    overflow: auto;
    border: 1px solid var(--border-strong);
    border-radius: 10px;
    background: var(--bg);
    box-shadow: var(--shadow);
  }
  header {
    display: flex;
    align-items: baseline;
    gap: 10px;
  }
  h2 {
    margin: 0;
    font-size: 14px;
  }
  .spacer {
    flex: 1;
  }
  .ask {
    display: flex;
    gap: 8px;
    align-items: flex-start;
  }
  textarea {
    flex: 1;
    height: auto;
    padding: 8px 10px;
    resize: vertical;
    font: inherit;
    line-height: 1.4;
  }
  .notice {
    display: flex;
    align-items: center;
    gap: 10px;
    margin: 0;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-subtle);
  }
  .answer {
    display: flex;
    flex-direction: column;
    gap: 8px;
    user-select: text;
    -webkit-user-select: text;
  }
  .answer p {
    margin: 0;
  }
  pre {
    margin: 0;
    padding: 10px 12px;
    overflow: auto;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-inset);
    font-family: var(--editor-font);
    font-size: var(--editor-font-size);
    white-space: pre-wrap;
    user-select: text;
    -webkit-user-select: text;
  }
  .subject {
    max-height: 120px;
    color: var(--text-muted);
  }
  .explanation {
    white-space: pre-wrap;
    line-height: 1.5;
  }
  .error {
    color: var(--danger);
  }
  .badge-row {
    display: flex;
    align-items: baseline;
    gap: 8px;
    flex-wrap: wrap;
  }
  .badge.danger {
    flex: none;
    padding: 1px 7px;
    border: 1px solid var(--danger);
    border-radius: 999px;
    color: var(--danger);
    font-size: 11px;
    font-weight: 600;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  footer {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }
  .usage {
    font-size: 12px;
  }
</style>
