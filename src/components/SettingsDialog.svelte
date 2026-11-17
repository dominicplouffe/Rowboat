<script lang="ts">
  import { onMount } from 'svelte';
  import { ACTIONS } from '../lib/actions';
  import { settings } from '../lib/settings.svelte';
  import {
    FIELDS,
    matchesQuery,
    MILESTONE_NAMES,
    parseNumberField,
    SECTIONS,
    type Field,
  } from '../lib/settingsSchema';
  import { chordFor, chordFromEvent, formatChord, isBindable } from '../lib/shortcuts';
  import { ipc } from '../lib/ipc';
  import type { AppInfo, SectionId } from '../lib/types';
  import { ui } from '../lib/ui.svelte';
  import Icon from './Icon.svelte';
  import Logo from './Logo.svelte';

  let { info }: { info: AppInfo | null } = $props();

  let query = $state('');
  let searchInput: HTMLInputElement;
  let dialog: HTMLDivElement;
  let notice = $state<string | null>(null);
  let confirmingReset = $state(false);

  // Keyboard rebinding
  let capturing = $state<string | null>(null);
  let captureError = $state<string | null>(null);

  const overrides = $derived(settings.value?.keyboard.overrides ?? {});
  const searching = $derived(query.trim().length > 0);

  const visibleSections = $derived(
    SECTIONS.map((section) => ({
      section,
      fields: FIELDS.filter(
        (f) => f.section === section.id && matchesQuery(f, section.title, query),
      ),
      actions:
        section.id === 'keyboard'
          ? ACTIONS.filter((a) =>
              query
                .toLowerCase()
                .split(/\s+/)
                .filter(Boolean)
                .every((w) => `${a.label} ${a.group} keyboard shortcut`.toLowerCase().includes(w)),
            )
          : [],
    })).filter(({ section, fields, actions }) =>
      searching ? fields.length + actions.length > 0 : section.id === ui.settingsSection,
    ),
  );

  onMount(() => {
    const previous = document.activeElement as HTMLElement | null;
    searchInput.focus();
    return () => previous?.focus?.();
  });

  function close() {
    ui.settingsOpen = false;
  }

  function onKeydown(event: KeyboardEvent) {
    if (capturing) {
      captureShortcut(event);
      return;
    }
    if (event.key === 'Escape') {
      event.preventDefault();
      if (query) query = '';
      else close();
      return;
    }
    if (event.key === 'Tab') trapFocus(event);
  }

  function trapFocus(event: KeyboardEvent) {
    const focusable = dialog.querySelectorAll<HTMLElement>(
      'button:not(:disabled), input, select, [tabindex]:not([tabindex="-1"])',
    );
    if (focusable.length === 0) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (event.shiftKey && document.activeElement === first) {
      event.preventDefault();
      last.focus();
    } else if (!event.shiftKey && document.activeElement === last) {
      event.preventDefault();
      first.focus();
    }
  }

  function captureShortcut(event: KeyboardEvent) {
    event.preventDefault();
    event.stopPropagation();
    if (event.key === 'Escape') {
      capturing = null;
      captureError = null;
      return;
    }
    const chord = chordFromEvent(event);
    if (!chord) return; // Still holding modifiers.
    if (!isBindable(chord)) {
      captureError = 'Use a combination with ⌘/Ctrl or Alt, or a function key.';
      return;
    }
    const clash = ACTIONS.find((a) => a.id !== capturing && chordFor(a, overrides) === chord);
    if (clash) {
      captureError = `${formatChord(chord)} is already used by “${clash.label}”.`;
      return;
    }
    const action = ACTIONS.find((a) => a.id === capturing)!;
    settings.setShortcut(action.id, chord === action.chord ? null : chord);
    capturing = null;
    captureError = null;
  }

  function setNumber(field: Field & { kind: 'number' }, input: HTMLInputElement) {
    const value = parseNumberField(input.value, field.min, field.max);
    if (value === null) {
      // Empty or not a number: put the stored value back rather than guess.
      input.value = String(settings.getDynamic(field.section, field.key));
      return;
    }
    input.value = String(value);
    settings.setDynamic(field.section, field.key, value);
  }

  function setText(field: Field, input: HTMLInputElement) {
    settings.setDynamic(field.section, field.key, input.value.trim());
  }

  async function exportSettings() {
    try {
      const path = await settings.exportToFile();
      if (path) notice = `Exported to ${path}`;
    } catch (e) {
      notice = `Export failed: ${e}`;
    }
  }

  async function importSettings() {
    const imported = await settings.importFromFile();
    if (imported) notice = 'Settings imported.';
    else if (settings.error) notice = `Import failed: ${settings.error}`;
  }

  async function resetAll() {
    confirmingReset = false;
    await settings.reset();
    notice = 'Settings reset to defaults. Your Client ID was kept.';
  }

  function selectSection(id: SectionId) {
    query = '';
    notice = null;
    confirmingReset = false;
    ui.settingsSection = id;
  }
