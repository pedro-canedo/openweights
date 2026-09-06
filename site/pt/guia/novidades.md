# Novidades

A lista completa de versões, com os instaladores, fica no
[GitHub](https://github.com/pedro-canedo/openweights/releases). Esta página
conta o que mudou no app que você baixaria hoje.

## 0.16.0 — as telas ganham forma, e o motor passa a ser conferido

### O motor é verificado, não presumido

O app instala o seu próprio llama.cpp, numa pasta dele, isolado de qualquer
coisa que você tenha no sistema. Até agora o card de Configurações dizia
`b10441 · cuda13 · instalado` — um número de versão que vinha da build que o
app *instalaria*, mais a presença de um arquivo no disco. As duas metades
podiam estar erradas ao mesmo tempo: quem atualizava o OpenWeights lia o número
novo e continuava rodando a build antiga, e um pacote CUDA sem as DLLs do
runtime passa numa checagem de arquivo e só falha quando você carrega um
modelo.

Agora o app **executa** `llama-server --version` e lê a build que o próprio
binário reporta. A resposta é uma de cinco, cada uma com a sua providência:

| Veredito | O que quer dizer |
|---|---|
| **Pronto** | Executou e respondeu com a build que esta versão espera |
| **Não instalado** | Primeira execução, ou a pasta foi apagada |
| **Atualização disponível** | O disco tem outra build, diferente da testada com esta versão |
| **Variante errada** | Sua placa ou o driver mudaram desde a instalação |
| **Não executa** | Os arquivos estão lá e o executável não sobe |

A verificação roda sozinha pouco depois de o app abrir, e o que pede
providência aparece como um ponto no item **Configurações** da barra lateral —
você descobre antes de uma conversa falhar, não durante. Builds deixadas para
trás por atualizações antigas aparecem com o tamanho e um botão que devolve o
espaço; a que está em uso nunca é tocada.

### Servidor Local: uma faixa fixa e quatro abas

A tela era onze cards de peso idêntico. Agora o estado do servidor — ligado ou
parado, o endereço com botão de copiar, o modelo carregado e a velocidade de
geração — fica preso no topo enquanto você rola, e o resto se organiza pela
pergunta que você traz:

| Aba | O que tem |
|---|---|
| **Visão geral** | Conectar, usar em outro app e o que já foi servido |
| **Desempenho** | Configurar llama.cpp, especulação, histórico de benchmark, energia |
| **Rede** | Porta, acesso pela rede local, simultaneidade, GPU extra na rede |
| **Avançado** | Flags globais e o log do servidor |

Numa instalação nova, a Visão geral abre com três passos numerados — ligue,
copie o endereço, cole no app que vai usar. Eles somem sozinhos quando o
servidor atende a primeira requisição.

### O harness avisa quando está desatualizado

A versão mostrada agora é a que está realmente instalada, lida do manifesto do
próprio pacote. Toda vez que você abre a tela, o app compara com a versão que
esta release traz; divergência vira **atualização pendente**, com o botão que
resolve. A última versão publicada no npm aparece como informação — o app
instala a que passou pelos nossos testes.

### Os cartões de modelo voltam a ser texto

Os READMEs do Hugging Face são Markdown misturado com HTML, e o app mostrava
esse HTML literalmente: uma parede de `<div style="display: flex">` no lugar da
descrição do modelo. Agora ele é traduzido para Markdown puro — links, imagens,
tabelas e blocos de código inclusive — antes de chegar à tela.

### Configurações, Fontes e a marca

Configurações está ordenada pelo que importa (motor, credencial, preferências,
máquina), e o token do Hugging Face passa a dizer se existe um gravado — um
campo de senha preenchido é idêntico a um vazio. Fontes trocou a lista de
bolinhas por um cartão por fonte, e clicar no cartão leva à aba onde aquilo se
resolve; o catálogo do OpenRouter diz quantos modelos você fixou (só os fixados
aparecem no seletor do chat) e avisa quando a lista foi cortada.

A marca dentro do app passou a ser a mesma geometria dos ícones, gerada de uma
fonte única em vez de redesenhada à mão em dois lugares.
