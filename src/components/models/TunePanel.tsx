// "Ajustar para esta máquina": a configuração recomendada para um modelo.
//
// O nome importa. Não é "otimizar" — o que o painel faz é descobrir o que
// cabe nesta placa e explicar por quê, com o número que o próprio llama.cpp
// devolveu. Prometer otimização criaria expectativa de mágica onde há conta.
//
// Duas propostas lado a lado, e não uma: com a mesma placa, "mais janela" e
// "mais folga" são as duas respostas defensáveis, e mostrar as duas é o que
// torna a explicação verificável em vez de assertiva.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { engineBusyReason } from "../../lib/api";
import { formatBytes } from "../../lib/format";
import { errorMessage } from "../../lib/serverSession";
import { listen } from "../../lib/tauri";
import {
  tuneAdvise,
  tuneApply,
  tuneBench,
  tuneBenchCancel,
  tuneSpecBench,
  type BenchProgress,
  type BenchResult,
  type ModelProfile,
  type SpecOutcome,
  type TuneAdvice,
  type TuneOption,
} from "../../lib/tuning";

/** Resumo de uma configuração em uma linha. */
function resumo(t: TFunction, p: ModelProfile): string {
  const partes: string[] = [];
  if (p.ctx) partes.push(t("tune.ctxChip", { n: (p.ctx / 1024).toFixed(0) }));
  if (p.kvK && p.kvK !== "f16") partes.push(t("tune.kvChip", { kv: p.kvK }));
  if (p.flashAttn) partes.push(t("tune.flashChip"));
  return partes.join(" · ");
}

function OptionCard({
  option,
  recommended,
  selected,
  onSelect,
  vramBytes,
  measured,
}: {
  option: TuneOption;
  recommended: boolean;
  selected: boolean;
  onSelect: () => void;
  vramBytes: number;
  /** Tokens por segundo medidos nesta máquina, quando já houve medição. */
  measured?: BenchResult | null;
}) {
  const { t } = useTranslation();
  return (
    <button
      type="button"
      onClick={onSelect}
      aria-pressed={selected}
      className={`flex min-w-0 flex-col items-start gap-1 rounded-xl border px-3 py-2.5 text-left transition-colors ${
        selected
          ? "border-accent bg-accent/10"
          : "border-edge hover:border-accent/60"
      }`}
    >
      <span className="flex w-full items-center gap-2">
        <span className="text-[12px] font-medium text-ink">
          {t(`tune.intent.${option.intent}`)}
        </span>
        {recommended && (
          <span className="rounded-md bg-ok/15 px-1.5 py-0.5 text-[10px] font-medium text-ok">
            {t("tune.recommended")}
          </span>
        )}
      </span>
      <span className="text-[11px] text-dim">{resumo(t, option.profile)}</span>
      {/* O número que sustenta tudo: veio da sonda, não da nossa conta. */}
      <span
        className={`text-[11px] tabular-nums ${option.fitsGpu ? "text-dim" : "text-warn"}`}
      >
        {vramBytes > 0
          ? t("tune.memGpu", {
              used: formatBytes(option.gpuBytes),
              total: formatBytes(vramBytes),
            })
          : t("tune.memHost", { used: formatBytes(option.hostBytes) })}
      </span>
      {!option.fitsGpu && (
        <span className="text-[11px] leading-relaxed text-warn">
          {t("tune.partial")}
        </span>
      )}
      {/* Medido vale mais que estimado, e a tela precisa dizer qual é qual. */}
      {measured && (
        <span className="mt-0.5 rounded-md bg-ok/10 px-1.5 py-0.5 text-[11px] tabular-nums text-ok">
          {t("tune.tested", { tps: measured.genTps.toFixed(1) })}
        </span>
      )}
    </button>
  );
}

