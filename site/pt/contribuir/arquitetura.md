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
| `store` | SQLite local: conversas, mensagens, presets, ajustes e o estado do agente — execuções, ferramentas, permissões, MCP, memória |
| `engine` | Motores de inferência. O principal é o llama-server do llama.cpp em **Router mode**: um processo só que carrega, descarrega e troca modelos conforme o campo `model` de cada requisição |
| `runtime` | Escolhe a build certa do llama.cpp para a máquina, baixa da release fixada no GitHub, verifica e extrai |
| `hw` | Detecção de hardware na inicialização e telemetria ao vivo a 1–2 Hz |
| `models` | Cliente do Hugging Face Hub para arquivos GGUF e o gerenciador de downloads |
| `advisor` | Estima a memória que cada arquivo GGUF precisa e o classifica contra o seu hardware — o veredito verde/amarelo/cinza |
| `agent` | O laço de execução: passos, ferramentas, confirmações, guard-rails, a trilha |
| `tools` | O catálogo de ferramentas: nome, descrição, JSON Schema, execução |
| `policy` | Para cada chamada: roda, pergunta, ou recusa |
| `checkpoint` | Fotos do projeto — git sombra ou cópia de arquivos |
| `codemode` | O SDK do Code Mode, a ponte e a execução isolada no Node |
| `mcp` | Conectores MCP, com o portão de aprovação contra troca de ferramentas |
| `memory` | Memória de longo prazo: fatos curados em arquivos Markdown |
| `rag` | Índice do projeto: busca híbrida FTS5 + vetor, fundida por RRF |
| `webtools`, `codetools`, `gittools`, `datatools`, `desktop` | Implementações de ferramentas: internet, build/teste, Git, CSV/SQLite, o computador |
| `providers`, `ninerouter`, `gateway`, `nodejs` | Fontes externas de modelo, o roteador local, o ponto de entrada único, o Node portátil |
| `proc`, `fetch` | Supervisão de processos filhos de longa duração, e HTTP |

## Router mode

O motor é um **único** processo llama-server que carrega e descarrega modelos sob
demanda, guiado pelo campo `model` de cada requisição. É isso que permite ao
chat, ao agente, à indexação do projeto e à API local dividirem um motor só, em
vez de subir um processo por modelo — e é isso que faz "modelos simultâneos" ser
um ajuste de verdade, e não um desejo.

## O frontend

React 19 + Vite + Tailwind 4, com i18next para pt-BR/en. `npm run dev` roda a
interface no navegador com dados simulados, sem Rust envolvido — é onde a maior
parte do trabalho de interface acontece.

As telas espelham os crates acima: Descobrir (`models` + `advisor`), Meus
Modelos, Chat (`engine` + `agent`), Atividade (histórico de execuções), Servidor
Local (`engine`), Fontes de modelo (`providers`), Configurações.
