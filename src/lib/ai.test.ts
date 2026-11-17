import { beforeEach, describe, expect, it } from 'vitest';
import { ai } from './ai.svelte';
import { blankConnection, connections } from './connections.svelte';
import { editorBridge } from './editorBridge';
import { ipc } from './ipc';
import { blankProvider } from './providers';
import { prompts, run } from './runner';
import { settings } from './settings.svelte';
import { tabs, type Tab } from './tabs.svelte';

/** A stand-in for the CodeMirror editor that edits the tab's text. */
function attachEditor(tab: Tab, selection = { from: 0, to: 0 }) {
  editorBridge.attach({
    selection: () => selection,
    markError: () => {},
    focus: () => {},
    text: () => tab.text,
    replace: (insert, from = 0, to = tab.text.length) => {
      tab.text = tab.text.slice(0, from) + insert + tab.text.slice(to);
    },
  });
}

async function addProvider(withKey = true) {
  const provider = { ...blankProvider('openai', 'p1'), model: 'openai-large' };
  settings.set('ai', 'providers', [provider]);
  settings.set('ai', 'defaultProvider', 'p1');
  await ipc.aiKeySet('p1', withKey ? 'sk-test' : '');
  await ai.loadKeys(true);
}

describe('the copilot', () => {
  let tab: Tab;

  beforeEach(async () => {
    await ipc.settingsReset();
    await settings.load();
    settings.set('general', 'onboardingCompleted', true);
    for (const t of [...tabs.list]) tabs.close(t.id);
    for (const c of [...connections.list]) await connections.remove(c.id);
    const saved = await connections.save({ ...blankConnection(), name: 'dev' }, {});
    tab = tabs.open(1000, '', { connectionId: saved.id });
    attachEditor(tab);
    prompts.failed = (t, statement) => void ai.autoFix(t, statement);
    ai.close();
    await addProvider();
  });

  it('says what is missing instead of calling a provider that is not set up', async () => {
    settings.set('ai', 'providers', []);
    settings.set('ai', 'defaultProvider', '');
    expect(ai.problem).toMatch(/Add an AI provider/);

    await addProvider(false);
    expect(ai.problem).toMatch(/enter the api key/i);
    await ai.openAsk();
    ai.prompt = 'revenue per customer';
    await ai.submit();
    expect(ai.error?.kind).toBe('not-configured');
    expect(ai.outcome).toBeNull();
  });

  it('Ask streams an answer and Insert puts the SQL in the editor without running it', async () => {
    tab.text = 'SELECT 1;';
    attachEditor(tab, { from: 9, to: 9 });
    await ai.openAsk();
    ai.prompt = 'revenue per customer';
    await ai.submit();

    expect(ai.error).toBeNull();
    expect(ai.outcome?.sql).toMatch(/^SELECT c\.name/);
    expect(ai.outcome?.explanation).toBe('Total order value per customer, highest first.');
    expect(ai.context?.request.messages[0].content).toBe('revenue per customer');
    expect(ai.outcome?.usage).toEqual({ inputTokens: 180, outputTokens: 42 });

    ai.insert('insert');
    expect(tab.text.startsWith('SELECT 1;\nSELECT c.name')).toBe(true);
    expect(tab.runs).toEqual([]); // nothing ran
    expect(ai.open).toBe(false);
  });

  it('a -- comment at the cursor becomes the prompt and the SQL lands under it', async () => {
    tab.text = '-- revenue per customer\n';
    attachEditor(tab, { from: 5, to: 5 });
    await ai.openAsk();
    expect(ai.prompt).toBe('revenue per customer');
    await ai.submit();
    ai.insert('insert');
    expect(tab.text).toMatch(/^-- revenue per customer\nSELECT c\.name/);
  });

  it('Replace swaps the selection for the generated SQL', async () => {
    tab.text = 'SELECT old; SELECT keep;';
    attachEditor(tab, { from: 0, to: 11 });
    await ai.openAsk();
    ai.prompt = 'revenue per customer';
    await ai.submit();
    ai.insert('replace');
    expect(tab.text.endsWith(' SELECT keep;')).toBe(true);
    expect(tab.text.startsWith('SELECT c.name')).toBe(true);
  });

  it('destructive SQL comes back flagged', async () => {
    await ai.openAsk();
    ai.prompt = 'delete the empty orders';
    await ai.submit();
    expect(ai.outcome?.destructive).toBeTruthy();
  });

  it('an answer with no SQL offers nothing to insert', async () => {
    await ai.openAsk();
    ai.prompt = 'nothing useful';
    await ai.submit();
    expect(ai.outcome?.sql).toBeNull();
    ai.insert('insert');
    expect(tab.text).toBe('');
  });

  it('cancel stops a running request without showing an error', async () => {
    await ai.openAsk();
    ai.prompt = 'something slow';
    const pending = ai.submit();
    await new Promise((r) => setTimeout(r, 200));
    expect(ai.busy).toBe(true);
    ai.cancel();
    await pending;
    expect(ai.busy).toBe(false);
    expect(ai.error).toBeNull();
    expect(ai.outcome).toBeNull();
  });

  it('Explain sends the statement under the cursor', async () => {
    tab.text = 'select 1; select * from orders';
    attachEditor(tab, { from: 14, to: 14 });
    await ai.explain();
    expect(ai.subject).toBe('select * from orders');
    expect(ai.outcome?.sql).toBeNull();
    expect(ai.outcome?.explanation).toMatch(/Returns every row/);
  });

  it('a syntax error gets a suggested fix; accepting it edits the text and runs nothing', async () => {
    tab.text = 'select 1;\nselect * fromm orders';
    await run(tab, { from: 12, to: 12 }, 'statement');
    const failed = tab.runs[0];
    expect(failed.status).toBe('failed');
    await expect.poll(() => failed.fix?.status).toBe('ready');
    expect(failed.fix?.outcome?.sql).toBe('select * FROM orders');
    expect(failed.fix?.outcome?.explanation).toBe('FROM was misspelled.');

    expect(ai.acceptFix(tab, failed)).toBe(true);
    expect(tab.text).toBe('select 1;\nselect * FROM orders');
    expect(failed.fix?.status).toBe('accepted');
    expect(tab.runs).toHaveLength(1); // still only the failed run
  });

  it('accepting a fix after the text changed does not overwrite something else', async () => {
    tab.text = 'select * fromm orders';
    await run(tab, { from: 0, to: 0 }, 'statement');
    await expect.poll(() => tab.runs[0].fix?.status).toBe('ready');
    tab.text = 'select 42';
    expect(ai.acceptFix(tab, tab.runs[0])).toBe(false);
    expect(tab.text).toBe('select 42');
  });

  it('with automatic fixes off nothing is sent until asked', async () => {
    settings.set('ai', 'autoSuggestFixes', false);
    tab.text = 'select * fromm orders';
    await run(tab, { from: 0, to: 0 }, 'statement');
    await new Promise((r) => setTimeout(r, 50));
    expect(tab.runs[0].fix).toBeUndefined();

    await ai.suggestFix(tab, tab.runs[0]);
    expect(tab.runs[0].fix?.status).toBe('ready');
  });

  it('connection errors are never sent for a fix', async () => {
    tab.text = 'select lost_connection';
    await run(tab, { from: 0, to: 0 }, 'statement');
    await new Promise((r) => setTimeout(r, 50));
    expect(tab.runs[0].error?.kind).toBe('connection');
    expect(ai.fixable(tab, tab.runs[0])).toBe(false);
    expect(tab.runs[0].fix).toBeUndefined();
  });

  it('a connection with AI turned off gets no fixes and no questions', async () => {
    const connection = JSON.parse(JSON.stringify(connections.byId(tab.connectionId)));
    await connections.save({ ...connection, aiEnabled: false }, {});
    tab.text = 'select * fromm orders';
    await run(tab, { from: 0, to: 0 }, 'statement');
    await new Promise((r) => setTimeout(r, 50));
    expect(tab.runs[0].fix).toBeUndefined();
    expect(ai.allowed(tab.connectionId)).toBe(false);
  });

  it('dismissing a fix hides it', async () => {
    tab.text = 'select * fromm orders';
    await run(tab, { from: 0, to: 0 }, 'statement');
    await expect.poll(() => tab.runs[0].fix?.status).toBe('ready');
    ai.dismissFix(tab.runs[0]);
    expect(tab.runs[0].fix?.status).toBe('dismissed');
  });
});
