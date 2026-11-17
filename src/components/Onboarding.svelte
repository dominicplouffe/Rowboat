<script lang="ts">
  // First-launch setup. Only the Client ID is required; everything else can
  // be skipped and changed later in Settings.
  import { ai } from '../lib/ai.svelte';
  import { connections } from '../lib/connections.svelte';
  import { dialogs } from '../lib/dialogs.svelte';
  import { asDbError, ipc } from '../lib/ipc';
  import { blankProvider, providerProblem } from '../lib/providers';
  import { ROW_LIMIT_PRESETS } from '../lib/rowLimit';
  import { settings } from '../lib/settings.svelte';
  import { tabs } from '../lib/tabs.svelte';
  import type { AiProvider, AppInfo, Theme } from '../lib/types';
  import Logo from './Logo.svelte';
  import ProviderForm from './ProviderForm.svelte';

  let { info }: { info: AppInfo | null } = $props();

  const STEPS = ['Client ID', 'AI provider', 'Look and feel', 'First connection'] as const;
  let step = $state(0);

  const general = $derived(settings.value!.general);
  let clientId = $state(settings.value!.general.clientId);
  const clientIdOk = $derived(clientId.trim().length > 0);

  // AI: start from the provider already in use, if this is a re-run.
  const existing = settings.value!.ai.providers.find(
    (p) => p.id === settings.value!.ai.defaultProvider,
  );
  let useAi = $state(true);
  let provider = $state<AiProvider>(
    existing
      ? structuredClone($state.snapshot(existing))
      : blankProvider('anthropic', crypto.randomUUID()),
  );
  let form = $state<ProviderForm>();
  let typedKey = $state(false);
  const providerReady = $derived(
    providerProblem(provider, (ai.keys[provider.id] ?? false) || typedKey) === null,
  );

  let sampleError = $state<string | null>(null);
  let working = $state(false);

  void ai.loadKeys();

  function saveClientId() {
    if (clientIdOk && clientId.trim() !== general.clientId) {
      settings.set('general', 'clientId', clientId.trim());
    }
  }

  async function saveProvider() {
    await form?.commit();
    const saved = $state.snapshot(provider);
    const list = settings.value!.ai.providers;
    settings.set(
      'ai',
      'providers',
      list.some((p) => p.id === saved.id)
        ? list.map((p) => (p.id === saved.id ? saved : p))
        : [...list, saved],
    );
    settings.set('ai', 'defaultProvider', saved.id);
  }

  async function next() {
    if (step === 0) {
      if (!clientIdOk) return;
      saveClientId();
    }
    if (step === 1 && useAi && providerReady) await saveProvider();
    step = Math.min(step + 1, STEPS.length - 1);
  }

  /** Leaves a step without keeping what was entered on it. */
  async function skip() {
    if (step === 1 && !existing) {
      // A key typed for a provider that is not kept must not stay behind.
      await ipc.aiKeySet(provider.id, '').catch(() => {});
      delete ai.keys[provider.id];
    }
    step = Math.min(step + 1, STEPS.length - 1);
  }

  function finish() {
    saveClientId();
    settings.set('general', 'onboardingCompleted', true);
  }

  function addConnection() {
    finish();
    dialogs.connection = { editing: null };
  }

  async function openSample() {
    working = true;
    sampleError = null;
    try {
      const sample = await ipc.sampleDatabase();
      await connections.load();
      await connections.open(sample.id);
      tabs.open(
        settings.value!.execution.defaultRowLimit,
        `-- The sample shop: customers, products, orders, order_items.\n-- Run this with Ctrl/⌘+Enter, or press Ctrl/⌘+K and ask for something.\n\nSELECT c.name, count(*) AS orders, round(sum(t.total), 2) AS spent\nFROM order_totals t\nJOIN customers c ON c.id = t.customer_id\nGROUP BY c.id\nORDER BY spent DESC;\n`,
        { connectionId: sample.id, title: 'Sample shop' },
      );
      finish();
    } catch (e) {
      sampleError = asDbError(e).message;
    } finally {
      working = false;
    }
  }

  function onKeydown(event: KeyboardEvent) {
    // The wizard is modal: keep app shortcuts from firing behind it.
    event.stopPropagation();
  }
