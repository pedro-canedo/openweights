# Conectores MCP

Servidores do [Model Context Protocol](https://modelcontextprotocol.io/) viram
ferramentas que o agente pode usar — com as mesmas confirmações das nativas.

## Adicionando um

**Configurações → Conectores** aceita as duas formas:

- **Preencher manualmente** — nome, tipo e os detalhes dele.
- **Colar JSON** — o bloco `{"mcpServers": { ... }}` que você já tem de outro
  cliente funciona como está.

Dois transportes:

| Tipo | O que é |
|---|---|
| **Programa local** (stdio) | Um comando na sua máquina, com argumentos e variáveis de ambiente |
| **Servidor remoto** (HTTP) | Uma URL, com cabeçalhos |

**Testar conexão** conecta e lê o catálogo. É preciso testar antes de revisar as
ferramentas — o app não lista ferramenta que ele não viu de verdade.

## O portão de aprovação

Um servidor MCP anuncia suas ferramentas em tempo de execução e pode trocá-las
depois de aprovado. Isso é o *rug pull*, e o harness fecha essa porta:

Toda vez que o catálogo é listado, seu hash é comparado com o que você aprovou.
Enquanto os dois divergirem, **o servidor não expõe ferramenta nenhuma** — nem
para o modelo (nunca chega ao prompt) nem para execução (as chamadas são
recusadas com uma explicação). O conector fica marcado como *Aguardando revisão*
até você olhar o que mudou e aprovar de novo.

Fechar os dois caminhos importa. Bloquear só a execução deixaria o modelo
insistindo numa ferramenta fantasma.

## No catálogo de ferramentas

Cada servidor vira um provedor de ferramentas, e os nomes são prefixados com o id
do servidor — `github__create_issue`. É isso que faz as ferramentas MCP passarem
pela mesma política, pelos mesmos eventos e pela mesma barra de confirmação das
nativas. Não existe caminho paralelo.

As ferramentas carregam os selos que o servidor declara: **somente leitura**,
**destrutiva**, **alcança a internet**. Conectores são uma [família
própria](/pt/agente/ferramentas#as-familias), então desligar a família aposenta
todas de uma vez.

Quando um conector precisa de informação no meio da execução — uma *elicitação*
—, a execução pausa e pergunta a você, na trilha, em vez de chutar.
