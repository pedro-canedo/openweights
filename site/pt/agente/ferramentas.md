# Ferramentas

O catálogo passa de trinta ferramentas. Um modelo de 8B com janela de 8k gastaria
um pedaço grande dela só lendo descrições — e, pior, escolhe *pior* quanto mais
opções enxerga.

Por isso o harness serve um cardápio, não o catálogo inteiro.

## Como o cardápio é montado

Como um restaurante: mostrar poucos pratos e ter um garçom para o resto.

1. **O núcleo entra primeiro** — `fs_read`, `fs_list`, `fs_grep`, `fs_edit`,
   `fs_write`, `terminal_run`. Numa janela minúscula, `fs_read` é a última a
   sair.
2. **Depois o que o objetivo pediu** — o próprio pedido é a melhor pista sobre
   quais ferramentas fazem falta agora.
3. **Até o teto que a janela aguenta.**
4. **O garçom** — `tools_find` — deixa o modelo pedir o que ficou de fora, pelo
   nome ou pelo que precisa fazer. O que o modelo ativou por conta própria nunca
   sai pelas costas dele.

No modo laço o cardápio é reavaliado a cada etapa do plano: a instrução da etapa
atual é a pista mais fresca que existe.

Tudo isso é determinístico e testável sem rede: mesma entrada, mesmo cardápio.

## As famílias

| Família | O que cobre |
|---|---|
| **Arquivos** | Ler, criar, editar e achar arquivos na pasta do projeto |
| **Terminal** | Rodar comandos no seu computador |
| **Código** | Detectar o projeto, compilar, rodar testes, lint e formatação |
| **Git** | Histórico, diferenças, staging, commits, ramos, stash, descartar |
| **Dados** | Prever e consultar CSV e bancos SQLite |
| **Web** | Buscar, abrir páginas e baixar arquivos da internet |
| **Memória** | Guardar o que deve ser lembrado em conversas futuras |
| **Índice do projeto** | Buscar por significado nos arquivos indexados |
| **Planejamento** | Dividir a tarefa em etapas e perguntar quando não tiver certeza |
| **Conectores** | Ferramentas emprestadas pelos [conectores MCP](/pt/agente/mcp) que você ligou |
| **Computador** | Área de transferência, notificações do sistema, abrir arquivos e links |

## O catálogo

| Ferramenta | O que faz |
|---|---|
| `fs_read`, `fs_write`, `fs_edit`, `fs_append` | Ler, criar/substituir, editar, acrescentar |
| `fs_list`, `fs_glob`, `fs_grep` | Listar pasta, achar arquivos, buscar no conteúdo |
| `terminal_run` | Rodar um comando |
| `project_info` | Detectar que tipo de projeto é este |
| `build_run`, `test_run`, `lint_run`, `format_run` | Compilar, testar, lint, formatar |
| `code_run` | Rodar um script |
| `git_status`, `git_diff`, `git_log` | Ver o estado e o histórico |
| `git_add`, `git_commit`, `git_branch`, `git_stash`, `git_restore` | Alterá-lo |
| `csv_preview`, `csv_query`, `data_summary` | Trabalho com CSV |
| `sql_query`, `sql_schema` | Trabalho com SQLite |
| `web_search`, `web_fetch`, `web_download`, `http_request` | A internet |
| `workspace_search` | Buscar no índice do projeto por significado |
| `memory_save` | Guardar um fato |
| `plan_create`, `plan_update`, `task_complete`, `todo_update` | Dividir em etapas, atualizar, concluir |
| `ask_user` | Perguntar a você, quando chutar seria pior |
| `agent_delegate` | Passar uma investigação a um ajudante com contexto novo |
| `tools_find` | Pedir uma ferramenta que não está no cardápio atual |
| `clipboard_read`, `clipboard_write`, `notify_user`, `open_path` | O computador em volta do app |
| `run_code` | Escrever um programa que usa as ferramentas de uma vez — [Code Mode](/pt/agente/code-mode) |

Todas passam pela [autorização](/pt/agente/autorizacao) — inclusive as chamadas
que um programa do Code Mode faz.

## Qualidade da busca web

Sem chave, o agente busca pelo DuckDuckGo: grátis, qualidade menor e sujeito a
bloqueio. Uma chave da **Brave** ou da **Tavily** nas Configurações melhora
bastante os resultados. No automático, o formato da chave escolhe o provedor —
`tvly-…` é Tavily, qualquer outra é Brave.
