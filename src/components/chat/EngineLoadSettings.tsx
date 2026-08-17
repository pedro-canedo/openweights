// Opções de carga do modelo no painel Parâmetros: KV cache, MTP, visão,
// flash attention e o resto do perfil. Não são da conversa — valem no
// próximo carregamento, como a janela de contexto.

import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  engineBusyReason,
  getModelProfile,
  getServerProps,
  getServerStatus,
  listLocalModels,
  restartServer,
  setModelProfile,
} from "../../lib/api";
import { describesModel } from "../../lib/agent/types";
import {
  emptyProfile,
  type KvType,
  type ModelProfile,
  type SpecType,
  type VisionMode,
} from "../../lib/tuning";

const CTX_CHIPS = [8192, 16384, 32768, 65536];
const CTX_MIN = 512;
const CTX_MAX = 262_144;

function formatCtx(n: number): string {
  if (n >= 1024 && n % 1024 === 0) return `${n / 1024}k`;
  return String(n);
}

function sameModel(a: string, b: string): boolean {
  const stem = (s: string) => s.replace(/\.gguf$/i, "").toLowerCase();
  return stem(a) === stem(b);
}

function chipClass(active: boolean): string {
  return `rounded-lg border px-2 py-1 text-[11px] transition-colors disabled:opacity-40 ${
    active
      ? "border-accent bg-accent/15 text-ink"
      : "border-edge text-dim hover:border-accent hover:text-ink"
  }`;
}

