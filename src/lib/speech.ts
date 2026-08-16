// Ler a resposta em voz alta.
//
// O ditado (microfone) já existe no composer; isto é o outro lado do par —
// serve para acompanhar uma resposta longa sem ficar preso à tela, e para
// quem lê com dificuldade. É a fala do próprio navegador (`speechSynthesis`),
// então não custa dependência nem manda áudio para lugar nenhum.
//
// Uma fala por vez, e o estado é global: se a pessoa manda ler outra
// mensagem, a anterior para. Duas vozes ao mesmo tempo não se entende.

/** Texto de uma mensagem em algo que faça sentido ouvir. */
export function speakable(markdown: string): string {
  return (
    markdown
      // Bloco de código lido em voz alta é ruído puro.
      .replace(/```[\s\S]*?```/g, " ")
      .replace(/`([^`]+)`/g, "$1")
      // Imagem vira o texto alternativo; link vira o rótulo.
      .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
      .replace(/\[([^\]]+)\]\([^)]*\)/g, "$1")
      .replace(/^\s{0,3}#{1,6}\s+/gm, "")
      .replace(/^\s{0,3}>\s?/gm, "")
      .replace(/(\*\*|__|\*|_|~~)/g, "")
      .replace(/^\s*[-*+]\s+/gm, "")
      .replace(/\|/g, " ")
      .replace(/\n{2,}/g, ".\n")
      .replace(/[ \t]{2,}/g, " ")
      .trim()
  );
}

type Listener = () => void;

const listeners = new Set<Listener>();
let speakingId: string | null = null;

function emit(): void {
  for (const fn of listeners) fn();
}

function synth(): SpeechSynthesis | null {
  return typeof window !== "undefined" && "speechSynthesis" in window
    ? window.speechSynthesis
    : null;
}

export const speechStore = {
  subscribe(fn: Listener): () => void {
    listeners.add(fn);
    return () => listeners.delete(fn);
  },

  /** `null` = ninguém falando. O id é o de quem pediu (índice da mensagem). */
  get(): string | null {
    return speakingId;
  },

  available(): boolean {
    return synth() != null;
  },

  stop(): void {
    synth()?.cancel();
    if (speakingId !== null) {
      speakingId = null;
      emit();
    }
  },

  /** Lê o texto. Chamar de novo com o mesmo id para e não recomeça. */
  speak(id: string, text: string, lang: string): void {
    const s = synth();
    if (!s) return;
    const falando = speakingId;
    this.stop();
    if (falando === id) return;

    const limpo = speakable(text);
    if (!limpo) return;

    const fala = new SpeechSynthesisUtterance(limpo);
    fala.lang = lang;
    // O fim natural e o cancelamento passam pelos dois: sem isto o botão
    // ficaria eternamente em "parar" depois que a fala acaba sozinha.
    fala.onend = () => {
      if (speakingId === id) {
        speakingId = null;
        emit();
      }
    };
    fala.onerror = fala.onend;

    speakingId = id;
    emit();
    s.speak(fala);
  },
};
