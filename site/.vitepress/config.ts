import { defineConfig } from "vitepress";

// Site público do OpenWeights: apresentação + documentação, em inglês e
// português. O README continua sendo o cartão de visita do repositório; aqui
// mora o que não cabe nele — guia, modelos e as integrações por partes.
//
// `base` é o subcaminho do GitHub Pages do projeto. Se um dia houver domínio
// próprio, isto vira "/" e o CNAME entra em `public/`.
const base = "/openweights/";

// Endereço público, para o sitemap e para as tags sociais: og:image e og:url
// exigem URL absoluta — um caminho relativo não vira preview em rede nenhuma.
const site = "https://pedro-canedo.github.io";

const enSidebar = [
  {
    text: "Getting started",
    items: [
      { text: "What OpenWeights is", link: "/guide/" },
      { text: "What's new", link: "/guide/whats-new" },
      { text: "Changelog", link: "/guide/changelog" },
      { text: "Install", link: "/guide/install" },
      { text: "First run", link: "/guide/first-run" },
      { text: "Models and quantization", link: "/guide/models" },
      { text: "Chat", link: "/guide/chat" },
      { text: "Chat performance", link: "/guide/performance" },
      { text: "The coding agent", link: "/guide/harness" },
      { text: "Troubleshooting", link: "/guide/troubleshooting" },
    ],
  },
  {
    text: "Integrations",
    items: [
      { text: "Local API server", link: "/integrations/local-api" },
      { text: "Extra GPU on the network", link: "/integrations/cluster" },
      { text: "External model sources", link: "/integrations/providers" },
      { text: "API reference", link: "/integrations/api-reference" },
      { text: "Configuration reference", link: "/integrations/configuration" },
    ],
  },
  {
    text: "Project",
    items: [
      { text: "Build from source", link: "/contribute/" },
      { text: "Architecture", link: "/contribute/architecture" },
    ],
  },
];

const ptSidebar = [
  {
    text: "Primeiros passos",
    items: [
      { text: "O que é o OpenWeights", link: "/pt/guia/" },
      { text: "Novidades", link: "/pt/guia/novidades" },
      { text: "Changelog", link: "/pt/guia/changelog" },
      { text: "Instalação", link: "/pt/guia/instalacao" },
      { text: "Primeira execução", link: "/pt/guia/primeira-execucao" },
      { text: "Modelos e quantização", link: "/pt/guia/modelos" },
      { text: "Chat", link: "/pt/guia/chat" },
      { text: "Desempenho no chat", link: "/pt/guia/desempenho" },
      { text: "O agente de código", link: "/pt/guia/harness" },
      { text: "Solução de problemas", link: "/pt/guia/solucao-de-problemas" },
    ],
  },
  {
    text: "Integrações",
    items: [
      { text: "Servidor de API local", link: "/pt/integracoes/api-local" },
      { text: "GPU extra na rede", link: "/pt/integracoes/cluster" },
      { text: "Fontes externas de modelo", link: "/pt/integracoes/provedores" },
      { text: "Referência da API", link: "/pt/integracoes/referencia-api" },
      { text: "Referência de configuração", link: "/pt/integracoes/configuracao" },
    ],
  },
  {
    text: "Projeto",
    items: [
      { text: "Compilar do código-fonte", link: "/pt/contribuir/" },
      { text: "Arquitetura", link: "/pt/contribuir/arquitetura" },
    ],
  },
];