export default function TunePanel({
  model,
  onClose,
}: {
  model: string;
  onClose: () => void;
}) {
  const { t } = useTranslation();
  const [advice, setAdvice] = useState<TuneAdvice | null>(null);
  const [escolhido, setEscolhido] = useState(0);
  const [carregando, setCarregando] = useState(true);
  const [erro, setErro] = useState<string | null>(null);
  const [aplicando, setAplicando] = useState(false);
  const [resultado, setResultado] = useState<string | null>(null);
  const [ocupado, setOcupado] = useState<string[]>([]);
  /// Medição: tok/s reais por configuração, indexados como em `options`.
  const [medidos, setMedidos] = useState<(BenchResult | null)[]>([]);
  const [medindo, setMedindo] = useState<BenchProgress | null>(null);
  const [suspeito, setSuspeito] = useState(false);
  /// Medição de especulação (o eixo MTP/n-grama).
  const [spec, setSpec] = useState<SpecOutcome | null>(null);
  const [medindoSpec, setMedindoSpec] = useState(false);

  useEffect(() => {
    let vivo = true;
    setCarregando(true);
    setErro(null);
    tuneAdvise(model)
      .then((a) => {
        if (!vivo) return;
        setAdvice(a);
        setEscolhido(a.recommended);
      })
      .catch((e) => vivo && setErro(errorMessage(e)))
      .finally(() => vivo && setCarregando(false));
    return () => {
      vivo = false;
    };
  }, [model]);

  // A varredura automática pode ter trocado o perfil por baixo do painel —
  // parear com outra máquina muda o que cabe tanto quanto trocar de placa.
  useEffect(() => {
    const parar = listen<number>("tune-auto", () => {
      tuneAdvise(model)
        .then((a) => {
          setAdvice(a);
          setEscolhido(a.recommended);
        })
        .catch(() => {});
    });
    return () => {
      void parar.then((f) => f());
    };
  }, [model]);

  // O progresso vem por evento: uma medição leva minutos, e um spinner mudo
  // durante minutos é indistinguível de travamento.
  useEffect(() => {
    const parar = listen<BenchProgress>("tune-bench", (p) => {
      if (p.model === model || p.model.startsWith(model)) setMedindo(p);
    });
    return () => {
      void parar.then((f) => f());
    };
  }, [model]);

  async function medir(force = false) {
    if (!advice) return;
    setMedindo({ model, step: 0, total: advice.options.length });
    setOcupado([]);
    setResultado(null);
    try {
      const r = await tuneBench(
        model,
        advice.options.map((o) => o.profile),
        force,
      );
      const porIndice = advice.options.map(
        (o) =>
          r.results.find(([p]) => p.ctx === o.profile.ctx && p.kvK === o.profile.kvK)?.[1] ??
          null,
      );
      setMedidos(porIndice);
      setSuspeito(r.suspect);
      // O que rendeu mais passa a ser a escolha em foco: foi medido, não
      // estimado.
      const melhor = porIndice.reduce(
        (best, atual, i) =>
          atual && (best < 0 || (porIndice[best]?.genTps ?? 0) < atual.genTps) ? i : best,
        -1,
      );
      if (melhor >= 0) setEscolhido(melhor);
    } catch (e) {
      const quem = engineBusyReason(e);
      if (quem) setOcupado(quem);
      else setResultado(errorMessage(e));
    } finally {
      setMedindo(null);
    }
  }

  async function medirSpec(force = false) {
    if (!advice) return;
    setMedindoSpec(true);
    setOcupado([]);
    setResultado(null);
    try {
      setSpec(
        await tuneSpecBench(model, advice.options[escolhido].profile, force),
      );
    } catch (e) {
      const quem = engineBusyReason(e);
      if (quem) setOcupado(quem);
      else setResultado(errorMessage(e));
    } finally {
      setMedindoSpec(false);
    }
  }

  async function aplicar(force = false) {
    if (!advice) return;
    setAplicando(true);
    setResultado(null);
    setOcupado([]);
    try {
      const r = await tuneApply(model, advice.options[escolhido].profile, force);
      // Não carregar não é exceção: o backend já restaurou o perfil anterior,
      // e o que falta é contar o que houve.
      setResultado(r.ok ? "ok" : (r.error ?? "erro"));
    } catch (e) {
      const quem = engineBusyReason(e);
      if (quem) setOcupado(quem);
      else setResultado(errorMessage(e));
    } finally {
      setAplicando(false);
    }
  }

  return (
    <div className="rounded-xl border border-accent/40 bg-panel2 p-4">
      <div className="flex items-baseline gap-2">
        <span className="text-[13px] font-medium text-ink">
          {t("tune.title")}
        </span>
        <span className="min-w-0 flex-1 truncate text-[11px] text-dim" title={model}>
          {model}
        </span>
        <button
          type="button"
          onClick={onClose}
          className="rounded px-1 text-[11px] text-dim hover:text-ink"
        >
          {t("common.close")}
        </button>
      </div>

      {carregando && (
        <p className="mt-2 text-[11px] text-dim">{t("tune.measuring")}</p>
      )}
      {erro && <p className="mt-2 text-[11px] text-bad">{erro}</p>}

      {advice && (
        <>
          <div className="mt-3 grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-4">
            {advice.options.map((o, i) => (
              <OptionCard
                key={i}
                option={o}
                recommended={i === advice.recommended}
                selected={i === escolhido}
                onSelect={() => setEscolhido(i)}
                vramBytes={advice.vramBytes}
                measured={medidos[i]}
              />
            ))}
          </div>

          {/* Por que esta configuração — com os números, não com adjetivos. */}
          <ul className="mt-2.5 flex flex-col gap-1">
            {advice.reasons.map((r, i) => (
              <li key={i} className="text-[11px] leading-relaxed text-dim">
                {t(
                  `tune.reason.${r.key}`,
                  Object.fromEntries(
                    r.values.map(([k, v]) => [
                      k,
                      k === "ctx" ? v : /^\d+$/.test(v) ? formatBytes(Number(v)) : v,
                    ]),
                  ),
                )}
              </li>
            ))}
          </ul>

          {advice.facts.length > 0 && (
            <div className="mt-2 flex flex-wrap gap-1.5">
              {advice.facts.map((f) => (
                <span
                  key={f}
                  className="rounded-md border border-edge px-1.5 py-0.5 text-[10px] text-dim"
                  title={t(`tune.fact.${f}Hint`)}
                >
                  {t(`tune.fact.${f}`)}
                </span>
              ))}
            </div>
          )}

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <button
              type="button"
              disabled={aplicando}
              onClick={() => void aplicar()}
              className="rounded-lg bg-accent px-3 py-1.5 text-xs font-medium text-white transition-opacity hover:opacity-90 disabled:opacity-40"
            >
              {aplicando ? t("tune.applying") : t("tune.apply")}
            </button>
            {medindo ? (
              <>
                <span className="text-[11px] text-dim">
                  {medindo.total > 0
                    ? t("tune.benchProgress", {
                        step: Math.max(1, medindo.step),
                        total: medindo.total,
                      })
                    : t("tune.benchStarting")}
                </span>
                <button
                  type="button"
                  onClick={() => void tuneBenchCancel()}
                  className="rounded-lg border border-edge px-2 py-1 text-[11px] text-dim hover:text-ink"
                >
                  {t("common.cancel")}
                </button>
              </>
            ) : (
              <button
                type="button"
                disabled={aplicando}
                onClick={() => void medir()}
                title={t("tune.benchHint")}
                className="rounded-lg border border-edge px-3 py-1.5 text-xs text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
              >
                {t("tune.bench")}
              </button>
            )}
            {resultado === "ok" && (
              <span className="text-[11px] text-ok">{t("tune.applied")}</span>
            )}
            {resultado && resultado !== "ok" && (
              <span className="text-[11px] leading-relaxed text-warn">
                {t("tune.rolledBack", { error: resultado })}
              </span>
            )}
          </div>

          {/* Especulação: só aparece quando há o que medir, e o resultado é
              dito como ele é — inclusive "não muda nada aqui". */}
          {(advice.facts.includes("mtp") || spec) && (
            <div className="mt-2.5 rounded-lg border border-edge px-2.5 py-2">
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-[11px] text-dim">
                  {t("tune.spec.title")}
                </span>
                <button
                  type="button"
                  disabled={medindoSpec || !!medindo}
                  onClick={() => void medirSpec()}
                  title={t("tune.spec.hint")}
                  className="rounded-lg border border-edge px-2 py-1 text-[11px] text-dim transition-colors hover:border-accent hover:text-ink disabled:opacity-40"
                >
                  {medindoSpec ? t("tune.spec.measuring") : t("tune.spec.measure")}
                </button>
              </div>
              {spec && (
                <ul className="mt-1.5 flex flex-col gap-0.5">
                  {spec.arms.map((a, i) => (
                    <li
                      key={a.spec}
                      className={`text-[11px] tabular-nums ${
                        i === spec.best && !spec.inconclusive
                          ? "text-ok"
                          : "text-dim"
                      }`}
                    >
                      {t(`tune.spec.arm.${a.spec}`)}
                      {": "}
                      {a.byPrompt
                        .map(
                          ([tipo, tps]) =>
                            `${t(`tune.spec.prompt.${tipo}`)} ${tps.toFixed(1)}`,
                        )
                        .join(" · ")}
                      {" tok/s"}
                    </li>
                  ))}
                </ul>
              )}
              {spec?.inconclusive && (
                <p className="mt-1 text-[11px] leading-relaxed text-dim">
                  {t("tune.spec.inconclusive")}
                </p>
              )}
            </div>
          )}

          {suspeito && (
            <p className="mt-2 text-[11px] leading-relaxed text-warn">
              {t("tune.benchSuspect")}
            </p>
          )}

          {ocupado.length > 0 && (
            <div className="mt-2 flex flex-wrap items-center gap-2">
              <span className="text-[11px] leading-relaxed text-warn">
                {t("server.busyToApply", {
                  who: ocupado.map((w) => t(`server.busyWith.${w}`)).join(", "),
                })}
              </span>
              <button
                type="button"
                onClick={() => void (medidos.length ? aplicar(true) : medir(true))}
                className="rounded-lg border border-warn/40 px-2 py-1 text-[11px] text-warn hover:bg-warn/10"
              >
                {t("tune.applyAnyway")}
              </button>
            </div>
          )}
        </>
      )}
    </div>
  );
}
