import { describe, expect, it } from 'vitest';
import { defaultSettings } from './devBackend';
import { FIELDS, matchesQuery, SECTIONS } from './settingsSchema';

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
    const exempt = new Set(['keyboard.overrides', 'general.onboardingCompleted']);
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
});