</script>

<svelte:window onkeydowncapture={onKeydown} />

<div class="backdrop">
  <div class="wizard" role="dialog" aria-modal="true" aria-labelledby="wizard-title">
    <header>
      <Logo width={56} />
      <div>
        <h1 id="wizard-title">Welcome to Rowboat</h1>
        <ol class="steps" aria-label="Setup steps">
          {#each STEPS as name, i (name)}
            <li
              class:current={i === step}
              class:done={i < step}
              aria-current={i === step ? 'step' : undefined}
            >
              {i + 1}. {name}
            </li>
          {/each}
        </ol>
      </div>
    </header>

    <div class="body">
      {#if step === 0}
        <h2>Pick your Client ID</h2>
        <p>
          Rowboat identifies your sessions with this name: it is sent as
          <code>application_name</code> on PostgreSQL and as the client name on ClickHouse, so whoever
          runs the database can see who is running what.
        </p>
        <label for="wizard-client-id">Client ID</label>
        <input
          id="wizard-client-id"
          class="input"
          type="text"
          maxlength="63"
          spellcheck="false"
          bind:value={clientId}
          placeholder={info?.defaultClientId ?? 'rowboat-you'}
        />
        {#if !clientIdOk}<p class="bad" role="alert">The Client ID cannot be empty.</p>{/if}
      {:else if step === 1}
        <h2>Set up the AI copilot</h2>
        <p>
          The copilot writes, fixes and explains SQL using your schema. It never runs anything, and
          it sends table and column names, not your rows. Your key is stored in the OS keychain{info &&
          !info.secretsPersistent
            ? ' (no keychain was found on this system, so it is kept for this session only)'
            : ''}.
        </p>
        <div class="choice">
          <label class="check">
            <input type="radio" name="use-ai" checked={useAi} onchange={() => (useAi = true)} />
            Use an AI provider
          </label>
          <label class="check">
            <input type="radio" name="use-ai" checked={!useAi} onchange={() => (useAi = false)} />
            Skip — no AI
          </label>
        </div>
        {#if useAi}
          <ProviderForm bind:provider bind:this={form} bind:typedKey idPrefix="wizard" />
        {:else}
          <p class="muted">You can add a provider later in Settings → AI.</p>
        {/if}
      {:else if step === 2}
        <h2>Look and feel</h2>
        <div class="grid">
          <label for="wizard-theme">Theme</label>
          <select
            id="wizard-theme"
            class="input"
            value={settings.value!.appearance.theme}
            onchange={(e) => settings.set('appearance', 'theme', e.currentTarget.value as Theme)}
          >
            <option value="system">Match the system</option>
            <option value="light">Light</option>
            <option value="dark">Dark</option>
          </select>

          <label for="wizard-font">Editor font size</label>
          <select
            id="wizard-font"
            class="input"
            value={String(settings.value!.appearance.editorFontSize)}
            onchange={(e) =>
              settings.set('appearance', 'editorFontSize', Number(e.currentTarget.value))}
          >
            {#each [11, 12, 13, 14, 15, 16, 18, 20] as size (size)}
              <option value={String(size)}>{size} px</option>
            {/each}
            {#if ![11, 12, 13, 14, 15, 16, 18, 20].includes(settings.value!.appearance.editorFontSize)}
              <option value={String(settings.value!.appearance.editorFontSize)}>
                {settings.value!.appearance.editorFontSize} px
              </option>
            {/if}
          </select>

          <label for="wizard-limit">Default row limit</label>
          <select
            id="wizard-limit"
            class="input"
            value={String(settings.value!.execution.defaultRowLimit)}
            onchange={(e) =>
              settings.set('execution', 'defaultRowLimit', Number(e.currentTarget.value))}
          >
            {#each ROW_LIMIT_PRESETS as limit (limit)}
              <option value={String(limit)}>{limit.toLocaleString('en-US')} rows</option>
            {/each}
            {#if !(ROW_LIMIT_PRESETS as readonly number[]).includes(settings.value!.execution.defaultRowLimit)}
              <option value={String(settings.value!.execution.defaultRowLimit)}>
                {settings.value!.execution.defaultRowLimit.toLocaleString('en-US')} rows
              </option>
            {/if}
          </select>
        </div>
        <p class="muted">
          The row limit is how many rows a query fetches at first. You can change it for any tab
          from the results toolbar.
        </p>
      {:else}
        <h2>Your first connection</h2>
        <p>Connect to a database of your own, or look around a small sample first.</p>
        <div class="cards">
          <button class="card" onclick={addConnection} disabled={working}>
            <strong>Add a connection</strong>
            <span class="muted">PostgreSQL, ClickHouse or a SQLite file.</span>
          </button>
          <button class="card" onclick={openSample} disabled={working}>
            <strong>{working ? 'Opening…' : 'Open the sample database'}</strong>
            <span class="muted">A small SQLite shop: customers, products and orders.</span>
          </button>
        </div>
        {#if sampleError}<p class="bad" role="alert">{sampleError}</p>{/if}
      {/if}
    </div>

    <footer>
      {#if step > 0}<button class="btn" onclick={() => (step -= 1)} disabled={working}>Back</button
        >{/if}
      <span class="spacer"></span>
      {#if step === STEPS.length - 1}
        <button class="btn" onclick={finish} disabled={working}>Finish without a connection</button>
      {:else}
        {#if step > 0}<button class="btn ghost" onclick={skip}>Skip</button>{/if}
        <button
          class="btn primary"
          onclick={next}
          disabled={(step === 0 && !clientIdOk) || (step === 1 && useAi && !providerReady)}
          title={step === 1 && useAi && !providerReady
            ? 'Finish the provider, or choose "Skip — no AI"'
            : ''}
        >
          Next
        </button>
      {/if}
    </footer>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 120;
    display: grid;
    place-items: center;
    background: var(--bg-subtle);
  }
  .wizard {
    display: flex;
    flex-direction: column;
    width: min(680px, calc(100vw - 48px));
    max-height: calc(100vh - 48px);
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    background: var(--bg);
    box-shadow: var(--shadow);
  }
  header {
    display: flex;
    align-items: center;
    gap: 16px;
    padding: 18px 22px 14px;
    border-bottom: 1px solid var(--border);
  }
  h1 {
    margin: 0 0 6px;
    font-size: 17px;
  }
  .steps {
    display: flex;
    flex-wrap: wrap;
    gap: 4px 14px;
    margin: 0;
    padding: 0;
    list-style: none;
    font-size: 12px;
    color: var(--text-faint);
  }
  .steps .done {
    color: var(--text-muted);
  }
  .steps .current {
    color: var(--text);
    font-weight: 600;
  }
  .body {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 18px 22px;
    overflow: auto;
  }
  h2 {
    margin: 0;
    font-size: 14px;
  }
  p {
    margin: 0;
    line-height: 1.5;
  }
  code {
    font-family: var(--editor-font);
    font-size: 12px;
  }
  .choice {
    display: flex;
    gap: 20px;
  }
  .check {
    display: flex;
    align-items: center;
    gap: 6px;
  }
  .grid {
    display: grid;
    grid-template-columns: 150px minmax(0, 240px);
    gap: 8px 12px;
    align-items: center;
  }
  .grid label,
  .body > label {
    color: var(--text-muted);
  }
  .cards {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
  }
  .card {
    display: flex;
    flex-direction: column;
    gap: 4px;
    padding: 14px;
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    background: var(--bg);
    color: inherit;
    font: inherit;
    text-align: left;
    cursor: pointer;
  }
  .card:hover:not(:disabled) {
    background: var(--bg-hover);
  }
  .card:focus-visible {
    outline: 2px solid var(--focus);
    outline-offset: 1px;
  }
  .bad {
    color: var(--danger);
  }
  footer {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 12px 22px;
    border-top: 1px solid var(--border);
  }
  .spacer {
    flex: 1;
  }
</style>
