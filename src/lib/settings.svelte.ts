// Reactive copy of the settings held by the Rust core. Every change is sent to
// the core, which validates, clamps and persists it; the UI then adopts what
// the core actually stored.

import { ipc } from './ipc';
import { applyAppearance } from './theme';
import type { SectionId, Settings } from './types';

class SettingsState {
  value = $state<Settings | null>(null);
  error = $state<string | null>(null);
  #queue: Promise<void> = Promise.resolve();

  async load(): Promise<void> {
    this.adopt(await ipc.settingsGet());
  }

  /** Changes one field and persists it. */
  set<S extends SectionId, K extends keyof Settings[S]>(section: S, key: K, value: Settings[S][K]) {
    if (!this.value) return;
    this.value[section][key] = value;
    if (section === 'appearance') applyAppearance(this.value.appearance);
    this.#save();
  }

  /** `set` for callers that pick the field at runtime (the Settings window). */
  setDynamic(section: SectionId, key: string, value: unknown): void {
    this.set(section, key as never, value as never);
  }

  /** Reads a field picked at runtime. */
  getDynamic(section: SectionId, key: string): unknown {
    return (this.value?.[section] as Record<string, unknown> | undefined)?.[key];
  }

  setShortcut(actionId: string, chord: string | null): void {
    if (!this.value) return;
    const overrides = { ...this.value.keyboard.overrides };
    if (chord === null) delete overrides[actionId];
    else overrides[actionId] = chord;
    this.value.keyboard.overrides = overrides;
    this.#save();
  }

  async reset(): Promise<void> {
    await this.#run(async () => this.adopt(await ipc.settingsReset()));
  }

  /** Returns the path written, or null when cancelled. */
  async exportToFile(): Promise<string | null> {
    await this.#queue;
    return ipc.settingsExport();
  }

  /** Returns true when a file was imported. */
  async importFromFile(): Promise<boolean> {
    let imported = false;
    await this.#run(async () => {
      const next = await ipc.settingsImport();
      if (next) {
        this.adopt(next);
        imported = true;
      }
    });
    return imported;
  }

  adopt(next: Settings): void {
    this.value = next;
    applyAppearance(next.appearance);
  }

  #save(): void {
    const snapshot = $state.snapshot(this.value) as Settings;
    void this.#run(async () => {
      const stored = await ipc.settingsSet(snapshot);
      // Adopt the core's normalized copy unless newer edits are already queued.
      if (JSON.stringify($state.snapshot(this.value)) === JSON.stringify(snapshot)) {
        if (JSON.stringify(stored) !== JSON.stringify(snapshot)) this.adopt(stored);
      }
    });
  }

  #run(task: () => Promise<void>): Promise<void> {
    this.#queue = this.#queue.then(async () => {
      try {
        await task();
        this.error = null;
      } catch (e) {
        this.error = String(e);
      }
    });
    return this.#queue;
  }
}

export const settings = new SettingsState();
