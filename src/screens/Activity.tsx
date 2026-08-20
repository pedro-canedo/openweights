// Tela Atividade: o histórico do agente atravessando conversas e automações.
//
// Toda outra tela mostra uma execução por dentro da conversa onde ela nasceu.
// Esta responde a pergunta que nenhuma responde: o que este agente andou
// fazendo NA MINHA MÁQUINA, quanto custou e o que eu autorizei. Inclui as
// automações, que rodam sem ninguém olhando e não têm conversa nenhuma.
//
// Unidades, que não são iguais em toda parte: `createdAt`/`finishedAt` de uma
// execução vêm em SEGUNDOS; `startedAt`/`finishedAt` de uma ação e todo
// `durationMs` vêm em milissegundos. Tudo que entra em `formatAgo` ou em um
// `Intl.DateTimeFormat` aqui já foi convertido para milissegundos.

import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import {
  PERIODS,
  activityCalls,
  activityRunCalls,
  activityRuns,
  activityStats,
  since,
  type ActivityCall,
  type ActivityStats,
} from "../lib/agent/activity";
import type { RunStatus, RunSummary } from "../lib/agent/types";
import { runAnswer, runsWaitingAnswer } from "../lib/agent/agentApi";
import QuestionCard from "../components/agent/QuestionCard";
import TraceDrawer from "../components/agent/TraceDrawer";
import { toolLabel } from "../components/agent/toolMeta";
import { chatStore } from "../lib/chatStore";
import { formatAgo, formatDuration } from "../lib/format";
import { navigate } from "../lib/nav";
import { errorMessage } from "../lib/serverSession";
import type { ChatRow } from "../lib/types";

/**
 * Teto da lista cronológica. É a resposta para "o que ele mexeu ontem?", não
 * um arquivo morto: passado esse ponto o caminho é abrir a execução.
 */
const CALLS_LIMIT = 300;

/**
 * HEURÍSTICA DE APRESENTAÇÃO, só para o filtro "alteraram algo".
 *
 * `ActivityCall` traz o nome da ferramenta, não a categoria (`ToolCategory`
 * vive no evento `tool.requested`, que esta tela não lê). Então a lista abaixo
 * é um palpite informado sobre o que deixa marca fora do modelo — disco, repo,
 * máquina, rede. Ferramenta que não está aqui (inclusive TODA ferramenta de
 * conector MCP, cuja natureza só o servidor conhece) fica de fora do filtro:
 * errar aqui muda o que a lista mostra, nunca o que o agente pode fazer.
 *
 * As ambíguas (`sql_query` roda o SQL que vier, `http_request` também escreve)
 * entram: esconder uma alteração é pior do que mostrar uma leitura a mais.
 */
const MUTATING_TOOLS = new Set([
  "fs_write",
  "fs_edit",
  "terminal_run",
  "code_run",
  "build_run",
  "format_run",
  "git_add",
  "git_commit",
  "git_branch",
  "git_stash",
  "git_restore",
  "memory_save",
  "sql_query",
  "clipboard_write",
  "open_path",
  "web_download",
  "http_request",
]);

/** Rótulo de cada período de `PERIODS` (em horas). */
const PERIOD_KEYS: Record<number, string> = {
  24: "h24",
  168: "d7",
  0: "all",
};

type CallFilter = "all" | "changed" | "denied";

const CALL_FILTERS: CallFilter[] = ["all", "changed", "denied"];

const ghostButton =
  "rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:border-accent hover:text-ink";

// ------------------------------------------------------------- formatação ---

// `Intl.*Format` é caro de construir e estas listas têm centenas de linhas:
// uma instância por idioma basta.
const numberFormats = new Map<string, Intl.NumberFormat>();
const dateFormats = new Map<string, Intl.DateTimeFormat>();

function num(lang: string, value: number): string {
  let f = numberFormats.get(lang);
  if (!f) {
    f = new Intl.NumberFormat(lang);
    numberFormats.set(lang, f);
  }
  return f.format(value);
}

/** Data absoluta, para o `title` de quem quer o instante exato. */
function fullDate(lang: string, tsMs: number): string {
  let f = dateFormats.get(lang);
  if (!f) {
    f = new Intl.DateTimeFormat(lang, {
      dateStyle: "medium",
      timeStyle: "short",
    });
    dateFormats.set(lang, f);
  }
  return f.format(tsMs);
}

