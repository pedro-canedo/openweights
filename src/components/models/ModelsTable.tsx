// A biblioteca em tabela: ordenável por coluna, com seleção múltipla para
// excluir de uma vez (o total que volta ao disco vem no diálogo) e "abrir a
// pasta" de cada modelo. A visão em cartões continua sendo a padrão.

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { LocalModel } from "../../lib/types";
import { deleteModel, revealWorkspace } from "../../lib/api";
import {
  idDoModelo,
  ordenarModelos,
  pastaEArquivo,
  somaDeBytes,
  type ColunaDaBiblioteca,
  type Sentido,
} from "../../lib/bibliotecaModelos";
import { formatBytes } from "../../lib/format";
import { navigate } from "../../lib/nav";
import { Button } from "../ui/Button";
import { confirmar } from "../ui/Dialog";
import { toast } from "../ui/Toast";

export default function ModelsTable({
  models,
  aoMudar,
}: {
  models: LocalModel[];
  /** Depois de excluir: a tela recarrega a lista. */
  aoMudar: () => void;
}) {
  const { t } = useTranslation();
  const [coluna, setColuna] = useState<ColunaDaBiblioteca>("name");
  const [sentido, setSentido] = useState<Sentido>("asc");
  const [marcados, setMarcados] = useState<ReadonlySet<string>>(new Set());
  const [apagando, setApagando] = useState(false);

  const ordenados = useMemo(() => ordenarModelos(models, coluna, sentido), [models, coluna, sentido]);
  // O que sumiu da lista (excluído) sai da seleção.
  const presentes = useMemo(() => new Set(models.map(idDoModelo)), [models]);
  const selecionados = ordenados.filter((m) => marcados.has(idDoModelo(m)) && presentes.has(idDoModelo(m)));
  const todos = ordenados.length > 0 && selecionados.length === ordenados.length;

  const ordenarPor = (c: ColunaDaBiblioteca) => {
    if (c === coluna) setSentido((s) => (s === "asc" ? "desc" : "asc"));
    else {
      setColuna(c);
      // Tamanho começa do maior: é o que se procura para liberar espaço.
      setSentido(c === "size" ? "desc" : "asc");
    }
  };
  const alternar = (m: LocalModel) =>
    setMarcados((atual) => {
      const novo = new Set(atual);
      const id = idDoModelo(m);
      if (novo.has(id)) novo.delete(id);
      else novo.add(id);
      return novo;
    });

  const excluir = async (alvo: LocalModel[]) => {
    if (alvo.length === 0) return;
    const total = formatBytes(somaDeBytes(alvo));
    const sim = await confirmar({
      title:
        alvo.length === 1
          ? t("models.table.deleteOne", { name: alvo[0].name })
          : t("models.table.deleteMany", { n: alvo.length }),
      message: t("models.table.deleteFrees", { size: total }),
      confirmLabel: t("models.delete"),
      tone: "danger",
    });
    if (!sim) return;
    setApagando(true);
    const falhas: string[] = [];
    for (const m of alvo) {
      try {
        await deleteModel(m.repoId, m.name);
      } catch (e) {
        falhas.push(`${m.name}: ${String(e)}`);
      }
    }
    setApagando(false);
    setMarcados(new Set());
    aoMudar();
    if (falhas.length > 0) {
      toast({
        tone: "bad",
        message: t("models.table.deleteFailed", { n: falhas.length, details: falhas.join("; ") }),
        duration: 0,
      });
    } else {
      toast({ tone: "ok", message: t("models.table.deleted", { n: alvo.length, size: total }), duration: 5000 });
    }
  };

  const abrirPasta = (m: LocalModel) => {
    const { pasta, arquivo } = pastaEArquivo(m.primaryPath);
    void revealWorkspace(pasta, arquivo).catch((e) =>
      toast({ tone: "bad", message: t("server.preview.revealFailed", { error: String(e) }), duration: 6000 }),
    );
  };

  const cabecalho = (c: ColunaDaBiblioteca, rotulo: string, alinhar = "text-left") => (
    <th
      scope="col"
      aria-sort={coluna === c ? (sentido === "asc" ? "ascending" : "descending") : "none"}
      className={`px-3 py-2 font-medium ${alinhar}`}
    >
      <button
        type="button"
        onClick={() => ordenarPor(c)}
        className="inline-flex items-center gap-1 hover:text-ink"
      >
        {rotulo}
        <span aria-hidden className="text-[10px]">
          {coluna === c ? (sentido === "asc" ? "▲" : "▼") : ""}
        </span>
      </button>
    </th>
  );

  return (
    <div className="flex flex-col gap-3">
      {selecionados.length > 0 && (
        <div role="status" className="flex flex-wrap items-center gap-3 rounded-xl border border-accent/40 bg-accent/5 px-3 py-2 text-sm">
          <span>
            {t("models.table.selected", { n: selecionados.length, size: formatBytes(somaDeBytes(selecionados)) })}
          </span>
          <Button size="sm" variant="danger" busy={apagando} onClick={() => void excluir(selecionados)}>
            {t("models.table.deleteSelected")}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setMarcados(new Set())}>
            {t("models.table.clear")}
          </Button>
        </div>
      )}
      <div className="overflow-x-auto rounded-xl border border-edge">
        <table className="w-full min-w-[640px] text-sm">
          <caption className="sr-only">{t("models.title")}</caption>
          <thead className="border-b border-edge bg-panel2/50 text-xs text-dim">
            <tr>
              <th scope="col" className="w-9 px-3 py-2">
                <input
                  type="checkbox"
                  aria-label={t("models.table.selectAll")}
                  checked={todos}
                  onChange={() =>
                    setMarcados(todos ? new Set() : new Set(ordenados.map(idDoModelo)))
                  }
                  className="accent-accent"
                />
              </th>
              {cabecalho("name", t("models.table.name"))}
              {cabecalho("repo", t("models.table.source"))}
              {cabecalho("quant", t("models.table.quant"))}
              {cabecalho("size", t("models.size"), "text-right")}
              <th scope="col" className="px-3 py-2 text-right font-medium">
                <span className="sr-only">{t("models.table.actions")}</span>
              </th>
            </tr>
          </thead>
          <tbody>
            {ordenados.map((m) => (
              <tr key={idDoModelo(m)} className="border-b border-edge/60 last:border-b-0 hover:bg-panel2/30">
                <td className="px-3 py-2">
                  <input
                    type="checkbox"
                    aria-label={t("models.table.select", { name: m.name })}
                    checked={marcados.has(idDoModelo(m))}
                    onChange={() => alternar(m)}
                    className="accent-accent"
                  />
                </td>
                <td className="max-w-[18rem] truncate px-3 py-2 font-medium text-ink" title={m.name}>
                  {m.name}
                </td>
                <td className="max-w-[14rem] truncate px-3 py-2 text-dim" title={m.repoId}>
                  {m.repoId || t("models.loose")}
                </td>
                <td className="px-3 py-2 text-dim">{m.quantLabel}</td>
                <td className="px-3 py-2 text-right tabular-nums">{formatBytes(m.totalBytes)}</td>
                <td className="px-3 py-2">
                  <div className="flex justify-end gap-1.5">
                    <Button size="sm" variant="primary" onClick={() => navigate("chat", { chatModel: m.name })}>
                      {t("models.chatWith")}
                    </Button>
                    <Button size="sm" onClick={() => abrirPasta(m)} aria-label={t("models.table.openFolderOf", { name: m.name })}>
                      {t("models.table.openFolder")}
                    </Button>
                    <Button size="sm" variant="ghost" onClick={() => void excluir([m])} aria-label={t("models.table.deleteOf", { name: m.name })}>
                      {t("models.delete")}
                    </Button>
                  </div>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}
