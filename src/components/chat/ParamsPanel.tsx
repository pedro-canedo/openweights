// Painel lateral (coluna direita colapsável) de parâmetros da conversa:
// presets, system prompt e sliders de amostragem. A persistência por
// conversa (debounce) fica no Chat — aqui só editamos o objeto ChatParams.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  deletePreset,
  getModelCtxMap,
  getServerProps,
  getServerStatus,
  listPresets,
  lookupModelCtx,
  savePreset,
  setModelCtx,
  engineBusyReason,
  restartServer,
} from "../../lib/api";
import type { ChatParams, PresetRow } from "../../lib/types";
import { DEFAULT_CHAT_PARAMS } from "../../lib/types";

const CTX_CHIPS = [8192, 16384, 32768, 65536];
const CTX_MIN = 512;
const CTX_MAX = 262_144;

function formatCtx(n: number): string {
  if (n >= 1024 && n % 1024 === 0) return `${n / 1024}k`;
  return String(n);
}

function Slider({
  label,
  value,
  display,
  min,
  max,
  step,
  onChange,
}: {
  label: string;
  value: number;
  display: string;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
}) {
  return (
    <label className="block">
      <div className="mb-1 flex items-center justify-between text-xs">
        <span className="text-dim">{label}</span>
        <span className="tabular-nums text-ink">{display}</span>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="w-full accent-accent"
      />
    </label>
  );
}

