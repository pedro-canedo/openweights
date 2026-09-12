# Desenvolvimento do Studio

O código `app/` foi incorporado do MVP `C:\AI\llm-trainning`, preservando sua
licença Apache-2.0. Os dados do MVP não fazem parte do repositório.

## Ambiente privado

Use CPython 3.12 no ambiente `studio/.venv`. Instale PyTorch 2.9.1 pelo índice
oficial `https://download.pytorch.org/whl/cu128` e depois `runtime-lock.txt`.
O lock registra o conjunto usado na bancada; a distribuição requer ainda
homologação em Windows limpo e inventário de licenças das dependências.

Execute `python -m pytest tests` dentro de `studio/`. Testes de CUDA reais não
fazem parte do conjunto unitário. O servidor independente é `python -m app`.

## Pacote Windows

Use o arquivo oficial do llama.cpp no commit
`b95502ba9aa0eb73a2f4fc8878d7fbe6a847a0b9`; o SHA-256 do tar.gz é
`c2ec2a837346b7ecb2a0ff4e2ac343667067b02dd795f296573d50bdf11cc37f`.
Compile `llama-quantize` em Release com CUDA desligado. Conversão e merge usam CPU.

`build_runtime.py --output <nova-pasta> --llama-source <fonte> --llama-binaries <binários>`
copia Python, bibliotecas, backend e ferramentas para uma pasta relocável. Execute
com o Python do ambiente privado. `--no-archive` permite validar antes de compactar.

Em builds debug do desktop, `OW_STUDIO_DEV_RUNTIME` aponta para essa pasta. Release
não aceita esse override. O catálogo remoto `windows-x64.json` precisa de assinatura
Minisign da chave de releases existente; o instalador rejeita catálogo sem assinatura
e pacote sem o hash correto. Nenhuma chave privada pertence a este repositório.

## Persistência e contratos

O Tauri passa `LAB_DATA`, `OW_MODELS_DIR`, `HF_HOME`, `OW_LLAMA_DIR` e um token efêmero.
`/api/v1` é o contrato guiado; `/api/studio` e as rotas legadas continuam disponíveis.
O SQLite e os snapshots ficam separados do banco de conversas do desktop.

Publicação de GGUF usa staging e rename; o catálogo do desktop recebe somente o
arquivo final. Dataset, configuração e runtime são verificados antes da retomada.
Não altere snapshots para adaptar checkpoints incompatíveis.

## Portão de release

`build_ocr.py` monta o pacote OCR a partir de um ambiente privado conda-forge;
`build_ocr_sources.py` reúne receitas, patches e fontes correspondentes das
dependências copyleft. Os modelos por/eng são fixados por commit e SHA-256.
`split_runtime.py` divide o ZIP de treino em partes abaixo do limite do GitHub.
O instalador recompõe o arquivo e confere o SHA-256 integral antes de extrair.

O workflow `studio-runtime.yml` baixa os arquivos descritos nos catálogos
versionados, verifica partes e ZIP completo, extrai em Windows e executa
`verify_runtime.py` pelo Python privado com PATH restrito. Somente após esses
testes utiliza o secret de assinatura do CI e anexa os catálogos assinados.

Antes de publicar, validar Windows sem Python global, pacote assinado, cancelamento,
retomada, OCR empacotado e o fluxo PDF → GPU → GGUF → chat no desktop. Sem essas
evidências, mantenha o módulo como alpha e não anuncie instalação pronta ao usuário.
