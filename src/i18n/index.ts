import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import ptBR from "./pt-BR.json";
import en from "./en.json";

i18n.use(initReactI18next).init({
  resources: {
    "pt-BR": { translation: ptBR },
    en: { translation: en },
  },
  lng: localStorage.getItem("language") ?? "pt-BR",
  fallbackLng: "en",
  interpolation: { escapeValue: false },
});

// O leitor de tela e a hifenização leem o idioma do <html>, que nasce pt-BR.
document.documentElement.lang = i18n.language;
i18n.on("languageChanged", (lng) => {
  document.documentElement.lang = lng;
});

export default i18n;