function durationLabel(ms: number): string {
  return formatDuration(ms);
}

/**
 * Duração de uma ação: quase sempre abaixo de um segundo, e "0s" não diz
 * nada — o formatador único já cuida disso (sub-segundo em ms, daí para
 * cima em minutos e segundos).
 */
function callDurationLabel(ms: number): string {
  return formatDuration(ms);
}

// ------------------------------------------------------------- vocabulário ---

/** Estado da execução, com as frases que o resto do app já usa. */
function runStatusLabel(t: TFunction, status: RunStatus): string {
  switch (status) {
    case "done":
      return t("agent.run.finished");
    case "error":
      return t("agent.run.error");
    case "cancelled":
      return t("agent.run.cancelled");
    case "waitingApproval":
      return t("agent.approval.pending");
    case "maxSteps":
      return t("activity.status.maxSteps");
    case "escalated":
      return t("activity.status.escalated");
    default:
      return t("agent.tool.running");
  }
}

function runStatusTone(status: RunStatus): string {
  switch (status) {
    case "done":
      return "text-ok";
    case "error":
      return "text-bad";
    case "waitingApproval":
    case "maxSteps":
    case "escalated":
      return "text-warn";
    default:
      return "text-dim";
  }
}

function callStateLabel(t: TFunction, state: string): string {
  switch (state) {
    case "ok":
      return t("agent.tool.ok");
    case "error":
      return t("agent.tool.failed");
    case "denied":
      return t("agent.tool.denied");
    case "requested":
      return t("agent.tool.waiting");
    case "running":
      return t("agent.tool.running");
    // Estado novo no backend: o nome cru diz menos, mas não mente.
    default:
      return state;
  }
}

function callStateTone(state: string): string {
  switch (state) {
    case "ok":
      return "text-ok";
    case "error":
      return "text-bad";
    // Recusa é dim no chat, onde a pessoa acabou de recusar. Aqui ela é metade
    // do assunto da tela — some se não tiver cor.
    case "denied":
    case "requested":
      return "text-warn";
    default:
      return "text-dim";
  }
}

/** Quem autorizou. `null` = a ferramenta nem precisou de confirmação. */
function authorLabel(t: TFunction, source: string | null): string {
  if (source === "user" || source === "policy" || source === "yolo") {
    return t(`activity.by.${source}`);
  }
  if (source) return source;
  return t("activity.by.none");
}

/** A decisão gravada, quando houve uma. Detalhe de segundo plano (`title`). */
function decisionLabel(t: TFunction, decision: string | null): string | null {
  switch (decision) {
    case "allowOnce":
    case "allowAlways":
    case "allowEdited":
    case "deny":
    case "denyAlways":
      return t(`activity.decision.${decision}`);
    default:
      return decision;
  }
}

/** `mcp:<servidor>` → nome do servidor; `builtin` → nada a dizer. */
function mcpServer(origin: string): string | null {
  return origin.startsWith("mcp:") ? origin.slice(4) : null;
}

/** De onde a execução veio: a conversa pelo nome, ou a automação. */
function sourceLabel(
  t: TFunction,
  chats: ChatRow[],
  run: RunSummary,
): string {
  // `0` é o mesmo que ausência: automação não tem conversa.
  if (run.chatId == null || run.chatId === 0) return t("activity.automation");
  const chat = chats.find((c) => c.id === run.chatId);
  // Conversa apagada (ou lista ainda não carregada): o id é o que sobrou de
  // verdadeiro — melhor do que inventar um título.
  return chat?.title ?? t("activity.chatGone", { id: run.chatId });
}

// --------------------------------------------------------------- contagens ---

/**
 * Duração da execução. `usage.durationMs` é a medida do motor; sem ela, o
 * relógio entre início e fim ainda é um número real (em SEGUNDOS na tabela).
 * Execução em curso não tem fim — e aí não há o que mostrar.
 */
function runDurationMs(run: RunSummary): number | null {
  if (run.usage) return run.usage.durationMs;
  if (run.finishedAt == null) return null;
  return (run.finishedAt - run.createdAt) * 1000;
}

