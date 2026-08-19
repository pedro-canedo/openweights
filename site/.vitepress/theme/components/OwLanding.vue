<script setup lang="ts">
// A home.
//
// O `layout: home` do VitePress resolve uma landing de template: nome em
// gradiente, mancha desfocada atrás do logo e uma grade de cartões iguais com
// emoji dentro. Serve para qualquer projeto, e é justamente esse o problema —
// não diz nada sobre este.
//
// Aqui o conteúdo dita a forma: cada bloco tem o formato do que precisa
// mostrar. O veredito de quantização é uma amostra da interface, porque é o
// que o app faz de mais próprio; o agente ocupa mais espaço porque é a parte
// mais densa; o Code Mode aparece como código e número medido; a privacidade
// é um diagrama de três caixas, e não um selo.
//
// Os dois idiomas moram juntos por decisão: separados, a estrutura de um
// diverge da do outro na primeira correção com pressa.
import { useData, withBase } from "vitepress";
import { computed } from "vue";
import OwIcon from "./OwIcon.vue";

const { lang } = useData();
const pt = computed(() => lang.value.startsWith("pt"));

const en = {
  tagline: "Models. Your machine. Your rules.",
  lede: "An open-source desktop app that runs large language models on your own computer: it reads your hardware, installs the llama.cpp build that matches, and tells you which quantization actually fits. Then it puts an agent on top — one built for the models you can run locally.",
  download: "Download",
  learn: "How it works",
  meta: ["MIT", "Windows, macOS and Linux", "Rust + Tauri 2", "No account"],

  modelsKicker: "Models",
  modelsTitle: "The quantization that actually fits",
  modelsBody: "OpenWeights searches GGUF on Hugging Face and grades every file against the machine it is running on — VRAM, file size and the context window you asked for, because the KV cache lives in that same memory. The verdict comes before the download, not after it.",
  verdicts: [
    ["gpu", "Fits on the GPU", "The whole model in video memory. The fast case."],
    ["split", "Splits with the CPU", "Part of the layers go to system RAM. It works; it is slower."],
    ["cpu", "CPU only", "Fine for small models, painful for large ones."],
  ],
  modelsNote: "A smaller quantization fully on the GPU usually beats a better one spilling into RAM — which is why the app grades files instead of ranking them.",
  modelsLink: "Models and quantization",

  hwKicker: "Hardware",
  hwTitle: "The setup step, removed",
  hwBody: "On first launch OpenWeights reads CPU, RAM, GPU and VRAM, then downloads the llama.cpp runtime that matches your card. No terminal, no CUDA install, no guessing which build to pick. It is also why the installer is small: no GPU stack ships inside it.",
  hwRows: [
    ["NVIDIA", "CUDA build"],
    ["AMD, Intel, Apple", "Vulkan or Metal build"],
    ["No usable GPU", "CPU-only build"],
  ],
  hwLink: "First run",

  agentKicker: "The agent harness",
  agentTitle: "Built for the models you can actually run",
  agentBody: "An 8B model with an 8k window is not a smaller GPT-4: it loses the thread, repeats itself, picks the wrong tool when shown thirty of them, and claims it wrote a file it never wrote. Every decision in the harness answers to that.",
  agentStepsTitle: "One step",
  agentSteps: [
    ["Policy", "Runs, asks you, or is refused — before anything happens."],
    ["Checkpoint", "The project is photographed before the first change."],
    ["Tool", "Call, arguments, result and duration land in the run trail."],
    ["Back to the model", "Only the result returns, as the next step's input."],
  ],
  guardTitle: "Guard-rails",
  guardNote: "All deterministic. No second model judging the first.",
  guards: [
    ["Step budget", "A hard ceiling. The run always ends."],
    ["Error streak", "Three failures in a row hands the decision back to you."],
    ["Repetition", "The same call three times over is a loop, not progress."],
    ["Re-read ledger", "A file the model already has only burns context."],
    ["Context budget", "The history is summarized before the window overflows."],
    ["Verification", "Do the files it claims to have written exist?"],
  ],
  agentLink: "How a run works",

  codeKicker: "Code Mode",
  codeTitle: "One step, many calls",
  codeBody: "Instead of asking for one tool per step, the agent writes a program that uses them all at once. The harness runs it and only what the program prints comes back. Every call still travels the same path: policy, confirmation, checkpoint, trail.",
  codeSample: `const files = await fs_glob({ pattern: "logs/*.log" });
let total = 0;
for (const file of files) {
  const text = await fs_read({ path: file });
  total += (text.match(/ERROR/g) ?? []).length;
}
say(\`\${total} errors across \${files.length} files\`);`,
  measureTitle: "Measured on this machine",
  measureSub: "2026-08-18 · qwen2.5-coder:14b · RTX 5060 Ti · same task, same fixture",
  measureHead: ["", "Steps", "Tool calls", "Time"],
  measureRows: [
    ["Native loop", "37", "34", "390.1 s"],
    ["Program", "5", "17", "115.5 s"],
  ],
  measureNote: "Neither mode finished the whole task — that is the 14B model, not the harness. The full numbers, and the four failures it took to get them, are in the docs.",
  codeLink: "The measurement",

  memKicker: "Memory and project index",
  memTitle: "Two different things",
  memCols: [
    ["Memory is what the agent learned", "Few facts, not whole conversations — what goes into the prompt goes into every following run. Each one is curated before it exists, and all of them are Markdown files you can read, correct and commit."],
    ["The index is what your project contains", "grep only finds what you knew how to spell. Vectors alone are bad at proper nouns. So search is hybrid: full-text and vectors in parallel, fused — and the vectors come from your own server, with no download and no external service."],
  ],
  memLink: "Memory and project index",

  privKicker: "Privacy",
  privTitle: "Nothing is sent to a server of ours",
  privBody: "There is no server of ours. Models run on your machine and conversations are stored in a local SQLite file. The only network traffic OpenWeights starts on its own is downloading the engine, the models you pick, and checking whether a new version exists.",
  privFlow: ["Your computer", "OpenWeights", "Local model"],
  privFlowNote: "No intermediary. No account.",
  privOptTitle: "If you choose an external provider",
  privOptBody: "OpenRouter or 9router answer from where you told them to, and the screen says so. It is a separate decision, made per conversation — never the default.",

  endTitle: "Read the documentation",
  endBody: "Installation, models and quantization, the harness by parts, and the integrations.",
  endDocs: "Documentation",
  endRepo: "Source on GitHub",
};

