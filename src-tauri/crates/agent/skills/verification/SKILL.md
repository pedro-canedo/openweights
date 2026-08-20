---
name: verification
description: Proves a delivery with real checks and files on disk. Use when finishing a stage or choosing check_cmd.
phase: verify
---
# Qualidade

- Conferência **falha** se nenhum arquivo desta etapa foi escrito.
- Nomeie os arquivos criados/alterados. Sem lista, não está pronto.
- `check_cmd` tem que quebrar ANTES do trabalho e passar DEPOIS.
- Não serve: `node -v`, `npm -v`, `ls`, `dir`, `pwd`, `echo`.
- Serve: teste, `tsc --noEmit`, grep no arquivo criado, build da etapa.
