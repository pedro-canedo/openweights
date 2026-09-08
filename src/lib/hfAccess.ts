// Saber se ESTA conta já pode baixar de um repositório — e ficar sabendo
// sozinho quando isso mudar.
//
// O aceite da licença acontece fora do app, no navegador, e o app não tem
// como ser avisado: não há webhook, não há cookie a herdar. O que dá para
// fazer é perguntar de novo. Enquanto a pessoa está na aba do Hugging Face
// decidindo, este hook pergunta a cada poucos segundos; no instante em que o
// portão abre, quem chamou descobre e segue com o download que ficou parado.
//
// A espera tem prazo. Alguns repositórios não liberam na hora — o autor
// aprova a mão, às vezes dias depois — e um app que fica batendo na API para
// sempre é um app que ninguém pediu para fazer isso.

import { useCallback, useEffect, useState } from "react";
import { modelAccess } from "./api";
import type { AccessReport } from "./types";

const INTERVALO_MS = 3000;
const LIMITE_MS = 5 * 60 * 1000;

export function useHfAccess(
  repoId: string,
  probeFile: string | undefined,
  ativo: boolean,
) {
  const [report, setReport] = useState<AccessReport | null>(null);
  const [checando, setChecando] = useState(false);
  const [falhou, setFalhou] = useState(false);
  const [aguardando, setAguardando] = useState(false);
  const [tentativa, setTentativa] = useState(0);

  useEffect(() => {
    if (!ativo) return;
    let cancelado = false;
    setChecando(true);
    setFalhou(false);
    modelAccess(repoId, probeFile)
      .then((r) => {
        if (cancelado) return;
        setReport(r);
        // Abriu: não há mais o que esperar.
        if (r.access === "granted") setAguardando(false);
      })
      .catch(() => !cancelado && setFalhou(true))
      .finally(() => !cancelado && setChecando(false));
    return () => {
      cancelado = true;
    };
  }, [repoId, probeFile, tentativa, ativo]);

  // A espera pelo aceite é a mesma sondagem, repetida — e com hora para
  // acabar.
  useEffect(() => {
    if (!aguardando) return;
    const fim = Date.now() + LIMITE_MS;
    const id = setInterval(() => {
      if (Date.now() > fim) {
        setAguardando(false);
        return;
      }
      setTentativa((n) => n + 1);
    }, INTERVALO_MS);
    return () => clearInterval(id);
  }, [aguardando]);

  // Trocar de modelo cancela uma espera que era sobre outro repositório.
  useEffect(() => setAguardando(false), [repoId]);

  const revalidar = useCallback(() => setTentativa((n) => n + 1), []);
  const aguardarAceite = useCallback(() => setAguardando(true), []);

  const liberado = report?.access === "granted";
  const bloqueado = ativo && report != null && !liberado;

  return {
    report,
    checando,
    falhou,
    aguardando,
    liberado,
    bloqueado,
    revalidar,
    aguardarAceite,
  };
}
