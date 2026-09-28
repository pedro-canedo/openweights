// OwCLI: terminais de verdade dentro do app.
//
// À esquerda, as sessões — cada uma um processo no backend (lr_pty): um
// shell, e em breve o agente OwCLI e os harnesses. Ali se vê, sem abrir,
// quem terminou, quem ainda roda e quem está pedindo você (o agente manda
// OSC 9 ao pedir aprovação; com a janela sem foco vira aviso do sistema).
// À direita, o terminal da sessão escolhida. O terminal mora em
// `lib/terminals.ts` e continua vivo, e desenhando, com a tela fechada.

import { useEffect, useRef, useState, useSyncExternalStore, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import {
  ajustar,
  ativar,
  colarNo,
  copiarSelecao,
  desmontar,
  fechar,
  iniciar,
  limpar,
  marcarTelaVisivel,
  montar,
  novoShell,
  temSelecao,
  terminaisStore,
  type SessaoTerminal,
} from "../lib/terminals";
import { Button, IconButton } from "../components/ui/Button";
import Icon from "../components/ui/Icon";
import { Menu } from "../components/ui/Popover";
import { StatusDot } from "../components/ui/Shell";

export default function OwCLI() {
  const { t } = useTranslation();
  const e = useSyncExternalStore(terminaisStore.subscribe, terminaisStore.get);
  const painel = useRef<HTMLDivElement>(null);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);

  useEffect(() => {
    void iniciar();
    marcarTelaVisivel(true);
    return () => marcarTelaVisivel(false);
  }, []);

  // A sessão ativa fica pendurada no painel enquanto a tela existir.
  const ativa = e.ativa;
  useEffect(() => {
    const el = painel.current;
    if (ativa == null || !el) return;
    montar(ativa, el, terminaisStore.get().focar);
    return () => desmontar(ativa);
  }, [ativa]);

  // O painel muda de tamanho com a janela e com a barra lateral.
  useEffect(() => {
    const el = painel.current;
    if (!el) return;
    const ro = new ResizeObserver(() => {
      const id = terminaisStore.get().ativa;
      if (id != null) ajustar(id);
    });
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  function setas(ev: KeyboardEvent<HTMLDivElement>) {
    const i = e.sessoes.findIndex((s) => s.id === ativa);
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

  return (
    <div className="flex h-full min-h-0 bg-bg">
      <aside className="flex w-60 shrink-0 flex-col border-r border-edge bg-panel">
        <div className="flex items-center justify-between gap-2 px-3 pt-3 pb-2">
          <h2 className="text-[11px] font-semibold tracking-wide text-dim uppercase">
            {t("owcli.sessions")}
          </h2>
          <IconButton icon="plus" label={t("owcli.newShell")} onClick={() => void novoShell()} />
        </div>
        {/* Lista, não tablist: cada sessão tem o próprio botão de fechar, e
            um tablist só pode conter abas. A ativa leva aria-current. */}
        <div
          role="list"
          aria-label={t("owcli.tabs")}
          onKeyDown={setas}
          className="min-h-0 flex-1 overflow-y-auto px-2 pb-2"
        >
          {e.sessoes.map((s) => (
            <ItemSessao key={s.id} sessao={s} ativa={s.id === ativa} />
          ))}
        </div>
      </aside>

      <section className="flex min-w-0 flex-1 flex-col">
        {e.erro && (
          <p
            role="alert"
            className="border-b border-bad/40 bg-bad/10 px-4 py-2 text-[12px] text-bad"
          >
            {e.erro}
          </p>
        )}
        <div className="relative min-h-0 flex-1">
          {/* O painel existe sempre: é nele que o ResizeObserver mora. */}
          <div
            ref={painel}
            role="region"
            aria-label={e.sessoes.find((s) => s.id === ativa)?.titulo ?? t("owcli.tabs")}
            onContextMenu={(ev) => {
              if (ativa == null) return;
              ev.preventDefault();
              setMenu({ x: ev.clientX, y: ev.clientY });
            }}
            className="absolute inset-0 overflow-hidden px-2 pt-2"
          />
          {e.pronto && e.sessoes.length === 0 && (
            <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 p-8 text-center">
              <div className="workspace-icon">
                <Icon name="terminal" className="h-6 w-6" />
              </div>
              <h1 className="text-lg font-semibold text-ink">{t("owcli.emptyTitle")}</h1>
              <p className="max-w-md text-sm leading-relaxed text-dim">{t("owcli.emptyHint")}</p>
              <Button variant="primary" icon="plus" onClick={() => void novoShell()}>
                {t("owcli.newShell")}
              </Button>
            </div>
          )}
        </div>
      </section>

      {menu && ativa != null && (
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
                disabled: !temSelecao(ativa),
                onSelect: () => void copiarSelecao(ativa),
              },
              { id: "colar", label: t("owcli.paste"), onSelect: () => void colarNo(ativa) },
              { id: "limpar", label: t("owcli.clear"), onSelect: () => limpar(ativa) },
              {
                id: "fechar",
                label: t("owcli.closeSession"),
                icon: "close",
                danger: true,
                onSelect: () => void fechar(ativa),
              },
            ]}
          />
        </div>
      )}
    </div>
  );
}

/** "/home/pedro/projetos/api" → "projetos/api": o fim do caminho é o que distingue. */
function pastaCurta(pasta: string): string {
  const partes = pasta.split(/[\\/]+/).filter(Boolean);
  return partes.slice(-2).join("/") || pasta;
}

function ItemSessao({ sessao, ativa }: { sessao: SessaoTerminal; ativa: boolean }) {
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
        ativa ? "bg-panel2" : "hover:bg-panel2/60"
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
