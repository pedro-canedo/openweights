---
name: implementation
description: Writes real code every stage. Use when executing a delivery, coding, or editing project files.
phase: build
---
# Desenvolvimento

- Esta etapa só acaba quando os arquivos existem no disco. Pensar não conta.
- Não anuncie: chame `fs_write` / `fs_append` / `run_code` agora.
- Siga a arquitetura que já está no projeto. Não invente um app paralelo.
- Passos pequenos. Stub, TODO ou "depois eu implemento" ≠ pronto.
- Rede falhou (`npm create`, ping)? Escreva os arquivos na mão.
- Web/Three.js: canvas e WebGL no cliente; não bloqueie SSR com GPU.
- Leia `.openweights/progress.md` se precisar de contexto. Não peça o spec de novo.
