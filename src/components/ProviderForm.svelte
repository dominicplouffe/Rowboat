<script lang="ts">
  // One AI provider: where it is, its key, and which model to use. The key
  // goes to the OS keychain through the core and is never read back.
  import { ai } from '../lib/ai.svelte';
  import { asAiError, ipc } from '../lib/ipc';
  import { chatModels, providerInfo, providerProblem, PROVIDERS } from '../lib/providers';
  import type { AiProvider, ProviderKind } from '../lib/types';

  let {
    provider = $bindable(),
    idPrefix = 'provider',
    typedKey = $bindable(false),
  }: {
    provider: AiProvider;
    idPrefix?: string;
    /** True while a key is typed but not yet stored. */
    typedKey?: boolean;
  } = $props();

  let key = $state('');
  let models = $state<string[] | null>(null);
  let busy = $state<'models' | 'test' | null>(null);
  let message = $state<{ ok: boolean; text: string } | null>(null);

  const info = $derived(providerInfo(provider.kind));
  const hasKey = $derived(ai.keys[provider.id] ?? false);
  const problem = $derived(providerProblem(provider, hasKey || key.trim().length > 0));

  $effect(() => {
    typedKey = key.trim().length > 0;
  });

  function setKind(kind: ProviderKind) {
    const previous = providerInfo(provider.kind).label;
    provider.kind = kind;
    // Keep a name the user typed; replace one we filled in.
    if (!provider.name || provider.name === previous) provider.name = providerInfo(kind).label;
    provider.baseUrl = '';
    provider.model = '';
    provider.fixModel = '';
    models = null;
    message = null;
  }

  /** Stores a typed key before anything that needs it. */
  async function saveKey(): Promise<void> {
    if (!key.trim()) return;
    await ipc.aiKeySet(provider.id, key.trim());
    ai.keys[provider.id] = true;
    key = '';
  }

  async function removeKey() {
    await ipc.aiKeySet(provider.id, '');
    ai.keys[provider.id] = false;
    models = null;
    message = null;
  }

  async function loadModels() {
    busy = 'models';
    message = null;
    try {
      await saveKey();
      const all = await ipc.aiModels($state.snapshot(provider));
      models = chatModels(all);
      if (models.length === 0) {
        message = { ok: false, text: 'The provider returned no models. Type the model name.' };
      } else if (!provider.model) {
        // Nothing chosen yet: the user picks. We do not guess a default model.
        message = { ok: true, text: `${models.length} models found. Choose one.` };
      }
    } catch (e) {
      models = null;
      message = { ok: false, text: asAiError(e).message };
    } finally {
      busy = null;
    }
  }

  async function test() {
    busy = 'test';
    message = null;
    try {
      await saveKey();
      const reply = await ipc.aiTest($state.snapshot(provider));
      message = { ok: true, text: `It works. The model replied: ${reply || '(empty)'}` };
    } catch (e) {
      message = { ok: false, text: asAiError(e).message };
    } finally {
      busy = null;
    }
  }

  /** For the parent: store a key that was typed but not yet used. */
  export async function commit(): Promise<void> {
    await saveKey();
  }
</script>

<div class="form">
  <label for="{idPrefix}-kind">Provider</label>
  <select
    id="{idPrefix}-kind"
    class="input"
    value={provider.kind}
    onchange={(e) => setKind(e.currentTarget.value as ProviderKind)}
  >
    {#each PROVIDERS as option (option.kind)}
      <option value={option.kind}>{option.label}</option>
    {/each}
  </select>

  <label for="{idPrefix}-name">Name</label>
  <input id="{idPrefix}-name" class="input" type="text" bind:value={provider.name} />

  {#if info.key !== 'none'}
    <label for="{idPrefix}-key">API key{info.key === 'optional' ? ' (optional)' : ''}</label>
    <div class="row">
      <input
        id="{idPrefix}-key"
        class="input grow"
        type="password"
        autocomplete="off"
        spellcheck="false"
        bind:value={key}
        placeholder={hasKey ? 'Saved in your keychain. Type to replace it.' : 'Paste the key'}
      />
      {#if hasKey}<button class="btn" onclick={removeKey}>Remove key</button>{/if}
    </div>
  {/if}

  <label for="{idPrefix}-url">Base URL</label>
  <input
    id="{idPrefix}-url"
    class="input"
    type="text"
    spellcheck="false"
    bind:value={provider.baseUrl}
    placeholder={info.defaultBaseUrl || 'https://…'}
  />

  {#if provider.kind === 'azure'}
    <label for="{idPrefix}-deployment">Deployment</label>
    <input
      id="{idPrefix}-deployment"
      class="input"
      type="text"
      spellcheck="false"
      bind:value={provider.deployment}
      oninput={() => (provider.model = provider.deployment)}
    />
  {/if}

  <span></span>
  <p class="muted hint">{info.hint}</p>

  <label for="{idPrefix}-model">Model</label>
  <div class="row">
    {#if models && models.length > 0}
      <select id="{idPrefix}-model" class="input grow" bind:value={provider.model}>
        <option value="">Choose a model…</option>
        {#if provider.model && !models.includes(provider.model)}
          <option value={provider.model}>{provider.model}</option>
        {/if}
        {#each models as model (model)}<option value={model}>{model}</option>{/each}
      </select>
    {:else}
      <input
        id="{idPrefix}-model"
        class="input grow"
        type="text"
        spellcheck="false"
        bind:value={provider.model}
        placeholder="Load the list, or type a model id"
      />
    {/if}
    <button class="btn" disabled={busy !== null} onclick={loadModels}>
      {busy === 'models' ? 'Loading…' : models ? 'Reload models' : 'Load models'}
    </button>
  </div>

  <label for="{idPrefix}-fix">Model for fixes</label>
  {#if models && models.length > 0}
    <select id="{idPrefix}-fix" class="input" bind:value={provider.fixModel}>
      <option value="">Same as above</option>
      {#if provider.fixModel && !models.includes(provider.fixModel)}
        <option value={provider.fixModel}>{provider.fixModel}</option>
      {/if}
      {#each models as model (model)}<option value={model}>{model}</option>{/each}
    </select>
  {:else}
    <input
      id="{idPrefix}-fix"
      class="input"
      type="text"
      spellcheck="false"
      bind:value={provider.fixModel}
      placeholder="Same as above (a cheaper model works well for fixes)"
    />
  {/if}

  <span></span>
  <div class="row">
    <button class="btn" disabled={busy !== null || problem !== null} onclick={test}>
      {busy === 'test' ? 'Testing…' : 'Test'}
    </button>
    {#if message}
      <span class={message.ok ? 'ok' : 'bad'} role="status">{message.text}</span>
    {:else if problem}
      <span class="muted">{problem}</span>
    {/if}
  </div>
</div>

<style>
  .form {
    display: grid;
    grid-template-columns: 130px minmax(0, 1fr);
    gap: 8px 12px;
    align-items: center;
  }
  label {
    color: var(--text-muted);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    min-width: 0;
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .hint {
    margin: 0;
    font-size: 12px;
  }
  .ok {
    color: var(--text);
  }
  .bad {
    color: var(--danger);
  }
  .ok,
  .bad {
    user-select: text;
    -webkit-user-select: text;
  }
</style>
