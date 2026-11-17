// The copilot's UI state: the Ask panel, suggested fixes, and which provider
// is ready. All AI work happens in the Rust core; this only sends requests
// and shows what comes back. Nothing here runs SQL.

import { commentPrompt } from './aiText';
import { connections } from './connections.svelte';
import { editorBridge } from './editorBridge';
import { asAiError, ipc } from './ipc';
import { providerProblem } from './providers';
import { statementAtCursor } from './runner';
import { settings } from './settings.svelte';
import type { FixState, StatementRun, Tab } from './tabs.svelte';
import { tabs } from './tabs.svelte';
import type { AiError, AiEvent, AiOutcome, AiProvider, IndexInfo } from './types';

type ContextEvent = Extract<AiEvent, { type: 'context' }>;

/** Editor text beyond this is not attached to an Ask request. */
const MAX_REFERENCE_CHARS = 4000;

function newId(): string {
  return crypto.randomUUID();
}

class Ai {
  /** Which configured providers have a key in the keychain. */
  keys = $state<Record<string, boolean>>({});
  #keysLoaded = false;

  open = $state(false);
  task = $state<'ask' | 'explain'>('ask');
  prompt = $state('');
  /** The answer as it streams in. */
  text = $state('');
  busy = $state(false);
  outcome = $state<AiOutcome | null>(null);
  error = $state<AiError | null>(null);
  /** Set while a second attempt runs because the first SQL did not parse. */
  retrying = $state<string | null>(null);
  context = $state<ContextEvent | null>(null);
  /** Where in the editor the panel was opened: Insert and Replace act there. */
  target = $state<{ from: number; to: number }>({ from: 0, to: 0 });
  /** The SQL being explained. */
  subject = $state('');
  index = $state<Record<string, IndexInfo | undefined>>({});
  #requestId: string | null = null;

  get provider(): AiProvider | undefined {
    const ai = settings.value?.ai;
    return ai?.providers.find((p) => p.id === ai.defaultProvider);
  }

  /** Null when the copilot can be used; otherwise what to do about it. */
  get problem(): string | null {
    const provider = this.provider;
    if (!provider) return 'Add an AI provider in Settings to use the copilot.';
    const problem = providerProblem(provider, this.keys[provider.id] ?? false);
    return problem ? `${provider.name}: ${problem.toLowerCase()}` : null;
  }

  /** Whether AI may be used with this connection (off per connection wins). */
  allowed(connectionId: string | null): boolean {
    const connection = connections.byId(connectionId);
    return !connection || connection.aiEnabled !== false;
  }

