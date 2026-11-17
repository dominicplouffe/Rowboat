import { describe, expect, it } from 'vitest';
import { commentPrompt, streamingSql, tokenCount } from './aiText';

describe('commentPrompt', () => {
  const text = 'SELECT 1;\n\n-- orders per customer\n-- for last month\n\nSELECT 2;';

  it('uses the comment the cursor is on, joining consecutive lines', () => {
    const at = text.indexOf('orders');
    expect(commentPrompt(text, at)).toEqual({
      prompt: 'orders per customer for last month',
      from: text.indexOf('-- orders'),
      to: text.indexOf('\n\nSELECT 2'),
    });
  });

  it('uses the comment directly above an empty line', () => {
    const emptyLine = text.indexOf('\n\nSELECT 2') + 1;
    expect(commentPrompt(text, emptyLine)?.prompt).toBe('orders per customer for last month');
  });

  it('is null on SQL and on empty comments', () => {
    expect(commentPrompt(text, 3)).toBeNull();
    expect(commentPrompt('--\n', 1)).toBeNull();
    expect(commentPrompt('', 0)).toBeNull();
  });
});

describe('streamingSql', () => {
  it('shows the SQL as soon as the fence opens', () => {
    expect(streamingSql('```sql\nSELECT')).toEqual({ sql: 'SELECT', rest: '' });
    expect(streamingSql('```sq')).toEqual({ sql: '', rest: '' });
  });

  it('separates the explanation once the fence closes', () => {
    expect(streamingSql('```sql\nSELECT 1;\n```\nReturns one.')).toEqual({
      sql: 'SELECT 1;',
      rest: 'Returns one.',
    });
  });

  it('an answer with no SQL is all text', () => {
    expect(streamingSql('No such table.')).toEqual({ sql: '', rest: 'No such table.' });
  });
});

describe('tokenCount', () => {
  it('formats counts and admits when a provider gave none', () => {
    expect(tokenCount(12345)).toBe('12,345');
    expect(tokenCount(null)).toBe('?');
  });
});
