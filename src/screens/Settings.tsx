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

import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  getHardwareProfile,
  getSetting,
  hfLogin,
  hfLogout,
  hfWhoami,
  setSetting,
} from "../lib/api";
import { invoke, isTauri } from "../lib/tauri";
import { formatBytes } from "../lib/format";
import { navigate } from "../lib/nav";
import type { HardwareProfile, HfWhoami } from "../lib/types";
import { Card, Page, Row, StatusDot } from "../components/ui/Shell";
import Icon from "../components/ui/Icon";
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
  const [manual, setManual] = useState(false);
  const [saved, setSaved] = useState(false);
  const [entrando, setEntrando] = useState(false);
  const [erro, setErro] = useState<string | null>(null);
  // O que o Hub responde sobre a credencial. `null` é "ainda perguntando":
  // um token gravado não é a mesma coisa que um token que funciona, e a
  // diferença entre os dois só aparece perguntando.
  const [who, setWho] = useState<HfWhoami | null>(null);

  const verificar = useCallback(() => {
    setWho(null);
    getSetting("hf_token").then((v) => setGravado(!!v));
    hfWhoami()
      .then(setWho)
      // Rede fora do ar não é credencial inválida: sem resposta, o cartão
      // volta a dizer só o que sabe — se há ou não token gravado.
      .catch(() => setWho(null));
  }, []);

  useEffect(verificar, [verificar]);

  async function entrar() {
    setEntrando(true);
    setErro(null);
    try {
      setWho(await hfLogin());
      setGravado(true);
      setManual(false);
    } catch (e) {
      setErro(String(e));
    } finally {
      setEntrando(false);
    }
  }

  async function sair() {
    await hfLogout();
    setToken("");
    setGravado(false);
    setWho({ kind: "noToken" });
  }

  async function salvarManual() {
    await setSetting("hf_token", token.trim());
    setManual(false);
    setSaved(true);
    setTimeout(() => setSaved(false), 1500);
    verificar();
  }

  const conta = who?.kind === "ok" ? who : null;
  const invalido = who?.kind === "invalid";
  const tone = conta ? "ok" : invalido ? "bad" : gravado ? "warn" : "off";
  const estado = entrando
    ? t("settings.hfLoginWaiting")
    : conta
      ? t("settings.hfTokenAs", { user: conta.name })
      : invalido
        ? t("settings.hfTokenInvalid")
        : gravado
          ? t("settings.hfTokenSet")
          : t("settings.hfTokenNone");

  return (
    <Card
      title={t("settings.hfToken")}
      hint={t("settings.hfTokenHint")}
      action={
        conta ? (
          <button
            type="button"
            onClick={() => void sair()}
            className="rounded-lg border border-edge px-3 py-1.5 text-[12px] text-dim transition-colors hover:border-accent hover:text-ink"
          >
            {t("settings.hfLogout")}
          </button>
        ) : undefined
      }
    >
      <div className="mt-3 flex items-center gap-2.5">
        <StatusDot tone={tone} pulse={entrando} />
        <span className="text-sm">{estado}</span>
        {gravado && who === null && !entrando && (
          <span className="text-[12px] text-dim">
            {t("settings.hfTokenChecking")}
          </span>
        )}
        {saved && (
          <span className="flex text-ok" role="status">
            <Icon name="check" className="h-3.5 w-3.5" title={t("common.saved")} />
          </span>
        )}
        {gravado && who !== null && !entrando && (
          <button
            type="button"
            onClick={verificar}
            className="text-[12px] text-dim underline-offset-2 transition-colors hover:text-ink hover:underline"
          >
            {t("settings.hfTokenRecheck")}
          </button>
        )}
      </div>

      {/* A armadilha do token fine-grained: a conta está certa, a licença
          está aceita, e o download responde 403 porque o token não alcança
          o conteúdo de repositórios com licença. Nada no Hub diz isso na
          hora do erro — por isso o aviso mora aqui, onde o token é colado.
          Quem entra pela conta não passa por isto: o escopo vem certo. */}
      {conta?.canReadGated === false && (
        <p className="mt-2 text-[12px] text-warn">
          {t("settings.hfTokenScopeWarn")}
        </p>
      )}

      {erro && <p className="mt-2 text-[12px] text-bad">{erro}</p>}

      {!conta && (
        <div className="mt-3 flex flex-wrap items-center gap-3">
          <button
            onClick={() => void entrar()}
            disabled={entrando}
            className={`rounded-lg px-4 py-2 text-sm font-medium transition-opacity ${
              entrando
                ? "cursor-default bg-panel2 text-dim"
                : "bg-accent text-white hover:opacity-90"
            }`}
          >
            {entrando ? t("settings.hfLoginWaiting") : t("settings.hfLogin")}
          </button>
          <button
            type="button"
            onClick={() => setManual((v) => !v)}
            className="text-[12px] text-dim underline-offset-2 transition-colors hover:text-ink hover:underline"
          >
            {t("settings.hfManual")}
          </button>
        </div>
      )}

      {manual && !conta && (
        <div className="mt-3">
          <p className="text-[12px] text-dim">{t("settings.hfManualHint")}</p>
          <div className="mt-2 flex gap-2">
            <input
              type="password"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="hf_..."
              className="flex-1 rounded-lg border border-edge bg-panel2 px-3 py-2 text-sm outline-none placeholder:text-dim focus:border-accent"
            />
            <button
              onClick={() => void salvarManual()}
              className="rounded-lg border border-edge px-4 py-2 text-sm font-medium text-ink transition-colors hover:border-accent"
            >
              {t("common.save")}
            </button>
          </div>
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
