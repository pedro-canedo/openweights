# Novidades

## 0.19.3 — um espaço de trabalho mais claro

Configurações, Fontes, DeepSeek Harness e Meus Modelos agora compartilham uma
hierarquia visual mais clara. Os cabeçalhos explicam a finalidade da tela, a
ação principal aparece no lugar esperado e os detalhes técnicos continuam
disponíveis sem disputar atenção com a próxima decisão.

Meus Modelos ganhou busca, totais da biblioteca e mais espaço para os nomes.
Os cartões de fontes explicam para que servem llama.cpp local, 9router e
OpenRouter e abrem os controles certos. O tema claro, telas estreitas, foco de
teclado e as traduções foram revisados juntos.

Esta página conta o que mudou no app que você baixaria hoje, e por quê. O
histórico completo de todas as versões está no
[changelog](/pt/guia/changelog); os instaladores ficam no
[GitHub](https://github.com/pedro-canedo/openweights/releases).

## 0.19.2 — o que a escolha nova levou junto

Deixar você escolher qual configuração medida usar exigiu afrouxar as guardas
em torno de aplicar uma delas. Uma dessas guardas fazia dois trabalhos, e só um
deles atrapalhava.

A verificação antiga só deixava aplicar se o perfil em uso fosse exatamente o
de antes da medição. Isso impedia trocar entre as opções medidas — que é o
sentido da tela nova —, então ela saiu. Mas ela também impedia aplicar por cima
de um ajuste feito à mão *depois* de medir, e essa metade ficou sem substituto:
bastava mexer numa flag, clicar em "usar configuração", e o ajuste sumia sem
uma palavra. Os dois casos agora são distinguidos, e um perfil editado à mão é
avisado antes do clique, em vez de recusado depois dele.

Mais duas coisas saíram da mesma revisão. Um perfil que não diz nada sobre qual
motor usar estava sendo lido como "use o oficial" — e o assistente de ajuste
nunca preenche esse campo, então aceitar uma recomendação de flags devolvia ao
motor oficial, caladamente, quem estava no opcional. E uma falha ao reverter o
motor podia abortar antes de o perfil ser restaurado, deixando a configuração
nova gravada com o motor parado.

Os três passavam na suíte de testes, no lint e no formatador. Nenhum era
visível sem ler o que a mudança tirou junto com o que ela queria tirar.

## 0.19.1 — a medição estava cronometrando a própria repetição

O mesmo modelo, o mesmo perfil e a mesma máquina apareciam com 129 tok/s na
tela de otimização, 28 no histórico de desempenho e 31 numa conversa de
verdade. Quem estava errado era a otimização.

O prompt longo dela era o curto repetido trinta e duas vezes. Parecia
inofensivo e não era: num perfil com especulação por n-grama, o rascunho
encontra a continuação *dentro do próprio prompt*, a aceitação vai ao teto, e
tokens-gerados-sobre-tempo-de-geração deixa de descrever o modelo e passa a
descrever o quanto o texto de teste se repete. A aritmética bate com o sintoma
exatamente — um rascunho de até 4 tokens, e 32 × 4 = 128.

O histórico de desempenho nunca caiu nessa, porque o `llama-bench` não usa
especulação nenhuma; o chat também não, porque lê texto real. Foi por isso que
esses dois concordavam em torno de 30 e só a otimização destoava.

O prompt longo agora traz quatro formas de função e quatro enunciados
diferentes. Variar só o identificador não bastaria: um mesmo corpo com nomes
diferentes ainda repete `for value in values { if *value > limit {` em todos os
blocos, e uma sequência dessas é justamente o que um n-grama copia.

## 0.19.0 — escolha a configuração que você mediu

- **Escolha qualquer opção concluída na otimização.** Cada configuração tem
  geração, leitura do prompt e tempo total visíveis. A recomendação continua
  considerando tempo total e estabilidade, mas agora você pode selecionar uma
  alternativa com geração mais rápida e clicar em **Usar configuração**, mesmo
  sem um vencedor recomendado. A medição preserva o perfil atual até esse clique.
- **O histórico funciona com o servidor ligado e ocioso.** Perfis completos
  salvos têm uma ação explícita para aplicar e carregar o modelo. Medições
  antigas sem perfil completo são identificadas; o limite de energia mostrado
  pertence à medição e não é alterado ao reaplicar o perfil.
- **A aba de desempenho foi reorganizada.** Cards com ícones, métricas
  separadas, indicação da configuração em uso e layout responsivo destacam
  otimização e histórico. Ajustes manuais e presets ficam em uma seção expansível.
- **Aplicar mantém os controles sincronizados.** O editor e o histórico são
  atualizados após uma aplicação bem-sucedida. A escolha usa o motor associado
  ao perfil; atividade em andamento impede a aplicação, e falhas de carga
  acionam a recuperação da configuração anterior.

As velocidades exibidas são resultados do teste realizado, não uma promessa
para todas as conversas. Esta versão não altera automaticamente a configuração
ao terminar a otimização: você escolhe a opção e confirma em **Usar configuração**.

## 0.18.4 — o download estava travando a máquina inteira

O download em paralelo, que chegou na 0.18.2, deixou as transferências muito
mais rápidas e passou a travar tudo: o app parava de responder, o botão de
pausa não fazia nada, e as outras aplicações travavam junto. O Gerenciador de
Tarefas mostrava o defeito inteiro numa linha — **85 MB/s de disco com 0 Mbps
de rede**. O app escrevia 85 megabytes por segundo sem baixar nada.

No NTFS, definir o tamanho de um arquivo move o fim dele, mas não o *valid data
length*. Escrever depois no meio faz o sistema zerar fisicamente tudo que vem
antes, para o arquivo novo não expor o que havia naqueles setores. Enquanto o
download era sequencial isso nunca aparecia: o arquivo crescia byte a byte e o
valid data length ia junto. Pré-alocar e então escrever de oito conexões ao
mesmo tempo inverteu o jogo — cada conexão começa longe do início, e cada uma
dispara um zeramento de vários gigabytes. O disco é do sistema inteiro, então o
sistema inteiro esperou.

Marcar o arquivo como esparso diz ao NTFS que as regiões intocadas são buracos,
e buraco não precisa ser zerado. Medido aqui num arquivo de 16 GiB, escrevendo
1 MiB na marca de 14 GB: 9,54 s antes, 0,51 ms depois.

A barra de status também parou de quebrar. Os nomes escritos custavam mais
largura que os números que apresentavam, então em telas menores "26 GB / 64 GB"
se partia em duas linhas e a barra crescia. Agora cada medidor traz um
pictograma e guarda o nome para quando o ponteiro passar.

Duas coisas menores na mesma versão. A tabela do histórico de desempenho
sempre filtrou por modelo, mas nunca disse qual — e um número de geração não
significa nada sem o modelo que o produziu, que foi como 32 tokens/s numa
tabela e 143 em outra tela puderam parecer uma contradição. Cada linha também
virou um botão que volta àquela configuração, para quando uma mudança sai pior.
E uma otimização que termina sem vencedor deixa de oferecer "restaurar": nada
foi aplicado, então não há o que desfazer.

## 0.18.3 — um catálogo público escondido atrás de uma sessão vencida

O login do Hugging Face dura algumas horas. O **Descobrir**, o README do modelo
e a lista de quantizações pegavam cada um o cliente montado quando a janela
abriu, com token e tudo, e nunca o renovavam. Vencido esse token, o Hub
respondia 401 e a tela inteira desabava em "Algo deu errado" — sendo que o
catálogo de modelos é público e não precisa de token nenhum. Renovar a sessão
antes de perguntar resolve; além disso, a busca agora repete sem token quando
ele é recusado, para que uma sessão revogada também não esconda uma lista
pública.

O cartão de erro era parte do problema. Ele dizia "Algo deu errado" e jogava o
motivo no console do navegador, onde ninguém olha. Agora traz um "detalhes
técnicos" com a mensagem real — que foi justamente como este defeito apareceu,
lendo um 401 que o app vinha engolindo.

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

No Windows, o atalho na Área de Trabalho voltou. Ele nascia só na instalação
nova: a rotina de atalho do instalador do Tauri desiste quando está rodando
como atualização, e o updater do app sempre a roda assim. Então quem instalou
uma vez e desde então só atualiza nunca ganhou um atalho, por mais versões que
passassem — o app estava lá, e achá-lo era procurar no menu Iniciar. A regra em
si está certa, porque um atalho apagado de propósito não deve voltar pelas
costas de quem apagou; ela só não distinguia "apaguei" de "nunca tive". Agora
uma marca no registro separa os dois casos.

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
