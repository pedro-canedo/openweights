import { useEffect, useRef, useState } from "react";
import { Page } from "../components/ui/Shell";
import { isTauri, listen } from "../lib/tauri";
import { navigate } from "../lib/nav";
import { finished, studioRequest, studioInvoke as invoke, standaloneStudio, uploadStudioFile, type StudioDataset, type StudioJob } from "../lib/studio";

const stages: Record<string, string> = {
  queued: "Na fila", preparing: "Preparando", running: "Em andamento", cancelling: "Salvando antes de cancelar",
  cancelled: "Cancelado", interrupted: "Interrompido", failed: "Precisa de atenção", completed: "Concluído",
  benchmark: "Conferindo se o treino cabe na GPU", training: "Treinando seu modelo",
  evaluating: "Conferindo o resultado com o texto reservado", exporting: "Gerando GGUF", ready: "Pronto para conversar",
};
const button = "rounded-xl bg-ink px-5 py-3 text-sm font-medium text-panel disabled:opacity-40";
const secondary = "rounded-xl border border-edge px-4 py-2 text-sm disabled:opacity-40";

export default function Studio() {
  const [installed, setInstalled] = useState<boolean>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [sources, setSources] = useState<{ id: string; name: string }[]>([]);
  const [preset, setPreset] = useState("auto");
  const [dataset, setDataset] = useState<StudioDataset>();
  const [job, setJob] = useState<StudioJob>();
  const [history, setHistory] = useState<StudioJob[]>([]);
  const [stopRequired, setStopRequired] = useState(false);
  const [resumePending, setResumePending] = useState<string>();
  const [context, setContext] = useState(1024);
  const [download, setDownload] = useState("");
  const requestId = useRef<string>("");
  const files = useRef<HTMLInputElement>(null);
  const folder = useRef<HTMLInputElement>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    let unlisten: (() => void) | undefined;
    if (isTauri || standaloneStudio) {
      void invoke<{ installed: boolean }>("studio_status").then(async value => {
        if (!mounted.current) return;
        setInstalled(value.installed);
        if (value.installed) {
          const runs = await studioRequest<StudioJob[]>("/runs");
          if (mounted.current) { setHistory(runs); setJob(runs.find(run => !finished(run))); }
        }
      }).catch(e => { if (mounted.current) setError(String(e)); });
      if (isTauri) void listen<{ received: number; total: number }>("studio-install", e => {
        setDownload(`${Math.round(e.received / e.total * 100)}%`);
      }).then(un => { if (mounted.current) unlisten = un; else un(); });
    } else setInstalled(false);
    return () => { mounted.current = false; unlisten?.(); };
  }, []);

  useEffect(() => {
    if (!job || finished(job)) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const next = await studioRequest<StudioJob>(`/runs/${job.id}`);
        if (cancelled) return;
        setJob(next);
        if (next.kind === "guided_prepare" && next.status === "completed") setDataset(next.result);
        if (finished(next)) {
          setHistory(await studioRequest<StudioJob[]>("/runs"));
          return;
        }
      } catch (e) { if (!cancelled) setError(String(e)); }
      if (!cancelled) timer = setTimeout(poll, 1500);
    };
    timer = setTimeout(poll, 500);
    return () => { cancelled = true; clearTimeout(timer); };
  }, [job?.id, job?.status]);

  async function action(work: () => Promise<void>) {
    setBusy(true); setError("");
    try { await work(); } catch (e) { setError(String(e)); } finally { setBusy(false); }
  }
  async function addFiles(list: File[]) {
    await action(async () => {
      for (const file of list) {
        const added = await uploadStudioFile(file);
        setSources(previous => [...previous, ...added]);
        setDataset(undefined);
      }
    });
  }
  async function train(stopEngine = false) {
    if (!dataset) return;
    await action(async () => {
      requestId.current ||= crypto.randomUUID();
      try {
        const next = await invoke<StudioJob>("studio_train", {
          body: { dataset_id: dataset.id, request_id: requestId.current, context }, stopEngine, resumeId: null,
        });
        setJob(next); setStopRequired(false); requestId.current = "";
      } catch (e) { if (String(e) === "motor_active") setStopRequired(true); else throw e; }
    });
  }
  async function resume(id: string, stopEngine = false) {
    await action(async () => {
      try {
        setJob(await invoke<StudioJob>("studio_train", { body: {}, stopEngine, resumeId: id }));
        setResumePending(undefined);
      } catch (e) { if (String(e) === "motor_active") setResumePending(id); else throw e; }
    });
  }
  const active = !!job && !finished(job);
  const step = job?.chat_model ? 3 : dataset || job?.kind === "workflow" ? 2 : 1;
  const latest = job?.metrics?.at(-1);

  return <Page title="Studio de treinamento" subtitle="Transforme seus textos em um experimento local. Seus arquivos ficam nesta máquina.">
    <ol className="my-8 flex gap-6 text-sm" aria-label="Etapas do treinamento">
      {["Dados", "Treinamento", "Resultado"].map((label, i) => <li key={label} aria-current={step === i + 1 ? "step" : undefined} className={step === i + 1 ? "font-semibold text-ink" : "text-dim"}>{i + 1}. {label}</li>)}
    </ol>
    {error && <div role="alert" className="mb-5 rounded-xl border border-red-400/40 p-4 text-sm">{error}</div>}
    {installed === undefined ? <p role="status">Verificando o módulo…</p> : !installed ?
      <section className="rounded-2xl border border-edge bg-panel p-7">
        <h2 className="text-lg font-semibold">Uma oficina para seus modelos</h2>
        <p className="my-4 max-w-xl text-sm text-dim">Instale o módulo opcional para preparar documentos, treinar na GPU NVIDIA e abrir o resultado no chat. O download inclui as dependências em uma pasta própria.</p>
        <button className={button} disabled={busy || !isTauri} onClick={() => void action(async () => {
          await invoke("studio_install"); setInstalled(true);
        })}>{busy ? `Instalando ${download}` : "Instalar módulo de treinamento"}</button>
        {!isTauri && <p className="mt-3 text-sm text-dim">Abra o aplicativo desktop para usar sua GPU.</p>}
      </section> : <>
        {!standaloneStudio && <details className="mb-5 text-sm"><summary>Já usava o MVP?</summary><p className="my-3 text-dim">Importe uma cópia dos seus dados, modelos e checkpoints em um Studio vazio. Os arquivos originais serão preservados.</p><button className={secondary} disabled={busy || active} onClick={() => void action(async () => { await invoke("studio_import_legacy"); setHistory(await studioRequest<StudioJob[]>("/runs")); })}>Importar dados do MVP</button></details>}
        <section className="rounded-2xl border border-edge bg-panel p-6">
          <h2 className="font-semibold">1. Seus dados</h2>
          <div className="my-4 rounded-xl border-2 border-dashed border-edge p-8 text-center" onDragOver={e => e.preventDefault()} onDrop={e => { e.preventDefault(); if (!busy && !active) void addFiles(Array.from(e.dataTransfer.files)); }}>
            <p className="mb-2">Solte um PDF, textos ou conversas aqui</p>
            <p className="mb-4 text-sm text-dim">Um livro basta para começar. Até 32 MB por arquivo.</p>
            <button className={secondary} disabled={busy || active} onClick={() => files.current?.click()}>Escolher arquivos</button>{" "}
            <button className={secondary} disabled={busy || active} onClick={() => folder.current?.click()}>Escolher pasta</button>
            <input ref={files} type="file" multiple hidden onChange={e => { void addFiles(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
            <input ref={folder} type="file" multiple hidden {...{ webkitdirectory: "" }} onChange={e => { void addFiles(Array.from(e.target.files ?? [])); e.target.value = ""; }} />
          </div>
          {sources.length > 0 && <p className="mb-4 text-sm text-dim">{sources.length} arquivo(s): {sources.slice(0, 3).map(s => s.name).join(", ")}</p>}
          <label className="mr-4 text-sm">Conteúdo <select className="ml-2 rounded-lg border border-edge bg-panel p-2" value={preset} disabled={active || busy} onChange={e => { setPreset(e.target.value); setDataset(undefined); }}>
            <option value="auto">Detectar automaticamente</option><option value="documents">Livro</option><option value="conversations">Conversas</option><option value="code">Código</option>
          </select></label>
          <button className={button} disabled={!sources.length || busy || active} onClick={() => void action(async () => {
            requestId.current = "";
            setDataset(undefined);
            const prepared = await studioRequest<StudioJob>("/prepare", { source_ids: sources.map(s => s.id), preset, name: sources[0].name.replace(/\.[^.]+$/, "").slice(0, 100) });
            setJob(prepared);
            if (prepared.status === "completed") setDataset(prepared.result);
          })}>Preparar</button>
        </section>
        {dataset && <section className="mt-5 rounded-2xl border border-edge p-6">
          <h2 className="font-semibold">2. Treinamento de {dataset.name}</h2>
          <p className="my-3 text-sm text-dim">{dataset.count} trechos aproveitados · {dataset.split_counts.train ?? 0} para treinar · {dataset.split_counts.validation ?? 0} para acompanhar · {dataset.split_counts.test ?? 0} reservados para conferir.</p>
          <p className="my-3 text-sm">Base: Qwen3 0.6B · até 50 passos · GGUF automático · reserve 8 GB de disco.</p>
          {dataset.warnings?.map(w => <p key={w} className="mb-3 text-sm text-dim">{w}</p>)}
          {dataset.next_action && <p className="my-3 text-sm" role="status">{dataset.next_action}</p>}
          <details className="my-4 text-sm"><summary>Avançado</summary><label className="mt-3 block">Contexto <select disabled={active} className="ml-2 bg-panel p-2" value={context} onChange={e => { setContext(Number(e.target.value)); requestId.current = ""; }}><option value={1024}>1.024 tokens</option><option value={512}>512 tokens — usa menos memória</option></select></label></details>
          <button className={button} disabled={!dataset.trainable || busy || active} onClick={() => void train()}>Treinar e gerar modelo</button>
          {stopRequired && <div className="mt-4 rounded-xl border border-edge p-4" role="alert"><p className="mb-3 text-sm">Para iniciar o treinamento, o motor precisa ser parado. Isso encerra as respostas e requisições em andamento.</p><button className={secondary} disabled={busy} onClick={() => void train(true)}>Parar motor e iniciar treino</button></div>}
        </section>}
        {job && <section className="mt-5 rounded-2xl border border-edge bg-panel p-6" aria-live="polite">
          <h2 className="font-semibold">{stages[job.status] ?? job.status}{active && job.stage ? ` · ${stages[job.stage] ?? job.stage}` : ""}</h2>
          {active && <progress className="my-4 w-full" aria-label="Progresso do trabalho" />}
          {latest && <p className="my-3 text-sm text-dim">Passo {latest.step} · memória {latest.vram_gb?.toFixed(1) ?? "—"} GB</p>}
          {job.error && <p className="my-3 text-sm" role="alert">{job.error}</p>}
          {job.kind === "guided_prepare" && finished(job) && job.status !== "completed" && <div className="my-3 flex gap-3">
            {!standaloneStudio && job.error?.includes("OCR") && <button className={secondary} disabled={busy} onClick={() => void action(async () => { await invoke("studio_install", { component: "ocr" }); })}>Instalar leitura de páginas digitalizadas {download}</button>}
            <button className={secondary} disabled={busy} onClick={() => void action(async () => { setJob(await studioRequest<StudioJob>(`/preparations/${job.id}/resume`, {})); })}>Retomar preparação</button>
          </div>}
          {active && <button className={secondary} disabled={busy || job.status === "cancelling"} onClick={() => void action(async () => { setJob(await studioRequest<StudioJob>(`/runs/${job.id}/cancel`, {})); })}>Cancelar e preservar checkpoints</button>}
          {job.chat_model && <><p className="my-3 text-sm text-dim">O modelo está na sua biblioteca. Um treino curto não garante respostas corretas sobre o documento.</p>{standaloneStudio ? <a className={button} href={`/api/v1/runs/${job.id}/download`}>Baixar GGUF</a> : <button className={button} onClick={() => navigate("chat", { chatModel: job.chat_model })}>Abrir no chat</button>}</>}
          {finished(job) && job.status !== "completed" && (!!job.checkpoints?.length || job.can_retry_export) && <button className={secondary} disabled={busy} onClick={() => void resume(job.id)}>{job.can_retry_export ? "Tentar exportação novamente" : "Retomar checkpoint"}</button>}
          {resumePending === job.id && <div className="mt-4" role="alert"><p className="mb-3 text-sm">Retomar exige parar o motor e encerrar respostas e requisições em andamento.</p><button className={secondary} disabled={busy} onClick={() => void resume(job.id, true)}>Parar motor e retomar treino</button></div>}
          <details className="mt-4 text-sm"><summary>Detalhes técnicos</summary><pre className="mt-3 max-h-60 overflow-auto whitespace-pre-wrap text-xs text-dim">{job.log || "Os registros aparecerão durante a execução."}</pre></details>
        </section>}
        {history.length > 0 && <section className="mt-6"><h2 className="mb-3 font-semibold">Trabalhos anteriores</h2><ul className="space-y-2">{history.slice(0, 10).map(run => <li key={run.id}><button className={secondary} disabled={active || busy} onClick={() => void action(async () => {
          const selected = await studioRequest<StudioJob>(`/runs/${run.id}`); setJob(selected);
          if (selected.kind === "guided_prepare" && selected.status === "completed") setDataset(selected.result);
        })}>{run.name} · {stages[run.status] ?? run.status}</button></li>)}</ul></section>}
        {!standaloneStudio && <details className="mt-6 text-sm"><summary>Gerenciar módulo</summary><p className="my-3 text-dim">Remover as dependências do treinamento preserva seus datasets, checkpoints e modelos.</p><button className={secondary} disabled={active || busy} onClick={() => void action(async () => { await invoke("studio_uninstall"); setInstalled(false); })}>Remover módulo de treinamento</button></details>}
      </>}
  </Page>;
}