export default defineConfig({
  base,
  title: "OpenWeights",
  description:
    "Run LLMs on your own machine — no terminal, no CUDA setup, no guessing which quantization fits.",
  lastUpdated: true,
  cleanUrls: true,
  ignoreDeadLinks: false,

  // Sem sitemap, um site de 36 páginas depende de o buscador tropeçar em cada
  // link. Com ele, o buscador recebe a lista.
  sitemap: { hostname: `${site}${base}` },

  head: [
    ["link", { rel: "icon", type: "image/svg+xml", href: `${base}icon.svg` }],
    ["meta", { name: "theme-color", content: "#7b5cff" }],
    ["meta", { property: "og:type", content: "website" }],
    ["meta", { property: "og:title", content: "OpenWeights" }],
    [
      "meta",
      {
        property: "og:description",
        content: "Models. Your machine. Your rules.",
      },
    ],
    ["meta", { property: "og:image", content: `${site}${base}logo-1024.png` }],
    ["meta", { property: "og:url", content: `${site}${base}` }],
    ["meta", { property: "og:site_name", content: "OpenWeights" }],
    ["meta", { name: "twitter:card", content: "summary_large_image" }],
    ["meta", { name: "twitter:title", content: "OpenWeights" }],
    [
      "meta",
      {
        name: "twitter:description",
        content: "Models. Your machine. Your rules.",
      },
    ],
    ["meta", { name: "twitter:image", content: `${site}${base}logo-1024.png` }],
  ],

  locales: {
    root: {
      label: "English",
      lang: "en",
      themeConfig: {
        nav: [
          { text: "Guide", link: "/guide/", activeMatch: "/guide/" },
          {
            text: "Integrations",
            link: "/integrations/local-api",
            activeMatch: "/integrations/",
          },
          { text: "Download", link: "/guide/install" },
        ],
        sidebar: enSidebar,
        editLink: {
          pattern:
            "https://github.com/pedro-canedo/openweights/edit/main/site/:path",
          text: "Suggest changes to this page",
        },
        outline: { level: [2, 3], label: "On this page" },
        docFooter: { prev: "Previous", next: "Next" },
        lastUpdated: { text: "Last updated" },
        darkModeSwitchLabel: "Theme",
        returnToTopLabel: "Back to top",
      },
    },

    pt: {
      label: "Português",
      lang: "pt-BR",
      link: "/pt/",
      themeConfig: {
        nav: [
          { text: "Guia", link: "/pt/guia/", activeMatch: "/pt/guia/" },
          {
            text: "Integrações",
            link: "/pt/integracoes/api-local",
            activeMatch: "/pt/integracoes/",
          },
          { text: "Baixar", link: "/pt/guia/instalacao" },
        ],
        sidebar: ptSidebar,
        editLink: {
          pattern:
            "https://github.com/pedro-canedo/openweights/edit/main/site/:path",
          text: "Sugerir uma correção nesta página",
        },
        outline: { level: [2, 3], label: "Nesta página" },
        docFooter: { prev: "Anterior", next: "Próxima" },
        lastUpdated: { text: "Atualizado em" },
        darkModeSwitchLabel: "Tema",
        returnToTopLabel: "Voltar ao topo",
        langMenuLabel: "Mudar de idioma",
      },
    },
  },

  themeConfig: {
    logo: { light: "/mark-light.svg", dark: "/mark.svg" },
    siteTitle: "OpenWeights",
    socialLinks: [
      {
        icon: "github",
        link: "https://github.com/pedro-canedo/openweights",
      },
    ],
    search: {
      provider: "local",
      options: {
        // O índice é por idioma; sem isto a caixa continuaria em inglês na
        // versão em português, e é a primeira coisa que a pessoa toca.
        locales: {
          pt: {
            translations: {
              button: { buttonText: "Buscar", buttonAriaLabel: "Buscar" },
              modal: {
                displayDetails: "Mostrar detalhes",
                resetButtonTitle: "Limpar busca",
                backButtonTitle: "Fechar busca",
                noResultsText: "Nenhum resultado para",
                footer: {
                  selectText: "selecionar",
                  navigateText: "navegar",
                  closeText: "fechar",
                },
              },
            },
          },
        },
      },
    },
    footer: {
      message:
        '<a href="https://github.com/pedro-canedo/openweights">GitHub</a> · ' +
        '<a href="https://github.com/pedro-canedo/openweights/blob/main/LICENSE">MIT</a> · ' +
        '<a href="https://github.com/pedro-canedo/openweights/releases/latest">Releases</a>',
      copyright: "© OpenWeights contributors",
    },
  },
});
