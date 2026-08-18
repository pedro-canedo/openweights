// Ler a resposta em voz alta.
//
// O ditado (microfone) já existe no composer; isto é o outro lado do par —
// serve para acompanhar uma resposta longa sem ficar preso à tela, e para
// quem lê com dificuldade.
//
// São dois caminhos, nesta ordem: a **voz neutra** (um serviço externo, som
// bem melhor) e, se ela não puder atender, a **voz do sistema**
// (`speechSynthesis`), que não custa rede nem manda nada para lugar nenhum.
// A troca é silenciosa de propósito: quem clicou em ouvir quer ouvir, não
// quer saber de qual motor saiu o som. Sem chave no build, sem internet ou
// em português, o segundo caminho é o único — e é o de sempre.
//
// Uma fala por vez, e o estado é global: se a pessoa manda ler outra
// mensagem, a anterior para. Duas vozes ao mesmo tempo não se entende.

import { ttsSpeak } from "./api";

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
/** O <audio> da voz neutra, quando é ela que está tocando. */
let tocando: HTMLAudioElement | null = null;
/** URL do blob em uso — precisa ser revogada, senão o mp3 fica na memória. */
let blobUrl: string | null = null;

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
    if (tocando) {
      tocando.pause();
      tocando.src = "";
      tocando = null;
    }
    if (blobUrl) {
      URL.revokeObjectURL(blobUrl);
      blobUrl = null;
    }
    if (speakingId !== null) {
      speakingId = null;
      emit();
    }
  },

  /** Lê o texto. Chamar de novo com o mesmo id para e não recomeça. */
  speak(id: string, text: string, lang: string): void {
    const falando = speakingId;
    this.stop();
    if (falando === id) return;

    const limpo = speakable(text);
    if (!limpo) return;

    // A voz neutra é uma ida à rede: marca como falando já, senão o botão
    // fica inerte pelos segundos da síntese e a pessoa clica de novo.
    speakingId = id;
    emit();

    ttsSpeak(limpo, lang)
      .then((mp3) => {
        // O pedido pode ter sido cancelado enquanto o áudio vinha.
        if (speakingId !== id) return;
        const url = URL.createObjectURL(new Blob([mp3], { type: "audio/mpeg" }));
        const audio = new Audio(url);
        tocando = audio;
        blobUrl = url;
        const terminou = () => {
          if (speakingId === id) speechStore.stop();
        };
        audio.onended = terminou;
        // Falha na REPRODUÇÃO não cai para a voz do sistema: o áudio já veio,
        // e começar a falar de novo do zero seria pior que parar.
        audio.onerror = terminou;
        void audio.play().catch(terminou);
      })
      .catch(() => {
        if (speakingId !== id) return;
        falaDoSistema(id, limpo, lang);
      });
  },
};

/** O caminho de sempre: a voz que o próprio sistema já tem. */
function falaDoSistema(id: string, limpo: string, lang: string): void {
  const s = synth();
  if (!s) {
    if (speakingId === id) {
      speakingId = null;
      emit();
    }
    return;
  }

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
}