const ptBR = {
  tagline: "Modelos. Sua máquina. Suas regras.",
  lede: "Um app de desktop, de código aberto, que roda modelos de linguagem no seu próprio computador: ele lê o seu hardware, instala a build do llama.cpp que combina e diz qual quantização realmente cabe. E põe um agente por cima — feito para os modelos que você consegue rodar localmente.",
  download: "Baixar",
  learn: "Como funciona",
  meta: ["MIT", "Windows, macOS e Linux", "Rust + Tauri 2", "Sem conta"],

  modelsKicker: "Modelos",
  modelsTitle: "A quantização que realmente cabe",
  modelsBody: "O OpenWeights busca GGUF no Hugging Face e classifica cada arquivo contra a máquina em que está rodando — VRAM, tamanho do arquivo e a janela de contexto que você pediu, porque o cache KV mora nessa mesma memória. O veredito vem antes do download, não depois.",
  verdicts: [
    ["gpu", "Cabe na GPU", "O modelo inteiro na memória de vídeo. O caso rápido."],
    ["split", "Divide com a CPU", "Parte das camadas vai para a RAM do sistema. Funciona; é mais lento."],
    ["cpu", "Só CPU", "Tudo bem em modelo pequeno, sofrido em modelo grande."],
  ],
  modelsNote: "Uma quantização menor inteira na GPU costuma ganhar de uma melhor transbordando para a RAM — é por isso que o app classifica os arquivos em vez de ordená-los.",
  modelsLink: "Modelos e quantização",

  hwKicker: "Hardware",
  hwTitle: "A etapa de configuração, removida",
  hwBody: "Na primeira execução o OpenWeights lê CPU, RAM, GPU e VRAM e baixa o runtime do llama.cpp que combina com a sua placa. Sem terminal, sem instalar CUDA, sem chutar qual build pegar. É também por isso que o instalador é pequeno: nenhuma pilha de GPU vai dentro dele.",
  hwRows: [
    ["NVIDIA", "Build CUDA"],
    ["AMD, Intel, Apple", "Build Vulkan ou Metal"],
    ["Sem GPU aproveitável", "Build só de CPU"],
  ],
  hwLink: "Primeira execução",

  agentKicker: "O harness agêntico",
  agentTitle: "Feito para os modelos que você consegue rodar",
  agentBody: "Um modelo de 8B com janela de 8k não é um GPT-4 menor: ele perde o fio, se repete, escolhe a ferramenta errada quando vê trinta delas e afirma ter escrito um arquivo que nunca escreveu. Toda decisão do harness responde a isso.",
  agentStepsTitle: "Um passo",
  agentSteps: [
    ["Política", "Roda, pergunta a você, ou é recusada — antes de qualquer coisa acontecer."],
    ["Checkpoint", "O projeto é fotografado antes da primeira alteração."],
    ["Ferramenta", "Chamada, argumentos, resultado e duração entram na trilha."],
    ["De volta ao modelo", "Só o resultado volta, como entrada do passo seguinte."],
  ],
  guardTitle: "Guard-rails",
  guardNote: "Todos determinísticos. Nenhum segundo modelo julgando o primeiro.",
  guards: [
    ["Teto de passos", "Limite duro. A execução sempre termina."],
    ["Erros seguidos", "Três falhas em sequência devolvem a decisão para você."],
    ["Repetição", "A mesma chamada três vezes é laço, não progresso."],
    ["Releitura", "Um arquivo que o modelo já tem só gasta contexto."],
    ["Orçamento de contexto", "O histórico é resumido antes de a janela estourar."],
    ["Verificação", "Os arquivos que ele afirma ter escrito existem?"],
  ],
  agentLink: "Como uma execução funciona",

  codeKicker: "Code Mode",
  codeTitle: "Um passo, muitas chamadas",
  codeBody: "Em vez de pedir uma ferramenta por passo, o agente escreve um programa que usa todas de uma vez. O harness executa e só o que o programa imprime volta. Cada chamada continua passando pelo mesmo caminho: política, confirmação, checkpoint, trilha.",
  codeSample: `const arquivos = await fs_glob({ pattern: "logs/*.log" });
let total = 0;
for (const arquivo of arquivos) {
  const texto = await fs_read({ path: arquivo });
  total += (texto.match(/ERROR/g) ?? []).length;
}
say(\`\${total} erros em \${arquivos.length} arquivos\`);`,
  measureTitle: "Medido nesta máquina",
  measureSub: "2026-08-18 · qwen2.5-coder:14b · RTX 5060 Ti · mesma tarefa, mesmo fixture",
  measureHead: ["", "Passos", "Chamadas", "Tempo"],
  measureRows: [
    ["Laço nativo", "37", "34", "390,1 s"],
    ["Programa", "5", "17", "115,5 s"],
  ],
  measureNote: "Nenhum dos dois modos terminou a tarefa inteira — isso é o modelo de 14B, não o harness. Os números completos, e as quatro falhas até chegar neles, estão na documentação.",
  codeLink: "A medição",

  memKicker: "Memória e índice do projeto",
  memTitle: "Duas coisas diferentes",
  memCols: [
    ["A memória é o que o agente aprendeu", "Poucos fatos, não conversas inteiras — o que entra no prompt entra em toda execução seguinte. Cada um passa por curadoria antes de existir, e todos são arquivos Markdown que você lê, corrige e versiona."],
    ["O índice é o que o seu projeto contém", "O grep só acha o que você soube escrever. Embedding sozinho erra em nome próprio. Então a busca é híbrida: texto e vetor em paralelo, fundidos — e os vetores saem do seu próprio servidor, sem download e sem serviço externo."],
  ],
  memLink: "Memória e índice do projeto",

  privKicker: "Privacidade",
  privTitle: "Nada é enviado para um servidor nosso",
  privBody: "Não existe servidor nosso. Os modelos rodam na sua máquina e as conversas ficam num arquivo SQLite local. O único tráfego de rede que o OpenWeights inicia por conta própria é baixar o motor, os modelos que você escolhe e checar se existe versão nova.",
  privFlow: ["Seu computador", "OpenWeights", "Modelo local"],
  privFlowNote: "Sem intermediário. Sem conta.",
  privOptTitle: "Se você escolher um provedor externo",
  privOptBody: "OpenRouter ou 9router respondem de onde você mandou, e a tela diz isso. É uma decisão à parte, tomada por conversa — nunca o padrão.",

  endTitle: "Leia a documentação",
  endBody: "Instalação, modelos e quantização, o harness por partes e as integrações.",
  endDocs: "Documentação",
  endRepo: "Código no GitHub",
};