</script>

<svelte:window onkeydowncapture={onKeydown} />

<div
  class="backdrop"
  role="presentation"
  onpointerdown={(e) => e.target === e.currentTarget && close()}
>
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Settings" bind:this={dialog}>
    <nav aria-label="Settings sections">
      <label class="search">
        <Icon name="search" size={14} />
        <input
          bind:this={searchInput}
          bind:value={query}
          type="search"
          placeholder="Search settings"
          aria-label="Search settings"
          spellcheck="false"
        />
      </label>
      <ul>
        {#each SECTIONS as section (section.id)}
          <li>
            <button
              class:active={!searching && ui.settingsSection === section.id}
              aria-current={!searching && ui.settingsSection === section.id ? 'page' : undefined}
              onclick={() => selectSection(section.id)}
            >
              {section.title}
            </button>
          </li>
        {/each}
      </ul>
    </nav>

    <div class="content">
      <button
        class="btn ghost icon dismiss"
        onclick={close}
        aria-label="Close settings"
        title="Close (Esc)"
      >
        <Icon name="close" />
      </button>

      {#if visibleSections.length === 0}
        <p class="none muted">No settings match “{query.trim()}”.</p>
      {/if}

      {#each visibleSections as { section, fields, actions } (section.id)}
        <section aria-labelledby="settings-{section.id}">
          <h2 id="settings-{section.id}">{section.title}</h2>
          {#if !searching}<p class="blurb muted">{section.blurb}</p>{/if}

          {#each fields as field (field.key)}
            {@const id = `set-${field.section}-${field.key}`}
            {@const value = settings.getDynamic(field.section, field.key)}
            <div class="field">
              <div class="text">
                <label for={id}>{field.label}</label>
                {#if field.arrives}
                  <span class="badge" title={`Takes effect with ${MILESTONE_NAMES[field.arrives]}`}>
                    Arrives in {field.arrives}
                  </span>
                {/if}
                {#if field.help}<p class="muted">{field.help}</p>{/if}
              </div>
              <div class="control">
                {#if field.kind === 'toggle'}
                  <input
                    {id}
                    class="switch"
                    disabled={!!field.arrives}
                    type="checkbox"
                    role="switch"
                    checked={value as boolean}
                    onchange={(e) =>
                      settings.setDynamic(field.section, field.key, e.currentTarget.checked)}
                  />
                {:else if field.kind === 'select'}
                  <select
                    {id}
                    class="input"
                    disabled={!!field.arrives}
                    value={value as string}
                    onchange={(e) =>
                      settings.setDynamic(field.section, field.key, e.currentTarget.value)}
                  >
                    {#each field.options as option (option.value)}
                      <option value={option.value}>{option.label}</option>
                    {/each}
                  </select>
                {:else if field.kind === 'number'}
                  <input
                    {id}
                    class="input number"
                    disabled={!!field.arrives}
                    type="number"
                    min={field.min}
                    max={field.max}
                    step={field.step ?? 1}
                    value={value as number}
                    onchange={(e) => setNumber(field, e.currentTarget)}
                  />
                  {#if field.unit}<span class="unit muted">{field.unit}</span>{/if}
                {:else}
                  <input
                    {id}
                    class="input text"
                    disabled={!!field.arrives}
                    type="text"
                    spellcheck="false"
                    autocomplete="off"
                    maxlength={field.maxLength}
                    placeholder={field.placeholder}
                    value={value as string}
                    onchange={(e) => setText(field, e.currentTarget)}
                  />
                {/if}
              </div>
            </div>
          {/each}

          {#if section.id === 'keyboard'}
            {#each actions as action (action.id)}
              {@const chord = chordFor(action, overrides)}
              <div class="field">
                <div class="text">
                  <span class="label">{action.label}</span>
                  <p class="muted">{action.group}</p>
                </div>
                <div class="control">
                  {#if capturing === action.id}
                    <span class="capturing" role="status"
                      >Press a shortcut… <kbd>Esc</kbd> cancels</span
                    >
                  {:else}
                    <button
                      class="btn chord"
                      title="Click, then press the new shortcut"
                      aria-label={`${action.label}: ${formatChord(chord)}. Change shortcut`}
                      onclick={() => {
                        capturing = action.id;
                        captureError = null;
                      }}
                    >
                      {formatChord(chord)}
                    </button>
                    {#if action.id in overrides}
                      <button
                        class="btn ghost icon"
                        title={`Reset to ${formatChord(action.chord)}`}
                        aria-label={`Reset shortcut for ${action.label}`}
                        onclick={() => settings.setShortcut(action.id, null)}
                      >
                        <Icon name="reset" />
                      </button>
                    {/if}
                  {/if}
                </div>
              </div>
              {#if capturing === action.id && captureError}
                <p class="error" role="alert">{captureError}</p>
              {/if}
            {/each}
          {/if}

          {#if !searching && section.id === 'ai'}
            <p class="note muted">
              Providers, API keys and per-task models arrive with the AI copilot (M3). Keys will be
              stored in your OS keychain, never in the settings file.
            </p>
          {/if}

          {#if !searching && section.id === 'privacy'}
            <div class="field">
              <div class="text">
                <span class="label">Settings file</span>
                <p class="muted path">{info?.settingsPath ?? ''}</p>
                <p class="muted">Plain JSON with no secrets, so it is safe to export and share.</p>
              </div>
            </div>
            <div class="buttons">
              <button class="btn" onclick={exportSettings}>Export settings…</button>
              <button class="btn" onclick={importSettings}>Import settings…</button>
              <button
                class="btn"
                onclick={() => ipc.openDataFolder().catch((e) => (notice = String(e)))}
              >
                Open data folder
              </button>
              {#if confirmingReset}
                <button class="btn danger" onclick={resetAll}>Really reset everything</button>
                <button class="btn ghost" onclick={() => (confirmingReset = false)}>Cancel</button>
              {:else}
                <button class="btn" onclick={() => (confirmingReset = true)}
                  >Reset all settings…</button
                >
              {/if}
            </div>
            <p class="note muted">
              Rowboat has no telemetry and no crash reporting. Nothing leaves this computer unless
              you run a query or ask the AI.
            </p>
          {/if}

          {#if !searching && section.id === 'general'}
            <div class="about">
              <Logo width={72} />
              <div>
                <strong>Rowboat {info?.version ?? ''}</strong>
                <p class="muted">
                  A fast SQL client with an AI copilot that knows your schema.<br />
                  MIT License · © 2026 Dominic Plouffe
                </p>
              </div>
            </div>
          {/if}
        </section>
      {/each}

      {#if notice}<p class="notice" role="status">{notice}</p>{/if}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 100;
    display: grid;
    place-items: center;
    background: rgba(8, 20, 34, 0.42);
  }
  .dialog {
    display: flex;
    width: min(860px, calc(100vw - 48px));
    height: min(620px, calc(100vh - 48px));
    background: var(--bg);
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  nav {
    width: 212px;
    flex: none;
    padding: 12px 8px;
    background: var(--bg-subtle);
    border-right: 1px solid var(--border);
    overflow-y: auto;
  }
  .search {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 28px;
    padding: 0 8px;
    margin-bottom: 10px;
    border: 1px solid var(--border-strong);
    border-radius: var(--radius);
    background: var(--bg);
    color: var(--text-muted);
  }
  .search:focus-within {
    outline: 2px solid var(--focus);
    outline-offset: -1px;
  }
  .search input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: none;
    user-select: text;
    -webkit-user-select: text;
    -webkit-appearance: none;
    appearance: none;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  nav button {
    width: 100%;
    height: 28px;
    padding: 0 10px;
    border: none;
    border-radius: var(--radius);
    background: none;
    text-align: left;
  }
  nav button:hover {
    background: var(--bg-hover);
  }
  nav button.active {
    background: var(--bg-active);
    font-weight: 600;
  }
  .content {
    position: relative;
    flex: 1;
    min-width: 0;
    padding: 20px 28px 28px;
    overflow-y: auto;
  }
  .dismiss {
    position: absolute;
    top: 12px;
    right: 12px;
  }
  h2 {
    margin: 0 0 2px;
    font-size: 18px;
    font-weight: 700;
  }
  section + section {
    margin-top: 28px;
  }
  .blurb {
    margin: 0 0 10px;
  }
  .field {
    display: flex;
    align-items: center;
    gap: 24px;
    padding: 10px 0;
    border-top: 1px solid var(--border);
  }
  .text {
    flex: 1;
    min-width: 0;
  }
  .text label,
  .text .label {
    font-weight: 600;
  }
  .text p {
    margin: 2px 0 0;
    font-size: 12px;
    line-height: 1.45;
  }
  .path {
    user-select: text;
    -webkit-user-select: text;
    word-break: break-all;
    font-family: var(--editor-font);
  }
  .badge {
    margin-left: 6px;
    padding: 1px 6px;
    border: 1px solid var(--border-strong);
    border-radius: 8px;
    font-size: 10px;
    font-weight: 600;
    color: var(--text-muted);
    white-space: nowrap;
  }
  .field :disabled {
    opacity: 0.5;
  }
  .control {
    display: flex;
    align-items: center;
    gap: 6px;
    flex: none;
  }
  .input.number {
    width: 92px;
    text-align: right;
  }
  .input.text {
    width: 220px;
  }
  select.input {
    padding-right: 4px;
    max-width: 240px;
  }
  .unit {
    min-width: 48px;
    font-size: 12px;
  }
  .switch {
    appearance: none;
    -webkit-appearance: none;
    position: relative;
    width: 34px;
    height: 20px;
    margin: 0;
    border-radius: 10px;
    background: var(--border-strong);
    transition: background 0.12s;
  }
  .switch::after {
    content: '';
    position: absolute;
    top: 2px;
    left: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.3);
    transition: transform 0.12s;
  }
  .switch:checked {
    background: var(--accent);
  }
  .switch:checked::after {
    transform: translateX(14px);
  }
  .chord {
    min-width: 76px;
    justify-content: center;
    font-variant-numeric: tabular-nums;
  }
  .capturing {
    color: var(--accent);
    font-weight: 600;
  }
  .error {
    margin: 0 0 8px;
    color: var(--danger);
    font-size: 12px;
    text-align: right;
  }
  .note {
    margin: 12px 0 0;
    padding-top: 12px;
    border-top: 1px solid var(--border);
    font-size: 12px;
    line-height: 1.5;
  }
  .buttons {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    padding: 4px 0 4px;
  }
  .btn.danger {
    border-color: var(--danger);
    color: var(--danger);
  }
  .about {
    display: flex;
    align-items: center;
    gap: 16px;
    margin-top: 12px;
    padding-top: 16px;
    border-top: 1px solid var(--border);
  }
  .about p {
    margin: 2px 0 0;
    font-size: 12px;
    line-height: 1.5;
  }
  .none {
    margin: 48px 0;
    text-align: center;
  }
  .notice {
    margin: 16px 0 0;
    padding: 8px 10px;
    border-radius: var(--radius);
    background: var(--bg-inset);
    font-size: 12px;
    user-select: text;
    -webkit-user-select: text;
    word-break: break-all;
  }
</style>