function Chips<T extends string>({
  value,
  options,
  disabled,
  onChange,
}: {
  value: T;
  options: { id: T; label: string }[];
  disabled?: boolean;
  onChange: (v: T) => void;
}) {
  return (
    <div className="flex flex-wrap gap-1">
      {options.map((o) => (
        <button
          key={o.id}
          type="button"
          disabled={disabled}
          onClick={() => onChange(o.id)}
          className={chipClass(value === o.id)}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function OptionalNum({
  label,
  hint,
  value,
  min,
  max,
  step,
  placeholder,
  disabled,
  onCommit,
}: {
  label: string;
  hint?: string;
  value: number | null | undefined;
  min: number;
  max: number;
  step?: number;
  placeholder: string;
  disabled?: boolean;
  onCommit: (n: number | null) => void;
}) {
  const [text, setText] = useState(value == null ? "" : String(value));
  useEffect(() => {
    setText(value == null ? "" : String(value));
  }, [value]);

  const commit = () => {
    const trimmed = text.trim();
    if (!trimmed) {
      onCommit(null);
      return;
    }
    const n = Number(trimmed);
    if (!Number.isFinite(n)) {
      setText(value == null ? "" : String(value));
      return;
    }
    onCommit(Math.round(Math.min(max, Math.max(min, n))));
  };

  return (
    <label className="flex flex-col gap-1">
      <span className="text-xs text-dim">{label}</span>
      {hint && <span className="text-[11px] leading-relaxed text-dim">{hint}</span>}
      <input
        type="number"
        min={min}
        max={max}
        step={step ?? 1}
        disabled={disabled}
        placeholder={placeholder}
        value={text}
        onChange={(e) => setText(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        }}
        className="w-24 rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-xs tabular-nums outline-none placeholder:text-dim focus:border-accent disabled:opacity-40"
      />
    </label>
  );
}

type Tri = "auto" | "on" | "off";

function triFrom(v: boolean | null | undefined): Tri {
  if (v == null) return "auto";
  return v ? "on" : "off";
}

function triTo(v: Tri): boolean | null {
  if (v === "auto") return null;
  return v === "on";
}

type KvChoice = "auto" | KvType;

export default function EngineLoadSettings({
  model,
  generating = false,
}: {
  model: string;
  generating?: boolean;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState<ModelProfile>(emptyProfile());
  const [ctxLive, setCtxLive] = useState<number | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [applying, setApplying] = useState(false);
  const [pendingReload, setPendingReload] = useState(false);
  const [busyWith, setBusyWith] = useState<string[]>([]);
  const [serverRunning, setServerRunning] = useState(false);
  const [hasVision, setHasVision] = useState(false);
  const [more, setMore] = useState(false);
  const draftRef = useRef(draft);
  draftRef.current = draft;
  const restartTimer = useRef<number | null>(null);

  const hasMtp = /mtp/i.test(model);

  useEffect(() => {
    let cancelled = false;
    setLoaded(false);
    setPendingReload(false);
    setBusyWith([]);
    void (async () => {
      const [profile, props, status, locals] = await Promise.all([
        getModelProfile(model).catch(() => null),
        getServerProps(model).catch(() => null),
        getServerStatus().catch(() => null),
        listLocalModels().catch(() => []),
      ]);
      if (cancelled) return;
      setDraft(profile ?? emptyProfile());
      setCtxLive(describesModel(props) ? props.nCtx : null);
      setServerRunning(Boolean(status?.running));
      setHasVision(
        locals.some(
          (m) => sameModel(m.name, model) && Boolean(m.visionProjector),
        ),
      );
      setLoaded(true);
    })();
    return () => {
      cancelled = true;
      if (restartTimer.current) window.clearTimeout(restartTimer.current);
    };
  }, [model]);

  const persist = async (next: ModelProfile, reload: boolean) => {
    if (!model) return;
    setDraft(next);
    setBusyWith([]);
    try {
      const stored = await setModelProfile(model, next);
      setDraft(stored);
      const status = await getServerStatus().catch(() => null);
      const running = Boolean(status?.running);
      setServerRunning(running);
      if (!running) {
        setPendingReload(false);
        return;
      }
      if (!reload || generating) {
        setPendingReload(true);
        return;
      }
      setApplying(true);
      try {
        await restartServer();
        setPendingReload(false);
        const props = await getServerProps(model).catch(() => null);
        setCtxLive(describesModel(props) ? props.nCtx : null);
      } catch (e) {
        const quem = engineBusyReason(e);
        if (!quem) throw e;
        setBusyWith(quem);
        setPendingReload(true);
      } finally {
        setApplying(false);
      }
    } catch (e) {
      console.error("falha ao gravar perfil do modelo:", e);
    }
  };

  const patch = (partial: Partial<ModelProfile>, reloadNow = false) => {
    const next: ModelProfile = {
      ...draftRef.current,
      ...partial,
      source: "manual",
    };
    setDraft(next);
    if (restartTimer.current) window.clearTimeout(restartTimer.current);
    if (reloadNow) {
      restartTimer.current = window.setTimeout(() => {
        void persist(next, true);
      }, 500);
      return;
    }
    void persist(next, false);
  };

  const applyNow = () => void persist(draftRef.current, true);

  const kvChoice: KvChoice = draft.kvK ?? draft.kvV ?? "auto";
  const specChoice: SpecType = draft.spec ?? "none";
  const visionChoice: VisionMode = draft.vision ?? "onDemand";
  const disabled = !model || applying;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-col gap-1.5">
        <div className="text-xs text-dim">{t("chat.engine.title")}</div>
        <p className="text-[11px] leading-relaxed text-dim">
          {t("chat.engine.hint")}
        </p>
      </div>

      <div className="flex flex-col gap-2">
        <label className="text-xs text-dim">{t("chat.ctx.label")}</label>
        <p className="text-[11px] leading-relaxed text-dim">{t("chat.ctx.hint")}</p>
        <div className="flex flex-wrap gap-1">
          <button
            type="button"
            disabled={disabled}
            onClick={() => patch({ ctx: null }, true)}
            className={chipClass(draft.ctx == null)}
          >
            {t("chat.ctx.auto")}
          </button>
          {CTX_CHIPS.map((n) => (
            <button
              key={n}
              type="button"
              disabled={disabled}
              onClick={() => patch({ ctx: n }, true)}
              className={`${chipClass(draft.ctx === n)} tabular-nums`}
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
            disabled={disabled || draft.ctx == null}
            value={draft.ctx ?? ""}
            onChange={(e) => {
              const n = Number(e.target.value);
              setDraft({
                ...draft,
                ctx: Number.isFinite(n) && n > 0 ? Math.round(n) : CTX_MIN,
                source: "manual",
              });
            }}
            onBlur={() => {
              if (draft.ctx != null) patch({ ctx: draft.ctx }, true);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") (e.target as HTMLInputElement).blur();
            }}
            className="w-24 rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-xs tabular-nums outline-none focus:border-accent disabled:opacity-40"
          />
          <span className="text-[11px] text-dim">{t("chat.ctx.tokens")}</span>
        </div>
        {loaded && ctxLive != null && (
          <p className="text-[11px] leading-relaxed text-dim">
            {t("chat.ctx.inUse", { n: ctxLive.toLocaleString() })}
          </p>
        )}
        {loaded && ctxLive == null && (
          <p className="text-[11px] leading-relaxed text-dim">
            {t("chat.ctx.notLoaded")}
          </p>
        )}
      </div>

      <div className="flex flex-col gap-2">
        <label className="text-xs text-dim">{t("chat.engine.vision.label")}</label>
        <p className="text-[11px] leading-relaxed text-dim">
          {t("chat.engine.vision.hint")}
        </p>
        {hasVision ? (
          <Chips
            value={visionChoice}
            disabled={disabled}
            onChange={(v) => patch({ vision: v }, true)}
            options={[
              { id: "off", label: t("chat.engine.vision.off") },
              { id: "onDemand", label: t("chat.engine.vision.onDemand") },
              { id: "always", label: t("chat.engine.vision.always") },
            ]}
          />
        ) : (
          <p className="text-[11px] leading-relaxed text-dim">
            {t("chat.engine.vision.none")}
          </p>
        )}
      </div>

      <div className="flex flex-col gap-2">
        <label className="text-xs text-dim">{t("chat.engine.kv.label")}</label>
        <p className="text-[11px] leading-relaxed text-dim">
          {t("chat.engine.kv.hint")}
        </p>
        <Chips
          value={kvChoice}
          disabled={disabled}
          onChange={(v) => {
            if (v === "auto") patch({ kvK: null, kvV: null }, true);
            else patch({ kvK: v, kvV: v }, true);
          }}
          options={[
            { id: "auto", label: t("chat.engine.kv.auto") },
            { id: "f16", label: t("chat.engine.kv.f16") },
            { id: "q8_0", label: t("chat.engine.kv.q8") },
            { id: "q4_0", label: t("chat.engine.kv.q4") },
          ]}
        />
      </div>

      <div className="flex flex-col gap-2">
        <label className="text-xs text-dim">{t("chat.engine.flash.label")}</label>
        <p className="text-[11px] leading-relaxed text-dim">
          {t("chat.engine.flash.hint")}
        </p>
        <Chips
          value={triFrom(draft.flashAttn)}
          disabled={disabled}
          onChange={(v) => patch({ flashAttn: triTo(v) }, true)}
          options={[
            { id: "auto", label: t("chat.engine.flash.auto") },
            { id: "on", label: t("chat.engine.flash.on") },
            { id: "off", label: t("chat.engine.flash.off") },
          ]}
        />
      </div>

      <div className="flex flex-col gap-2">
        <label className="text-xs text-dim">{t("chat.engine.spec.label")}</label>
        <p className="text-[11px] leading-relaxed text-dim">
          {t("chat.engine.spec.hint")}
        </p>
        {hasMtp && (
          <p className="text-[11px] leading-relaxed text-ok">
            {t("chat.engine.spec.mtpHint")}
          </p>
        )}
        <Chips
          value={specChoice}
          disabled={disabled}
          onChange={(v) => patch({ spec: v }, true)}
          options={[
            { id: "none", label: t("chat.engine.spec.off") },
            { id: "mtp", label: t("chat.engine.spec.mtp") },
            { id: "ngram", label: t("chat.engine.spec.ngram") },
          ]}
        />
      </div>

      <div className="flex flex-col gap-2">
        <label className="text-xs text-dim">{t("chat.engine.kvOffload.label")}</label>
        <p className="text-[11px] leading-relaxed text-dim">
          {t("chat.engine.kvOffload.hint")}
        </p>
        <Chips
          value={triFrom(draft.kvOffload)}
          disabled={disabled}
          onChange={(v) => patch({ kvOffload: triTo(v) }, true)}
          options={[
            { id: "auto", label: t("chat.engine.kvOffload.auto") },
            { id: "on", label: t("chat.engine.kvOffload.on") },
            { id: "off", label: t("chat.engine.kvOffload.off") },
          ]}
        />
      </div>

      <button
        type="button"
        onClick={() => setMore((v) => !v)}
        className="self-start text-[11px] text-dim transition-colors hover:text-ink"
      >
        {t("chat.engine.more")}
        {more ? " ▾" : " ▸"}
      </button>

      {more && (
        <div className="flex flex-col gap-3">
          <OptionalNum
            label={t("chat.engine.ngl")}
            hint={t("chat.engine.nglHint")}
            value={draft.ngl}
            min={0}
            max={256}
            placeholder={t("chat.engine.autoPlaceholder")}
            disabled={disabled}
            onCommit={(n) => patch({ ngl: n }, true)}
          />
          <OptionalNum
            label={t("chat.engine.ncmoe")}
            hint={t("chat.engine.ncmoeHint")}
            value={draft.ncmoe}
            min={0}
            max={256}
            placeholder={t("chat.engine.autoPlaceholder")}
            disabled={disabled}
            onCommit={(n) => patch({ ncmoe: n }, true)}
          />
          <OptionalNum
            label={t("chat.engine.batch")}
            value={draft.batch}
            min={1}
            max={8192}
            step={32}
            placeholder={t("chat.engine.autoPlaceholder")}
            disabled={disabled}
            onCommit={(n) => patch({ batch: n }, true)}
          />
          <OptionalNum
            label={t("chat.engine.ubatch")}
            value={draft.ubatch}
            min={1}
            max={8192}
            step={32}
            placeholder={t("chat.engine.autoPlaceholder")}
            disabled={disabled}
            onCommit={(n) => patch({ ubatch: n }, true)}
          />
          <OptionalNum
            label={t("chat.engine.threads")}
            value={draft.threads}
            min={1}
            max={256}
            placeholder={t("chat.engine.autoPlaceholder")}
            disabled={disabled}
            onCommit={(n) => patch({ threads: n }, true)}
          />
          <OptionalNum
            label={t("chat.engine.parallel")}
            hint={t("chat.engine.parallelHint")}
            value={draft.parallel}
            min={1}
            max={64}
            placeholder={t("chat.engine.autoPlaceholder")}
            disabled={disabled}
            onCommit={(n) => patch({ parallel: n }, true)}
          />
          <div className="flex flex-col gap-2">
            <label className="text-xs text-dim">{t("chat.engine.mmap.label")}</label>
            <p className="text-[11px] leading-relaxed text-dim">
              {t("chat.engine.mmap.hint")}
            </p>
            <Chips
              value={triFrom(draft.mmap)}
              disabled={disabled}
              onChange={(v) => patch({ mmap: triTo(v) }, true)}
              options={[
                { id: "auto", label: t("chat.engine.mmap.auto") },
                { id: "on", label: t("chat.engine.mmap.on") },
                { id: "off", label: t("chat.engine.mmap.off") },
              ]}
            />
          </div>
          <div className="flex flex-col gap-2">
            <label className="text-xs text-dim">{t("chat.engine.mlock.label")}</label>
            <p className="text-[11px] leading-relaxed text-dim">
              {t("chat.engine.mlock.hint")}
            </p>
            <Chips
              value={triFrom(draft.mlock)}
              disabled={disabled}
              onChange={(v) => patch({ mlock: triTo(v) }, true)}
              options={[
                { id: "auto", label: t("chat.engine.mlock.auto") },
                { id: "on", label: t("chat.engine.mlock.on") },
                { id: "off", label: t("chat.engine.mlock.off") },
              ]}
            />
          </div>
        </div>
      )}

      {busyWith.length > 0 && (
        <p className="text-[11px] leading-relaxed text-warn">
          {t("server.busyToApply", {
            who: busyWith.map((w) => t(`server.busyWith.${w}`)).join(", "),
          })}
        </p>
      )}
      {pendingReload && busyWith.length === 0 && (
        <p className="text-[11px] leading-relaxed text-warn">
          {t("chat.engine.saved")}
        </p>
      )}
      {serverRunning &&
        (pendingReload ||
          (draft.ctx != null && ctxLive != null && draft.ctx !== ctxLive)) && (
          <button
            type="button"
            disabled={disabled || generating}
            title={generating ? t("chat.ctx.busy") : undefined}
            onClick={applyNow}
            className="self-start rounded-lg border border-edge px-2.5 py-1.5 text-xs text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
          >
            {applying ? t("chat.ctx.applying") : t("chat.ctx.apply")}
          </button>
        )}
    </div>
  );
}
