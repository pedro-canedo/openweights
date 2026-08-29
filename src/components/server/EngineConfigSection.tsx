// "Configurar llama.cpp": toda a configuração por modelo, na tela do
// servidor — recomendação para o hardware, os controles essenciais, TODAS as
// flags da build (curadas + extraídas do --help), presets nomeados, preview
// do INI real e o botão de carregar o modelo dali mesmo.
//
// Regra de honestidade: o preview vem do backend, renderizado pelas mesmas
// funções que escrevem o INI do boot. E como o Router só lê o INI no boot,
// mudança pendente exige reiniciar ANTES de carregar — o botão de carga
// cuida da ordem sozinho.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  engineBusyReason,
  getModelProfile,
  listLocalModels,
  restartServer,
  setModelProfile,
} from "../../lib/api";
import {
  enginePresetApply,
  enginePresetDelete,
  enginePresetSave,
  enginePresetsList,
  enginePreview,
  flagsCatalog,
  flagsValidate,
  modelCapabilities,
  routerLoadModel,
  routerModels,
  routerUnloadModel,
  type EnginePresetView,
  type EnginePreview,
  type FlagCatalog,
  type FlagIssue,
  type FlagSpec,
  type ModelCaps,
} from "../../lib/flags";
import { takePendingServerModel } from "../../lib/nav";
import { tuneAdvise } from "../../lib/tuning";
import {
  emptyProfile,
  type KvType,
  type ModelProfile,
  type SpecType,
  type VisionMode,
} from "../../lib/tuning";
import {
  Chips,
  NumChips,
  OptionalNum,
  Select,
  chipClass,
  triFrom,
  triTo,
} from "../form/controls";
import FlagControl, { RequirementBadges } from "../form/FlagControl";
import HarnessLauncher from "./HarnessLauncher";

const CTX_CHIPS = [8192, 16384, 32768, 65536];
const CTX_MIN = 512;
const CTX_MAX = 262_144;

function formatCtx(n: number): string {
  if (n >= 1024 && n % 1024 === 0) return `${n / 1024}k`;
  return String(n);
}

type KvChoice = "auto" | KvType;

