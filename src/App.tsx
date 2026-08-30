import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { onNavigate, type Screen } from "./lib/nav";
import StatusBar from "./components/StatusBar";
import DownloadsPanel from "./components/DownloadsPanel";
import GenerationPanel from "./components/GenerationPanel";
import Onboarding from "./components/Onboarding";
import NavConversations from "./components/NavConversations";
import UpdateBadge from "./components/UpdateBadge";
import { OwWordmark } from "./components/OpenWeightsLogo";
import Discover from "./screens/Discover";
import MyModels from "./screens/MyModels";
import Chat from "./screens/Chat";
import Harness from "./screens/Harness";
import LocalServer from "./screens/LocalServer";
import Providers from "./screens/Providers";
import Settings from "./screens/Settings";

const icons: Record<Screen, string> = {
  discover: "M21 21l-4.35-4.35M17 10a7 7 0 11-14 0 7 7 0 0114 0z",
  models:
    "M20 7l-8-4-8 4m16 0l-8 4m8-4v10l-8 4m0-10L4 7m8 4v10M4 7v10l8 4",
  chat: "M8 12h8m-8-4h8m-9 12l-3 3V6a2 2 0 012-2h12a2 2 0 012 2v10a2 2 0 01-2 2H7z",
  harness: "M4 17l6-6-6-6m8 12h8",
  server:
    "M5 12h14M5 12a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v4a2 2 0 01-2 2M5 12a2 2 0 00-2 2v4a2 2 0 002 2h14a2 2 0 002-2v-4a2 2 0 00-2-2m-2-5h.01M17 16h.01",
  providers:
    "M7 18a4 4 0 01-.44-7.976A6 6 0 0117.66 9.1 3.5 3.5 0 0117 18H7zm5-6v6m0-6l-2.5 2.5M12 12l2.5 2.5",
  settings:
    "M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.065 2.572c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.572 1.065c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.065-2.572c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065zM15 12a3 3 0 11-6 0 3 3 0 016 0z",
};

export default function App() {
  const { t } = useTranslation();
  const [screen, setScreen] = useState<Screen>("discover");

  useEffect(() => onNavigate(setScreen), []);

  const items: Screen[] = [
    "discover",
    "models",
    "chat",
    "harness",
    "server",
    "providers",
    "settings",
  ];

  return (
    <div className="flex h-full flex-col">
      <div className="flex min-h-0 flex-1">
        <nav className="flex w-56 shrink-0 flex-col border-r border-edge bg-panel">
          <div className="px-4 py-4">
            <OwWordmark className="text-[17px]" />
          </div>
          <div className="flex shrink-0 flex-col gap-0.5 px-2">
            {items.map((s) => (
              <button
                key={s}
                onClick={() => setScreen(s)}
                className={`flex items-center gap-3 rounded-lg px-3 py-2 text-left text-sm transition-colors ${
                  screen === s
                    ? "bg-panel2 text-ink"
                    : "text-dim hover:bg-panel2/60 hover:text-ink"
                }`}
              >
                <svg
                  className="h-4.5 w-4.5 shrink-0"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1.8"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  viewBox="0 0 24 24"
                >
                  <path d={icons[s]} />
                </svg>
                {t(`nav.${s}`)}
              </button>
            ))}
          </div>
          <div className="mx-3 mt-3 h-px bg-edge" />
          <NavConversations />
          <UpdateBadge />
        </nav>

        {/* O palco não rola: quem rola é a tela.

            O Chat monta a própria altura (a lista de mensagens rola, o
            compositor fica parado no rodapé). Enquanto o `main` rolava, uma
            coluna alta — a trilha da execução, o explorador de arquivos —
            esticava o conteúdo e levava a interface inteira junto, compositor
            incluso: só a barra lateral ficava no lugar. As demais telas são
            documentos, e rolam por dentro do palco. */}
        <main className="min-w-0 flex-1 overflow-hidden">
          {screen === "chat" ? (
            <Chat />
          ) : screen === "harness" ? (
            /* O harness embutido é uma moldura de altura fixa como o Chat: o
               quadro do agente rola por dentro, não empurra o palco. */
            <Harness />
          ) : (
            <div className="h-full overflow-y-auto">
              {screen === "discover" && <Discover />}
              {screen === "models" && <MyModels />}
              {screen === "server" && <LocalServer />}
              {screen === "providers" && <Providers />}
              {screen === "settings" && <Settings />}
            </div>
          )}
        </main>
      </div>

      <StatusBar />
      <GenerationPanel />
      <DownloadsPanel />
      <Onboarding />
    </div>
  );
}
