import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  clusterAccept,
  clusterApplyEngine,
  clusterDisconnect,
  clusterForget,
  clusterReject,
  clusterRequestPair,
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

  useEffect(() => {
    if (
      snap?.role === "host" &&
      snap.connected &&
      applied.current !== snap.connected.rpcAddr
    ) {
      applied.current = snap.connected.rpcAddr;
      clusterApplyEngine().then(setSnap).catch((e) => {
        const who = engineBusyReason(e);
        setError(
          who
            ? t("server.busyToApply", {
                who: who.map((w) => t(`server.busyWith.${w}`)).join(", "),
              })
            : String(e),
        );
      });
    }
  }, [snap?.role, snap?.connected?.rpcAddr, t]);

  async function run(fn: () => Promise<ClusterSnapshot>) {
    setBusy(true);
    setError(null);
    try {
      setSnap(await fn());
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

      {!snap.rpcReady && (
        <p className="mt-3 text-[12px] text-warn">{t("server.cluster.noRpc")}</p>
      )}
      {snap.warning && (
        <p className="mt-2 text-[12px] text-warn">{snap.warning}</p>
      )}
      {error && <p className="mt-2 text-[12px] text-bad">{error}</p>}

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
              disabled={busy || !canShare}
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
            <p className="mt-2 text-[11px] leading-relaxed text-dim">
              {t("server.cluster.firstLoad")}
            </p>
          )}
        </div>
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
              {p.tagOk && snap.role !== "worker" && !snap.connected && (
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void run(() => clusterRequestPair(p.id))}
                  className="rounded-lg bg-accent px-3 py-1.5 text-[12px] font-medium text-white disabled:opacity-50"
                >
                  {t("server.cluster.useExtra")}
                </button>
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
    </div>
  );
}

/** Chip da StatusBar — só aparece com o par ligado. */
export function ClusterChip() {
  const { t } = useTranslation();
  const [snap, setSnap] = useState<ClusterSnapshot | null>(null);

  useEffect(() => {
    let un: (() => void) | undefined;
    getClusterStatus()
      .then(setSnap)
      .catch(() => {});
    onCluster(setSnap).then((f) => {
      un = f;
    });
    return () => un?.();
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
