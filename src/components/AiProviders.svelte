<script lang="ts">
  // Settings → AI: the configured providers. Several can exist; one is used.
  import { onMount } from 'svelte';
  import { ai } from '../lib/ai.svelte';
  import { ipc } from '../lib/ipc';
  import { blankProvider, providerInfo, providerProblem } from '../lib/providers';
  import { settings } from '../lib/settings.svelte';
  import type { AiProvider } from '../lib/types';
  import ProviderForm from './ProviderForm.svelte';

  const providers = $derived(settings.value?.ai.providers ?? []);
  const defaultId = $derived(settings.value?.ai.defaultProvider ?? '');

  let draft = $state<AiProvider | null>(null);
  let adding = $state(false);
  let form = $state<ProviderForm>();

  onMount(() => void ai.loadKeys(true));

  function add() {
    draft = blankProvider('anthropic', crypto.randomUUID());
    adding = true;
  }

  function edit(provider: AiProvider) {
    draft = structuredClone($state.snapshot(provider));
    adding = false;
  }

  async function save() {
    if (!draft) return;
    await form?.commit();
    const saved = $state.snapshot(draft);
    const next = adding
      ? [...providers, saved]
      : providers.map((p) => (p.id === saved.id ? saved : p));
    settings.set('ai', 'providers', next);
    if (!defaultId) settings.set('ai', 'defaultProvider', saved.id);
    draft = null;
  }

  async function cancel() {
    // A key typed for a provider that was never saved must not stay behind.
    if (draft && adding) {
      await ipc.aiKeySet(draft.id, '').catch(() => {});
      delete ai.keys[draft.id];
    }
    draft = null;
  }

  async function remove(provider: AiProvider) {
    settings.set(
      'ai',
      'providers',
      providers.filter((p) => p.id !== provider.id),
    );
    if (defaultId === provider.id) {
      settings.set('ai', 'defaultProvider', providers.find((p) => p.id !== provider.id)?.id ?? '');
    }
    await ipc.aiKeySet(provider.id, '').catch(() => {});
    delete ai.keys[provider.id];
  }
</script>

<div class="providers">
  <div class="head">
    <div>
      <span class="label">Providers</span>
      <p class="muted">
        API keys are stored in your OS keychain, never in the settings file. The provider marked "In
        use" answers every request.
      </p>
    </div>
    {#if !draft}<button class="btn" onclick={add}>Add provider</button>{/if}
  </div>

  {#if providers.length === 0 && !draft}
    <p class="muted empty">No provider yet. The copilot stays off until you add one.</p>
  {/if}

  {#each providers as provider (provider.id)}
    {#if draft && !adding && draft.id === provider.id}
      <div class="card editing">
        <ProviderForm bind:provider={draft} bind:this={form} idPrefix="edit" />
        <div class="buttons">
          <button class="btn primary" onclick={save}>Save</button>
          <button class="btn" onclick={cancel}>Cancel</button>
        </div>
      </div>
    {:else}
      {@const problem = providerProblem(provider, ai.keys[provider.id] ?? false)}
      <div class="card">
        <label class="use">
          <input
            type="radio"
            name="default-provider"
            checked={defaultId === provider.id}
            onchange={() => settings.set('ai', 'defaultProvider', provider.id)}
          />
          <span class="name">{provider.name || providerInfo(provider.kind).label}</span>
          {#if defaultId === provider.id}<span class="badge">In use</span>{/if}
        </label>
        <span class="muted detail">
          {providerInfo(provider.kind).label} · {provider.model || 'no model chosen'}
          {#if provider.fixModel}· fixes: {provider.fixModel}{/if}
          {#if problem}· <span class="bad">{problem}</span>{/if}
        </span>
        <span class="spacer"></span>
        <button class="btn" disabled={draft !== null} onclick={() => edit(provider)}>Edit</button>
        <button class="btn" disabled={draft !== null} onclick={() => remove(provider)}
          >Remove</button
        >
      </div>
    {/if}
  {/each}

  {#if draft && adding}
    <div class="card editing">
      <ProviderForm bind:provider={draft} bind:this={form} idPrefix="add" />
      <div class="buttons">
        <button class="btn primary" onclick={save}>Add</button>
        <button class="btn" onclick={cancel}>Cancel</button>
      </div>
    </div>
  {/if}
</div>

<style>
  .providers {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px 0;
    border-bottom: 1px solid var(--border);
  }
  .head {
    display: flex;
    align-items: flex-start;
    justify-content: space-between;
    gap: 16px;
  }
  .label {
    font-weight: 600;
  }
  p {
    margin: 2px 0 0;
  }
  .card {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border: 1px solid var(--border);
    border-radius: var(--radius);
  }
  .card.editing {
    flex-direction: column;
    align-items: stretch;
    background: var(--bg-subtle);
  }
  .use {
    display: flex;
    align-items: center;
    gap: 8px;
  }
  .name {
    font-weight: 600;
  }
  .badge {
    padding: 0 6px;
    border: 1px solid var(--border-strong);
    border-radius: 999px;
    font-size: 11px;
  }
  .detail {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .spacer {
    flex: 1;
  }
  .bad {
    color: var(--danger);
  }
  .buttons {
    display: flex;
    gap: 8px;
    justify-content: flex-end;
  }
</style>
