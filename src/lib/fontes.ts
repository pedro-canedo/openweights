/** Espelho do `catalogo::EstadoDasFontes`: o que cada fonte do OpenWeights tem agora. */
export interface EstadoDasFontes {
  /** Modelos GGUF na biblioteca. */
  localModels: number;
  serverRunning: boolean;
  /** OpenRouter ligado e com chave. */
  openrouterKey: boolean;
  /** Favoritos do OpenRouter (só eles entram no catálogo). */
  openrouterFavorites: number;
  ninerouterInstalled: boolean;
  ninerouterRunning: boolean;
}
