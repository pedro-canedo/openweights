# Arquitetura

O OpenWeights é um app Tauri 2: núcleo em Rust, frontend em React, e um workspace
Rust dividido em um crate por assunto.

```
src/                  Frontend React (telas, componentes, i18n pt-BR/en)
src-tauri/src/        O app Tauri em si (comandos, estado, telemetria)
src-tauri/crates/     O núcleo, um crate por assunto
site/                 Este site de documentação (VitePress)
```

## Os crates

| Crate | Do que ele cuida |
|---|---|
| `types` | Tipos compartilhados entre os crates, serializados para o frontend em `camelCase` |
| `store` | SQLite local: conversas, mensagens, presets, ajustes e as comparações de configuração |
| `engine` | Motores de inferência. O principal é o llama-server do llama.cpp em **Router mode**: um processo só que carrega, descarrega e troca modelos conforme o campo `model` de cada requisição |
| `runtime` | Escolhe a build certa do llama.cpp para a máquina, baixa da release fixada no GitHub, verifica e extrai |
| `hw` | Detecção de hardware na inicialização e telemetria ao vivo a 1–2 Hz |
| `models` | Cliente do Hugging Face Hub para arquivos GGUF e o gerenciador de downloads |
| `advisor` | Estima a memória que cada arquivo GGUF precisa e o classifica contra o seu hardware — o veredito verde/amarelo/cinza |
| `cluster` | Cluster RPC do llama.cpp: um host e um worker na rede local |
| `providers` | Provedores de LLM: o catálogo remoto (OpenRouter) e a resolução de endpoint |
| `ninerouter` | Instala, supervisiona e remove o 9router de forma isolada |
| `agenticow` | O AgenticOw, nosso fork do DeepSeek Harness: instala o runtime pré-compilado contra os pins sha256 gravados no app, supervisiona o Host por um protocolo de controle em JSON por linha e empurra para ele o catálogo de modelos (detalhes em `docs/agenticow.md`) |
| `gateway` | Ponto de entrada único (um Traefik local) para os provedores de LLM |
| `nodejs` | Runtime Node.js portátil, isolado do Node do sistema |
| `proc` | Supervisão de processos filhos de longa duração: árvore, Job Object e porta livre |
| `fetch` | Baixa, verifica por SHA256 e extrai pacotes de release em instalação atômica |

## Router mode

O motor é um **único** processo llama-server que carrega e descarrega modelos sob
demanda, guiado pelo campo `model` de cada requisição. É isso que permite ao
chat e à API local dividirem um motor só, em vez de subir um processo por
modelo — e é isso que faz "modelos simultâneos" ser um ajuste de verdade, e não
um desejo.

## O frontend

React 19 + Vite + Tailwind 4, com i18next para pt-BR/en. `npm run dev` roda a
interface no navegador com dados simulados, sem Rust envolvido — é onde a maior
parte do trabalho de interface acontece.

As telas espelham os crates acima: Descobrir (`models` + `advisor`), Meus
Modelos, Chat (`engine`), Servidor Local (`engine`), Fontes de modelo
(`providers`), Configurações.
