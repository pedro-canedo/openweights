// OwCLI: terminais de verdade dentro do app.
//
// Cada aba é uma sessão do backend (lr_pty) — um shell, e em breve o agente
// OwCLI e os harnesses. A tela só escolhe qual sessão fica pendurada no
// painel; o terminal em si mora em `lib/terminals.ts` e continua vivo, e
// desenhando, com a tela fechada.

import { useEffect, useRef, useSyncExternalStore, type KeyboardEvent } from "react";
import { useTranslation } from "react-i18next";
import {
  ajustar,
  ativar,
  desmontar,
  fechar,
  iniciar,
  montar,
  novoShell,
  terminaisStore,
  type SessaoTerminal,
} from "../lib/terminals";
import { Button, IconButton } from "../components/ui/Button";
import Icon from "../components/ui/Icon";
import { StatusDot } from "../components/ui/Shell";

export default function OwCLI() {
  const { t } = useTranslation();
  const e = useSyncExternalStore(terminaisStore.subscribe, terminaisStore.get);
  const painel = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void iniciar();
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
      ev.key === "ArrowRight" ? (i + 1) % n
      : ev.key === "ArrowLeft" ? (i - 1 + n) % n
      : -1;
    if (destino < 0) return;
    ev.preventDefault();
    ativar(e.sessoes[destino].id, false);
    ev.currentTarget
      .querySelectorAll<HTMLButtonElement>("[data-sessao]")
      [destino]?.focus();
  }

  return (
    <div className="flex h-full min-h-0 flex-col bg-bg">
      <div className="flex shrink-0 items-center gap-2 border-b border-edge bg-panel px-2 py-1.5">
        {/* Lista, não tablist: cada sessão tem o próprio botão de fechar, e
            um tablist só pode conter abas. A ativa leva aria-current. */}
        <div
          role="list"
          aria-label={t("owcli.tabs")}
          onKeyDown={setas}
          className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto"
        >
          {e.sessoes.map((s) => (
            <Aba key={s.id} sessao={s} ativa={s.id === ativa} />
          ))}
        </div>
        <Button size="sm" variant="ghost" icon="plus" onClick={() => void novoShell()}>
          {t("owcli.newShell")}
        </Button>
      </div>

      {e.erro && (
        <p role="alert" className="border-b border-bad/40 bg-bad/10 px-4 py-2 text-[12px] text-bad">
          {e.erro}
        </p>
      )}

      <div className="relative min-h-0 flex-1">
        {/* O painel existe sempre: é nele que o ResizeObserver mora. */}
        <div
          ref={painel}
          role="region"
          aria-label={e.sessoes.find((s) => s.id === ativa)?.titulo ?? t("owcli.tabs")}
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
    </div>
  );
}

function Aba({ sessao, ativa }: { sessao: SessaoTerminal; ativa: boolean }) {
  const { t } = useTranslation();
  const tom = sessao.atencao ? "warn" : sessao.viva ? "ok" : "off";
  return (
    <div
      role="listitem"
      className={`group flex max-w-56 shrink-0 items-center gap-1 rounded-lg pl-2.5 pr-1 transition-colors ${
        ativa ? "bg-panel2 text-ink" : "text-dim hover:bg-panel2/60 hover:text-ink"
      }`}
    >
      <button
        type="button"
        data-sessao={sessao.id}
        aria-current={ativa ? "true" : undefined}
        onClick={() => ativar(sessao.id)}
        title={sessao.pasta}
        className="flex min-w-0 items-center gap-2 py-1.5 text-[13px]"
      >
        <StatusDot tone={tom} pulse={sessao.atencao} />
        <span className="truncate">{sessao.titulo}</span>
        {sessao.atencao && <span className="sr-only">{t("owcli.needsYou")}</span>}
        {!sessao.viva && <span className="text-[11px] text-dim">· {t("owcli.ended")}</span>}
      </button>
      <IconButton
        icon="close"
        size="sm"
        label={t("owcli.close", { title: sessao.titulo })}
        onClick={() => void fechar(sessao.id)}
      />
    </div>
  );
}
