# Validação do Studio — 11/09/2026

Status: **release candidata 0.20.0-rc.1**. Os testes abaixo não substituem uma
homologação ampla de uso pela janela Tauri em diferentes máquinas.

## Evidência real na bancada

Windows x64, RTX 3090 24 GB, driver 616.92. Python privado 3.12,
PyTorch 2.9.1+cu128, Transformers 4.57.3, TRL 0.26.2, PEFT 0.18.0,
bitsandbytes 0.49.1. Base Qwen3-0.6B na revisão
`c1899de289a04d12100db370d81485cdf75e47ca`.

- O Alienista, PDF nativo de 36 páginas: 61 trechos, separados em 49/6/6.
  Treino causal QLoRA, uma passagem, dois passos de otimização, pico de
  memória alocada de aproximadamente 3,63 GB. Loss no teste reservado: 3,8141.
- Merge em CPU e GGUF Q4_K_M: 396.704.480 bytes. SHA-256:
  `d9659777e1ea0b32ee2bd0810a48418ca9960793fa25b858dfe13d7956923a25`.
- GGUF carregado pelo llama-server instalado com o desktop, health e resposta
  OpenAI compatível verificados. A resposta sobre a autoria foi incorreta
  (Oscar Wilde). Isso comprova execução técnica, não qualidade factual.
- Cancelamento pela fila, checkpoint completo e retomada até GGUF: execução
  `44a4a52b566e416cb69a401c65f85bb2`, checkpoint-2.
- Conversas: fixture artificial de 30 conversas, apenas para teste técnico;
  SFT QLoRA, avaliação reservada e exportação concluídos. Execução
  `6c27650e9c834d9eab4736abafc3e31c`. GGUF de 396.704.480 bytes, SHA-256
  `035d5d07d62ea7cd7330049264c736a507b65e0f45d5903e7a053c0e793c6692`.
- Interface React independente conectada à API real: histórico recuperado,
  preparação de O Alienista exibindo 49/6/6 e ação de treino habilitada.
  Nenhum erro de console observado nesta verificação.

Os artefatos de bancada estão em `.runtime/` (ignorados pelo Git); logs do
chat em `gguf-chat-result.json` e das conversas em `smoke-conversations.log`.
O MVP original não foi movido nem alterado.

## Verificações automatizadas

Build TypeScript/Vite e paridade de traduções/documentação aprovados; 16 testes
do frontend aprovados. Binário Rust: 39 testes aprovados, um ignorado.
Backend: suíte original e testes de preparação, autenticação, importação,
exclusão de processos e publicação. A publicação tem testes que verificam
ausência de GGUF parcial na biblioteca, repetição idempotente e hash inválido.

## Validação adicional do pacote — 12/09/2026

OCR portátil Tesseract 5.5.3 / Poppler 26.09.0 com modelos `por` e `eng`:
três páginas rasterizadas de O Alienista, similaridade normalizada de 0,998683
com o texto nativo. PDF misto preservou a página nativa sem OCR; a retomada
funcionou sem o motor instalado, reutilizando checkpoints. Resultado em
`.runtime/ocr-acceptance/result.json`.

Python privado do runtime 1.0.0-rc.1 importou PyTorch, PEFT, TRL e bitsandbytes
e executou OCR real com PATH restrito ao System32, sem depender de Python,
Conda ou CUDA Toolkit globais. Resultado em `.runtime/private-verify.log`.
Fontes correspondentes e receitas das dependências copyleft do OCR foram
reunidas para acompanhar os binários. Os catálogos registram hashes e tamanhos.

O instalador tem testes de rejeição de ZIP corrompido e caminhos inseguros,
além de ativação somente após extração completa. Clippy do workspace aprovado.

## Pendências para aprovação estável

- Montar, assinar e publicar o pacote e catálogo Windows; testar download
  interrompido, pacote corrompido, falta de disco e ativação em instalação limpa.
- Ampliar a amostra OCR para digitalizações de câmera, baixa resolução,
  layouts complexos e outros livros além da fixture rasterizada.
- Executar a jornada inteira na janela Tauri, incluindo instalação sob demanda,
  disputa com chat/servidor/cluster e abertura no chat pelo botão de resultado.
- Homologar pasta de código e variantes próximas de conversas com dados reais;
  os testes atuais de deduplicação não comprovam ausência de todo vazamento semântico.
- Inventariar licenças redistribuídas e validar desinstalação/atualizações,
  preservando runtimes exigidos por execuções antigas.

Enquanto essas pendências existirem, não promover a candidata para o canal
estável nem anunciar homologação completa em todas as máquinas.
