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
// que o app faz de mais próprio; o trabalho de agente é uma tabela de
// harnesses externos abertos com um clique; a privacidade é um diagrama de
// três caixas, e não um selo.
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
  lede: "An open-source desktop app that runs large language models on your own computer: it reads your hardware, installs the llama.cpp build that matches, and tells you which quantization actually fits. And when you want work done, not just answers, it hands those models to an external coding agent — opened with one click, already configured.",
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

  harnessKicker: "Agent work",
  harnessTitle: "A real harness, one click away",
  harnessBody: "OpenWeights does not ship a chat-with-tools of its own. When you want work done, it hands your models to an external coding agent — pre-configured with every provider and model the app knows, key by environment variable, never on the command line. The DeepSeek Harness is installed and managed by the app itself, in an isolated folder with a portable Node, and opens in a window of its own.",
  harnessRows: [
    ["DeepSeek Harness", "Installed, configured and opened by the app — one click"],
    ["Claude Code", "Launched against your local API, model already selected"],
    ["Aider", "Starts pointed at your API, model already selected"],
    ["OpenCode", "Same, through the environment variables it expects"],
  ],
  harnessLink: "Open in a harness",

  privKicker: "Privacy",
  privTitle: "Nothing is sent to a server of ours",
  privBody: "There is no server of ours. Models run on your machine and conversations are stored in a local SQLite file. The only network traffic OpenWeights starts on its own is downloading the engine, the models you pick, and checking whether a new version exists.",
  privFlow: ["Your computer", "OpenWeights", "Local model"],
  privFlowNote: "No intermediary. No account.",
  privOptTitle: "If you choose an external provider",
  privOptBody: "OpenRouter or 9router answer from where you told them to, and the screen says so. It is a separate decision, made per conversation — never the default.",

  endTitle: "Read the documentation",
  endBody: "Installation, models and quantization, and the integrations.",
  endDocs: "Documentation",
  endRepo: "Source on GitHub",
};

const ptBR = {
  tagline: "Modelos. Sua máquina. Suas regras.",
  lede: "Um app de desktop, de código aberto, que roda modelos de linguagem no seu próprio computador: ele lê o seu hardware, instala a build do llama.cpp que combina e diz qual quantização realmente cabe. E quando você quer trabalho feito, não só resposta, ele entrega esses modelos a um agente de código externo — aberto com um clique, já configurado.",
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

  harnessKicker: "Trabalho de agente",
  harnessTitle: "Um harness de verdade, a um clique",
  harnessBody: "O OpenWeights não traz um chat-com-ferramentas próprio. Quando você quer trabalho feito, ele entrega os seus modelos a um agente de código externo — pré-configurado com todos os provedores e modelos que o app conhece, chave por variável de ambiente, nunca na linha de comando. O DeepSeek Harness é instalado e gerenciado pelo próprio app, numa pasta isolada com Node portátil, e abre em janela própria.",
  harnessRows: [
    ["DeepSeek Harness", "Instalado, configurado e aberto pelo próprio app — um clique"],
    ["Claude Code", "Lançado contra a sua API local, com o modelo já escolhido"],
    ["Aider", "Sobe apontado para a sua API, com o modelo já escolhido"],
    ["OpenCode", "Idem, pelas variáveis de ambiente que ele espera"],
  ],
  harnessLink: "Abrir em um harness",

  privKicker: "Privacidade",
  privTitle: "Nada é enviado para um servidor nosso",
  privBody: "Não existe servidor nosso. Os modelos rodam na sua máquina e as conversas ficam num arquivo SQLite local. O único tráfego de rede que o OpenWeights inicia por conta própria é baixar o motor, os modelos que você escolhe e checar se existe versão nova.",
  privFlow: ["Seu computador", "OpenWeights", "Modelo local"],
  privFlowNote: "Sem intermediário. Sem conta.",
  privOptTitle: "Se você escolher um provedor externo",
  privOptBody: "OpenRouter ou 9router respondem de onde você mandou, e a tela diz isso. É uma decisão à parte, tomada por conversa — nunca o padrão.",

  endTitle: "Leia a documentação",
  endBody: "Instalação, modelos e quantização e as integrações.",
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
        harness: "/integracoes/api-local#abrir-em-um-harness",
        providers: "/integracoes/provedores",
      }
    : {
        install: "/guide/install",
        guide: "/guide/",
        firstRun: "/guide/first-run",
        models: "/guide/models",
        harness: "/integrations/local-api#open-in-a-harness",
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

    <!-- ---------------------------------------------------------- harness -->
    <section id="harness" class="ow-section ow-section--split">
      <div class="ow-section__lead">
        <p class="ow-kicker">
          <OwIcon name="terminal" :size="14" />{{ t.harnessKicker }}
        </p>
        <h2>{{ t.harnessTitle }}</h2>
        <p class="ow-body">{{ t.harnessBody }}</p>
        <a class="ow-link" :href="href(links.harness)">
          {{ t.harnessLink }}<OwIcon name="arrowRight" :size="14" />
        </a>
      </div>
      <table class="ow-table">
        <tbody>
          <tr v-for="row in t.harnessRows" :key="row[0]">
            <th scope="row">{{ row[0] }}</th>
            <td>{{ row[1] }}</td>
          </tr>
        </tbody>
      </table>
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