/** Sem `usage` não há conta de tokens: zero seria uma afirmação falsa. */
function runTokens(run: RunSummary): number | null {
  if (!run.usage) return null;
  return run.usage.promptTokens + run.usage.completionTokens;
}

function callDurationMs(call: ActivityCall): number | null {
  if (call.startedAt == null || call.finishedAt == null) return null;
  return Math.max(0, call.finishedAt - call.startedAt);
}

function matchesFilter(call: ActivityCall, filter: CallFilter): boolean {
  if (filter === "denied") {
    return (
      call.state === "denied" ||
      call.decision === "deny" ||
      call.decision === "denyAlways"
    );
  }
  if (filter === "changed") {
    // O que foi recusado ou falhou não alterou nada — por definição.
    const aconteceu = call.state === "ok" || call.state === "running";
    return aconteceu && MUTATING_TOOLS.has(call.toolName);
  }
  return true;
}

// ------------------------------------------------------------------ peças ---

function Segmented<T extends string | number>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (value: T) => void;
}) {
  return (
    <div
      role="group"
      aria-label={label}
      className="flex shrink-0 gap-0.5 rounded-lg border border-edge bg-panel2 p-0.5"
    >
      {options.map((option) => (
        <button
          key={String(option.value)}
          type="button"
          aria-pressed={value === option.value}
          onClick={() => onChange(option.value)}
          className={`rounded-md px-2.5 py-1 text-[12px] transition-colors ${
            value === option.value
              ? "bg-panel text-ink"
              : "text-dim hover:text-ink"
          }`}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

function StatTile({
  label,
  value,
  hint,
  tone = "text-ink",
}: {
  label: string;
  /** Já formatado — ou "—" quando o número não existe. */
  value: string;
  hint?: string;
  tone?: string;
}) {
  return (
    <div className="rounded-xl border border-edge bg-panel2 px-3 py-2.5">
      <div className="text-[11px] text-dim">{label}</div>
      <div className={`mt-0.5 text-lg leading-tight tabular-nums ${tone}`}>
        {value}
      </div>
      {hint && <div className="mt-0.5 text-[11px] text-dim">{hint}</div>}
    </div>
  );
}

/** Uma ação do agente, em uma linha: o que foi, como acabou, quem autorizou. */
function CallLine({
  call,
  lang,
  onOpenTrace,
}: {
  call: ActivityCall;
  lang: string;
  /** Ausente dentro da própria execução: abrir a trilha dali seria redundante. */
  onOpenTrace?: (runId: string) => void;
}) {
  const { t } = useTranslation();
  const server = mcpServer(call.origin);
  const ms = callDurationMs(call);
  const decision = decisionLabel(t, call.decision);

  const body = (
    <>
      <span className="shrink-0 text-[12px] text-ink">
        {toolLabel(t, call.toolName)}
      </span>
      {server && (
        <span className="shrink-0 rounded-md bg-panel px-1.5 py-0.5 text-[10px] text-fuchsia-400">
          {server}
        </span>
      )}
      <span className={`shrink-0 text-[11px] ${callStateTone(call.state)}`}>
        {callStateLabel(t, call.state)}
      </span>
      <span
        className="min-w-0 flex-1 truncate text-[11px] text-dim"
        title={decision ?? undefined}
      >
        {authorLabel(t, call.source)}
      </span>
      {ms != null && (
        <span className="shrink-0 text-[11px] tabular-nums text-dim">
          {callDurationLabel(ms)}
        </span>
      )}
      {call.startedAt != null && (
        <span
          className="shrink-0 text-[11px] text-dim"
          title={fullDate(lang, call.startedAt)}
        >
          {formatAgo(lang, call.startedAt)}
        </span>
      )}
    </>
  );

  // O motivo só aparece quando existe: é o que responde "por que não deu
  // certo?" sem precisar abrir a trilha inteira.
  const reason = call.error && (
    <p
      className={`pl-2 text-[11px] leading-relaxed ${
        call.state === "error" ? "text-bad" : "text-dim"
      }`}
    >
      {call.error}
    </p>
  );

  if (!onOpenTrace) {
    return (
      <div className="rounded-lg px-2 py-1.5">
        <div className="flex items-center gap-2">{body}</div>
        {reason}
      </div>
    );
  }
  return (
    <button
      type="button"
      onClick={() => onOpenTrace(call.runId)}
      title={t("agent.run.showTrace")}
      className="w-full rounded-lg px-2 py-1.5 text-left transition-colors hover:bg-panel2"
    >
      <div className="flex items-center gap-2">{body}</div>
      {reason}
    </button>
  );
}

/** Uma execução: cabeçalho clicável e, aberta, o resumo e as ações dela. */
function RunRow({
  run,
  chats,
  lang,
  open,
  onToggle,
  onOpenTrace,
}: {
  run: RunSummary;
  chats: ChatRow[];
  lang: string;
  open: boolean;
  onToggle: () => void;
  onOpenTrace: (runId: string) => void;
}) {
  const { t } = useTranslation();
  const [calls, setCalls] = useState<ActivityCall[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  // As ações de uma execução só são buscadas quando a linha abre: carregar
  // todas de antemão seria uma consulta por execução da lista inteira.
  useEffect(() => {
    if (!open || calls || error) return;
    let cancelled = false;
    void activityRunCalls(run.id)
      .then((rows) => {
        if (cancelled) return;
        // Dentro de uma execução a ordem natural é a do acontecido: primeiro
        // passo primeiro. Sem hora (ação que nunca começou) vai para o fim.
        setCalls(
          [...rows].sort(
            (a, b) => (a.startedAt ?? Infinity) - (b.startedAt ?? Infinity),
          ),
        );
      })
      .catch((e) => {
        if (!cancelled) setError(errorMessage(e));
      });
    return () => {
      cancelled = true;
    };
  }, [open, calls, error, run.id]);

  const ms = runDurationMs(run);
  const tokens = runTokens(run);
  const panelId = `activity-run-${run.id}`;

  return (
    <div
      className={`rounded-xl border bg-panel ${open ? "border-accent" : "border-edge"}`}
    >
      <button
        type="button"
        aria-expanded={open}
        aria-controls={panelId}
        onClick={onToggle}
        className="flex w-full items-start gap-2 px-4 py-3 text-left transition-colors hover:bg-panel2/60"
      >
        <svg
          aria-hidden
          className={`mt-1 h-3 w-3 shrink-0 text-dim transition-transform ${
            open ? "rotate-90" : ""
          }`}
          fill="none"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          strokeLinejoin="round"
          viewBox="0 0 24 24"
        >
          <path d="M9 18l6-6-6-6" />
        </svg>
        <div className="min-w-0 flex-1">
          <div className="flex items-start gap-3">
            <span className="min-w-0 flex-1 truncate text-[13px] text-ink">
              {run.prompt}
            </span>
            <span
              className={`shrink-0 text-[11px] ${runStatusTone(run.status)}`}
            >
              {runStatusLabel(t, run.status)}
            </span>
          </div>
          <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[11px] text-dim">
            {/* createdAt vem em segundos; o formatador só entende milissegundos. */}
            <span title={fullDate(lang, run.createdAt * 1000)}>
              {formatAgo(lang, run.createdAt * 1000)}
            </span>
            <span aria-hidden>·</span>
            <span className="truncate">{sourceLabel(t, chats, run)}</span>
            <span aria-hidden>·</span>
            <span className="truncate">{run.model}</span>
            <span aria-hidden>·</span>
            <span>{t(`agent.mode.${run.mode}`)}</span>
            <span aria-hidden>·</span>
            <span className="tabular-nums">
              {ms == null ? "—" : durationLabel(ms)}
            </span>
            <span aria-hidden>·</span>
            {/* Sem `usage`, o traço fica no lugar do número e a palavra
                "tokens" continua dizendo do que ele é o traço. */}
            <span className="tabular-nums">
              {t("activity.runs.tokens", {
                total: tokens == null ? "—" : num(lang, tokens),
              })}
            </span>
          </div>
        </div>
      </button>

      {open && (
        <div id={panelId} className="border-t border-edge px-4 py-3">
          <p className="text-[12px] leading-relaxed text-dim select-text">
            {run.summary ?? t("activity.runs.noSummary")}
          </p>
          {run.workspaceDir && (
            <p className="mt-1.5 truncate font-mono text-[11px] text-dim">
              {t("activity.runs.workspace")}: {run.workspaceDir}
            </p>
          )}

          <div className="mt-3 text-[11px] tracking-wide text-dim uppercase">
            {t("activity.runs.callsTitle")}
          </div>
          {error && <p className="mt-1 text-[12px] text-bad">{error}</p>}
          {!error && !calls && (
            <p className="mt-1 text-[12px] text-dim">{t("common.loading")}</p>
          )}
          {calls?.length === 0 && (
            <p className="mt-1 text-[12px] text-dim">
              {t("activity.runs.callsNone")}
            </p>
          )}
          {calls && calls.length > 0 && (
            <div className="mt-1 flex flex-col">
              {calls.map((call) => (
                <CallLine key={call.callId} call={call} lang={lang} />
              ))}
            </div>
          )}

          <button
            type="button"
            onClick={() => onOpenTrace(run.id)}
            className={`mt-3 ${ghostButton}`}
          >
            {t("agent.run.showTrace")}
          </button>
        </div>
      )}
    </div>
  );
}

// ------------------------------------------------------------------- tela ---

/**
 * Um run pausado numa pergunta, respondível daqui. Os botões de opção enviam
 * direto (não há compositor nesta tela); o campo livre cobre o resto.
 */
function WaitingAnswer({
  run,
  onAnswered,
}: {
  run: RunSummary;
  onAnswered: () => void;
}) {
  const { t } = useTranslation();
  const [texto, setTexto] = useState("");
  const [sending, setSending] = useState(false);
  const [erro, setErro] = useState<string | null>(null);

  const enviar = async (resposta: string) => {
    if (!resposta.trim() || sending) return;
    setSending(true);
    setErro(null);
    try {
      // Sem ouvinte ao vivo: esta tela não acompanha o run novo — a trilha
      // persiste e a conversa (quando houver) reata sozinha ao ser aberta.
      // `runAnswer` nunca rejeita: devolve `null` quando o backend recusou
      // (pergunta já respondida em outra janela, servidor fora). Sem checar,
      // o card sumia e voltava sem uma linha de explicação.
      const novo = await runAnswer(run.id, resposta.trim(), () => {});
      if (!novo) {
        setErro(t("agent.question.failed"));
        return;
      }
      onAnswered();
    } catch (e) {
      setErro(errorMessage(e));
    } finally {
      setSending(false);
    }
  };

  return (
    <div className="rounded-xl border border-accent/40 bg-panel2 px-4 py-3">
      <p className="mb-2 text-[12px] leading-relaxed text-dim select-text">
        {run.prompt}
      </p>
      {run.question && (
        <QuestionCard
          question={run.question}
          onPick={(op) => void enviar(op)}
        />
      )}
      <div className="mt-2 flex items-center gap-2">
        <input
          value={texto}
          onChange={(e) => setTexto(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") void enviar(texto);
          }}
          placeholder={t("agent.question.answerPlaceholder")}
          className="min-w-0 flex-1 rounded-lg border border-edge bg-panel px-2.5 py-1.5 text-[12px] text-ink outline-none focus:border-accent"
        />
        <button
          type="button"
          disabled={sending || !texto.trim()}
          onClick={() => void enviar(texto)}
          className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:text-ink disabled:opacity-50"
        >
          {t("agent.question.send")}
        </button>
      </div>
      {erro && <p className="mt-1 text-[11px] text-bad">{erro}</p>}
    </div>
  );
}

export default function Activity() {
  const { t, i18n } = useTranslation();
  const lang = i18n.language;
  const { chats } = useSyncExternalStore(chatStore.subscribe, chatStore.get);

  // null = ainda carregando; lista vazia é um estado normal e tem tela própria.
  const [runs, setRuns] = useState<RunSummary[] | null>(null);
  const [waiting, setWaiting] = useState<RunSummary[]>([]);
  const [calls, setCalls] = useState<ActivityCall[] | null>(null);
  const [stats, setStats] = useState<ActivityStats | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [period, setPeriod] = useState<number>(PERIODS[0]);
  const [tab, setTab] = useState<"runs" | "calls">("runs");
  const [filter, setFilter] = useState<CallFilter>("all");
  const [openRunId, setOpenRunId] = useState<string | null>(null);
  const [traceId, setTraceId] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const [rows, recent, pendentes] = await Promise.all([
        activityRuns(),
        activityCalls(CALLS_LIMIT),
        runsWaitingAnswer(),
      ]);
      setWaiting(pendentes);
      // A ordem vem do banco, mas quem depende dela é a leitura da tela:
      // ordenar aqui custa nada e não deixa a promessa "mais recente
      // primeiro" nas mãos de outra camada.
      setRuns([...rows].sort((a, b) => b.createdAt - a.createdAt));
      setCalls(
        [...recent].sort((a, b) => (b.startedAt ?? 0) - (a.startedAt ?? 0)),
      );
      setError(null);
    } catch (e) {
      setRuns((prev) => prev ?? []);
      setCalls((prev) => prev ?? []);
      setError(errorMessage(e));
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  // As contas são do backend e mudam com o período; as listas já estão na mão.
  useEffect(() => {
    let cancelled = false;
    setStats(null);
    void activityStats(since(period))
      .then((s) => {
        if (!cancelled) setStats(s);
      })
      .catch((e) => {
        if (!cancelled) setError(errorMessage(e));
      });
    return () => {
      cancelled = true;
    };
  }, [period]);

  const sinceS = since(period);
  const periodRuns = (runs ?? []).filter((r) => r.createdAt >= sinceS);
  // Ação sem hora de início nunca é escondida por um recorte de tempo: não
  // sabemos quando foi, e sumir com ela seria pior do que mostrá-la fora do
  // período.
  const periodCalls = (calls ?? []).filter(
    (c) => c.startedAt == null || c.startedAt >= sinceS * 1000,
  );
  const visibleCalls = periodCalls.filter((c) => matchesFilter(c, filter));

  const loading = runs == null || calls == null;
  /** Quem nunca rodou o agente: a tela precisa se explicar, não listar nada. */
  const nunca = runs?.length === 0 && calls?.length === 0;

  const closeTrace = useCallback(() => {
    setTraceId(null);
    // A gaveta responde confirmações: o que estava parado pode ter andado.
    void reload();
  }, [reload]);

  return (
    <div className="mx-auto max-w-5xl px-8 py-8">
      <h1 className="text-xl font-semibold">{t("activity.title")}</h1>
      <p className="mt-1 text-[13px] text-dim">{t("activity.subtitle")}</p>

      {error && (
        <p className="mt-4 rounded-lg border border-bad/40 bg-bad/10 px-3 py-2 text-[12px] text-bad">
          {error}
        </p>
      )}

      {loading && (
        <p className="mt-6 text-[12px] text-dim">{t("common.loading")}</p>
      )}

      {nunca && (
        <div className="mt-6 rounded-xl border border-dashed border-edge p-10 text-center">
          <p className="text-sm text-ink">{t("activity.empty.title")}</p>
          <p className="mx-auto mt-2 max-w-md text-[13px] leading-relaxed text-dim">
            {t("activity.empty.body")}
          </p>
          <button
            type="button"
            onClick={() => {
              chatStore.requestNew();
              navigate("chat");
            }}
            className="mt-4 rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white transition-opacity hover:opacity-90"
          >
            {t("activity.empty.cta")}
          </button>
        </div>
      )}

      {/* Runs pausados numa pergunta: respondível DAQUI — automações não têm
          conversa, e sem esta fila a pausa delas ficaria invisível. */}
      {waiting.length > 0 && (
        <section className="mt-6 flex flex-col gap-2">
          <h2 className="text-[12px] font-medium tracking-wide text-accent uppercase">
            {t("activity.waitingAnswer")}
          </h2>
          {waiting.map((r) => (
            <WaitingAnswer key={r.id} run={r} onAnswered={() => void reload()} />
          ))}
        </section>
      )}

      {!loading && !nunca && (
        <>
          <section className="mt-6 rounded-xl border border-edge bg-panel px-5 py-4">
            <div className="flex flex-wrap items-center justify-between gap-3">
              <span className="text-sm">{t("activity.stats.title")}</span>
              <Segmented
                label={t("activity.period.label")}
                value={period}
                onChange={setPeriod}
                options={PERIODS.map((hours) => {
                  const key = PERIOD_KEYS[hours];
                  return {
                    value: hours as number,
                    label: key ? t(`activity.period.${key}`) : `${hours} h`,
                  };
                })}
              />
            </div>

            <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
              <StatTile
                label={t("activity.stats.runs")}
                value={stats ? num(lang, stats.runs) : "—"}
              />
              <StatTile
                label={t("activity.stats.failed")}
                value={stats ? num(lang, stats.failed) : "—"}
                tone={stats && stats.failed > 0 ? "text-bad" : "text-ink"}
              />
              <StatTile
                label={t("activity.stats.calls")}
                value={stats ? num(lang, stats.toolCalls) : "—"}
              />
              <StatTile
                label={t("activity.stats.denied")}
                value={stats ? num(lang, stats.denied) : "—"}
                tone={stats && stats.denied > 0 ? "text-warn" : "text-ink"}
              />
              <StatTile
                label={t("activity.stats.tokens")}
                value={
                  stats
                    ? num(lang, stats.promptTokens + stats.completionTokens)
                    : "—"
                }
                hint={
                  stats
                    ? t("activity.stats.tokensSplit", {
                        prompt: num(lang, stats.promptTokens),
                        completion: num(lang, stats.completionTokens),
                      })
                    : undefined
                }
              />
              <StatTile
                label={t("activity.stats.duration")}
                value={stats ? durationLabel(stats.durationMs) : "—"}
              />
            </div>

            <p className="mt-3 text-[11px] leading-relaxed text-dim">
              {t("activity.noPrice")}
            </p>
          </section>

          <div className="mt-6 flex flex-wrap items-center justify-between gap-3">
            <div role="tablist" aria-label={t("activity.title")} className="flex gap-1">
              {(["runs", "calls"] as const).map((key) => (
                <button
                  key={key}
                  type="button"
                  role="tab"
                  id={`activity-tab-${key}`}
                  aria-selected={tab === key}
                  aria-controls={`activity-panel-${key}`}
                  onClick={() => setTab(key)}
                  className={`rounded-lg px-3 py-1.5 text-[13px] transition-colors ${
                    tab === key
                      ? "bg-panel2 text-ink"
                      : "text-dim hover:text-ink"
                  }`}
                >
                  {t(`activity.tab.${key}`)}
                </button>
              ))}
            </div>
            {tab === "calls" && (
              <Segmented
                label={t("activity.calls.filterLabel")}
                value={filter}
                onChange={setFilter}
                options={CALL_FILTERS.map((key) => ({
                  value: key,
                  label: t(`activity.calls.filter.${key}`),
                }))}
              />
            )}
          </div>

          {tab === "runs" && (
            <div
              role="tabpanel"
              id="activity-panel-runs"
              aria-labelledby="activity-tab-runs"
              className="mt-3 flex flex-col gap-2"
            >
              {periodRuns.length === 0 && (
                <p className="rounded-xl border border-dashed border-edge px-4 py-8 text-center text-[12px] text-dim">
                  {t("activity.runs.none")}
                </p>
              )}
              {periodRuns.map((run) => (
                <RunRow
                  key={run.id}
                  run={run}
                  chats={chats}
                  lang={lang}
                  open={openRunId === run.id}
                  onToggle={() =>
                    setOpenRunId((cur) => (cur === run.id ? null : run.id))
                  }
                  onOpenTrace={setTraceId}
                />
              ))}
            </div>
          )}

          {tab === "calls" && (
            <div
              role="tabpanel"
              id="activity-panel-calls"
              aria-labelledby="activity-tab-calls"
              className="mt-3 rounded-xl border border-edge bg-panel px-2 py-2"
            >
              {visibleCalls.length === 0 ? (
                <p className="px-2 py-6 text-center text-[12px] text-dim">
                  {t("activity.calls.none")}
                </p>
              ) : (
                visibleCalls.map((call) => (
                  <CallLine
                    key={call.callId}
                    call={call}
                    lang={lang}
                    onOpenTrace={setTraceId}
                  />
                ))
              )}
              {periodCalls.length >= CALLS_LIMIT && (
                <p className="px-2 py-2 text-[11px] text-dim">
                  {t("activity.calls.limit", { limit: num(lang, CALLS_LIMIT) })}
                </p>
              )}
            </div>
          )}
        </>
      )}

      {traceId && <TraceDrawer runId={traceId} onClose={closeTrace} />}
    </div>
  );
}
