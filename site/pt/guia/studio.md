# Studio de treinamento

O Studio opcional acrescenta treinamento local ao OpenWeights. Ele mantém
Python, CUDA, OCR, checkpoints e SQLite na pasta de dados do app e não instala
Python nem Docker globalmente.

## O caminho de cinco cliques

1. Abra **Treinar** e escolha **Instalar módulo de treinamento**. O primeiro
   download tem cerca de 3 GB; o OCR de páginas digitalizadas acrescenta cerca
   de 66 MB, sob demanda.
2. Solte um PDF, texto, conversas ou uma pasta de código em **Dados**. Um livro
   sozinho é válido; PDF com texto nativo é processado página a página.
3. Escolha detecção automática ou corrija o preset (**Livro**, **Conversas** ou
   **Código**) e clique em **Preparar**. O Studio limpa o layout, agrupa duplicatas
   próximas e separa treino, validação e teste.
4. Confira o resumo e clique em **Treinar e gerar modelo**. O padrão é Qwen3 0.6B,
   QLoRA, contexto 1.024, batch 1, acumulação 16, rank 16, até 50 passos e uma
   passagem pelos dados disponíveis.
5. Ao terminar, clique em **Abrir no chat**. O GGUF e o manifesto entram na mesma
   biblioteca de modelos do OpenWeights.

Antes do treino o módulo confere VRAM, RAM e disco livres e executa um benchmark
curto. Se o motor do chat usar a GPU, ele pede confirmação antes de pará-lo.

## PDFs e OCR

Texto nativo não é rejeitado por o PDF ser longo. OCR roda somente nas páginas
sem texto utilizável. Clique em **Instalar leitura de páginas digitalizadas** e
depois em **Retomar preparação** quando solicitado. O reconhecimento é local,
com português e inglês. Progresso e checkpoints evitam truncamento silencioso.

## Dados e limites

Livros e código usam objetivo causal; conversas reais usam ajuste supervisionado.
O Studio não inventa perguntas e respostas a partir de prosa. Conteúdo pequeno
continua salvo, mas o treino é bloqueado sem três conjuntos utilizáveis: **“Este
conteúdo é curto demais para treinar e conferir o resultado. Adicione mais
texto.”** O teste reservado não escolhe checkpoint nem ajusta parâmetros.

Cancelar salva um checkpoint seguro. A retomada confere dataset, revisão da base,
receita e runtime; checkpoint incompatível recebe diagnóstico.

## Solução de problemas

- **Acesso negado (erro 5 do Windows):** feche o OpenWeights, aguarde alguns
  segundos e tente novamente. O antivírus pode segurar o marcador do runtime.
  A pasta de dados precisa permitir gravação e não deve ser protegida pelo
  sistema.
- **Pouca memória de GPU:** feche outros consumidores ou escolha contexto de
  512 tokens em **Avançado**.
- **Módulo indisponível:** o download pode ser retomado; catálogo, assinatura e
  SHA-256 são verificados antes da ativação.

Remover o módulo apaga apenas runtime e caches descartáveis. Datasets,
checkpoints e modelos permanecem por padrão.
