// A paleta de comandos (Ctrl+K) e a folha de atalhos (?).
//
// A busca é um combobox com a lista de comandos: as setas andam, Enter
// executa, Esc fecha. O registro mora em `lib/comandos.ts`.

import { Fragment, useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { filtrarComandos, type Comando, type GrupoDeComando } from "../lib/comandos";
import { Dialog } from "./ui/Dialog";

export default function CommandPalette({
  aberta,
  aoFechar,
  comandos,
}: {
  aberta: boolean;
  aoFechar: () => void;
  comandos: Comando[];
}) {
  const { t } = useTranslation();
  const [busca, setBusca] = useState("");
  const [ativo, setAtivo] = useState(0);
  const achados = useMemo(() => filtrarComandos(comandos, busca), [comandos, busca]);

  useEffect(() => {
    if (aberta) {
      setBusca("");
      setAtivo(0);
    }
  }, [aberta]);
  useEffect(() => setAtivo(0), [busca]);

  // A opção ativa sempre à vista quando as setas passam do fim da lista.
  useEffect(() => {
    document
      .getElementById(`comando-${achados[ativo]?.id}`)
      ?.scrollIntoView({ block: "nearest" });
  }, [ativo, achados]);

  function executar(c: Comando | undefined) {
    if (!c) return;
    aoFechar();
    c.executar();
  }

  function tecla(e: React.KeyboardEvent<HTMLInputElement>) {
    const n = achados.length;
    if (e.key === "ArrowDown" && n > 0) {
      e.preventDefault();
      setAtivo((i) => (i + 1) % n);
    } else if (e.key === "ArrowUp" && n > 0) {
      e.preventDefault();
      setAtivo((i) => (i - 1 + n) % n);
    } else if (e.key === "Home" && n > 0) {
      e.preventDefault();
      setAtivo(0);
    } else if (e.key === "End" && n > 0) {
      e.preventDefault();
      setAtivo(n - 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      executar(achados[ativo]);
    }
  }

  // Sem busca, os comandos aparecem em grupos; com busca, na ordem do achado.
  const grupos: [GrupoDeComando | null, Comando[]][] = busca.trim()
    ? [[null, achados]]
    : (["screens", "actions", "server", "preferences"] as const)
        .map((g) => [g, achados.filter((c) => c.grupo === g)] as [GrupoDeComando, Comando[]])
        .filter(([, cs]) => cs.length > 0);

  return (
    <Dialog open={aberta} onClose={aoFechar} title={t("palette.title")} hideTitle className="p-3">
      <input
        type="text"
        role="combobox"
        aria-label={t("palette.title")}
        aria-expanded="true"
        aria-controls="paleta-lista"
        aria-autocomplete="list"
        aria-activedescendant={achados[ativo] ? `comando-${achados[ativo].id}` : undefined}
        placeholder={t("palette.placeholder")}
        value={busca}
        onChange={(e) => setBusca(e.target.value)}
        onKeyDown={tecla}
        data-autofocus=""
        className="w-full rounded-lg border border-edge-strong bg-panel2 px-3 py-2 text-sm text-ink placeholder:text-dim outline-none focus:border-accent-ink"
      />
      <div
        id="paleta-lista"
        role="listbox"
        aria-label={t("palette.results")}
        className="mt-2 max-h-[55vh] overflow-y-auto"
      >
        {achados.length === 0 && (
          <p className="px-3 py-6 text-center text-sm text-dim">
            {t("palette.empty", { query: busca.trim() })}
          </p>
        )}
        {grupos.map(([g, cs]) => (
          <Fragment key={g ?? "busca"}>
            <div role="group" aria-labelledby={g ? `paleta-grupo-${g}` : undefined} aria-label={g ? undefined : t("palette.results")}>
              {g && (
                <div
                  id={`paleta-grupo-${g}`}
                  role="presentation"
                  className="px-3 pt-2 pb-1 text-[10.5px] font-semibold tracking-wide text-dim uppercase"
                >
                  {t(`palette.group.${g}`)}
                </div>
              )}
              {cs.map((c) => {
                const i = achados.indexOf(c);
                return (
                  <div
                    key={c.id}
                    id={`comando-${c.id}`}
                    role="option"
                    aria-selected={i === ativo}
                    onMouseMove={() => setAtivo(i)}
                    onClick={() => executar(c)}
                    className={`flex cursor-pointer items-center gap-3 rounded-lg px-3 py-2 text-[13px] ${
                      i === ativo ? "bg-panel2 text-ink" : "text-dim"
                    }`}
                  >
                    <span className="min-w-0 flex-1 truncate">{c.rotulo}</span>
                    {c.atalho && <Tecla>{c.atalho}</Tecla>}
                  </div>
                );
              })}
            </div>
          </Fragment>
        ))}
      </div>
    </Dialog>
  );
}

function Tecla({ children }: { children: string }) {
  return (
    <kbd className="shrink-0 rounded border border-edge bg-panel px-1.5 py-0.5 font-mono text-[10.5px] text-dim">
      {children}
    </kbd>
  );
}

/** A folha de atalhos: o que o teclado faz em cada lugar do app. */
export function FolhaDeAtalhos({ aberta, aoFechar }: { aberta: boolean; aoFechar: () => void }) {
  const { t } = useTranslation();
  const linhas: [string, string][] = [
    ["Ctrl+K", t("shortcuts.palette")],
    ["Ctrl+Shift+K", t("shortcuts.paletteInTerminal")],
    ["Ctrl+N", t("shortcuts.newChat")],
    ["Ctrl+,", t("shortcuts.settings")],
    ["Ctrl+1…9", t("shortcuts.screens")],
    ["Ctrl+Shift+L", t("shortcuts.logs")],
    ["?", t("shortcuts.sheet")],
    ["Enter", t("shortcuts.send")],
    ["Shift+Enter", t("shortcuts.newLine")],
    ["Ctrl+U", t("shortcuts.attach")],
    ["Ctrl+Shift+C", t("shortcuts.terminalCopy")],
    ["Ctrl+Shift+V", t("shortcuts.terminalPaste")],
  ];
  return (
    <Dialog open={aberta} onClose={aoFechar} title={t("shortcuts.title")} description={t("shortcuts.hint")}>
      <table className="mt-4 w-full text-[13px]">
        <tbody className="divide-y divide-edge">
          {linhas.map(([tecla, oQueFaz]) => (
            <tr key={tecla}>
              <td className="w-36 py-2 pr-3 align-top">
                <Tecla>{tecla}</Tecla>
              </td>
              <td className="py-2 text-ink">{oQueFaz}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </Dialog>
  );
}
