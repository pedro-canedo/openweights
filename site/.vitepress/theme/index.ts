import DefaultTheme from "vitepress/theme";
import OwLanding from "./components/OwLanding.vue";
import OwIcon from "./components/OwIcon.vue";
import "./styles/tokens.css";
import "./styles/base.css";
import "./styles/landing.css";

export default {
  extends: DefaultTheme,
  enhanceApp({ app }: { app: import("vue").App }) {
    app.component("OwLanding", OwLanding);
    app.component("OwIcon", OwIcon);
  },
};
