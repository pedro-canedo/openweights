<script setup lang="ts">
// Ícones próprios, todos no mesmo grid de 24 com traço de 1.5 e pontas
// arredondadas — a métrica que o Lucide popularizou e que o shadcn/ui usa.
// São poucos de propósito: ícone aqui apoia um rótulo, nunca decora uma
// seção. Emoji ficou de fora porque muda de desenho a cada sistema e não
// herda a paleta.
const paths: Record<string, string[]> = {
  cpu: [
    "M5 5h14v14H5z",
    "M9.5 9.5h5v5h-5z",
    "M9 2v3M15 2v3M9 19v3M15 19v3M2 9h3M2 15h3M19 9h3M19 15h3",
  ],
  model: ["M21 8l-9-5-9 5m18 0l-9 5m9-5v8l-9 5m0-8L3 8m9 5v8M3 8v8l9 5"],
  terminal: ["M4 17l6-5-6-5", "M12 19h8"],
  shield: ["M12 3l8 3v6c0 4.5-3.2 7.9-8 9-4.8-1.1-8-4.5-8-9V6l8-3z"],
  index: [
    "M4 6c0-1.7 3.6-3 8-3s8 1.3 8 3-3.6 3-8 3-8-1.3-8-3z",
    "M4 6v12c0 1.7 3.6 3 8 3s8-1.3 8-3V6",
    "M4 12c0 1.7 3.6 3 8 3s8-1.3 8-3",
  ],
  arrowRight: ["M5 12h14", "M13 6l6 6-6 6"],
  arrowDown: ["M12 5v14", "M6 13l6 6 6-6"],
  display: ["M4 4h16v12H4z", "M2 20h20"],
  check: ["M20 6L9 17l-5-5"],
};

defineProps<{ name: keyof typeof paths; size?: number }>();
</script>

<template>
  <svg
    class="ow-icon"
    :width="size ?? 16"
    :height="size ?? 16"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.5"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
    focusable="false"
  >
    <path v-for="(d, i) in paths[name]" :key="i" :d="d" />
  </svg>
</template>

<style scoped>
.ow-icon {
  display: inline-block;
  flex: none;
  vertical-align: middle;
}
</style>
