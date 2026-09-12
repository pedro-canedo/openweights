# OpenWeights Studio

O Studio é o módulo opcional de treinamento local do OpenWeights. Ele prepara seus
arquivos, adapta um modelo pequeno e gera um GGUF para abrir no chat.

Disponível a partir da versão 0.20.0. O primeiro download do treinamento
tem aproximadamente 3 GB; o OCR opcional acrescenta cerca de 66 MB.

1. Abra **Treinar** e instale o módulo.
2. Solte um PDF, textos, uma pasta de código ou um arquivo de conversas.
3. Clique em **Preparar**. Um único livro é suficiente; documentos curtos são
   preservados e recebem orientação para acrescentar mais conteúdo.
4. Clique em **Treinar e gerar modelo**. Se o motor estiver ligado, use a opção
   **Parar motor e iniciar treino** quando puder interromper as respostas atuais.
5. Aguarde a preparação do modelo e escolha **Abrir no chat**.

O primeiro download inclui as dependências em uma pasta privada. O computador
precisa de Windows x64, GPU NVIDIA e driver compatível. A configuração é conferida
antes do treino; reserve inicialmente 8 GB de disco e 4 GB de RAM livre.

Para páginas digitalizadas, escolha **Instalar leitura de páginas digitalizadas**
quando solicitado e depois **Retomar preparação**. O reconhecimento é local,
em português e inglês. Páginas com texto nativo dispensam OCR.

**Cancelar** solicita uma parada com checkpoint. Depois de uma interrupção, a
retomada exige um checkpoint completo e a mesma receita e versão do runtime.

Na opção **Já usava o MVP?**, importe uma cópia dos dados em um Studio vazio.
Os arquivos originais permanecem intactos. Checkpoints antigos podem exigir o
runtime original; a importação não os torna automaticamente compatíveis.

Um treino curto demonstra o processo. Ele não garante que o modelo responderá
corretamente sobre o livro. Livros continuam sendo texto; o Studio não inventa
perguntas e respostas para transformá-los em conversas.

Instruções para colaboradores: [desenvolvimento](development.md).