export default function ParamsPanel({
  params,
  onChange,
  model,
  generating = false,
}: {
  params: ChatParams;
  onChange: (p: ChatParams) => void;
  model: string;
  generating?: boolean;
}) {
  const { t } = useTranslation();
  const [presets, setPresets] = useState<PresetRow[]>([]);
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [naming, setNaming] = useState(false);
  const [presetName, setPresetName] = useState("");
  const [ctxDraft, setCtxDraft] = useState<number | null>(null);
  const [ctxSaved, setCtxSaved] = useState<number | null>(null);
  const [ctxLive, setCtxLive] = useState<number | null>(null);
  const [ctxLoaded, setCtxLoaded] = useState(false);
  const [ctxApplying, setCtxApplying] = useState(false);
  /// Quem estava usando o motor quando o reinício foi recusado.
  const [ctxBusyWith, setCtxBusyWith] = useState<string[]>([]);
  const [serverRunning, setServerRunning] = useState(false);

  const refreshPresets = () =>
    listPresets()
      .then(setPresets)
      .catch(() => {});

  useEffect(() => {
    void refreshPresets();
  }, []);

  useEffect(() => {
    let cancelled = false;
    setCtxLoaded(false);
    void (async () => {
      const [map, props, status] = await Promise.all([
        getModelCtxMap().catch(() => ({})),
        getServerProps().catch(() => null),
        getServerStatus().catch(() => null),
      ]);
      if (cancelled) return;
      const saved = lookupModelCtx(map, model);
      setCtxSaved(saved);
      setCtxDraft(saved);
      setCtxLive(props?.nCtx ?? null);
      setServerRunning(Boolean(status?.running));
      setCtxLoaded(true);
    })();
    return () => {
      cancelled = true;
    };
  }, [model]);

  const patch = (p: Partial<ChatParams>) => onChange({ ...params, ...p });

  const applyCtx = async (next: number | null) => {
    if (!model || ctxApplying) return;
    const value =
      next == null ? null : Math.round(Math.min(CTX_MAX, Math.max(CTX_MIN, next)));
    setCtxDraft(value);
    setCtxApplying(true);
    setCtxBusyWith([]);
    try {
      const stored = await setModelCtx(model, value);
      setCtxSaved(stored);
      const status = await getServerStatus().catch(() => null);
      const running = Boolean(status?.running);
      setServerRunning(running);
      if (running && !generating) {
        // Quem decide se dá para derrubar o motor é o backend: ele enxerga a
        // automação e a indexação, que esta tela não vê.
        try {
          await restartServer();
          setServerRunning(true);
        } catch (e) {
          const quem = engineBusyReason(e);
          if (!quem) throw e;
          setCtxBusyWith(quem);
        }
      }
      const props = await getServerProps().catch(() => null);
      setCtxLive(props?.nCtx ?? null);
    } catch (e) {
      console.error("falha ao gravar janela de contexto:", e);
    } finally {
      setCtxApplying(false);
    }
  };

  const applyPreset = (id: number) => {
    setSelectedId(id);
    const row = presets.find((p) => p.id === id);
    if (!row) return;
    try {
      const parsed = JSON.parse(row.json) as Partial<ChatParams>;
      // `model` é da conversa, não do preset: aplicar um preset não pode
      // trocar o modelo escolhido pelo usuário.
      onChange({ ...DEFAULT_CHAT_PARAMS, ...parsed, model: params.model });
    } catch {
      // preset corrompido — ignora
    }
  };

  const saveCurrent = async () => {
    const name = presetName.trim();
    if (!name) return;
    try {
      const id = await savePreset(
        name,
        JSON.stringify({ ...params, model: null }),
      );
      setNaming(false);
      setPresetName("");
      await refreshPresets();
      setSelectedId(id);
    } catch (e) {
      console.error("falha ao salvar preset:", e);
    }
  };

  const removeSelected = async () => {
    if (selectedId == null || presets.length <= 1) return;
    try {
      await deletePreset(selectedId);
      setSelectedId(null);
      await refreshPresets();
    } catch (e) {
      console.error("falha ao excluir preset:", e);
    }
  };

  return (
    <aside className="flex w-72 shrink-0 flex-col gap-4 overflow-y-auto border-l border-edge bg-panel p-4">
      <div className="text-[11px] font-medium tracking-wide text-dim uppercase">
        {t("chat.params")}
      </div>

      {/* Presets */}
      <div className="flex flex-col gap-2">
        <label className="text-xs text-dim">{t("chat.preset")}</label>
        <div className="flex items-center gap-1.5">
          <select
            value={selectedId ?? ""}
            onChange={(e) => {
              const v = e.target.value;
              if (v) applyPreset(Number(v));
              else setSelectedId(null);
            }}
            className="min-w-0 flex-1 rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-xs outline-none focus:border-accent"
          >
            <option value="">—</option>
            {presets.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
          <button
            onClick={() => void removeSelected()}
            disabled={selectedId == null || presets.length <= 1}
            title={t("chat.presetDelete")}
            className="flex h-7 w-7 shrink-0 items-center justify-center rounded-lg border border-edge text-dim transition-colors hover:border-bad hover:text-bad disabled:opacity-30 disabled:hover:border-edge disabled:hover:text-dim"
          >
            <svg
              className="h-3.5 w-3.5"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.8"
              strokeLinecap="round"
              strokeLinejoin="round"
              viewBox="0 0 24 24"
            >
              <path d="M3 6h18M8 6V4a1 1 0 011-1h6a1 1 0 011 1v2m3 0v14a2 2 0 01-2 2H7a2 2 0 01-2-2V6" />
            </svg>
          </button>
        </div>

        {naming ? (
          <div className="flex items-center gap-1.5">
            <input
              autoFocus
              value={presetName}
              onChange={(e) => setPresetName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") void saveCurrent();
                if (e.key === "Escape") {
                  setNaming(false);
                  setPresetName("");
                }
              }}
              placeholder={t("chat.presetName")}
              className="min-w-0 flex-1 rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-xs outline-none placeholder:text-dim focus:border-accent"
            />
            <button
              onClick={() => void saveCurrent()}
              disabled={!presetName.trim()}
              className="shrink-0 rounded-lg bg-accent px-2.5 py-1.5 text-xs font-medium text-white disabled:opacity-40"
            >
              {t("common.save")}
            </button>
          </div>
        ) : (
          <button
            onClick={() => setNaming(true)}
            className="self-start rounded-lg border border-edge px-2.5 py-1.5 text-xs text-dim transition-colors hover:border-accent hover:text-ink"
          >
            {t("chat.presetSave")}
          </button>
        )}
      </div>

      {/* Janela de contexto (por modelo, no carregamento) */}
      <div className="flex flex-col gap-2">
        <label className="text-xs text-dim">{t("chat.ctx.label")}</label>
        <p className="text-[11px] leading-relaxed text-dim">{t("chat.ctx.hint")}</p>
        <div className="flex flex-wrap gap-1">
          <button
            type="button"
            disabled={!model || ctxApplying}
            onClick={() => void applyCtx(null)}
            className={`rounded-lg border px-2 py-1 text-[11px] transition-colors disabled:opacity-40 ${
              ctxDraft == null
                ? "border-accent bg-accent/15 text-ink"
                : "border-edge text-dim hover:border-accent hover:text-ink"
            }`}
          >
            {t("chat.ctx.auto")}
          </button>
          {CTX_CHIPS.map((n) => (
            <button
              key={n}
              type="button"
              disabled={!model || ctxApplying}
              onClick={() => void applyCtx(n)}
              className={`rounded-lg border px-2 py-1 text-[11px] tabular-nums transition-colors disabled:opacity-40 ${
                ctxDraft === n
                  ? "border-accent bg-accent/15 text-ink"
                  : "border-edge text-dim hover:border-accent hover:text-ink"
              }`}
            >
              {formatCtx(n)}
            </button>
          ))}
        </div>
        <div className="flex items-center gap-2">
          <input
            type="number"
            min={CTX_MIN}
            max={CTX_MAX}
            step={256}
            disabled={!model || ctxDraft == null || ctxApplying}
            value={ctxDraft ?? ""}
            onChange={(e) => {
              const n = Number(e.target.value);
              setCtxDraft(
                Number.isFinite(n) && n > 0 ? Math.round(n) : CTX_MIN,
              );
            }}
            onBlur={() => {
              if (ctxDraft != null && ctxDraft !== ctxSaved) {
                void applyCtx(ctxDraft);
              }
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && ctxDraft != null) {
                (e.target as HTMLInputElement).blur();
              }
            }}
            className="w-24 rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-xs tabular-nums outline-none focus:border-accent disabled:opacity-40"
          />
          <span className="text-[11px] text-dim">{t("chat.ctx.tokens")}</span>
        </div>
        {ctxLoaded && ctxLive != null && (
          <p className="text-[11px] leading-relaxed text-dim">
            {t("chat.ctx.inUse", { n: ctxLive.toLocaleString() })}
          </p>
        )}
        {ctxLoaded && ctxLive == null && (
          <p className="text-[11px] leading-relaxed text-dim">
            {t("chat.ctx.notLoaded")}
          </p>
        )}
        {ctxSaved != null &&
          ctxLive != null &&
          ctxSaved !== ctxLive &&
          !ctxApplying && (
            <p className="text-[11px] leading-relaxed text-warn">
              {t("chat.ctx.pending", { n: ctxSaved.toLocaleString() })}
            </p>
          )}
        {/* A janela nova só vale depois que o motor reinicia, e ele não
            reinicia por cima de quem está usando. */}
        {ctxBusyWith.length > 0 && (
          <p className="text-[11px] leading-relaxed text-warn">
            {t("server.busyToApply", {
              who: ctxBusyWith.map((w) => t(`server.busyWith.${w}`)).join(", "),
            })}
          </p>
        )}
        {serverRunning &&
          ctxDraft !== ctxLive &&
          !(ctxDraft == null && ctxSaved == null) && (
            <button
              type="button"
              disabled={!model || ctxApplying || generating}
              title={generating ? t("chat.ctx.busy") : undefined}
              onClick={() => void applyCtx(ctxDraft)}
              className="self-start rounded-lg border border-edge px-2.5 py-1.5 text-xs text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
            >
              {ctxApplying ? t("chat.ctx.applying") : t("chat.ctx.apply")}
            </button>
          )}
      </div>

      {/* System prompt */}
      <div className="flex flex-col gap-1.5">
        <label className="text-xs text-dim">{t("chat.systemPrompt")}</label>
        <textarea
          value={params.systemPrompt}
          onChange={(e) => patch({ systemPrompt: e.target.value })}
          placeholder={t("chat.systemPromptPlaceholder")}
          rows={4}
          className="resize-y rounded-lg border border-edge bg-panel2 px-2.5 py-2 text-xs leading-relaxed outline-none select-text placeholder:text-dim focus:border-accent"
        />
      </div>

      {/* Amostragem */}
      <Slider
        label={t("chat.temperature")}
        value={params.temperature}
        display={params.temperature.toFixed(2)}
        min={0}
        max={2}
        step={0.05}
        onChange={(v) => patch({ temperature: v })}
      />
      <Slider
        label={t("chat.topP")}
        value={params.topP}
        display={params.topP.toFixed(2)}
        min={0}
        max={1}
        step={0.01}
        onChange={(v) => patch({ topP: v })}
      />
      <Slider
        label={t("chat.topK")}
        value={params.topK}
        display={String(params.topK)}
        min={0}
        max={100}
        step={1}
        onChange={(v) => patch({ topK: Math.round(v) })}
      />

      {/* Limite de tokens */}
      <div className="flex flex-col gap-1.5">
        <label className="text-xs text-dim">{t("chat.maxTokens")}</label>
        <div className="flex items-center gap-2">
          <input
            type="number"
            min={1}
            step={1}
            disabled={params.maxTokens == null}
            value={params.maxTokens ?? ""}
            onChange={(e) => {
              const n = Number(e.target.value);
              patch({
                maxTokens: Number.isFinite(n) && n > 0 ? Math.round(n) : 1,
              });
            }}
            className="w-24 rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-xs tabular-nums outline-none focus:border-accent disabled:opacity-40"
          />
          <label className="flex items-center gap-1.5 text-xs text-dim">
            <input
              type="checkbox"
              checked={params.maxTokens == null}
              onChange={(e) =>
                patch({ maxTokens: e.target.checked ? null : 1024 })
              }
              className="accent-accent"
            />
            {t("chat.noLimit")}
          </label>
        </div>
      </div>
    </aside>
  );
}