  async loadKeys(force = false): Promise<void> {
    if (this.#keysLoaded && !force) return;
    this.#keysLoaded = true;
    try {
      this.keys = await ipc.aiKeyStatus();
    } catch {
      this.keys = {};
    }
  }

  // ---- Ask and Explain ----

  /** Opens the Ask panel at the editor's cursor. */
  async openAsk(): Promise<void> {
    await this.loadKeys();
    this.#reset('ask');
    const selection = editorBridge.selection();
    this.target = { from: selection.from, to: selection.to };
    // A `-- comment` at the cursor is the prompt; the SQL goes under it.
    const text = editorBridge.text();
    const comment = selection.from === selection.to ? commentPrompt(text, selection.from) : null;
    if (comment) {
      this.prompt = comment.prompt;
      this.target = { from: comment.to, to: comment.to };
    }
    this.open = true;
  }

  /** Explains the selection, or the statement under the cursor. */
  async explain(): Promise<void> {
    const tab = tabs.active;
    if (!tab) return;
    await this.loadKeys();
    const selection = editorBridge.selection();
    const text = editorBridge.text();
    let sql = text.slice(selection.from, selection.to);
    if (!sql.trim()) {
      const kind = connections.byId(tab.connectionId)?.kind ?? 'postgres';
      const spans = await ipc.sqlSplit(kind, text);
      const [span] = statementAtCursor(spans, selection.from);
      sql = span ? text.slice(span.start, span.end) : '';
    }
    this.#reset('explain');
    this.subject = sql.trim();
    this.open = true;
    if (!this.subject) {
      this.error = { kind: 'other', message: 'There is no SQL here to explain.' };
      return;
    }
    await this.#send(tab);
  }

  async submit(): Promise<void> {
    const tab = tabs.active;
    if (!tab || this.busy || !this.prompt.trim()) return;
    await this.#send(tab);
  }

  async #send(tab: Tab): Promise<void> {
    if (this.problem) {
      this.error = { kind: 'not-configured', message: this.problem };
      return;
    }
    const id = newId();
    this.#requestId = id;
    this.busy = true;
    this.text = '';
    this.outcome = null;
    this.error = null;
    this.retrying = null;
    this.context = null;
    const reference = this.task === 'ask' ? this.#reference(tab) : this.subject;
    try {
      const outcome = await ipc.aiRun(
        {
          id,
          task: this.task,
          connectionId: tab.connectionId,
          prompt: this.task === 'ask' ? this.prompt.trim() : '',
          sql: reference,
        },
        (event) => {
          if (this.#requestId !== id) return;
          if (event.type === 'context') {
            this.context = event;
            // A retry starts a new answer.
            this.text = '';
          } else if (event.type === 'delta') this.text += event.text;
          else this.retrying = event.reason;
        },
      );
      if (this.#requestId !== id) return;
      this.outcome = outcome;
      this.text = outcome.text;
    } catch (e) {
      if (this.#requestId !== id) return;
      const error = asAiError(e);
      // A cancel the user asked for is not an error to show.
      this.error = error.kind === 'cancelled' ? null : error;
    } finally {
      if (this.#requestId === id) {
        this.busy = false;
        this.retrying = null;
      }
    }
  }

  /** What Ask sends along from the editor: the selection, else a short buffer. */
  #reference(tab: Tab): string {
    const selected = tab.text.slice(this.target.from, this.target.to);
    if (selected.trim()) return selected;
    return tab.text.length <= MAX_REFERENCE_CHARS ? tab.text : '';
  }

  cancel(): void {
    if (this.#requestId && this.busy) void ipc.aiCancel(this.#requestId);
  }

  close(): void {
    this.cancel();
    this.#requestId = null;
    this.busy = false;
    this.open = false;
    editorBridge.focus();
  }

  /** Puts the generated SQL into the editor. It is not run. */
  insert(mode: 'insert' | 'replace'): void {
    const sql = this.outcome?.sql;
    if (!sql) return;
    if (mode === 'replace') {
      editorBridge.replace(sql, this.target.from, this.target.to);
    } else {
      const at = this.target.to;
      const before = editorBridge.text().slice(0, at);
      const lead = before.length === 0 || before.endsWith('\n') ? '' : '\n';
      editorBridge.replace(`${lead}${sql}\n`, at, at);
    }
    this.close();
  }

  #reset(task: 'ask' | 'explain'): void {
    this.cancel();
    this.#requestId = null;
    this.task = task;
    this.prompt = '';
    this.text = '';
    this.subject = '';
    this.busy = false;
    this.outcome = null;
    this.error = null;
    this.retrying = null;
    this.context = null;
  }

  // ---- Fix my query ----

  /** Whether a failed statement is one the AI may see. */
  fixable(tab: Tab, statement: StatementRun): boolean {
    const kind = statement.error?.kind;
    return (kind === 'syntax' || kind === 'schema') && this.allowed(tab.connectionId);
  }

  /** Called by the runner after a failure: suggests a fix when that is on. */
  async autoFix(tab: Tab, statement: StatementRun): Promise<void> {
    if (!settings.value?.ai.autoSuggestFixes || !this.fixable(tab, statement)) return;
    await this.loadKeys();
    if (this.problem) return; // No provider: stay quiet rather than nag on every error.
    await this.suggestFix(tab, statement);
  }

  async suggestFix(tab: Tab, statement: StatementRun): Promise<void> {
    if (!statement.error || !this.fixable(tab, statement)) return;
    await this.loadKeys();
    const fix: FixState = { id: newId(), status: 'loading', text: '' };
    statement.fix = fix;
    // Read back through the proxy so later writes are reactive.
    const state = statement.fix;
    if (this.problem) {
      state.status = 'failed';
      state.error = { kind: 'not-configured', message: this.problem };
      return;
    }
    try {
      const outcome = await ipc.aiRun(
        {
          id: fix.id,
          task: 'fix',
          connectionId: tab.connectionId,
          sql: statement.sql,
          error: [statement.error.message, statement.error.detail, statement.error.hint]
            .filter(Boolean)
            .join('\n'),
          errorKind: statement.error.kind,
        },
        (event) => {
          if (event.type === 'context') {
            state.context = event;
            state.text = '';
          } else if (event.type === 'delta') state.text += event.text;
        },
      );
      if (state.status !== 'loading') return; // Dismissed while it ran.
      state.outcome = outcome;
      state.status = 'ready';
    } catch (e) {
      if (state.status !== 'loading') return;
      const error = asAiError(e);
      if (error.kind === 'cancelled') state.status = 'dismissed';
      else {
        state.error = error;
        state.status = 'failed';
      }
    }
  }

  /**
   * Replaces the failed statement in the editor with the fix. The user still
   * has to run it. Returns false when the statement is no longer in the text.
   */
  acceptFix(tab: Tab, statement: StatementRun): boolean {
    const sql = statement.fix?.outcome?.sql;
    if (!sql || !statement.fix) return false;
    let from = statement.offset;
    if (tab.text.slice(from, from + statement.sql.length) !== statement.sql) {
      // The text moved since the run; find the statement again.
      from = tab.text.indexOf(statement.sql);
      if (from < 0) return false;
    }
    editorBridge.markError(null);
    editorBridge.replace(sql, from, from + statement.sql.length);
    statement.fix.status = 'accepted';
    editorBridge.focus();
    return true;
  }

  dismissFix(statement: StatementRun): void {
    if (!statement.fix) return;
    if (statement.fix.status === 'loading') void ipc.aiCancel(statement.fix.id);
    statement.fix.status = 'dismissed';
  }

  // ---- Schema index ----

  /** Builds the connection's schema index in the background. */
  async reindex(connectionId: string, refresh: boolean): Promise<IndexInfo | null> {
    if (!this.allowed(connectionId)) return null;
    try {
      const info = await ipc.aiIndex(connectionId, refresh);
      if (info) this.index[connectionId] = info;
      return info;
    } catch {
      return null;
    }
  }
}

export const ai = new Ai();
