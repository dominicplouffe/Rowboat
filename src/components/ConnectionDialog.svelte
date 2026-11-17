<script lang="ts">
  import { onMount } from 'svelte';
  import { parseConnectionUrl } from '../lib/connectionUrl';
  import {
    blankConnection,
    connections,
    DEFAULT_PORTS,
    KIND_LABELS,
  } from '../lib/connections.svelte';
  import { dialogs } from '../lib/dialogs.svelte';
  import { asDbError, ipc } from '../lib/ipc';
  import { schema } from '../lib/schema.svelte';
  import { tabs } from '../lib/tabs.svelte';
  import type { ConnectionConfig, DbKind, FilePurpose, SecretInput } from '../lib/types';
  import Icon from './Icon.svelte';

  const editing = dialogs.connection?.editing ?? null;

  // The form works on a copy; nothing changes until Save.
  let config = $state<ConnectionConfig>(
    editing ? (structuredClone($state.snapshot(editing)) as ConnectionConfig) : blankConnection(),
  );
  // Secrets live only in these fields until they are sent to the core. An
  // untouched field (undefined) leaves the stored secret as it is.
  let password = $state<string | undefined>(undefined);
  let sshPassword = $state<string | undefined>(undefined);
  let sshPassphrase = $state<string | undefined>(undefined);

  let url = $state('');
  let urlError = $state(false);
  let busy = $state<'test' | 'save' | null>(null);
  let outcome = $state<{ ok: boolean; text: string; hint?: string | null } | null>(null);
  let nameInput: HTMLInputElement;
  let dialog: HTMLDivElement;

  const COLORS = ['', '#2d9cdb', '#27ae60', '#e8a33d', '#eb5757', '#9b51e0', '#607d8b'];
  const server = $derived(config.kind !== 'sqlite');

  onMount(() => nameInput.focus());

  function secrets(): SecretInput {
    const out: SecretInput = {};
    if (password !== undefined) out.password = password;
    if (sshPassword !== undefined) out.sshPassword = sshPassword;
    if (sshPassphrase !== undefined) out.sshPassphrase = sshPassphrase;
    return out;
  }

  function setKind(kind: DbKind) {
    if (kind === config.kind) return;
    // Keep what the user typed; move the port only if it was the old default.
    if (config.port === DEFAULT_PORTS[config.kind]) config.port = DEFAULT_PORTS[kind];
    config.tls.mode = kind === 'clickhouse' ? 'disable' : 'prefer';
    config.kind = kind;
    outcome = null;
  }

  function applyUrl() {
    const parsed = parseConnectionUrl(url);
    urlError = !parsed;
    if (!parsed) return;
    const { tls, ...rest } = parsed.config;
    Object.assign(config, rest);
    if (tls) config.tls = tls;
    if (parsed.password !== undefined) password = parsed.password;
    if (!config.name) {
      config.name =
        parsed.config.kind === 'sqlite'
          ? (config.path.split(/[\\/]/).pop() ?? '')
          : [config.database, config.host].filter(Boolean).join(' @ ');
    }
    // The pasted text may contain a password: do not leave it on screen.
    url = '';
    outcome = null;
  }

  async function browse(purpose: FilePurpose, assign: (path: string) => void) {
    const path = await ipc.pickFile(purpose).catch(() => null);
    if (path) assign(path);
  }

  async function test() {
    busy = 'test';
    outcome = null;
    try {
      const version = await connections.withHostKeyPrompt((fingerprint) =>
        ipc.connectionTest($state.snapshot(config) as ConnectionConfig, secrets(), fingerprint),
      );
      outcome = { ok: true, text: `Connected: ${version}` };
    } catch (e) {
      const error = asDbError(e);
      outcome = { ok: false, text: error.message, hint: error.hint ?? error.detail };
    } finally {
      busy = null;
    }
  }

  async function save(connect: boolean) {
    busy = 'save';
    outcome = null;
    try {
      const saved = await connections.save($state.snapshot(config) as ConnectionConfig, secrets());
      schema.forget(saved.id);
      dialogs.connection = null;
      if (connect) {
        const tab = tabs.active;
        if (tab && !tab.connectionId) tab.connectionId = saved.id;
        if (await connections.open(saved.id)) {
          schema.expanded[schema.key(saved.id, [])] = true;
          void schema.load(saved.id, []);
        }
      }
    } catch (e) {
      const error = asDbError(e);
      outcome = { ok: false, text: error.message, hint: error.hint };
    } finally {
      busy = null;
    }
  }

  async function remove() {
    if (!editing) return;
    const ok = await dialogs.confirm({
      title: `Delete “${editing.name}”?`,
      message:
        'The connection and its saved password are removed from this computer. The database itself is not touched.',
      confirmLabel: 'Delete connection',
      danger: true,
    });
    if (!ok) return;
    await connections.remove(editing.id);
    tabs.forgetConnection(editing.id);
    schema.forget(editing.id);
    dialogs.connection = null;
  }

  function onKeydown(event: KeyboardEvent) {
    if (dialogs.confirmRequest) return; // a prompt is on top
    if (event.key === 'Escape') {
      event.preventDefault();
      dialogs.connection = null;
    } else if (event.key === 'Tab') {
      const focusable = dialog.querySelectorAll<HTMLElement>(
        'button:not(:disabled), input:not(:disabled), select:not(:disabled)',
      );
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
  }
</script>

<svelte:window onkeydowncapture={onKeydown} />

<div class="backdrop" role="presentation">
  <div
    class="dialog"
    role="dialog"
    aria-modal="true"
    aria-labelledby="conn-title"
    bind:this={dialog}
  >
    <header>
      <h2 id="conn-title">{editing ? 'Edit connection' : 'New connection'}</h2>
      <button
        class="btn ghost icon"
        onclick={() => (dialogs.connection = null)}
        aria-label="Close"
        title="Close (Esc)"
      >
        <Icon name="close" />
      </button>
    </header>

    <form
      onsubmit={(e) => {
        e.preventDefault();
        void save(true);
      }}
    >
      <div class="kinds" role="radiogroup" aria-label="Database">
        {#each ['postgres', 'clickhouse', 'sqlite'] as const as kind (kind)}
          <button
            type="button"
            role="radio"
            aria-checked={config.kind === kind}
            class:active={config.kind === kind}
            onclick={() => setKind(kind)}
          >
            {KIND_LABELS[kind]}
          </button>
        {/each}
      </div>

      <label class="row paste">
        <span>Paste URL</span>
        <input
          class="input"
          class:invalid={urlError}
          type="text"
          spellcheck="false"
          autocomplete="off"
          placeholder={config.kind === 'sqlite'
            ? '/path/to/database.sqlite'
            : config.kind === 'clickhouse'
              ? 'clickhouse://… or https://…'
              : 'postgres://user:password@host:5432/database'}
          bind:value={url}
          oninput={() => (urlError = false)}
          onpaste={() => setTimeout(applyUrl)}
          onkeydown={(e) => {
            if (e.key === 'Enter') {
              e.preventDefault();
              applyUrl();
            }
          }}
        />
      </label>
      {#if urlError}<p class="field-error">
          That does not look like a PostgreSQL, ClickHouse or SQLite address.
        </p>{/if}

      <fieldset>
        <legend>General</legend>
        <label class="row">
          <span>Name</span>
          <input
            class="input"
            type="text"
            required
            maxlength="80"
            spellcheck="false"
            autocomplete="off"
            bind:this={nameInput}
            bind:value={config.name}
          />
        </label>
        <div class="row">
          <label for="conn-folder">Folder</label>
          <input
            id="conn-folder"
            class="input"
            type="text"
            list="conn-folders"
            placeholder="None"
            spellcheck="false"
            autocomplete="off"
            bind:value={config.folder}
          />
          <datalist id="conn-folders">
            {#each connections.folders as folder (folder)}<option value={folder}></option>{/each}
          </datalist>
          <label for="conn-env" class="inline">Environment</label>
          <select id="conn-env" class="input" bind:value={config.environment}>
            <option value="none">None</option>
            <option value="dev">Dev</option>
            <option value="staging">Staging</option>
            <option value="prod">Prod</option>
          </select>
        </div>
        <div class="row">
          <span id="conn-color">Color</span>
          <div class="colors" role="radiogroup" aria-labelledby="conn-color">
            {#each COLORS as color (color)}
              <button
                type="button"
                role="radio"
                aria-checked={config.color === color}
                aria-label={color || 'No color'}
                class="swatch"
                class:none={!color}
                class:active={config.color === color}
                style:background={color || undefined}
                onclick={() => (config.color = color)}
              ></button>
            {/each}
          </div>
          <label class="check">
            <input type="checkbox" bind:checked={config.readOnly} />
            Read-only
          </label>
        </div>
        {#if config.environment === 'prod'}
          <p class="note prod">
            Prod connections get a red accent and ask before DROP, TRUNCATE, ALTER, and UPDATE or
            DELETE without WHERE.
          </p>
        {/if}
      </fieldset>

      <fieldset>
        <legend>AI copilot</legend>
        <label class="check">
          <input type="checkbox" bind:checked={config.aiEnabled} />
          Allow AI for this connection
        </label>
        <p class="note">
          {#if config.aiEnabled}
            Table and column names, types, keys, indexes and comments are sent with your questions.
            Result rows are never sent.
          {:else}
            Nothing about this connection is sent to an AI provider.
          {/if}
        </p>
        {#if config.kind === 'postgres'}
          <label class="check">
            <input
              type="checkbox"
              bind:checked={config.aiSampleValues}
              disabled={!config.aiEnabled}
            />
            Also send a few common values of low-cardinality columns
          </label>
          <p class="note">
            Helps with WHERE filters (for example the values of a status column). These are real
            values from your data, read from the planner's statistics.
          </p>
        {/if}
      </fieldset>

      {#if server}
        <fieldset>
          <legend>Server</legend>
          <div class="row">
            <label for="conn-host">Host</label>
            <input
              id="conn-host"
              class="input grow"
              type="text"
              required
              spellcheck="false"
              autocomplete="off"
              bind:value={config.host}
            />
            <label for="conn-port" class="inline">Port</label>
            <input
              id="conn-port"
              class="input port"
              type="number"
              min="1"
              max="65535"
              required
              bind:value={config.port}
            />
          </div>
          <label class="row">
            <span>Database</span>
            <input
              class="input"
              type="text"
              spellcheck="false"
              autocomplete="off"
              placeholder={config.kind === 'postgres' ? 'Same as user' : 'default'}
              bind:value={config.database}
            />
          </label>
          <label class="row">
            <span>User</span>
            <input
              class="input"
              type="text"
              spellcheck="false"
              autocomplete="off"
              placeholder={config.kind === 'postgres' ? 'postgres' : 'default'}
              bind:value={config.user}
            />
          </label>
          <div class="row">
            <label for="conn-password">Password</label>
            <input
              id="conn-password"
              class="input grow"
              type="password"
              autocomplete="off"
              placeholder={editing?.hasPassword && password === undefined
                ? 'Saved in your keychain'
                : ''}
              value={password ?? ''}
              oninput={(e) => (password = e.currentTarget.value)}
            />
            {#if editing?.hasPassword && password === undefined}
              <button
                type="button"
                class="btn"
                onclick={() => (password = '')}
                title="Remove the saved password">Forget</button
              >
            {/if}
          </div>
          {#if config.kind === 'postgres'}
            <label class="check indent">
              <input type="checkbox" bind:checked={config.usePgpass} />
              Use ~/.pgpass when no password is saved
            </label>
          {/if}
        </fieldset>

        <fieldset>
          <legend>TLS</legend>
          <label class="row">
            <span>{config.kind === 'postgres' ? 'SSL mode' : 'Protocol'}</span>
            <select class="input" bind:value={config.tls.mode}>
              {#if config.kind === 'postgres'}
                <option value="disable">disable — no encryption</option>
                <option value="prefer">prefer — encrypt if the server can</option>
                <option value="require">require — encrypt, do not verify</option>
                <option value="verify-ca">verify-ca — verify the certificate</option>
                <option value="verify-full">verify-full — verify certificate and host name</option>
              {:else}
                <option value="disable">HTTP (port 8123)</option>
                <option value="verify-full">HTTPS (port 8443), verify the certificate</option>
                <option value="require">HTTPS, do not verify</option>
              {/if}
            </select>
          </label>
          {#if config.tls.mode === 'verify-ca' || config.tls.mode === 'verify-full'}
            <div class="row">
              <label for="conn-ca">CA certificate</label>
              <input
                id="conn-ca"
                class="input grow"
                type="text"
                spellcheck="false"
                placeholder="System certificates"
                bind:value={config.tls.caCertPath}
              />
              <button
                type="button"
                class="btn"
                onclick={() => browse('certificate', (p) => (config.tls.caCertPath = p))}
                >Browse…</button
              >
            </div>
          {/if}
          {#if config.kind === 'postgres' && config.tls.mode !== 'disable'}
            <div class="row">
              <label for="conn-cert">Client certificate</label>
              <input
                id="conn-cert"
                class="input grow"
                type="text"
                spellcheck="false"
                placeholder="None"
                bind:value={config.tls.clientCertPath}
              />
              <button
                type="button"
                class="btn"
                onclick={() => browse('certificate', (p) => (config.tls.clientCertPath = p))}
                >Browse…</button
              >
            </div>
            {#if config.tls.clientCertPath}
              <div class="row">
                <label for="conn-key">Client key</label>
                <input
                  id="conn-key"
                  class="input grow"
                  type="text"
                  spellcheck="false"
                  bind:value={config.tls.clientKeyPath}
                />
                <button
                  type="button"
                  class="btn"
                  onclick={() => browse('key', (p) => (config.tls.clientKeyPath = p))}
                  >Browse…</button
                >
              </div>
            {/if}
          {/if}
        </fieldset>

        <fieldset>
          <legend>
            <label class="check">
              <input type="checkbox" bind:checked={config.ssh.enabled} />
              SSH tunnel
            </label>
          </legend>
          {#if config.ssh.enabled}
            <div class="row">
              <label for="ssh-host">SSH host</label>
              <input
                id="ssh-host"
                class="input grow"
                type="text"
                spellcheck="false"
                autocomplete="off"
                bind:value={config.ssh.host}
              />
              <label for="ssh-port" class="inline">Port</label>
              <input
                id="ssh-port"
                class="input port"
                type="number"
                min="1"
                max="65535"
                bind:value={config.ssh.port}
              />
            </div>
            <div class="row">
              <label for="ssh-user">SSH user</label>
              <input
                id="ssh-user"
                class="input grow"
                type="text"
                spellcheck="false"
                autocomplete="off"
                bind:value={config.ssh.user}
              />
              <label for="ssh-auth" class="inline">Sign in with</label>
              <select id="ssh-auth" class="input" bind:value={config.ssh.auth}>
                <option value="agent">SSH agent</option>
                <option value="key">Private key</option>
                <option value="password">Password</option>
              </select>
            </div>
            {#if config.ssh.auth === 'password'}
              <label class="row">
                <span>SSH password</span>
                <input
                  class="input"
                  type="password"
                  autocomplete="off"
                  placeholder={editing?.hasSshPassword && sshPassword === undefined
                    ? 'Saved in your keychain'
                    : ''}
                  value={sshPassword ?? ''}
                  oninput={(e) => (sshPassword = e.currentTarget.value)}
                />
              </label>
            {:else if config.ssh.auth === 'key'}
              <div class="row">
                <label for="ssh-key">Private key</label>
                <input
                  id="ssh-key"
                  class="input grow"
                  type="text"
                  spellcheck="false"
                  placeholder="~/.ssh/id_ed25519"
                  bind:value={config.ssh.keyPath}
                />
                <button
                  type="button"
                  class="btn"
                  onclick={() => browse('key', (p) => (config.ssh.keyPath = p))}>Browse…</button
                >
              </div>
              <label class="row">
                <span>Passphrase</span>
                <input
                  class="input"
                  type="password"
                  autocomplete="off"
                  placeholder={editing?.hasSshPassphrase && sshPassphrase === undefined
                    ? 'Saved in your keychain'
                    : 'If the key has one'}
                  value={sshPassphrase ?? ''}
                  oninput={(e) => (sshPassphrase = e.currentTarget.value)}
                />
              </label>
            {/if}
            <p class="note">
              Host and port above are as seen from the SSH server. Host keys are checked against
              ~/.ssh/known_hosts.
            </p>
          {/if}
        </fieldset>
      {:else}
        <fieldset>
          <legend>Database file</legend>
          <div class="row">
            <label for="conn-path">File</label>
            <input
              id="conn-path"
              class="input grow"
              type="text"
              required
              spellcheck="false"
              autocomplete="off"
              bind:value={config.path}
            />
            <button
              type="button"
              class="btn"
              onclick={() =>
                browse('sqlite-open', (p) => ((config.path = p), (config.create = false)))}
              >Open…</button
            >
            <button
              type="button"
              class="btn"
              onclick={() =>
                browse('sqlite-new', (p) => ((config.path = p), (config.create = true)))}
              >New…</button
            >
          </div>
          <label class="check indent">
            <input type="checkbox" bind:checked={config.create} disabled={config.readOnly} />
            Create the file if it does not exist
          </label>
        </fieldset>
      {/if}

      {#if outcome}
        <div class="outcome" class:ok={outcome.ok} role={outcome.ok ? 'status' : 'alert'}>
          <strong>{outcome.text}</strong>
          {#if outcome.hint}<span>{outcome.hint}</span>{/if}
        </div>
      {/if}

      <footer>
        {#if editing}
          <button type="button" class="btn delete" onclick={remove} disabled={busy !== null}
            >Delete</button
          >
        {/if}
        <span class="spacer"></span>
        <button type="button" class="btn" onclick={test} disabled={busy !== null}>
          {busy === 'test' ? 'Testing…' : 'Test'}
        </button>
        <button type="button" class="btn" onclick={() => save(false)} disabled={busy !== null}
          >Save</button
        >
        <button type="submit" class="btn primary" disabled={busy !== null}>
          {busy === 'save' ? 'Saving…' : 'Save & connect'}
        </button>
      </footer>
    </form>
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
    flex-direction: column;
    width: min(620px, calc(100vw - 48px));
    max-height: calc(100vh - 48px);
    background: var(--bg);
    border: 1px solid var(--border-strong);
    border-radius: 12px;
    box-shadow: var(--shadow);
    overflow: hidden;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 12px 12px 8px 20px;
    flex: none;
  }
  h2 {
    margin: 0;
    font-size: 15px;
  }
  form {
    display: flex;
    flex-direction: column;
    gap: 10px;
    padding: 0 20px;
    overflow-y: auto;
  }
  .kinds {
    display: flex;
    padding: 2px;
    border-radius: 8px;
    background: var(--bg-inset);
  }
  .kinds button {
    flex: 1;
    height: 26px;
    border: none;
    border-radius: 6px;
    background: none;
  }
  .kinds button.active {
    background: var(--bg);
    font-weight: 600;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.15);
  }
  fieldset {
    margin: 0;
    padding: 8px 12px 10px;
    border: 1px solid var(--border);
    border-radius: 8px;
    min-width: 0;
  }
  legend {
    padding: 0 6px;
    font-size: 11px;
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
    color: var(--text-muted);
  }
  legend .check {
    text-transform: none;
    letter-spacing: 0;
    font-size: 13px;
    color: var(--text);
  }
  .row {
    display: flex;
    align-items: center;
    gap: 8px;
    margin-top: 6px;
  }
  .row > span:first-child,
  .row > label:first-child {
    width: 104px;
    flex: none;
    color: var(--text-muted);
  }
  .row .input {
    flex: 1;
    min-width: 0;
  }
  .row .input.port {
    flex: none;
    width: 82px;
  }
  .row select.input {
    flex: 1;
  }
  .inline {
    color: var(--text-muted);
    flex: none;
  }
  .paste {
    margin-top: 0;
    padding: 0 12px;
  }
  .input.invalid {
    border-color: var(--danger);
  }
  .field-error {
    margin: -4px 0 0 124px;
    font-size: 12px;
    color: var(--danger);
  }
  .check {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    white-space: nowrap;
  }
  .check.indent {
    margin: 8px 0 0 112px;
  }
  .colors {
    display: flex;
    gap: 6px;
    flex: 1;
  }
  .swatch {
    width: 18px;
    height: 18px;
    padding: 0;
    border: 1px solid var(--border-strong);
    border-radius: 50%;
  }
  .swatch.none {
    background:
      linear-gradient(
        45deg,
        transparent 46%,
        var(--text-faint) 46%,
        var(--text-faint) 54%,
        transparent 54%
      ),
      var(--bg);
  }
  .swatch.active {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  .note {
    margin: 8px 0 0;
    font-size: 12px;
    line-height: 1.45;
    color: var(--text-muted);
  }
  .note.prod {
    color: var(--danger);
  }
  .outcome {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 8px 12px;
    border-radius: 8px;
    border: 1px solid var(--danger);
    color: var(--danger);
    font-size: 12px;
    user-select: text;
    -webkit-user-select: text;
    word-break: break-word;
  }
  .outcome.ok {
    border-color: var(--border-strong);
    color: var(--text);
  }
  .outcome span {
    color: var(--text-muted);
  }
  footer {
    position: sticky;
    bottom: 0;
    display: flex;
    gap: 8px;
    padding: 10px 0 16px;
    background: var(--bg);
  }
  .spacer {
    flex: 1;
  }
  .btn.delete {
    color: var(--danger);
  }
</style>
