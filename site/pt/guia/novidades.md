# Novidades

Esta página conta o que mudou no app que você baixaria hoje, e por quê. O
histórico completo de todas as versões está no
[changelog](/pt/guia/changelog); os instaladores ficam no
[GitHub](https://github.com/pedro-canedo/openweights/releases).

## 0.18.2 — um modelo que sempre esteve lá, e um download que usa a linha

Repositórios grandes do Hugging Face guardam cada quantização na sua pasta —
`UD-Q2_K_XL/`, `UD-Q4_K_XL/`. O download preservava esse caminho; a varredura
da biblioteca parava um nível antes, em `<autor>/<repo>`. Então um modelo de
73 GB podia terminar de baixar, ficar no disco com cada shard no tamanho exato
que o Hub publica, e nunca aparecer em **Meus Modelos**. Nada avisava que ele
estava lá. Excluir e baixar de novo daria no mesmo silêncio. A varredura agora
entra nessas pastas, e o que já está no disco aparece na próxima abertura sem
baixar nada de novo.

O download também deixou de usar um cano só. Uma transferência de 73 GB
andava a 8 MB/s numa linha de 206 Mbps, e não era limite do Hugging Face: a
CDN entrega 20 MB/s quando se pede em paralelo. Uma conexão TCP carrega no
máximo `janela ÷ tempo de ida e volta`, e a ida e volta até a CDN de LFS é de
146 ms. O app abria uma conexão por arquivo, e baixava os arquivos em fila.

Agora os shards de um modelo baixam juntos, e cada arquivo grande é dividido
em faixas com uma conexão para cada. Essa última parte veio com uma armadilha
que vale conhecer: a CDN fala HTTP/2, e sobre HTTP/2 um cliente HTTP multiplexa
as requisições concorrentes ao mesmo host numa *única* conexão TCP — então as
oito faixas voltavam a dividir a janela de uma só, e o paralelismo era enfeite.
Exigir HTTP/1.1 na transferência dos bytes foi o que tornou tudo real.

Medido de ponta a ponta contra o Hub, o mesmo arquivo de 742 MiB saiu de 41,9 s
para 21,9 s, com pico de 51,5 MB/s, e o SHA256 do arquivo remontado bateu
exatamente com o publicado. Arquivos abaixo de 16 MB continuam com uma conexão
só, porque um arquivo pequeno acaba antes de a conexão nova parar de
acelerar.

## 0.18.0 — um botão só decide o motor, as threads e o cache de especialistas

Ajustar um modelo com especialistas exigia saber que existe um fork do
llama.cpp com cache de especialistas na GPU, compilá-lo, descobrir quantos
slots cabem na placa e comparar à mão contra o motor oficial. Quase ninguém faz
isso — e quem faz, faz uma vez e nunca refaz quando troca de modelo.

**Otimizar para meu computador** faz o ciclo inteiro. Baixa o motor opcional
numa revisão fixada, confere identidade e SHA256 contra a release, executa o
binário para provar que ele expõe mesmo as capacidades que promete, e só então
mede: o seu perfil atual, dois números de threads e — quando o GGUF declara
especialistas roteados — o fork sem cache e com 16, 32 e 64 slots. Aquecimento
descartado, três repetições, um prompt curto e um longo, mediana com faixa,
RAM e VRAM observadas.

O motor oficial continua sendo o padrão, e o opcional só é cogitado quando o
arquivo do modelo diz que tem especialistas roteados. O dimensionamento do
cache lê a geometria do próprio arquivo, em vez de uma tabela de
bytes-por-parâmetro que envelheceria a cada quantização nova; sem essa
geometria, o braço do fork não roda e diz por quê. Se o pacote não tiver digest
publicado, o app se recusa a executar binário não verificado e segue no motor
oficial.

Nada é aplicado sozinho. Seu perfil manual é preservado, aplicar grava
exatamente o que foi medido, e trocar de placa, de motor ou de arquivo do
modelo depois invalida a evidência em vez de aplicá-la em silêncio.

Modelos com licença também deixaram de pedir token colado: entrar no Hugging
Face acontece no navegador. Quem já tinha aceitado a licença via o aviso de
"portão fechado" para sempre, porque ele vinha de um campo do repositório que
nunca muda; agora o veredito vem de perguntar ao próprio portão. E token
ausente, token revogado e licença não aceita — três problemas diferentes —
deixaram de dividir a mesma frase.

Os ícones deixaram de ser caracteres de texto. Trinta glifos — `✓ × ▾ ▸ ♥ ↓ ↑
↗ ⚠ ⧉ → • ⭐` — faziam papel de ícone em dezesseis arquivos, o que deixava a
fonte instalada no sistema, e não o app, decidir a forma e o peso de cada um.
Agora são pictogramas de linha, desenhados num grid só e com um traço só. Onde
o desenho carrega o sentido sozinho — a seta que distingue taxa de download da
de upload, o coração que diz que o número é de curtidas — ele é anunciado ao
leitor de tela em vez de escondido.

## 0.17.0 — chat mais fluido e configurações comparáveis

Conversas longas usam histórico virtualizado, o streaming atualiza a tela a cada
50 ms e o destaque de código espera a resposta terminar. Cancelar preserva o
texto parcial. Gerações em segundo plano deixam de atualizar todas as conversas.

Fases de espera e métricas persistidas distinguem fila, carregamento,
raciocínio e resposta visível. A fila local respeita a capacidade configurada;
títulos automáticos têm prioridade menor e entrada limitada.

**Servidor Local → Desempenho** compara a configuração atual com um candidato
do advisor, com aquecimento e três repetições por configuração. O resultado
mostra variação, não recomenda ganhos inconclusivos e oferece aplicar/restaurar.
Veja o [fluxo e os limites das medições](./desempenho). Esta versão não promete
aumento de velocidade do motor de inferência.

## 0.16.1 — a verificação do motor lia o número errado

Corrige um erro da 0.16.0 que aparecia para todo mundo: o card do motor dizia
**"o motor está instalado, mas não executa"** mesmo com o llama.cpp rodando um
modelo naquele instante.

A saída do `llama-server --version` é
`version: 0.1.0-dev (build 10441, commit 0177dcc73)`, e a verificação lia o
primeiro número depois de `version:` — o zero do `0.1.0-dev`. Como a pasta se
chama `b10441` e o binário "respondia" build 0, a conclusão era divergência
entre pasta e conteúdo. O número agora vem do `build`, que é onde ele sempre
esteve.

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
