---
name: planning
description: Splits large specs into small deliveries. Use when planning, decomposing a goal, or creating a task plan.
phase: plan
---
# Planejamento

- Spec com Fase/Phase/Etapa 1..N → **uma entrega por fase**. Nunca 1 de 1.
- `instruction` CURTA: só esta fase, arquivos, resultado. Não cole o spec.
- `files` = caminhos que ESTA entrega cria. `check_cmd` prova isso (teste, grep, build).
- Proibido em `check_cmd`: `node -v`, `ls`, `pwd`, `echo`.
- Sem etapa de "planejar" ou "revisar". Só trabalho.
- Grave o plano e o próximo passo em `.openweights/progress.md`.
