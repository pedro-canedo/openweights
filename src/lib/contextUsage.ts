// Estimativa do que ocupa a janela agora (~4 chars / token), até o
// llama-server expor tokenize. O raciocínio conta: é gerado no KV cache
// daquele passo, mesmo quando não volta no prompt seguinte.

import type { RunView } from "./agent/runStore";

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

/** Fatia do run do agente que entra na janela e não está nas bolhas do chat. */
export type AgentUsageSlice = {
  steps: { text: string; reasoning: string }[];
  tools: { argsJson: string; resultPreview: string; output: string }[];
  focusMd: string;
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

/** Só enquanto o run está vivo: depois o texto já foi (ou será) parar nas mensagens. */
export function sliceFromRun(
  run: RunView | undefined,
  active: boolean,
): AgentUsageSlice | null {
  if (!run || !active) return null;
  const steps: AgentUsageSlice["steps"] = [];
  const tools: AgentUsageSlice["tools"] = [];
  for (const item of run.items) {
    if (item.kind === "step") {
      steps.push({ text: item.text, reasoning: item.reasoning });
    } else if (item.kind === "tool") {
      const t = run.tools[item.id];
      if (!t) continue;
      tools.push({
        argsJson: t.argsJson,
        resultPreview: t.resultPreview,
        output: t.output,
      });
    }
  }
  return { steps, tools, focusMd: run.focusMd };
}

export function collectContextBuckets(opts: {
  messages: ChatUsageMessage[];
  draft: string;
  attachments: { kind: string; data: string }[];
  systemPrompt: string;
  agent?: AgentUsageSlice | null;
}): UsageBucket[] {
  let conversation = 0;
  let reasoning = 0;
  for (const m of opts.messages) {
    conversation += estimateTokens(m.content);
    if (m.images) conversation += m.images.length * IMAGE_TOKENS;
    reasoning += estimateTokens(m.reasoning ?? "");
  }
  if (opts.agent) {
    conversation += estimateTokens(opts.agent.focusMd);
    for (const s of opts.agent.steps) {
      conversation += estimateTokens(s.text);
      reasoning += estimateTokens(s.reasoning);
    }
    for (const t of opts.agent.tools) {
      conversation += estimateTokens(t.argsJson);
      conversation += estimateTokens(t.resultPreview);
      conversation += estimateTokens(t.output);
    }
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
