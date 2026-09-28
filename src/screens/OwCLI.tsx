// OwCLI: terminais de verdade dentro do app.
//
// À esquerda, as sessões — cada uma um processo no backend (lr_pty): o
// agente OwCLI, um shell ou um harness. Ali se vê, sem abrir, quem
// terminou, quem ainda roda e quem está pedindo você (o agente manda OSC 9
// ao pedir aprovação; com a janela sem foco vira aviso do sistema). Abaixo,
// o histórico: as conversas que o OwCLI gravou, para continuar ou renomear.
// À direita, uma grade de 1, 2 ou 4 painéis, cada um com uma sessão. O
// terminal mora em `lib/terminals.ts` e continua vivo, e desenhando, com a
// tela fechada ou fora da grade.

import {
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { useTranslation } from "react-i18next";
import {
  ajustar,
  ativar,
  carregarHistorico,
  colarNo,
  copiarSelecao,
  definirLayout,
  desmontar,
  fechar,
  focarPainel,
  iniciar,
  limpar,
  marcarTelaVisivel,
  montar,
  novoOwcli,
  novoShell,
  renomearConversa,
  retomarOwcli,
  temSelecao,
  terminaisStore,
  type Layout,
  type OpcoesOwcli,
  type SessaoPassada,
  type SessaoTerminal,
} from "../lib/terminals";
import { pickWorkspace } from "../lib/api";
import { formatAgo } from "../lib/format";
import { Button, IconButton, Input, inputClass } from "../components/ui/Button";
import { Dialog } from "../components/ui/Dialog";
import Icon, { type IconName } from "../components/ui/Icon";
import { Menu, usePopover } from "../components/ui/Popover";
import { StatusDot } from "../components/ui/Shell";
import { Split } from "../components/ui/Split";
import { toast } from "../components/ui/Toast";

type MenuAberto = { x: number; y: number; id: number };

export default function OwCLI() {
  const { t } = useTranslation();
  const e = useSyncExternalStore(terminaisStore.subscribe, terminaisStore.get);
  const [menu, setMenu] = useState<MenuAberto | null>(null);
  const [dialogo, setDialogo] = useState(false);
  const [renomeando, setRenomeando] = useState<SessaoPassada | null>(null);
  const nova = usePopover("menu");

  useEffect(() => {
    void iniciar();
    void carregarHistorico();
    marcarTelaVisivel(true);
    return () => marcarTelaVisivel(false);
  }, []);

  function setas(ev: KeyboardEvent<HTMLDivElement>) {
    const i = e.sessoes.findIndex((s) => s.id === e.ativa);
    const n = e.sessoes.length;
    if (n === 0) return;
    const destino =
      ev.key === "ArrowDown" ? (i + 1) % n
      : ev.key === "ArrowUp" ? (i - 1 + n) % n
      : -1;
    if (destino < 0) return;
    ev.preventDefault();
    ativar(e.sessoes[destino].id, false);
    ev.currentTarget
      .querySelectorAll<HTMLButtonElement>("[data-sessao]")
      [destino]?.focus();
  }

  const abrirMenu = (x: number, y: number, id: number) => setMenu({ x, y, id });
  const painel = (i: number) => <Painel indice={i} aoMenu={abrirMenu} />;

  let grade: ReactNode;
  if (e.layout === 1) {
    grade = painel(0);
  } else if (e.layout === 2) {
    grade = (
      <Split
        label={t("owcli.splitWidth")}
        defaultSize={560}
        min={240}
        minSecond={240}
        storageKey="ow.owcli.split.2"
        first={painel(0)}
        second={painel(1)}
        className="h-full"
      />
    );
  } else {
    const linha = (a: number, b: number, chave: string) => (
      <Split
        label={t("owcli.splitWidth")}
        defaultSize={560}
        min={240}
        minSecond={240}
        storageKey={chave}
        first={painel(a)}
        second={painel(b)}
        className="h-full"
      />
    );
    grade = (
      <Split
        direction="vertical"
        label={t("owcli.splitHeight")}
        defaultSize={340}
        min={140}
        minSecond={140}
        storageKey="ow.owcli.split.4v"
        first={linha(0, 1, "ow.owcli.split.4a")}
        second={linha(2, 3, "ow.owcli.split.4b")}
        className="h-full"
      />
    );
  }

  return (
    <div className="flex h-full min-h-0 bg-bg">
      <aside className="flex w-60 shrink-0 flex-col border-r border-edge bg-panel">
        <div className="flex items-center justify-between gap-2 px-3 pt-3 pb-2">
          <h2 className="text-[11px] font-semibold tracking-wide text-dim uppercase">
            {t("owcli.sessions")}
          </h2>
          <span className="relative">
            <IconButton icon="plus" label={t("owcli.newSession")} {...nova.triggerProps} />
            <Menu
              {...nova.popoverProps}
              label={t("owcli.newSession")}
              className="absolute right-0 top-full mt-1"
              items={[
                {
                  id: "agente",
                  label: t("owcli.newAgent"),
                  icon: "sparkles",
                  onSelect: () => setDialogo(true),
                },
                {
                  id: "terminal",
                  label: t("owcli.newShell"),
                  icon: "terminal",
                  onSelect: () => void novoShell(),
                },
              ]}
            />
          </span>
        </div>
        {/* Lista, não tablist: cada sessão tem o próprio botão de fechar, e
            um tablist só pode conter abas. A ativa leva aria-current. */}
        <div
          role="list"
          aria-label={t("owcli.tabs")}
          onKeyDown={setas}
          className={`overflow-y-auto px-2 pb-2 ${
            e.historico.length > 0 ? "max-h-[45%] shrink-0" : "min-h-0 flex-1"
          }`}
        >
          {e.sessoes.map((s) => (
            <ItemSessao
              key={s.id}
              sessao={s}
              ativa={s.id === e.ativa}
              naGrade={e.paineis.slice(0, e.layout).includes(s.id)}
            />
          ))}
        </div>
        {e.historico.length > 0 && (
          <section
            aria-labelledby="owcli-historico"
            className="flex min-h-0 flex-1 flex-col border-t border-edge"
          >
            <h2
              id="owcli-historico"
              className="px-3 pt-3 pb-2 text-[11px] font-semibold tracking-wide text-dim uppercase"
            >
              {t("owcli.history")}
            </h2>
            <ul className="min-h-0 flex-1 overflow-y-auto px-2 pb-2">
              {e.historico.map((c) => (
                <ItemConversa key={c.id} conversa={c} aoRenomear={() => setRenomeando(c)} />
              ))}
            </ul>
          </section>
        )}
      </aside>

      <section className="flex min-w-0 flex-1 flex-col">
        <div className="flex shrink-0 items-center justify-end gap-1 border-b border-edge bg-panel px-2 py-1">
          <span className="mr-1 text-[11px] text-dim">{t("owcli.layout")}</span>
          {(
            [
              [1, "painel-1", "owcli.layout1"],
              [2, "painel-2", "owcli.layout2"],
              [4, "painel-4", "owcli.layout4"],
            ] as [Layout, IconName, string][]
          ).map(([n, icone, rotulo]) => (
            <IconButton
              key={n}
              icon={icone}
              size="sm"
              label={t(rotulo)}
              aria-pressed={e.layout === n}
              onClick={() => definirLayout(n)}
              className={e.layout === n ? "bg-panel2 text-ink" : ""}
            />
          ))}
        </div>
        {e.erro && (
          <p
            role="alert"
            className="border-b border-bad/40 bg-bad/10 px-4 py-2 text-[12px] text-bad"
          >
            {e.erro}
          </p>
        )}
        <div className="relative min-h-0 flex-1">
          {e.pronto && e.sessoes.length === 0 ? (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 p-8 text-center">
              <div className="workspace-icon">
                <Icon name="terminal" className="h-6 w-6" />
              </div>
              <h1 className="text-lg font-semibold text-ink">{t("owcli.emptyTitle")}</h1>
              <p className="max-w-md text-sm leading-relaxed text-dim">{t("owcli.emptyHint")}</p>
              <div className="flex flex-wrap justify-center gap-2">
                <Button variant="primary" icon="sparkles" onClick={() => setDialogo(true)}>
                  {t("owcli.openAgent")}
                </Button>
                <Button icon="terminal" onClick={() => void novoShell()}>
                  {t("owcli.newShell")}
                </Button>
              </div>
            </div>
          ) : (
            <div className="absolute inset-0">{grade}</div>
          )}
        </div>
      </section>

      <RenomearConversa conversa={renomeando} aoFechar={() => setRenomeando(null)} />

      <NovaSessaoOwcli
        aberto={dialogo}
        aoFechar={() => setDialogo(false)}
        aoAbrir={(opcoes) => {
          setDialogo(false);
          void novoOwcli(opcoes);
        }}
      />

      {menu && (
        <div className="fixed z-50" style={{ left: menu.x, top: menu.y }}>
          <Menu
            open
            onClose={() => setMenu(null)}
            label={t("owcli.menu")}
            items={[
              {
                id: "copiar",
                label: t("owcli.copy"),
                icon: "copy",
                disabled: !temSelecao(menu.id),
                onSelect: () => void copiarSelecao(menu.id),
              },
              { id: "colar", label: t("owcli.paste"), onSelect: () => void colarNo(menu.id) },
              { id: "limpar", label: t("owcli.clear"), onSelect: () => limpar(menu.id) },
              {
                id: "fechar",
                label: t("owcli.closeSession"),
                icon: "close",
                danger: true,
                onSelect: () => void fechar(menu.id),
              },
            ]}
          />
        </div>
      )}
    </div>
  );
}

/** Pasta, aprovação e sandbox do agente, antes de abrir. */
function NovaSessaoOwcli({
  aberto,
  aoFechar,
  aoAbrir,
}: {
  aberto: boolean;
  aoFechar: () => void;
  aoAbrir: (opcoes: OpcoesOwcli) => void;
}) {
  const { t } = useTranslation();
  const [pasta, setPasta] = useState<string | null>(null);
  const [aprovacao, setAprovacao] = useState<OpcoesOwcli["aprovacao"]>("on-request");
  const [sandbox, setSandbox] = useState<OpcoesOwcli["sandbox"]>("workspace-write");
  return (
    <Dialog
      open={aberto}
      onClose={aoFechar}
      title={t("owcli.agentDialogTitle")}
      description={t("owcli.agentDialogHint")}
      footer={
        <>
          <Button onClick={aoFechar}>{t("common.cancel")}</Button>
          <Button
            variant="primary"
            icon="sparkles"
            data-autofocus=""
            onClick={() => aoAbrir({ pasta, aprovacao, sandbox })}
          >
            {t("owcli.open")}
          </Button>
        </>
      }
    >
      <div className="mt-5 grid gap-4">
        <div>
          <span className="mb-1 block text-[12px] text-dim">{t("owcli.folder")}</span>
          <div className="flex items-center gap-2">
            <span
              className="min-w-0 flex-1 truncate rounded-lg border border-edge-strong bg-panel2 px-3 py-1.5 font-mono text-[12px] text-ink select-text"
              title={pasta ?? undefined}
            >
              {pasta ?? t("owcli.folderHome")}
            </span>
            <Button
              size="sm"
              onClick={async () => {
                const escolhida = await pickWorkspace().catch(() => null);
                if (escolhida) setPasta(escolhida);
              }}
            >
              {t("owcli.chooseFolder")}
            </Button>
          </div>
        </div>
        <label className="grid gap-1">
          <span className="text-[12px] text-dim">{t("owcli.approval")}</span>
          <select
            value={aprovacao}
            onChange={(e) => setAprovacao(e.target.value as OpcoesOwcli["aprovacao"])}
            className={inputClass}
          >
            <option value="on-request">{t("owcli.approvalOnRequest")}</option>
            <option value="untrusted">{t("owcli.approvalUntrusted")}</option>
            <option value="never">{t("owcli.approvalNever")}</option>
          </select>
        </label>
        <label className="grid gap-1">
          <span className="text-[12px] text-dim">{t("owcli.sandbox")}</span>
          <select
            value={sandbox}
            onChange={(e) => setSandbox(e.target.value as OpcoesOwcli["sandbox"])}
            className={inputClass}
          >
            <option value="workspace-write">{t("owcli.sandboxWrite")}</option>
            <option value="read-only">{t("owcli.sandboxRead")}</option>
            <option value="danger-full-access">{t("owcli.sandboxFull")}</option>
          </select>
        </label>
      </div>
    </Dialog>
  );
}

/** Um nome para a conversa, no lugar do começo da primeira mensagem. */
function RenomearConversa({
  conversa,
  aoFechar,
}: {
  conversa: SessaoPassada | null;
  aoFechar: () => void;
}) {
  const { t } = useTranslation();
  const [nome, setNome] = useState("");
  const [salvando, setSalvando] = useState(false);
  useEffect(() => {
    if (conversa) setNome(conversa.renomeada ? conversa.titulo : "");
  }, [conversa]);

  async function salvar() {
    if (!conversa || !nome.trim()) return;
    setSalvando(true);
    try {
      await renomearConversa(conversa.id, nome);
      aoFechar();
    } catch (err) {
      toast({ message: t("owcli.renameFailed", { error: String(err) }), tone: "bad" });
    } finally {
      setSalvando(false);
    }
  }

  return (
    <Dialog
      open={conversa != null}
      onClose={aoFechar}
      title={t("owcli.renameTitle")}
      footer={
        <>
          <Button onClick={aoFechar}>{t("common.cancel")}</Button>
          <Button
            variant="primary"
            disabled={!nome.trim() || salvando}
            onClick={() => void salvar()}
          >
            {t("owcli.rename")}
          </Button>
        </>
      }
    >
      <form
        className="mt-4"
        onSubmit={(ev) => {
          ev.preventDefault();
          void salvar();
        }}
      >
        <Input
          label={t("owcli.renameLabel")}
          value={nome}
          placeholder={conversa?.titulo}
          maxLength={120}
          data-autofocus=""
          onChange={(ev) => setNome(ev.target.value)}
        />
      </form>
    </Dialog>
  );
}

/** Uma conversa gravada: clicar continua de onde parou, numa sessão nova. */
function ItemConversa({
  conversa,
  aoRenomear,
}: {
  conversa: SessaoPassada;
  aoRenomear: () => void;
}) {
  const { t, i18n } = useTranslation();
  const quando = formatAgo(i18n.language, conversa.atualizadaEm * 1000, "short");
  return (
    <li className="group flex items-center gap-1 rounded-lg pr-1 transition-colors hover:bg-panel2/60">
      <button
        type="button"
        onClick={() => void retomarOwcli(conversa)}
        title={`${conversa.titulo}\n${t("owcli.resumeHint", { folder: conversa.pasta })}`}
        aria-label={t("owcli.resume", { title: conversa.titulo })}
        className="flex min-w-0 flex-1 items-start gap-2.5 px-2 py-1.5 text-left"
      >
        <Icon name="history" className="mt-0.5 h-3.5 w-3.5 shrink-0 text-dim" />
        <span className="min-w-0">
          <span className="block truncate text-[13px] text-ink">{conversa.titulo}</span>
          <span className="block truncate text-[11px] text-dim">
            {quando} · {pastaCurta(conversa.pasta)}
          </span>
        </span>
      </button>
      <IconButton
        icon="pencil"
        size="sm"
        label={t("owcli.renameOf", { title: conversa.titulo })}
        onClick={aoRenomear}
        className="opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 focus-visible:opacity-100"
      />
    </li>
  );
}

/** Um painel da grade: pendura o terminal da sessão que é dele. */
function Painel({
  indice,
  aoMenu,
}: {
  indice: number;
  aoMenu: (x: number, y: number, id: number) => void;
}) {
  const { t } = useTranslation();
  const e = useSyncExternalStore(terminaisStore.subscribe, terminaisStore.get);
  const corpo = useRef<HTMLDivElement>(null);
  const id = e.paineis[indice] ?? null;
  const sessao = e.sessoes.find((s) => s.id === id);
  const grade = e.layout > 1;
  const comFoco = e.foco === indice;

  useEffect(() => {
    const el = corpo.current;
    if (id == null || !el) return;
    const s = terminaisStore.get();
    montar(id, el, s.focar && s.foco === indice);
    return () => desmontar(id);
  }, [id, indice]);

  // O painel muda de tamanho com a janela, a barra lateral e o divisor.
  useEffect(() => {
    const el = corpo.current;
    if (id == null || !el) return;
    const ro = new ResizeObserver(() => ajustar(id));
    ro.observe(el);
    return () => ro.disconnect();
  }, [id]);

  return (
    <div
      data-painel={indice}
      onMouseDownCapture={() => focarPainel(indice)}
      className={`flex h-full min-h-0 flex-col ${
        grade ? `border ${comFoco ? "border-accent/60" : "border-transparent"}` : ""
      }`}
    >
      {grade && (
        <div className="flex h-7 shrink-0 items-center gap-2 border-b border-edge px-2 text-[12px]">
          {sessao ? (
            <>
              <StatusDot
                tone={sessao.atencao ? "warn" : sessao.viva ? "ok" : "off"}
                pulse={sessao.atencao}
              />
              <span className={`truncate ${comFoco ? "text-ink" : "text-dim"}`}>
                {sessao.titulo}
              </span>
            </>
          ) : (
            <span className="text-dim">{t("owcli.paneLabel", { n: indice + 1 })}</span>
          )}
        </div>
      )}
      <div className="relative min-h-0 flex-1">
        <div
          ref={corpo}
          role="region"
          aria-label={sessao?.titulo ?? t("owcli.paneLabel", { n: indice + 1 })}
          onContextMenu={(ev) => {
            if (id == null) return;
            ev.preventDefault();
            aoMenu(ev.clientX, ev.clientY, id);
          }}
          className="absolute inset-0 overflow-hidden px-2 pt-1"
        />
        {id == null && (
          <div className="absolute inset-0 flex flex-col items-center justify-center gap-2 p-4 text-center">
            <p className="max-w-xs text-[12px] leading-relaxed text-dim">{t("owcli.emptyPane")}</p>
            <Button
              size="sm"
              icon="plus"
              onClick={() => {
                focarPainel(indice);
                void novoShell();
              }}
            >
              {t("owcli.newShell")}
            </Button>
          </div>
        )}
      </div>
    </div>
  );
}

/** "/home/pedro/projetos/api" → "projetos/api": o fim do caminho é o que distingue. */
function pastaCurta(pasta: string): string {
  const partes = pasta.split(/[\\/]+/).filter(Boolean);
  return partes.slice(-2).join("/") || pasta;
}

function ItemSessao({
  sessao,
  ativa,
  naGrade,
}: {
  sessao: SessaoTerminal;
  ativa: boolean;
  naGrade: boolean;
}) {
  const { t } = useTranslation();
  const tom = sessao.atencao ? "warn" : sessao.viva ? "ok" : "off";
  const detalhe = sessao.atencao
    ? t("owcli.needsYou")
    : !sessao.viva
      ? t("owcli.exited", { code: sessao.codigoSaida ?? "?" })
      : pastaCurta(sessao.pasta);
  return (
    <div
      role="listitem"
      className={`group flex items-center gap-1 rounded-lg pr-1 transition-colors ${
        ativa ? "bg-panel2" : naGrade ? "bg-panel2/50" : "hover:bg-panel2/60"
      }`}
    >
      <button
        type="button"
        data-sessao={sessao.id}
        aria-current={ativa ? "true" : undefined}
        onClick={() => ativar(sessao.id)}
        title={sessao.pasta}
        className="flex min-w-0 flex-1 items-start gap-2.5 px-2 py-1.5 text-left"
      >
        <span className="mt-1.5">
          <StatusDot tone={tom} pulse={sessao.atencao} />
        </span>
        <span className="min-w-0">
          <span className="block truncate text-[13px] text-ink">{sessao.titulo}</span>
          <span
            className={`block truncate text-[11px] ${sessao.atencao ? "text-warn" : "text-dim"}`}
          >
            {detalhe}
          </span>
        </span>
      </button>
      <IconButton
        icon="close"
        size="sm"
        label={t("owcli.close", { title: sessao.titulo })}
        onClick={() => void fechar(sessao.id)}
        className="opacity-0 group-focus-within:opacity-100 group-hover:opacity-100 focus-visible:opacity-100"
      />
    </div>
  );
}
