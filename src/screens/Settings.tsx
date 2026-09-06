// Ajustes: cinco coisas, três naturezas diferentes.
//
// A tela vinha empilhando tema, idioma, token do Hugging Face, motor de IA e
// hardware como se fossem cinco itens equivalentes — e não são. Tema e idioma
// se resolvem em um clique e nunca mais; o token é uma credencial que a
// pessoa cola uma vez; o motor é a peça de que o app inteiro depende, e o
// hardware não é ajuste nenhum, é informação.
//
// A ordem aqui é a da importância: o que pode estar quebrado primeiro (o
// motor), depois o que se conecta (Hugging Face), depois as preferências, e
// por último a máquina — que não se ajusta, se consulta.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { getHardwareProfile, getSetting, setSetting } from "../lib/api";
import { invoke, isTauri } from "../lib/tauri";
import { formatBytes } from "../lib/format";
import { navigate } from "../lib/nav";
import type { HardwareProfile } from "../lib/types";
import { Card, Page, Row, StatusDot } from "../components/ui/Shell";
import { CopyValue } from "../components/ui/Copy";
import EngineCard from "../components/settings/EngineCard";

export default function Settings() {
  const { t, i18n } = useTranslation();
  const [profile, setProfile] = useState<HardwareProfile | null>(null);
  const [paths, setPaths] = useState<{ modelsDir: string } | null>(null);
  const [theme, setTheme] = useState(
    () => localStorage.getItem("theme") ?? "dark",
  );

  useEffect(() => {
    getHardwareProfile().then(setProfile).catch(() => setProfile(null));
    if (isTauri) {
      invoke<{ dataDir: string; modelsDir: string }>("app_paths")
        .then(setPaths)
        .catch(() => {});
    }
  }, []);

  function applyTheme(next: string) {
    setTheme(next);
    localStorage.setItem("theme", next);
    if (next === "light") document.documentElement.dataset.theme = "light";
    else delete document.documentElement.dataset.theme;
  }

  function applyLanguage(lng: string) {
    localStorage.setItem("language", lng);
    i18n.changeLanguage(lng);
  }

  const select =
    "rounded-lg border border-edge bg-panel2 px-3 py-1.5 text-sm outline-none focus:border-accent";

  return (
    <Page title={t("settings.title")} subtitle={t("settings.subtitle")}>
      <EngineCard />

      <HfTokenCard />

      <Card title={t("settings.prefs")} hint={t("settings.prefsHint")}>
        <div className="mt-3 divide-y divide-edge">
          <Row label={t("settings.theme")}>
            <select
              value={theme}
              onChange={(e) => applyTheme(e.target.value)}
              className={select}
            >
              <option value="dark">{t("settings.dark")}</option>
              <option value="light">{t("settings.light")}</option>
            </select>
          </Row>
          <Row label={t("settings.language")}>
            <select
              value={i18n.language}
              onChange={(e) => applyLanguage(e.target.value)}
              className={select}
            >
              <option value="pt-BR">Português (Brasil)</option>
              <option value="en">English</option>
            </select>
          </Row>
        </div>
      </Card>

      <HardwareCard profile={profile} modelsDir={paths?.modelsDir} />
    </Page>
  );
}

/**
 * O token do Hub.
 *
 * O campo sozinho não dizia se havia token gravado — `type="password"` com
 * valor preenchido parece igual a vazio de longe. Agora o estado vem antes
 * do campo, e a troca é explícita.
 */
