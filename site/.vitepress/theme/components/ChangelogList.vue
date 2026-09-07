<script setup lang="ts">
// Renderiza as versões que o carregador leu de `docs/releases/`.
//
// O idioma sai da URL, como no OwLanding: as duas metades da nota vivem no
// mesmo arquivo de origem, então não há o que sincronizar entre traduções —
// só escolher qual metade mostrar.
import { computed } from "vue";
import { useData } from "vitepress";
import { data as versoes } from "../../releases.data";

const { lang } = useData();
const pt = computed(() => lang.value.startsWith("pt"));

const repo = "https://github.com/pedro-canedo/openweights";

const entradas = computed(() =>
  versoes.map((v) => ({
    ...v,
    // Sem nota naquele idioma, mostra a outra: uma versão sem texto nenhum
    // seria pior que uma versão no idioma errado. O HTML já vem pronto do
    // carregador, renderizado pelo markdown do próprio VitePress.
    corpo: (pt.value ? v.portugues : v.ingles) || v.ingles || v.portugues,
  })),
);
</script>

<template>
  <div class="ow-changelog">
    <section v-for="v in entradas" :key="v.versao" class="ow-changelog__item">
      <h2 :id="v.versao">
        <a :href="`#${v.versao}`">{{ v.versao }}</a>
        <span v-if="v.data" class="ow-changelog__data">{{ v.data }}</span>
      </h2>
      <p v-if="v.destaque" class="ow-changelog__destaque">{{ v.destaque }}</p>
      <div class="ow-changelog__corpo" v-html="v.corpo" />
      <p class="ow-changelog__links">
        <a :href="`${repo}/releases/tag/v${v.versao}`">
          {{ pt ? "Instaladores" : "Installers" }}
        </a>
        ·
        <a :href="`${repo}/blob/v${v.versao}/docs/releases/${v.versao}.md`">
          {{ pt ? "Notas completas" : "Full notes" }}
        </a>
      </p>
    </section>

    <p v-if="!entradas.length">
      {{ pt ? "Nenhuma versão encontrada." : "No versions found." }}
    </p>

    <p class="ow-changelog__rodape">
      {{ pt ? "Versões anteriores à 0.16.0 estão em" : "Versions before 0.16.0 are in" }}
      <a :href="`${repo}/blob/main/CHANGELOG.md`">CHANGELOG.md</a>
      {{ pt ? "e na" : "and on the" }}
      <a :href="`${repo}/releases`">{{ pt ? "página de releases" : "releases page" }}</a>.
    </p>
  </div>
</template>

<style scoped>
.ow-changelog__item {
  padding-block: 1.5rem;
  border-top: 1px solid var(--vp-c-divider);
}
.ow-changelog__item:first-child {
  border-top: 0;
}
.ow-changelog__item h2 {
  display: flex;
  align-items: baseline;
  gap: 0.75rem;
  margin: 0;
  border-top: 0;
  padding-top: 0;
}
.ow-changelog__data {
  font-size: 0.8rem;
  font-weight: 400;
  color: var(--vp-c-text-3);
}
.ow-changelog__destaque {
  margin: 0.25rem 0 0;
  color: var(--vp-c-text-2);
}
.ow-changelog__corpo :deep(h3) {
  font-size: 1.05rem;
  margin-top: 1.25rem;
}
.ow-changelog__links {
  font-size: 0.85rem;
}
.ow-changelog__rodape {
  margin-top: 2rem;
  padding-top: 1rem;
  border-top: 1px solid var(--vp-c-divider);
  font-size: 0.85rem;
  color: var(--vp-c-text-2);
}
</style>
