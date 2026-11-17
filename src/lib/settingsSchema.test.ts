import { describe, expect, it } from 'vitest';
import { defaultSettings } from './devBackend';
import { FIELDS, matchesQuery, parseNumberField, SECTIONS } from './settingsSchema';

const defaults = defaultSettings() as unknown as Record<string, Record<string, unknown>>;

describe('settings schema', () => {
  it('has the nine sections from the spec, in order', () => {
    expect(SECTIONS.map((s) => s.title)).toEqual([
      'General',
      'Appearance',
      'Editor',
      'Execution',
      'Results',
      'Connections',
      'AI',
      'Keyboard',
      'Privacy & data',
    ]);
  });

  it('every field points at a real setting of the right type', () => {
    for (const field of FIELDS) {
      const value = defaults[field.section]?.[field.key];
      const expected = { toggle: 'boolean', select: 'string', number: 'number', text: 'string' }[
        field.kind
      ];
      expect(typeof value, `${field.section}.${field.key}`).toBe(expected);
      if (field.kind === 'select') {
        expect(field.options.map((o) => o.value)).toContain(value);
      }
      if (field.kind === 'number') {
        expect(value as number).toBeGreaterThanOrEqual(field.min);
        expect(value as number).toBeLessThanOrEqual(field.max);
      }
    }
  });

  it('every editable setting has a field', () => {
    const covered = new Set(FIELDS.map((f) => `${f.section}.${f.key}`));
    // Edited elsewhere: shortcuts in the Keyboard list, onboarding by the wizard.
    // Providers have their own editor in the AI section.
    const exempt = new Set([
      'keyboard.overrides',
      'general.onboardingCompleted',
      'ai.providers',
      'ai.defaultProvider',
    ]);
    for (const [section, values] of Object.entries(defaults)) {
      if (typeof values !== 'object') continue;
      for (const key of Object.keys(values)) {
        const id = `${section}.${key}`;
        if (!exempt.has(id)) expect(covered, id).toContain(id);
      }
    }
  });

  it('has no duplicate fields', () => {
    const ids = FIELDS.map((f) => `${f.section}.${f.key}`);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it('search matches labels, help, keywords and section names', () => {
    const find = (query: string) =>
      FIELDS.filter((f) =>
        matchesQuery(f, SECTIONS.find((s) => s.id === f.section)!.title, query),
      ).map((f) => f.key);

    expect(find('application_name')).toEqual(['clientId']);
    expect(find('row limit')).toContain('defaultRowLimit');
    expect(find('DARK')).toContain('theme');
    expect(find('ssh')).toEqual(['sshKnownHosts']);
    expect(find('zzz-nothing')).toEqual([]);
    expect(find('  ')).toHaveLength(FIELDS.length);
  });

  it('clearing a number field is never read as 0', () => {
    // 0 means "no limit" / "no timeout" / "off" for several settings.
    expect(parseNumberField('', 0, 1_000_000)).toBeNull();
    expect(parseNumberField('   ', 0, 1_000_000)).toBeNull();
    expect(parseNumberField('abc', 0, 1_000_000)).toBeNull();
  });

  it('number fields round and clamp what was typed', () => {
    expect(parseNumberField('0', 0, 1_000_000)).toBe(0);
    expect(parseNumberField(' 250 ', 0, 1_000_000)).toBe(250);
    expect(parseNumberField('12.6', 8, 32)).toBe(13);
    expect(parseNumberField('500', 8, 32)).toBe(32);
    expect(parseNumberField('-3', 1, 8)).toBe(1);
  });

  it('every setting is acted on: none is left marked as arriving later', () => {
    expect(FIELDS.filter((f) => f.arrives)).toEqual([]);
    const working = FIELDS.filter((f) => !f.arrives).map((f) => `${f.section}.${f.key}`);
    // Everything else is stored but inert until its milestone, and is shown
    // disabled. Move a key here only when the app really acts on it.
    expect(working.sort()).toEqual(
      [
        'general.clientId',
        'appearance.theme',
        'appearance.density',
        'appearance.editorFontFamily',
        'appearance.editorFontSize',
        'editor.tabSize',
        'editor.indentWithTabs',
        'editor.wordWrap',
        'editor.lineNumbers',
        'editor.autoCloseBrackets',
        'execution.defaultRowLimit',
        'execution.queryTimeoutSecs',
        'execution.autoCommit',
        'execution.stopScriptOnError',
        'execution.confirmDestructive',
        'connections.connectTimeoutSecs',
        'connections.keepaliveSecs',
        'connections.autoReconnect',
        'connections.sshKnownHosts',
        'general.restoreTabs',
        'appearance.gridFontFamily',
        'appearance.gridFontSize',
        'editor.autocomplete',
        'editor.formatterKeywordCase',
        'execution.resultMemoryCapMb',
        'results.nullText',
        'results.dateTimeFormat',
        'results.timeZone',
        'results.numberFormat',
        'results.binaryDisplay',
        'results.maxCellChars',
        'results.jsonAutoDetect',
        'general.updateChecks',
        'general.updateChannel',
        'ai.autoSuggestFixes',
        'ai.sendSampleValues',
        'ai.schemaTokenBudget',
        'ai.requestTimeoutSecs',
        'privacy.keepHistory',
        'privacy.historyRetentionDays',
      ].sort(),
    );
  });
});
