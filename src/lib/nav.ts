// Navegação programática entre telas (ex.: "Conversar" em Meus Modelos
// leva ao Chat com o modelo já selecionado). O App registra o handler.

export type Screen =
  | "discover"
  | "models"
  | "chat"
  | "harness"
  | "server"
  | "providers"
  | "settings";

export interface NavPayload {
  /** Nome do modelo (artifact name) a pré-selecionar no Chat. */
  chatModel?: string;
  /** Modelo a pré-selecionar na configuração da tela Servidor Local. */
  serverModel?: string;
  /**
   * Aba da tela Servidor Local a abrir.
   *
   * A tela lembra a última aba visitada, e sem isto quem chega por um botão
   * de outra tela ("Ajustar para esta máquina", "GPU extra na rede") caía na
   * aba lembrada — sem sinal nenhum de que o que veio buscar está em outra.
   */
  serverTab?: "overview" | "performance" | "network" | "advanced";
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

export function takePendingServerModel(): string | undefined {
  const m = pendingNav.serverModel;
  delete pendingNav.serverModel;
  return m;
}

/**
 * A aba pedida pela navegação — ou a que o modelo pendente implica.
 *
 * O `serverModel` sozinho já diz para onde ir: quem manda um modelo quer a
 * configuração do motor, que mora em Desempenho. Não consome o
 * `serverModel`; quem faz isso é o card que o usa.
 */
export function takePendingServerTab(): NavPayload["serverTab"] | undefined {
  const t = pendingNav.serverTab;
  delete pendingNav.serverTab;
  return t ?? (pendingNav.serverModel ? "performance" : undefined);
}
