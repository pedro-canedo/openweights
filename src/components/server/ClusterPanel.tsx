import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  clusterAccept,
  clusterApplyEngine,
  clusterDisconnect,
  clusterForget,
  clusterReject,
  clusterRequestPair,
  clusterSetEnabled,
  engineBusyReason,
  getClusterStatus,
  onCluster,
} from "../../lib/api";
import { formatBytes } from "../../lib/format";
import { navigate } from "../../lib/nav";
import type { ClusterSnapshot } from "../../lib/types";

export default function ClusterPanel() {
  const { t } = useTranslation();
  const [snap, setSnap] = useState<ClusterSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const applied = useRef<string | null>(null);

  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;
    getClusterStatus()
      .then(setSnap)
      .catch(() => {});
    onCluster((s) => {
      if (!cancelled) setSnap(s);
    }).then((f) => {
      if (cancelled) f();
      else un = f;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  // Aplicar o `--rpc` quando o par sobe, e TIRÁ-LO quando o par cai. A queda
  // pelo heartbeat (o outro sumiu) acontece dentro do Rust, sem passar por
  // comando nenhum: sem esta segunda metade o llama-server segue apontando
  // para um endereço morto e o próximo modelo não carrega.
  useEffect(() => {
    const apply = () => {
      clusterApplyEngine()
        .then(setSnap)
        .catch((e) => {
          const who = engineBusyReason(e);
          setError(
            who
              ? t("server.busyToApply", {
                  who: who.map((w) => t(`server.busyWith.${w}`)).join(", "),
                })
              : String(e),
          );
        });
    };
    if (snap?.role === "host" && snap.connected) {
      if (applied.current !== snap.connected.rpcAddr) {
        applied.current = snap.connected.rpcAddr;
        apply();
      }
    } else if (applied.current !== null) {
      applied.current = null;
      apply();
    }
  }, [snap?.role, snap?.connected?.rpcAddr, t]);

  async function run(fn: () => Promise<ClusterSnapshot>) {
    setBusy(true);
    setError(null);
    try {
      const s = await fn();
      // Desligar e esquecer já reiniciam o motor no Rust; sincronizar o ref
      // aqui evita que o efeito acima peça um segundo reinício em seguida.
      applied.current = s.connected?.rpcAddr ?? null;
      setSnap(s);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  if (!snap) return null;

  const canShare = !!snap.deviceId && snap.advertisedBytes > 0;

  return (
    <div className="mt-4 rounded-xl border border-edge bg-panel p-5">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 className="text-sm font-semibold">{t("server.cluster.title")}</h2>
          <p className="mt-1 text-[12px] leading-relaxed text-dim">
            {t("server.cluster.subtitle")}
          </p>
        </div>
        {snap.connected && (
          <button
            type="button"
            disabled={busy}
            onClick={() => void run(clusterDisconnect)}
            className="shrink-0 rounded-lg border border-edge px-3 py-1.5 text-[12px] hover:border-bad hover:text-bad disabled:opacity-50"
          >
            {t("server.cluster.disconnect")}
          </button>
        )}
      </div>

      <p className="mt-3 text-[11px] leading-relaxed text-dim">
        {t("server.cluster.security")}
      </p>

      <label className="mt-3 flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={snap.enabled}
          disabled={busy}
          onChange={(e) => void run(() => clusterSetEnabled(e.target.checked))}
          className="accent-[var(--lr-accent)]"
        />
        {t("server.cluster.enable")}
        {busy && (
          <span className="text-[11px] text-dim">{t("server.cluster.preparing")}</span>
        )}
      </label>
      <p className="mt-1 text-[11px] leading-relaxed text-dim">
        {t("server.cluster.enableHint")}
      </p>

      {error && <p className="mt-2 text-[12px] text-bad">{error}</p>}

      {!snap.enabled ? (
        <p className="mt-4 text-[12px] text-dim">{t("server.cluster.off")}</p>
      ) : (
        <>
      {!snap.rpcReady && (
        <p className="mt-3 text-[12px] text-warn">{t("server.cluster.noRpc")}</p>
      )}
      {snap.warning && (
        <p className="mt-2 text-[12px] text-warn">{snap.warning}</p>
      )}
      {snap.pendingFrom && (
        <div className="mt-4 rounded-lg border border-accent/40 bg-panel2 p-3">
          <p className="text-sm">
            {t("server.cluster.incoming", { name: snap.pendingFrom.hostname })}
          </p>
          <p className="mt-1 text-[12px] text-dim">
            {snap.pendingFrom.gpuName} · {formatBytes(snap.pendingFrom.advertisedBytes)}
          </p>
          <div className="mt-3 flex gap-2">
            <button
              type="button"
              disabled={busy || !canShare || !snap.pendingFrom.tagOk}
              onClick={() => void run(clusterAccept)}
              className="rounded-lg bg-accent px-3 py-1.5 text-[12px] font-medium text-white disabled:opacity-50"
            >
              {t("server.cluster.accept")}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => void run(clusterReject)}
              className="rounded-lg border border-edge px-3 py-1.5 text-[12px]"
            >
              {t("server.cluster.reject")}
            </button>
          </div>
        </div>
      )}

      {snap.connected && (
        <div className="mt-4 rounded-lg border border-ok/40 bg-panel2 p-3">
          <p className="text-sm font-medium text-ok">
            {t("server.cluster.linked", { name: snap.connected.hostname })}
          </p>
          <p className="mt-1 font-mono text-[12px] text-dim">
            {snap.connected.gpuName}
            {snap.connected.devices ? ` · ${snap.connected.devices}` : ""}
            {snap.connected.tensorSplit
              ? ` · ${t("server.cluster.split", { ts: snap.connected.tensorSplit })}`
              : ""}
          </p>
          {snap.role === "host" && (
            <>
              <p className="mt-2 text-[11px] leading-relaxed text-dim">
                {t("server.cluster.measured")}
              </p>
              <p className="mt-1 text-[11px] leading-relaxed text-dim">
                {t("server.cluster.firstLoad")}
              </p>
            </>
          )}
        </div>
      )}

      {snap.deviceId === "MTL0" && (
        <p className="mt-3 text-[11px] leading-relaxed text-dim">
          {t("server.cluster.metalCap")}
          <code className="ml-1 rounded bg-panel px-1 py-0.5 font-mono text-[10px]">
            sudo sysctl iogpu.wired_limit_mb=12288
          </code>
        </p>
      )}

      <p className="mt-4 text-[11px] text-dim">
        {t("server.cluster.youAre", {
          name: snap.hostname,
          gpu: formatBytes(snap.advertisedBytes),
        })}
      </p>

      <ul className="mt-3 flex flex-col gap-2">
        {snap.peers.length === 0 && (
          <li className="text-[12px] text-dim">{t("server.cluster.empty")}</li>
        )}
        {snap.peers.map((p) => (
          <li
            key={p.id}
            className="flex items-center justify-between gap-3 rounded-lg border border-edge bg-panel2 px-3 py-2"
          >
            <div className="min-w-0">
              <div className="truncate text-sm">{p.hostname}</div>
              <div className="text-[11px] text-dim">
                {p.gpuName} · {formatBytes(p.advertisedBytes)}
                {!p.tagOk && (
                  <span className="ml-2 text-warn">
                    {t("server.cluster.tagMismatch")}
                  </span>
                )}
                {p.paired && (
                  <span className="ml-2">{t("server.cluster.paired")}</span>
                )}
              </div>
            </div>
            <div className="flex shrink-0 gap-2">
              {p.tagOk &&
                p.advertisedBytes > 0 &&
                snap.role !== "worker" &&
                !snap.connected && (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void run(() => clusterRequestPair(p.id))}
                  className="rounded-lg bg-accent px-3 py-1.5 text-[12px] font-medium text-white disabled:opacity-50"
                >
                  {t("server.cluster.useExtra")}
                </button>
              )}
              {p.advertisedBytes === 0 && (
                <span className="text-[11px] text-dim">
                  {t("server.cluster.noGpuPeer")}
                </span>
              )}
              {p.paired && (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void run(() => clusterForget(p.id))}
                  className="rounded-lg border border-edge px-3 py-1.5 text-[12px]"
                >
                  {t("server.cluster.forget")}
                </button>
              )}
            </div>
          </li>
        ))}
      </ul>
        </>
      )}
    </div>
  );
}

/** Chip da StatusBar — só aparece com o par ligado. */
export function ClusterChip() {
  const { t } = useTranslation();
  const [snap, setSnap] = useState<ClusterSnapshot | null>(null);

  useEffect(() => {
    let un: (() => void) | undefined;
    let cancelled = false;
    getClusterStatus()
      .then((s) => {
        if (!cancelled) setSnap(s);
      })
      .catch(() => {});
    onCluster((s) => {
      if (!cancelled) setSnap(s);
    }).then((f) => {
      if (cancelled) f();
      else un = f;
    });
    return () => {
      cancelled = true;
      un?.();
    };
  }, []);

  if (snap?.role !== "host" && snap?.role !== "worker") return null;

  return (
    <button
      type="button"
      onClick={() => navigate("server")}
      className="rounded-md border border-edge px-2 py-0.5 text-[11px] text-dim hover:text-ink"
      title={t("server.cluster.title")}
    >
      {snap.role === "worker"
        ? t("server.cluster.chipWorker")
        : t("server.cluster.chipHost", {
            name: snap.connected?.hostname ?? "",
          })}
    </button>
  );
}