const t = computed(() => (pt.value ? ptBR : en));
const base = computed(() => (pt.value ? "/pt" : ""));
const href = (p: string) => withBase(`${base.value}${p}`);

const links = computed(() =>
  pt.value
    ? {
        install: "/guia/instalacao",
        guide: "/guia/",
        firstRun: "/guia/primeira-execucao",
        models: "/guia/modelos",
        agent: "/agente/",
        code: "/agente/code-mode",
        memory: "/agente/memoria",
        providers: "/integracoes/provedores",
      }
    : {
        install: "/guide/install",
        guide: "/guide/",
        firstRun: "/guide/first-run",
        models: "/guide/models",
        agent: "/agent/",
        code: "/agent/code-mode",
        memory: "/agent/memory",
        providers: "/integrations/providers",
      },
);
</script>

<template>
  <div class="ow-landing">
    <!-- ------------------------------------------------------------ hero -->
    <header class="ow-hero">
      <div>
        <h1 class="ow-hero__name">OpenWeights</h1>
        <p class="ow-hero__tagline">{{ t.tagline }}</p>
        <p class="ow-hero__lede">{{ t.lede }}</p>
        <div class="ow-actions">
          <a class="ow-btn ow-btn--primary" :href="href(links.install)">
            {{ t.download }}
          </a>
          <a class="ow-btn" :href="href(links.guide)">
            {{ t.learn }}
            <OwIcon name="arrowRight" :size="14" />
          </a>
        </div>
        <ul class="ow-meta">
          <li v-for="m in t.meta" :key="m">{{ m }}</li>
        </ul>
      </div>

    </header>

    <!-- -------------------------------------------------------- hardware -->
    <section id="hardware" class="ow-section ow-section--split">
      <div class="ow-section__lead">
        <p class="ow-kicker"><OwIcon name="cpu" :size="14" />{{ t.hwKicker }}</p>
        <h2>{{ t.hwTitle }}</h2>
        <p class="ow-body">{{ t.hwBody }}</p>
        <a class="ow-link" :href="href(links.firstRun)">
          {{ t.hwLink }}<OwIcon name="arrowRight" :size="14" />
        </a>
      </div>
      <table class="ow-table">
        <tbody>
          <tr v-for="row in t.hwRows" :key="row[0]">
            <th scope="row">{{ row[0] }}</th>
            <td>{{ row[1] }}</td>
          </tr>
        </tbody>
      </table>
    </section>

    <!-- --------------------------------------------------------- modelos -->
    <section id="models" class="ow-section ow-section--split ow-section--reverse">
      <div class="ow-section__lead">
        <p class="ow-kicker">
          <OwIcon name="model" :size="14" />{{ t.modelsKicker }}
        </p>
        <h2>{{ t.modelsTitle }}</h2>
        <p class="ow-body">{{ t.modelsBody }}</p>
        <p class="ow-note">{{ t.modelsNote }}</p>
        <a class="ow-link" :href="href(links.models)">
          {{ t.modelsLink }}<OwIcon name="arrowRight" :size="14" />
        </a>
      </div>

      <dl class="ow-verdicts">
        <div v-for="v in t.verdicts" :key="v[0]">
          <dt>
            <i :class="`ow-verdict ow-verdict--${v[0]}`" />
            {{ v[1] }}
          </dt>
          <dd>{{ v[2] }}</dd>
        </div>
      </dl>
    </section>

    <!-- ---------------------------------------------------------- agente -->
    <section id="agent" class="ow-section ow-section--wide">
      <div class="ow-section__lead ow-section__lead--wide">
        <p class="ow-kicker">
          <OwIcon name="terminal" :size="14" />{{ t.agentKicker }}
        </p>
        <h2>{{ t.agentTitle }}</h2>
        <p class="ow-body">{{ t.agentBody }}</p>
      </div>

      <h3 class="ow-subhead ow-subhead--rule">{{ t.agentStepsTitle }}</h3>
      <ol class="ow-steps">
        <li v-for="(s, i) in t.agentSteps" :key="s[0]">
          <span class="ow-steps__n">{{ i + 1 }}</span>
          <strong>{{ s[0] }}</strong>
          <span>{{ s[1] }}</span>
        </li>
      </ol>

      <h3 class="ow-subhead ow-subhead--rule">{{ t.guardTitle }}</h3>
      <dl class="ow-defs">
        <div v-for="g in t.guards" :key="g[0]">
          <dt>{{ g[0] }}</dt>
          <dd>{{ g[1] }}</dd>
        </div>
      </dl>
      <p class="ow-note">{{ t.guardNote }}</p>

      <a class="ow-link" :href="href(links.agent)">
        {{ t.agentLink }}<OwIcon name="arrowRight" :size="14" />
      </a>
    </section>

    <!-- ------------------------------------------------------- code mode -->
    <section id="code-mode" class="ow-section ow-section--split ow-section--reverse">
      <div class="ow-section__lead">
        <p class="ow-kicker">
          <OwIcon name="model" :size="14" />{{ t.codeKicker }}
        </p>
        <h2>{{ t.codeTitle }}</h2>
        <p class="ow-body">{{ t.codeBody }}</p>

        <p class="ow-note">{{ t.measureNote }}</p>
        <a class="ow-link" :href="href(links.code)">
          {{ t.codeLink }}<OwIcon name="arrowRight" :size="14" />
        </a>
      </div>

      <div class="ow-stack">
        <pre class="ow-code"><code>{{ t.codeSample }}</code></pre>
        <div class="ow-measure">
          <p class="ow-measure__title">{{ t.measureTitle }}</p>
          <p class="ow-measure__sub">{{ t.measureSub }}</p>
          <table class="ow-table ow-table--measure">
            <thead>
              <tr>
                <th v-for="h in t.measureHead" :key="h" scope="col">{{ h }}</th>
              </tr>
            </thead>
            <tbody>
              <tr v-for="(row, i) in t.measureRows" :key="row[0]" :class="{ 'is-pick': i === 1 }">
                <th scope="row">{{ row[0] }}</th>
                <td v-for="(cell, j) in row.slice(1)" :key="j">{{ cell }}</td>
              </tr>
            </tbody>
          </table>
        </div>
      </div>
    </section>

    <!-- ---------------------------------------------------------- memória -->
    <section id="memory" class="ow-section">
      <p class="ow-kicker"><OwIcon name="index" :size="14" />{{ t.memKicker }}</p>
      <h2 class="ow-h2--compact">{{ t.memTitle }}</h2>
      <div class="ow-grid ow-grid--tight">
        <div v-for="c in t.memCols" :key="c[0]">
          <h3 class="ow-subhead">{{ c[0] }}</h3>
          <p class="ow-body ow-body--sm">{{ c[1] }}</p>
        </div>
      </div>
      <a class="ow-link" :href="href(links.memory)">
        {{ t.memLink }}<OwIcon name="arrowRight" :size="14" />
      </a>
    </section>

    <!-- ------------------------------------------------------ privacidade -->
    <section id="privacy" class="ow-section ow-section--split">
      <div class="ow-section__lead">
        <p class="ow-kicker">
          <OwIcon name="shield" :size="14" />{{ t.privKicker }}
        </p>
        <h2>{{ t.privTitle }}</h2>
        <p class="ow-body">{{ t.privBody }}</p>
      </div>

      <div class="ow-flow">
        <ol class="ow-flow__chain">
          <li v-for="(node, i) in t.privFlow" :key="node">
            <span class="ow-flow__node">
              <OwIcon v-if="i === 0" name="display" :size="14" />
              {{ node }}
            </span>
            <OwIcon
              v-if="i < t.privFlow.length - 1"
              class="ow-flow__arrow"
              name="arrowDown"
              :size="16"
            />
          </li>
        </ol>
        <p class="ow-flow__note">{{ t.privFlowNote }}</p>

        <div class="ow-flow__opt">
          <p class="ow-subhead">{{ t.privOptTitle }}</p>
          <p class="ow-body ow-body--sm">{{ t.privOptBody }}</p>
          <a class="ow-link" :href="href(links.providers)">
            {{ pt ? "Fontes de modelo" : "Model sources"
            }}<OwIcon name="arrowRight" :size="14" />
          </a>
        </div>
      </div>
    </section>

    <!-- --------------------------------------------------------- fechamento -->
    <section class="ow-end">
      <h2>{{ t.endTitle }}</h2>
      <p class="ow-body">{{ t.endBody }}</p>
      <div class="ow-actions">
        <a class="ow-btn ow-btn--primary" :href="href(links.guide)">
          {{ t.endDocs }}
        </a>
        <a class="ow-btn" href="https://github.com/pedro-canedo/openweights">
          {{ t.endRepo }}
          <OwIcon name="arrowRight" :size="14" />
        </a>
      </div>
    </section>
  </div>
</template>