export default function EngineConfigSection({
  running,
  hasGpu,
  selected: selectedProp,
  onSelect,
}: {
  running: boolean;
  hasGpu: boolean;
  /** Seleção elevada: com o par `selected`/`onSelect`, o pai é o dono do
   *  modelo escolhido (o histórico de benchmark precisa saber qual é).
   *  Sem o par, o componente se comporta como antes (estado interno). */
  selected?: string;
  onSelect?: (model: string) => void;
}) {
  const { t } = useTranslation();
  const [models, setModels] = useState<string[]>([]);
  const [selectedState, setSelectedState] = useState<string>("");
  const controlled = selectedProp !== undefined && onSelect !== undefined;
  const selected = controlled ? selectedProp : selectedState;
  const setSelected = controlled ? onSelect : setSelectedState;
  const [draft, setDraft] = useState<ModelProfile>(emptyProfile());
  const [caps, setCaps] = useState<ModelCaps | null>(null);
  const [catalog, setCatalog] = useState<FlagCatalog | null>(null);
  const [presets, setPresets] = useState<EnginePresetView[]>([]);
  const [routerState, setRouterState] = useState<Map<string, string>>(new Map());
  const [preview, setPreview] = useState<EnginePreview | null>(null);
  const [previewOpen, setPreviewOpen] = useState(false);
  const [issues, setIssues] = useState<FlagIssue[]>([]);
  const [search, setSearch] = useState("");
  const [more, setMore] = useState(false);
  const [pendingReload, setPendingReload] = useState(false);
  const [busyWith, setBusyWith] = useState<string[]>([]);
  const [applying, setApplying] = useState(false);
  const [loadBusy, setLoadBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [advising, setAdvising] = useState(false);
  const [presetName, setPresetName] = useState("");
  const [copied, setCopied] = useState(false);
  const draftRef = useRef(draft);
  draftRef.current = draft;
  const saveTimer = useRef<number | null>(null);

  // Biblioteca + catálogo + presets, uma vez.
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const [locals, cat, pre] = await Promise.all([
        listLocalModels().catch(() => []),
        flagsCatalog().catch(() => null),
        enginePresetsList().catch(() => []),
      ]);
      if (cancelled) return;
      const names = locals.map((m) => m.name);
      setModels(names);
      setCatalog(cat);
      setPresets(pre);
      const wanted = takePendingServerModel();
      const first =
        (wanted && names.find((n) => n === wanted || n.replace(/\.gguf$/i, "") === wanted)) ||
        names[0] ||
        "";
      setSelected(first);
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  // Estado do Router (carregado/descarregado), com o servidor de pé.
  useEffect(() => {
    if (!running) {
      setRouterState(new Map());
      return;
    }
    let alive = true;
    const poll = () =>
      routerModels()
        .then((list) => {
          if (alive) setRouterState(new Map(list.map((m) => [m.id, m.state])));
        })
        .catch(() => {});
    poll();
    const timer = setInterval(poll, 5000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [running, loadBusy]);

  // Perfil + capacidades do modelo selecionado.
  useEffect(() => {
    if (!selected) return;
    let cancelled = false;
    setPendingReload(false);
    setBusyWith([]);
    setError(null);
    void (async () => {
      const [profile, c] = await Promise.all([
        getModelProfile(selected).catch(() => null),
        modelCapabilities(selected).catch(() => null),
      ]);
      if (cancelled) return;
      setDraft(profile ?? emptyProfile());
      setCaps(c);
    })();
    return () => {
      cancelled = true;
    };
  }, [selected]);

  const refreshPreview = useCallback(() => {
    if (!selected) return;
    enginePreview(selected)
      .then(setPreview)
      .catch(() => setPreview(null));
  }, [selected]);
  useEffect(refreshPreview, [refreshPreview]);

  const validate = useCallback(
    (extras: [string, string][]) => {
      flagsValidate("perModel", extras, selected)
        .then(setIssues)
        .catch(() => setIssues([]));
    },
    [selected],
  );

  // Grava com debounce; aplicar (reiniciar) é sempre um gesto explícito.
  const persist = useCallback(
    (next: ModelProfile) => {
      if (!selected) return;
      if (saveTimer.current) window.clearTimeout(saveTimer.current);
      saveTimer.current = window.setTimeout(() => {
        void setModelProfile(selected, next)
          .then((stored) => {
            setDraft(stored);
            if (running) setPendingReload(true);
            refreshPreview();
            validate(stored.extras ?? []);
          })
          .catch((e) => setError(String(e)));
      }, 500);
    },
    [selected, running, refreshPreview, validate],
  );

  const patch = (partial: Partial<ModelProfile>) => {
    const next: ModelProfile = { ...draftRef.current, ...partial, source: "manual" };
    setDraft(next);
    persist(next);
  };

  const applyNow = async () => {
    setApplying(true);
    setBusyWith([]);
    setError(null);
    try {
      await restartServer();
      setPendingReload(false);
    } catch (e) {
      const quem = engineBusyReason(e);
      if (quem) setBusyWith(quem);
      else setError(String(e));
    } finally {
      setApplying(false);
    }
  };

  const load = async () => {
    if (!selected) return;
    setLoadBusy(true);
    setError(null);
    setBusyWith([]);
    try {
      // O INI só é lido no boot: mudança pendente reinicia primeiro.
      if (running && pendingReload) {
        await restartServer();
        setPendingReload(false);
      }
      await routerLoadModel(selected);
    } catch (e) {
      const quem = engineBusyReason(e);
      if (quem) setBusyWith(quem);
      else setError(String(e));
    } finally {
      setLoadBusy(false);
    }
  };

  const unload = async () => {
    if (!selected) return;
    setLoadBusy(true);
    setError(null);
    try {
      await routerUnloadModel(selected);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoadBusy(false);
    }
  };

  const recommend = async () => {
    if (!selected) return;
    setAdvising(true);
    setError(null);
    try {
      const advice = await tuneAdvise(selected);
      const opt = advice.options[advice.recommended];
      if (opt) {
        const stored = await setModelProfile(selected, opt.profile);
        setDraft(stored);
        if (running) setPendingReload(true);
        refreshPreview();
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setAdvising(false);
    }
  };

  const applyPreset = async (id: string) => {
    if (!selected) return;
    try {
      const stored = await enginePresetApply(selected, id);
      setDraft(stored);
      if (running) setPendingReload(true);
      refreshPreview();
      validate(stored.extras ?? []);
    } catch (e) {
      setError(String(e));
    }
  };

  const savePreset = async () => {
    const name = presetName.trim();
    if (!name) return;
    try {
      await enginePresetSave(name, { ...draftRef.current, source: "manual" });
      setPresetName("");
      setPresets(await enginePresetsList());
    } catch (e) {
      setError(String(e));
    }
  };

  // -------------------------------------------------- flags como extras ---
  const extras = draft.extras ?? [];
  const extraValue = (key: string): string | null =>
    extras.find(([k]) => k === key)?.[1] ?? null;
  const setExtra = (key: string, value: string | null) => {
    const rest = extras.filter(([k]) => k !== key);
    patch({ extras: value == null ? rest : [...rest, [key, value]] });
  };

  const matches: FlagSpec[] = useMemo(() => {
    if (!catalog) return [];
    const q = search.trim().toLowerCase();
    if (!q) return [];
    return catalog.flags
      .filter(
        (f) =>
          f.key.includes(q) ||
          f.aliases.some((a) => a.toLowerCase().includes(q)) ||
          (f.curated && t(`flags.catalog.${f.key}.label`, "").toLowerCase().includes(q)),
      )
      .slice(0, 30);
  }, [catalog, search, t]);

  const specChoice: SpecType = draft.spec ?? "none";
  const specOn = specChoice !== "none" || Boolean(draft.specDraftModel);
  const visionChoice: VisionMode = draft.vision ?? "onDemand";
  const kvChoice: KvChoice = draft.kvK ?? draft.kvV ?? "auto";
  const state = routerState.get(selected) ?? null;
  const disabled = !selected || applying;

  const label = "text-xs text-dim";
  const hintCls = "text-[11px] leading-relaxed text-dim";

  if (!models.length) {
    return (
      <div className="mt-4 rounded-xl border border-edge bg-panel p-5">
        <div className="text-sm font-medium">{t("server.engineConfig.title")}</div>
        <p className={`mt-2 ${hintCls}`}>{t("server.engineConfig.noModels")}</p>
      </div>
    );
  }

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel p-5">
      <div className="flex items-center justify-between gap-3">
        <div>
          <div className="text-sm font-medium">{t("server.engineConfig.title")}</div>
          <p className={`mt-0.5 ${hintCls}`}>{t("server.engineConfig.subtitle")}</p>
        </div>
      </div>

      {/* seletor de modelo + estado + carregar */}
      <div className="mt-4 flex flex-wrap items-center gap-2">
        <select
          value={selected}
          onChange={(e) => setSelected(e.target.value)}
          className="min-w-64 flex-1 rounded-lg border border-edge bg-panel2 px-3 py-2 text-sm outline-none focus:border-accent"
        >
          {models.map((m) => (
            <option key={m} value={m}>
              {routerState.get(m) === "loaded" ? "● " : ""}
              {m}
            </option>
          ))}
        </select>
        {state && (
          <span
            className={`rounded-full border px-2 py-0.5 text-[11px] ${
              state === "loaded"
                ? "border-ok/40 bg-ok/10 text-ok"
                : state === "loading"
                  ? "border-warn/40 bg-warn/10 text-warn"
                  : "border-edge text-dim"
            }`}
          >
            {t(`server.engineConfig.state.${state}`, state)}
          </span>
        )}
        {state === "loaded" ? (
          <button
            type="button"
            disabled={loadBusy}
            onClick={() => void unload()}
            className="rounded-lg border border-edge px-3 py-1.5 text-xs font-medium text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-50"
          >
            {loadBusy ? t("common.loading") : t("server.engineConfig.unload")}
          </button>
        ) : (
          <button
            type="button"
            disabled={loadBusy || !selected}
            onClick={() => void load()}
            className="rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-white disabled:opacity-50"
          >
            {loadBusy
              ? t("server.engineConfig.loading")
              : pendingReload && running
                ? t("server.engineConfig.loadRestart")
                : t("server.engineConfig.load")}
          </button>
        )}
      </div>

      {/* fatos do arquivo */}
      {caps && (
        <div className="mt-2 flex flex-wrap gap-1">
          {caps.moe === true && (
            <span className="rounded-full border border-edge px-2 py-0.5 text-[10px] text-dim">
              MoE
            </span>
          )}
          {caps.mtpHead === true && (
            <span className="rounded-full border border-ok/40 bg-ok/10 px-2 py-0.5 text-[10px] text-ok">
              MTP
            </span>
          )}
          {caps.hasMmproj && (
            <span className="rounded-full border border-edge px-2 py-0.5 text-[10px] text-dim">
              {t("server.engineConfig.visionBadge")}
            </span>
          )}
          {caps.trainCtx != null && (
            <span className="rounded-full border border-edge px-2 py-0.5 text-[10px] tabular-nums text-dim">
              {t("server.engineConfig.trainCtx", { n: formatCtx(caps.trainCtx) })}
            </span>
          )}
        </div>
      )}

      {/* presets */}
      <div className="mt-4 flex flex-col gap-2">
        <span className={label}>{t("server.enginePresets.title")}</span>
        <div className="flex flex-wrap items-center gap-1">
          {presets.map((p) => (
            <span key={p.id} className="flex items-center">
              <button
                type="button"
                disabled={disabled}
                onClick={() => void applyPreset(p.id)}
                className={chipClass(false)}
                title={t("server.enginePresets.applyHint")}
              >
                {p.builtin ? t(`server.enginePresets.${p.id}`) : p.name}
              </button>
              {!p.builtin && (
                <button
                  type="button"
                  onClick={() =>
                    void enginePresetDelete(Number(p.id)).then(async () =>
                      setPresets(await enginePresetsList()),
                    )
                  }
                  className="ml-0.5 px-1 text-[11px] text-dim hover:text-bad"
                  title={t("server.enginePresets.delete")}
                >
                  ×
                </button>
              )}
            </span>
          ))}
        </div>
        <div className="flex items-center gap-2">
          <input
            type="text"
            value={presetName}
            onChange={(e) => setPresetName(e.target.value)}
            placeholder={t("server.enginePresets.namePlaceholder")}
            className="w-48 rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-xs outline-none placeholder:text-dim focus:border-accent"
          />
          <button
            type="button"
            disabled={!presetName.trim()}
            onClick={() => void savePreset()}
            className="rounded-lg border border-edge px-2.5 py-1.5 text-xs text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
          >
            {t("server.enginePresets.saveAs")}
          </button>
          <button
            type="button"
            disabled={disabled || advising}
            onClick={() => void recommend()}
            className="ml-auto rounded-lg border border-edge px-2.5 py-1.5 text-xs text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
          >
            {advising
              ? t("server.engineConfig.recommending")
              : t("server.engineConfig.recommend")}
          </button>
        </div>
      </div>

      {/* essenciais */}
      <div className="mt-5 grid gap-5 sm:grid-cols-2">
        <div className="flex flex-col gap-2 sm:col-span-2">
          <span className={label}>{t("chat.ctx.label")}</span>
          <div className="flex flex-wrap items-center gap-1">
            <button
              type="button"
              disabled={disabled}
              onClick={() => patch({ ctx: null })}
              className={chipClass(draft.ctx == null)}
            >
              {t("chat.ctx.auto")}
            </button>
            {CTX_CHIPS.map((n) => (
              <button
                key={n}
                type="button"
                disabled={disabled}
                onClick={() => patch({ ctx: n })}
                className={`${chipClass(draft.ctx === n)} tabular-nums`}
              >
                {formatCtx(n)}
              </button>
            ))}
            <input
              type="number"
              min={CTX_MIN}
              max={CTX_MAX}
              step={256}
              disabled={disabled}
              placeholder={t("chat.ctx.auto")}
              value={draft.ctx ?? ""}
              onChange={(e) => {
                const n = Number(e.target.value);
                setDraft({
                  ...draft,
                  ctx: Number.isFinite(n) && n > 0 ? Math.round(n) : null,
                  source: "manual",
                });
              }}
              onBlur={() => persist(draftRef.current)}
              onKeyDown={(e) => {
                if (e.key === "Enter") (e.target as HTMLInputElement).blur();
              }}
              className="ml-2 w-24 rounded-lg border border-edge bg-panel2 px-2 py-1.5 text-xs tabular-nums outline-none placeholder:text-dim focus:border-accent disabled:opacity-40"
            />
          </div>
        </div>

        <div className="flex flex-col gap-2">
          <span className={label}>{t("chat.engine.kv.label")}</span>
          <p className={hintCls}>{t("chat.engine.kv.hint")}</p>
          <Chips
            value={kvChoice}
            disabled={disabled}
            onChange={(v) => {
              if (v === "auto") patch({ kvK: null, kvV: null });
              else patch({ kvK: v, kvV: v });
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
          <span className={label}>{t("chat.engine.flash.label")}</span>
          <p className={hintCls}>{t("chat.engine.flash.hint")}</p>
          <Chips
            value={triFrom(draft.flashAttn)}
            disabled={disabled}
            onChange={(v) => patch({ flashAttn: triTo(v) })}
            options={[
              { id: "auto", label: t("chat.engine.flash.auto") },
              { id: "on", label: t("chat.engine.flash.on") },
              { id: "off", label: t("chat.engine.flash.off") },
            ]}
          />
        </div>

        <div className="flex flex-col gap-2 sm:col-span-2">
          <span className={label}>{t("chat.engine.spec.label")}</span>
          <p className={hintCls}>{t("chat.engine.spec.hint")}</p>
          {caps?.mtpHead === true && (
            <p className="text-[11px] leading-relaxed text-ok">
              {t("chat.engine.spec.mtpHint")}
            </p>
          )}
          {specChoice === "mtp" && caps?.mtpHead === false && (
            <p className="text-[11px] leading-relaxed text-warn">
              {t("server.engineConfig.mtpMissing")}
            </p>
          )}
          <Chips
            value={specChoice}
            disabled={disabled}
            onChange={(v) => patch({ spec: v })}
            options={[
              { id: "none", label: t("chat.engine.spec.off") },
              { id: "mtp", label: t("chat.engine.spec.mtp") },
              { id: "ngram", label: t("chat.engine.spec.ngram") },
            ]}
          />
          {specOn && (
            <div className="mt-1 flex flex-wrap gap-4">
              <label className="flex flex-col gap-1">
                <span className="text-xs text-dim">
                  {t("server.engineConfig.specNMax")}
                </span>
                <span className="text-[11px] leading-relaxed text-dim">
                  {t("server.engineConfig.specNMaxHint")}
                </span>
                <Select
                  value={
                    draft.specDraftNMax == null
                      ? "auto"
                      : String(draft.specDraftNMax)
                  }
                  disabled={disabled}
                  onChange={(v) =>
                    patch({ specDraftNMax: v === "auto" ? null : Number(v) })
                  }
                  options={[
                    { value: "auto", label: t("server.fields.auto") },
                    ...Array.from({ length: 16 }, (_, i) => ({
                      value: String(i + 1),
                      label: String(i + 1),
                    })),
                  ]}
                  className="self-start"
                />
              </label>
              <OptionalNum
                label={t("server.engineConfig.specPMin")}
                hint={t("server.engineConfig.specPMinHint")}
                value={draft.specDraftPMin}
                min={0}
                max={1}
                step={0.05}
                placeholder={t("chat.engine.autoPlaceholder")}
                disabled={disabled}
                onCommit={(n) => patch({ specDraftPMin: n })}
              />
              <OptionalNum
                label={t("server.engineConfig.specNMin")}
                value={draft.specDraftNMin}
                min={0}
                max={16}
                placeholder={t("chat.engine.autoPlaceholder")}
                disabled={disabled}
                onCommit={(n) => patch({ specDraftNMin: n })}
              />
            </div>
          )}
        </div>

        {caps?.hasMmproj && (
          <div className="flex flex-col gap-2">
            <span className={label}>{t("chat.engine.vision.label")}</span>
            <p className={hintCls}>{t("chat.engine.vision.hint")}</p>
            <Chips
              value={visionChoice}
              disabled={disabled}
              onChange={(v) => patch({ vision: v })}
              options={[
                { id: "off", label: t("chat.engine.vision.off") },
                { id: "onDemand", label: t("chat.engine.vision.onDemand") },
                { id: "always", label: t("chat.engine.vision.always") },
              ]}
            />
          </div>
        )}

        <div className="flex flex-col gap-2">
          <span className={label}>{t("chat.engine.kvOffload.label")}</span>
          <p className={hintCls}>{t("chat.engine.kvOffload.hint")}</p>
          <Chips
            value={triFrom(draft.kvOffload)}
            disabled={disabled}
            onChange={(v) => patch({ kvOffload: triTo(v) })}
            options={[
              { id: "auto", label: t("chat.engine.kvOffload.auto") },
              { id: "on", label: t("chat.engine.kvOffload.on") },
              { id: "off", label: t("chat.engine.kvOffload.off") },
            ]}
          />
        </div>
      </div>

      <button
        type="button"
        onClick={() => setMore((v) => !v)}
        className="mt-4 self-start text-[11px] text-dim transition-colors hover:text-ink"
      >
        {t("chat.engine.more")}
        {more ? " ▾" : " ▸"}
      </button>

      {more && (
        <div className="mt-3 grid gap-4 sm:grid-cols-3">
          <OptionalNum
            label={t("chat.engine.ngl")}
            hint={t("chat.engine.nglHint")}
            value={draft.ngl}
            min={0}
            max={256}
            placeholder={t("chat.engine.autoPlaceholder")}
            disabled={disabled}
            onCommit={(n) => patch({ ngl: n })}
          />
          <OptionalNum
            label={t("chat.engine.ncmoe")}
            hint={t("chat.engine.ncmoeHint")}
            value={draft.ncmoe}
            min={0}
            max={256}
            placeholder={t("chat.engine.autoPlaceholder")}
            disabled={disabled}
            onCommit={(n) => patch({ ncmoe: n })}
          />
          <OptionalNum
            label={t("chat.engine.threads")}
            value={draft.threads}
            min={1}
            max={256}
            placeholder={t("chat.engine.autoPlaceholder")}
            disabled={disabled}
            onCommit={(n) => patch({ threads: n })}
          />
          <div className="flex flex-col gap-2">
            <span className={label}>{t("chat.engine.batch")}</span>
            <NumChips
              value={draft.batch ?? null}
              suggestions={[256, 512, 1024, 2048]}
              min={1}
              max={8192}
              step={32}
              allowAuto
              disabled={disabled}
              placeholder={t("chat.engine.autoPlaceholder")}
              onCommit={(n) => patch({ batch: n })}
            />
          </div>
          <div className="flex flex-col gap-2">
            <span className={label}>{t("chat.engine.ubatch")}</span>
            <NumChips
              value={draft.ubatch ?? null}
              suggestions={[128, 256, 512, 1024]}
              min={1}
              max={8192}
              step={32}
              allowAuto
              disabled={disabled}
              placeholder={t("chat.engine.autoPlaceholder")}
              onCommit={(n) => patch({ ubatch: n })}
            />
          </div>
          <div className="flex flex-col gap-2">
            <span className={label}>{t("chat.engine.parallel")}</span>
            <p className={hintCls}>{t("chat.engine.parallelHint")}</p>
            <NumChips
              value={draft.parallel ?? null}
              suggestions={[1, 2, 4, 8]}
              min={1}
              max={64}
              allowAuto
              disabled={disabled}
              placeholder={t("chat.engine.autoPlaceholder")}
              onCommit={(n) => patch({ parallel: n })}
            />
          </div>
          <div className="flex flex-col gap-2">
            <span className={label}>{t("chat.engine.mmap.label")}</span>
            <Chips
              value={triFrom(draft.mmap)}
              disabled={disabled}
              onChange={(v) => patch({ mmap: triTo(v) })}
              options={[
                { id: "auto", label: t("chat.engine.mmap.auto") },
                { id: "on", label: t("chat.engine.mmap.on") },
                { id: "off", label: t("chat.engine.mmap.off") },
              ]}
            />
          </div>
          <div className="flex flex-col gap-2">
            <span className={label}>{t("chat.engine.mlock.label")}</span>
            <Chips
              value={triFrom(draft.mlock)}
              disabled={disabled}
              onChange={(v) => patch({ mlock: triTo(v) })}
              options={[
                { id: "auto", label: t("chat.engine.mlock.auto") },
                { id: "on", label: t("chat.engine.mlock.on") },
                { id: "off", label: t("chat.engine.mlock.off") },
              ]}
            />
          </div>
          <div className="flex flex-col gap-2 sm:col-span-3">
            <span className={label}>{t("server.engineConfig.draftModel")}</span>
            <p className={hintCls}>{t("server.engineConfig.draftModelHint")}</p>
            <input
              type="text"
              disabled={disabled}
              value={draft.specDraftModel ?? ""}
              placeholder={t("flags.control.pathPlaceholder")}
              onChange={(e) =>
                setDraft({ ...draft, specDraftModel: e.target.value || null, source: "manual" })
              }
              onBlur={() => persist(draftRef.current)}
              className="w-full rounded-lg border border-edge bg-panel2 px-2 py-1.5 font-mono text-xs outline-none placeholder:text-dim focus:border-accent disabled:opacity-40"
            />
          </div>
        </div>
      )}

      {/* todas as flags */}
      <div className="mt-5 flex flex-col gap-2 border-t border-edge pt-4">
        <div className="flex items-center justify-between gap-2">
          <span className="text-sm font-medium">{t("server.engineConfig.allFlags")}</span>
          {catalog && (
            <span className="text-[11px] tabular-nums text-dim">
              {t("server.engineConfig.flagCount", {
                n: catalog.flags.length,
                tag: catalog.tag,
              })}
            </span>
          )}
        </div>
        {catalog?.degraded && (
          <p className="text-[11px] leading-relaxed text-warn">
            {t("server.engineConfig.degraded")}
          </p>
        )}
        {extras.length > 0 && (
          <div className="flex flex-wrap items-center gap-1">
            <span className="text-[11px] text-dim">{t("server.engineConfig.active")}</span>
            {extras.map(([k, v]) => (
              <span
                key={k}
                className="flex items-center gap-1 rounded-full border border-accent bg-accent/10 px-2 py-0.5 font-mono text-[11px] text-ink"
              >
                {k} = {v}
                <button
                  type="button"
                  onClick={() => setExtra(k, null)}
                  className="text-dim hover:text-bad"
                  title={t("server.engineConfig.remove")}
                >
                  ×
                </button>
              </span>
            ))}
          </div>
        )}
        {issues.length > 0 && (
          <div className="rounded-lg border border-warn/40 bg-warn/10 px-3 py-2">
            {issues.map((i) => (
              <p key={`${i.key}-${i.code}`} className="text-[11px] leading-relaxed text-warn">
                <span className="font-mono">{i.key}</span>:{" "}
                {t(`flags.issues.${i.code}`, { detail: i.detail })}
              </p>
            ))}
          </div>
        )}
        <input
          type="text"
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          placeholder={t("server.engineConfig.searchPlaceholder")}
          className="w-full rounded-lg border border-edge bg-panel2 px-3 py-2 text-sm outline-none placeholder:text-dim focus:border-accent"
        />
        {search.trim() && (
          <div className="flex flex-col gap-3">
            {matches.length === 0 && (
              <p className={hintCls}>{t("server.engineConfig.noMatches")}</p>
            )}
            {matches.map((f) => (
              <FlagRow
                key={f.key}
                spec={f}
                caps={caps}
                hasGpu={hasGpu}
                value={extraValue(f.key)}
                disabled={disabled}
                onChange={(v) => setExtra(f.key, v)}
              />
            ))}
          </div>
        )}
      </div>

      {/* preview */}
      <div className="mt-4 flex flex-col gap-2 border-t border-edge pt-4">
        <button
          type="button"
          onClick={() => setPreviewOpen((v) => !v)}
          className="flex items-center justify-between text-sm"
        >
          {t("server.engineConfig.preview")}
          <span className="text-dim">{previewOpen ? "▾" : "▸"}</span>
        </button>
        {previewOpen && preview && (
          <div className="relative">
            <pre className="select-text overflow-x-auto rounded-lg border border-edge bg-panel2 p-3 font-mono text-[11.5px] leading-relaxed text-dim">
              {`# llama-server ${preview.args.join(" ")}\n\n# ${preview.iniPath}\n${preview.ini}`}
            </pre>
            <button
              type="button"
              onClick={() => {
                void navigator.clipboard
                  .writeText(`llama-server ${preview.args.join(" ")}\n\n${preview.ini}`)
                  .then(() => {
                    setCopied(true);
                    setTimeout(() => setCopied(false), 1200);
                  });
              }}
              className="absolute right-2 top-2 rounded-md border border-edge bg-panel px-2 py-1 text-[11px] text-dim hover:text-ink"
            >
              {copied ? t("server.copied") : t("server.copy")}
            </button>
          </div>
        )}
      </div>

      {/* estado de aplicação */}
      {busyWith.length > 0 && (
        <p className="mt-3 text-[11px] leading-relaxed text-warn">
          {t("server.busyToApply", {
            who: busyWith.map((w) => t(`server.busyWith.${w}`)).join(", "),
          })}
        </p>
      )}
      {error && <p className="mt-3 text-[12px] text-bad">{error}</p>}
      {running && pendingReload && busyWith.length === 0 && (
        <div className="mt-3 flex items-center gap-3">
          <span className="text-[11px] text-warn">{t("server.engineConfig.needsRestart")}</span>
          <button
            type="button"
            disabled={applying}
            onClick={() => void applyNow()}
            className="rounded-lg border border-edge px-2.5 py-1.5 text-xs text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
          >
            {applying ? t("chat.ctx.applying") : t("chat.ctx.apply")}
          </button>
        </div>
      )}

      {/* abrir num harness externo */}
      <HarnessLauncher
        model={selected}
        loaded={state === "loaded"}
        running={running}
      />
    </div>
  );
}

function FlagRow({
  spec,
  caps,
  hasGpu,
  value,
  disabled,
  onChange,
}: {
  spec: FlagSpec;
  caps: ModelCaps | null;
  hasGpu: boolean;
  value: string | null;
  disabled?: boolean;
  onChange: (v: string | null) => void;
}) {
  const { t } = useTranslation();
  const curatedLabel = spec.curated ? t(`flags.catalog.${spec.key}.label`, "") : "";
  const curatedHint = spec.curated ? t(`flags.catalog.${spec.key}.hint`, "") : "";
  const managed = spec.scope === "managed";
  const typed = spec.typedField != null;
  const wrongScope = spec.scope === "global";

  return (
    <div className="rounded-lg border border-edge bg-panel2/50 p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-xs text-ink">--{spec.key}</span>
        {curatedLabel && <span className="text-xs text-dim">{curatedLabel}</span>}
        <RequirementBadges spec={spec} caps={caps} hasGpu={hasGpu} />
        {spec.default && (
          <span className="text-[10px] text-dim">
            {t("server.engineConfig.default", { v: spec.default })}
          </span>
        )}
      </div>
      {(curatedHint || spec.helpText) && (
        <p className="mt-1 text-[11px] leading-relaxed text-dim">
          {curatedHint || spec.helpText}
        </p>
      )}
      <div className="mt-2">
        {managed ? (
          <p className="text-[11px] text-dim">🔒 {t("server.engineConfig.managed")}</p>
        ) : typed ? (
          <p className="text-[11px] text-dim">{t("server.engineConfig.typedNote")}</p>
        ) : wrongScope ? (
          <p className="text-[11px] text-dim">{t("server.engineConfig.globalNote")}</p>
        ) : (
          <FlagControl spec={spec} value={value} disabled={disabled} onChange={onChange} />
        )}
      </div>
    </div>
  );
}
