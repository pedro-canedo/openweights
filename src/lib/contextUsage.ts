// Estimativa do que ocupa a janela agora (~4 chars / token), até o
// llama-server expor tokenize. O raciocínio conta: é gerado no KV cache
// daquele passo, mesmo quando não volta no prompt seguinte.

export const IMAGE_TOKENS = 256;

export type UsageBucket = {
  key: string;
  color: string;
  tokens: number;
};

export type ChatUsageMessage = {
  content: string;
  reasoning?: string;
  images?: unknown[];
};

const COLORS: Record<string, string> = {
  system: "#8b909a",
  conversation: "#e8a87c",
  reasoning: "#7dcea0",
  draft: "#7eb8da",
  attachments: "#c4a7e7",
};

export function estimateTokens(text: string): number {
  if (!text) return 0;
  return Math.max(0, Math.round(text.length / 4));
}

export function collectContextBuckets(opts: {
  messages: ChatUsageMessage[];
  draft: string;
  attachments: { kind: string; data: string }[];
  systemPrompt: string;
}): UsageBucket[] {
  let conversation = 0;
  let reasoning = 0;
  for (const m of opts.messages) {
    conversation += estimateTokens(m.content);
    if (m.images) conversation += m.images.length * IMAGE_TOKENS;
    reasoning += estimateTokens(m.reasoning ?? "");
  }
  let attached = 0;
  for (const a of opts.attachments) {
    attached += a.kind === "image" ? IMAGE_TOKENS : estimateTokens(a.data);
  }
  return [
    {
      key: "system",
      color: COLORS.system,
      tokens: estimateTokens(opts.systemPrompt),
    },
    { key: "conversation", color: COLORS.conversation, tokens: conversation },
    { key: "reasoning", color: COLORS.reasoning, tokens: reasoning },
    { key: "draft", color: COLORS.draft, tokens: estimateTokens(opts.draft) },
    { key: "attachments", color: COLORS.attachments, tokens: attached },
  ].filter((b) => b.tokens > 0);
}