function HfTokenCard() {
  const { t } = useTranslation();
  const [token, setToken] = useState("");
  const [gravado, setGravado] = useState(false);
  const [editando, setEditando] = useState(false);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    getSetting("hf_token").then((v) => {
      setToken(v ?? "");
      setGravado(!!v);
    });
  }, []);

  async function save() {
    await setSetting("hf_token", token.trim());
    setGravado(!!token.trim());
    setEditando(false);
    setSaved(true);
    setTimeout(() => setSaved(false), 1500);
  }

  return (
    <Card
      title={t("settings.hfToken")}
      hint={t("settings.hfTokenHint")}
      action={
        gravado && !editando ? (
          <button
            type="button"
            onClick={() => setEditando(true)}
            className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:border-accent hover:text-ink"
          >
            {t("settings.hfTokenChange")}
          </button>
        ) : undefined
      }
    >
      <div className="mt-3 flex items-center gap-2.5">
        <StatusDot tone={gravado ? "ok" : "off"} />
        <span className="text-sm">
          {gravado ? t("settings.hfTokenSet") : t("settings.hfTokenNone")}
        </span>
        {saved && <span className="text-[12px] text-ok">✓</span>}
      </div>

      {(!gravado || editando) && (
        <div className="mt-3 flex gap-2">
          <input
            type="password"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder="hf_..."
            className="flex-1 rounded-lg border border-edge bg-panel2 px-3 py-2 text-sm outline-none placeholder:text-dim focus:border-accent"
          />
          <button
            onClick={() => void save()}
            className="rounded-lg bg-accent px-4 py-2 text-sm font-medium text-white"
          >
            {t("common.save")}
          </button>
        </div>
      )}
    </Card>
  );
}

/**
 * A máquina, como o app a enxerga.
 *
 * É o que explica as decisões das outras telas — a variante do motor, a
 * quantização recomendada, quanto contexto cabe. Por isso mostra o que
 * decide (VRAM, driver, AVX), e não uma ficha técnica completa.
 */
function HardwareCard({
  profile,
  modelsDir,
}: {
  profile: HardwareProfile | null;
  modelsDir?: string;
}) {
  const { t } = useTranslation();

  if (!profile) {
    return (
      <Card title={t("settings.hardware")}>
        <p className="mt-3 text-[13px] text-dim">{t("common.loading")}</p>
      </Card>
    );
  }

  return (
    <Card
      title={t("settings.hardware")}
      hint={t("settings.hardwareHint")}
      action={
        <button
          type="button"
          onClick={() => navigate("server", { serverTab: "network" })}
          className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:border-accent hover:text-ink"
        >
          {t("settings.clusterLink")}
        </button>
      }
    >
      <div className="mt-4 grid gap-3 sm:grid-cols-2">
        <Bloco
          titulo={profile.cpuName}
          linhas={[
            t("settings.hw.cores", { n: profile.cpuCores }),
            profile.avx512 ? "AVX-512" : profile.avx2 ? "AVX2" : "SSE",
            t("settings.hw.ram", { size: formatBytes(profile.ramTotalBytes) }),
          ]}
        />
        {profile.gpus.map((g, i) => (
          <Bloco
            key={i}
            titulo={g.name}
            linhas={[
              t("settings.hw.vram", { size: formatBytes(g.vramTotalBytes) }),
              g.driverVersion
                ? t("settings.hw.driver", { v: g.driverVersion })
                : null,
              g.isIntegrated ? t("settings.hw.integrated") : null,
            ]}
          />
        ))}
        {profile.gpus.length === 0 && (
          <Bloco titulo={t("status.noGpu")} linhas={[t("settings.hw.cpuOnly")]} />
        )}
      </div>

      {modelsDir && (
        <div className="mt-4 border-t border-edge pt-3">
          <div className="text-[11px] text-dim">{t("settings.modelsDir")}</div>
          <div className="mt-1">
            <CopyValue value={modelsDir} className="max-w-full" />
          </div>
        </div>
      )}
    </Card>
  );
}

function Bloco({
  titulo,
  linhas,
}: {
  titulo: string;
  linhas: (string | null)[];
}) {
  return (
    <div className="rounded-lg border border-edge bg-panel2/40 p-3">
      <div className="truncate text-sm" title={titulo}>
        {titulo}
      </div>
      <div className="mt-1 flex flex-wrap gap-x-2 gap-y-0.5 text-[11px] text-dim">
        {linhas.filter(Boolean).map((l, i) => (
          <span key={i}>{l}</span>
        ))}
      </div>
    </div>
  );
}
