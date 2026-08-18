// Navegação programática entre telas (ex.: "Conversar" em Meus Modelos
// leva ao Chat com o modelo já selecionado). O App registra o handler.

export type Screen =
  | "discover"
  | "models"
  | "chat"
  | "activity"
  | "server"
  | "providers"
  | "settings";

export interface NavPayload {
  /** Nome do modelo (artifact name) a pré-selecionar no Chat. */
  chatModel?: string;
}

/** Payload pendente da última navegação; a tela de destino consome e limpa. */
export const pendingNav: NavPayload = {};

let handler: ((screen: Screen) => void) | null = null;

export function onNavigate(h: (screen: Screen) => void): () => void {
  handler = h;
  return () => {
    if (handler === h) handler = null;
  };
}

export function navigate(screen: Screen, payload?: NavPayload): void {
  Object.assign(pendingNav, payload);
  handler?.(screen);
}

export function takePendingChatModel(): string | undefined {
  const m = pendingNav.chatModel;
  delete pendingNav.chatModel;
  return m;
}
