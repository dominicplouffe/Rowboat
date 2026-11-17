import { describe, expect, it } from 'vitest';
import { blankProvider, chatModels, PROVIDERS, providerProblem } from './providers';

describe('AI providers', () => {
  it('offers every provider in the spec, DeepSeek included', () => {
    expect(PROVIDERS.map((p) => p.kind).sort()).toEqual(
      [
        'anthropic',
        'azure',
        'custom',
        'deepseek',
        'gemini',
        'local',
        'mistral',
        'openai',
        'openrouter',
        'xai',
      ].sort(),
    );
  });

  it('says what is still missing before a provider can be used', () => {
    const openai = blankProvider('openai', 'p1');
    expect(providerProblem(openai, false)).toBe('Enter the API key.');
    expect(providerProblem(openai, true)).toBe('Choose a model.');
    expect(providerProblem({ ...openai, model: 'gpt' }, true)).toBeNull();
  });

  it('local servers need no key; Azure and custom endpoints need a URL', () => {
    expect(providerProblem({ ...blankProvider('local', 'p'), model: 'qwen' }, false)).toBeNull();
    expect(providerProblem(blankProvider('custom', 'p'), false)).toBe('Enter the base URL.');
    const azure = { ...blankProvider('azure', 'p'), baseUrl: 'https://x.openai.azure.com' };
    expect(providerProblem(azure, true)).toBe('Enter the deployment name.');
  });

  it('leaves non-chat models out of the picker', () => {
    expect(chatModels(['gpt-x', 'text-embedding-3', 'whisper-1', 'claude-y'])).toEqual([
      'gpt-x',
      'claude-y',
    ]);
  });
});
