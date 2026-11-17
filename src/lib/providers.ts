// The AI providers Rowboat can talk to. The network side lives in the Rust
// core (src-tauri/src/ai); this is only what the forms need to show.

import type { AiProvider, ProviderKind } from './types';

export interface ProviderInfo {
  kind: ProviderKind;
  label: string;
  /** Shown as the placeholder; used by the core when the field is left empty. */
  defaultBaseUrl: string;
  key: 'required' | 'optional' | 'none';
  /** Where to get a key, or what to run. */
  hint: string;
}

export const PROVIDERS: ProviderInfo[] = [
  {
    kind: 'anthropic',
    label: 'Anthropic (Claude)',
    defaultBaseUrl: 'https://api.anthropic.com',
    key: 'required',
    hint: 'Create a key in the Claude Console under API keys.',
  },
  {
    kind: 'openai',
    label: 'OpenAI',
    defaultBaseUrl: 'https://api.openai.com/v1',
    key: 'required',
    hint: 'Create a key on the OpenAI platform under API keys.',
  },
  {
    kind: 'gemini',
    label: 'Google Gemini',
    defaultBaseUrl: 'https://generativelanguage.googleapis.com',
    key: 'required',
    hint: 'Create a key in Google AI Studio.',
  },
  {
    kind: 'deepseek',
    label: 'DeepSeek',
    defaultBaseUrl: 'https://api.deepseek.com/v1',
    key: 'required',
    hint: 'Create a key on the DeepSeek platform.',
  },
  {
    kind: 'mistral',
    label: 'Mistral',
    defaultBaseUrl: 'https://api.mistral.ai/v1',
    key: 'required',
    hint: 'Create a key in the Mistral console.',
  },
  {
    kind: 'xai',
    label: 'xAI (Grok)',
    defaultBaseUrl: 'https://api.x.ai/v1',
    key: 'required',
    hint: 'Create a key in the xAI console.',
  },
  {
    kind: 'openrouter',
    label: 'OpenRouter',
    defaultBaseUrl: 'https://openrouter.ai/api/v1',
    key: 'required',
    hint: 'Create a key on openrouter.ai under Keys.',
  },
  {
    kind: 'azure',
    label: 'Azure OpenAI',
    defaultBaseUrl: '',
    key: 'required',
    hint: 'Use your resource endpoint (https://NAME.openai.azure.com) and a deployment name.',
  },
  {
    kind: 'local',
    label: 'Local (Ollama / LM Studio)',
    defaultBaseUrl: 'http://localhost:11434/v1',
    key: 'none',
    hint: 'Ollama listens on port 11434. For LM Studio use http://localhost:1234/v1.',
  },
  {
    kind: 'custom',
    label: 'Custom endpoint',
    defaultBaseUrl: '',
    key: 'optional',
    hint: 'Any server that speaks the OpenAI chat completions API.',
  },
];

export function providerInfo(kind: ProviderKind): ProviderInfo {
  return PROVIDERS.find((p) => p.kind === kind) ?? PROVIDERS[0];
}

export function blankProvider(kind: ProviderKind, id: string): AiProvider {
  return {
    id,
    kind,
    name: providerInfo(kind).label,
    baseUrl: '',
    deployment: '',
    model: '',
    fixModel: '',
  };
}

/** Why this provider cannot be used yet, or null when it can. */
export function providerProblem(provider: AiProvider, hasKey: boolean): string | null {
  const info = providerInfo(provider.kind);
  if (!info.defaultBaseUrl && !provider.baseUrl.trim()) return 'Enter the base URL.';
  if (provider.kind === 'azure' && !provider.deployment.trim()) return 'Enter the deployment name.';
  if (info.key === 'required' && !hasKey) return 'Enter the API key.';
  if (!provider.model.trim()) return 'Choose a model.';
  return null;
}

/**
 * Orders a provider's model list for the picker: chat models a person would
 * want first is not something we can know, so this only drops the ones that
 * cannot write SQL (embeddings, speech, images, moderation).
 */
export function chatModels(models: string[]): string[] {
  const notChat = /embed|whisper|tts|dall-e|moderation|image|audio|rerank|transcribe|realtime/i;
  return models.filter((m) => !notChat.test(m));
}
