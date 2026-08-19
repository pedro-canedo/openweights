---
layout: home

hero:
  name: OpenWeights
  text: Modelos. Sua máquina. Suas regras.
  tagline: Um app de código aberto que roda LLMs localmente — e um harness agêntico que faz modelo pequeno entregar trabalho.
  image:
    src: /logo-1024.png
    alt: OpenWeights
  actions:
    - theme: brand
      text: Baixar
      link: /pt/guia/instalacao
    - theme: alt
      text: O que é
      link: /pt/guia/
    - theme: alt
      text: O harness agêntico
      link: /pt/agente/

features:
  - icon: 🔍
    title: Hardware no automático
    details: Detecta CPU, RAM, GPU e VRAM e baixa a build do llama.cpp que combina — CUDA, Vulkan ou só CPU. Sem terminal, sem instalar CUDA.
    link: /pt/guia/primeira-execucao
    linkText: Primeira execução
  - icon: 🤗
    title: Modelos já filtrados
    details: Busca GGUF no Hugging Face e recomenda a quantização para o seu PC. Verde roda inteiro na GPU, amarelo divide com a CPU, cinza é só CPU.
    link: /pt/guia/modelos
    linkText: Modelos e quantização
  - icon: 🤖
    title: Um agente que responde a você
    details: Lê e edita arquivos, roda comandos, usa Git e navega na web — tudo pelo nível de autorização que você escolheu, com um checkpoint antes da primeira alteração.
    link: /pt/agente/
    linkText: Como uma execução funciona
  - icon: ⚡
    title: Code Mode
    details: Em vez de pedir uma ferramenta por vez, o agente escreve um programa que usa todas de uma vez. Medido aqui - 3,4x mais rápido e 7,4x menos idas ao modelo.
    link: /pt/agente/code-mode
    linkText: A medição
  - icon: 🧠
    title: Memória e índice do projeto
    details: O agente guarda o que aprendeu entre conversas e busca no seu código por significado, não só por texto literal.
    link: /pt/agente/memoria
    linkText: Memória
  - icon: 🔌
    title: API compatível com OpenAI
    details: Aponte qualquer outro app para o localhost e use o mesmo modelo. Opcionalmente alcançável pela sua rede local.
    link: /pt/integracoes/api-local
    linkText: Servidor local
---

<div style="max-width: 780px; margin: 4rem auto 0; text-align: center;">

## Nada é enviado para um servidor nosso

Não existe servidor nosso. Os modelos rodam na sua máquina, as conversas ficam
num arquivo SQLite local, e o único tráfego de rede que o OpenWeights inicia por
conta própria é baixar o motor, os modelos que você escolhe e checar se existe
versão nova do app.

Se você apontar para um provedor externo — OpenRouter ou 9router — esse tráfego
vai para onde você mandou, e a tela diz isso.

</div>
