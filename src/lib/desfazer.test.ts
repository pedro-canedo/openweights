import { beforeEach, describe, expect, it, vi } from "vitest";
import { _avisos, dismissToast } from "../components/ui/Toast";
import { apagarComDesfazer, confirmarApagadosPendentes } from "./desfazer";

function apagar() {
  const confirmar = vi.fn();
  const desfazer = vi.fn();
  apagarComDesfazer({ message: "Apagada.", undoLabel: "Desfazer", confirmar, desfazer });
  const aviso = _avisos().at(-1)!;
  return { confirmar, desfazer, aviso };
}

describe("apagarComDesfazer", () => {
  beforeEach(() => {
    _avisos().forEach((a) => dismissToast(a.id, "acao"));
  });

  it("não apaga enquanto o aviso está na tela", () => {
    const { confirmar, desfazer } = apagar();
    expect(confirmar).not.toHaveBeenCalled();
    expect(desfazer).not.toHaveBeenCalled();
  });

  it("apaga de verdade quando o tempo acaba", () => {
    const { confirmar, desfazer, aviso } = apagar();
    dismissToast(aviso.id, "expirou");
    expect(confirmar).toHaveBeenCalledTimes(1);
    expect(desfazer).not.toHaveBeenCalled();
  });

  it("apaga de verdade se fecham o aviso no x", () => {
    const { confirmar, aviso } = apagar();
    dismissToast(aviso.id, "fechado");
    expect(confirmar).toHaveBeenCalledTimes(1);
  });

  it("desfazer devolve o item e nunca apaga", () => {
    const { confirmar, desfazer, aviso } = apagar();
    aviso.action!.run();
    dismissToast(aviso.id, "acao");
    expect(desfazer).toHaveBeenCalledTimes(1);
    expect(confirmar).not.toHaveBeenCalled();
    confirmarApagadosPendentes();
    expect(confirmar).not.toHaveBeenCalled();
  });

  it("confirmarApagadosPendentes apaga já e tira o aviso, uma vez só", () => {
    const { confirmar, desfazer, aviso } = apagar();
    confirmarApagadosPendentes();
    expect(confirmar).toHaveBeenCalledTimes(1);
    expect(_avisos().some((a) => a.id === aviso.id)).toBe(false);
    aviso.action!.run();
    confirmarApagadosPendentes();
    expect(confirmar).toHaveBeenCalledTimes(1);
    expect(desfazer).not.toHaveBeenCalled();
  });
});
