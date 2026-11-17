import { describe, expect, it } from 'vitest';
import { defaultSettings } from './devBackend';
import { FIELDS } from './settingsSchema';
import fixture from './settings.defaults.json';

// settings.defaults.json is written by the Rust core's own test
// (`default_settings_match_the_shared_fixture` in settings.rs), so it is what
// the core really serializes. These tests hold the TypeScript side to it.

describe('settings contract with the Rust core', () => {
  it('the TypeScript defaults are exactly what the core serializes', () => {
    expect(defaultSettings()).toEqual(fixture);
  });

  it('every select option in the UI is covered by the core-side enum test', async () => {
    // The Rust test `enum_values_offered_by_the_ui_are_accepted` lists these
    // values by hand; this fails when the UI offers one it does not list.
    const rust = (await import('../../src-tauri/src/settings.rs?raw')).default as string;
    for (const field of FIELDS) {
      if (field.kind !== 'select') continue;
      expect(rust, `${field.section}.${field.key}`).toContain(`"${field.key}"`);
      for (const option of field.options) {
        expect(rust, `${field.section}.${field.key} = ${option.value}`).toContain(
          `"${option.value}"`,
        );
      }
    }
  });
});
