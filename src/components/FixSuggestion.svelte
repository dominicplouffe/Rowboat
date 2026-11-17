<script lang="ts">
  // The AI's suggested fix for a failed statement, as a diff to accept or
  // dismiss. Accepting only changes the editor text; it does not run anything.
  import { ai } from '../lib/ai.svelte';
  import { streamingSql, tokenCount } from '../lib/aiText';
  import { changedSpan, diffLines, type DiffLine, type Segment } from '../lib/diff';
  import type { StatementRun, Tab } from '../lib/tabs.svelte';
  import AiContext from './AiContext.svelte';

  let { tab, statement }: { tab: Tab; statement: StatementRun } = $props();

  const fix = $derived(statement.fix!);
  const outcome = $derived(fix.outcome);
  let failedToApply = $state(false);

  interface Row extends DiffLine {
    segments: Segment[];
  }

  /** The diff, with a one-word change marked inside its line. */
  const rows = $derived.by((): Row[] => {
    if (!outcome?.sql) return [];
    const lines = diffLines(statement.sql.trim(), outcome.sql.trim());
    const out: Row[] = lines.map((line) => ({
      ...line,
      segments: [{ text: line.text, changed: line.kind !== 'same' }],
    }));
    for (let i = 0; i + 1 < out.length; i++) {
      const alone =
        out[i].kind === 'removed' &&
        out[i + 1].kind === 'added' &&
        out[i - 1]?.kind !== 'removed' &&
        out[i + 2]?.kind !== 'added';
      if (alone) {
        const span = changedSpan(out[i].text, out[i + 1].text);
        out[i].segments = span.before;
        out[i + 1].segments = span.after;
      }
    }
    return out;
  });
  const unchanged = $derived(rows.length > 0 && rows.every((r) => r.kind === 'same'));

  function accept() {
    failedToApply = !ai.acceptFix(tab, statement);
  }
</script>

<section class="fix" aria-label="Suggested fix">
  {#if fix.status === 'loading'}
    <p class="muted" role="status">
      Asking the AI for a fix… {streamingSql(fix.text).sql ? 'writing' : ''}
      <button class="btn" onclick={() => ai.dismissFix(statement)}>Cancel</button>
    </p>
  {:else if fix.status === 'failed'}
    <p class="muted">
      No fix was suggested: {fix.error?.message}
      <button class="btn" onclick={() => ai.suggestFix(tab, statement)}>Try again</button>
      <button class="btn ghost" onclick={() => ai.dismissFix(statement)}>Dismiss</button>
    </p>
  {:else if fix.status === 'accepted'}
    <p class="muted" role="status">The fix is in the editor. Run it when you are ready.</p>
  {:else if outcome}
    <h4>Suggested fix</h4>
    {#if outcome.explanation}<p class="why">{outcome.explanation}</p>{/if}
    {#if outcome.sql && !unchanged}
      {#if outcome.destructive}
        <p><span class="badge danger">Changes or removes data</span> {outcome.destructive}</p>
      {/if}
      {#if outcome.parseError}
        <p>
          <span class="badge danger">May not be valid SQL</span>
          Rowboat could not parse the suggestion: {outcome.parseError}
        </p>
      {/if}
      <div class="diff" role="group" aria-label="Changes">
        {#each rows as row, i (i)}
          <div class="line {row.kind}">
            <span class="sign" aria-hidden="true"
              >{row.kind === 'added' ? '+' : row.kind === 'removed' ? '−' : ' '}</span
            ><span class="sr"
              >{row.kind === 'added' ? 'added: ' : row.kind === 'removed' ? 'removed: ' : ''}</span
            >{#each row.segments as segment, j (j)}<span
                class:mark={segment.changed && row.kind !== 'same'}>{segment.text}</span
              >{/each}
          </div>
        {/each}
      </div>
      <div class="actions">
        <button class="btn primary" onclick={accept}>Accept</button>
        <button class="btn" onclick={() => ai.dismissFix(statement)}>Dismiss</button>
        <span class="muted">Accept puts the fix in the editor. It does not run it.</span>
      </div>
      {#if failedToApply}
        <p class="muted" role="alert">
          The statement is no longer in the editor as it was run, so the fix was not applied.
        </p>
      {/if}
    {:else}
      <div class="actions">
        <span class="muted">{unchanged ? 'The AI returned the statement unchanged.' : ''}</span>
        <button class="btn" onclick={() => ai.dismissFix(statement)}>Dismiss</button>
      </div>
    {/if}
    <footer>
      {#if fix.context}<AiContext context={fix.context} />{/if}
      <span class="muted">
        {tokenCount(outcome.usage.inputTokens)} tokens in · {tokenCount(outcome.usage.outputTokens)} out
        · {outcome.model}
      </span>
    </footer>
  {/if}
</section>

<style>
  .fix {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 900px;
    margin-top: 12px;
    padding: 10px 12px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg-subtle);
  }
  h4 {
    margin: 0;
    font-size: 12px;
    color: var(--text-muted);
  }
  p {
    margin: 0;
  }
  .diff {
    overflow: auto;
    max-height: 260px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
    background: var(--bg);
    font-family: var(--editor-font);
    font-size: var(--editor-font-size);
  }
  .line {
    padding: 0 8px;
    white-space: pre-wrap;
    line-height: 1.6;
  }
  .sign {
    display: inline-block;
    width: 1.4em;
    color: var(--text-faint);
  }
  .sr {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip-path: inset(50%);
  }
  /* Color is never the only cue: the +/− signs carry the same meaning. */
  .line.added {
    background: color-mix(in srgb, #2e9e5b 14%, transparent);
  }
  .line.removed {
    background: color-mix(in srgb, var(--danger) 13%, transparent);
  }
  .line.added .mark {
    background: color-mix(in srgb, #2e9e5b 34%, transparent);
    border-radius: 2px;
  }
  .line.removed .mark {
    background: color-mix(in srgb, var(--danger) 30%, transparent);
    border-radius: 2px;
    text-decoration: line-through;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .badge.danger {
    padding: 1px 7px;
    border: 1px solid var(--danger);
    border-radius: 999px;
    color: var(--danger);
    font-size: 11px;
    font-weight: 600;
  }
  footer {
    display: flex;
    flex-direction: column;
    gap: 4px;
    font-size: 12px;
  }
</style>
