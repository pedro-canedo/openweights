// Apagar com "Desfazer": a interface tira o item na hora, mas o apagar de
// verdade só acontece quando o aviso sai sem ninguém desfazer. Desfazer, então,
// não precisa recriar nada: basta devolver o item à tela. Se o app fechar
// dentro da janela, nada foi apagado (a falha é para o lado seguro).

import { dismissToast, toast } from "../components/ui/Toast";

interface Adiada {
  /** Apaga de verdade. Idempotente: só corre uma vez. */
  confirmar: () => void;
  /** O aviso na tela; sai junto quando o apagar se confirma por outro caminho. */
  aviso?: number;
}

const pendentes = new Set<Adiada>();

export interface ApagarComDesfazer {
  message: string;
  undoLabel: string;
  /** O apagar de verdade (o banco). */
  confirmar: () => void | Promise<void>;
  /** Devolve o item à tela. */
  desfazer: () => void;
  /** Milissegundos para desfazer. */
  duration?: number;
}

export function apagarComDesfazer(o: ApagarComDesfazer): void {
  let resolvida = false;
  const adiada: Adiada = {
    confirmar: () => {
      if (resolvida) return;
      resolvida = true;
      pendentes.delete(adiada);
      void Promise.resolve(o.confirmar()).catch(() => {});
      if (adiada.aviso != null) dismissToast(adiada.aviso);
    },
  };
  pendentes.add(adiada);
  adiada.aviso = toast({
    message: o.message,
    duration: o.duration ?? 5000,
    action: {
      label: o.undoLabel,
      run: () => {
        if (resolvida) return;
        resolvida = true;
        pendentes.delete(adiada);
        o.desfazer();
      },
    },
    onClose: (motivo) => {
      if (motivo !== "acao") adiada.confirmar();
    },
  });
}

/** Antes de uma ação que depende do estado do banco (enviar, regenerar). */
export function confirmarApagadosPendentes(): void {
  [...pendentes].forEach((p) => p.confirmar());
}
