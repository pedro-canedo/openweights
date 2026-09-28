// Um aviso do sistema quando a janela do app está sem foco. Com o app à
// vista, o aviso na própria tela basta — a notificação seria o mesmo recado
// duas vezes.

import { isTauri } from "./tauri";

export async function notificarSemFoco(titulo: string, corpo: string): Promise<void> {
  if (!isTauri || document.hasFocus()) return;
  const n = await import("@tauri-apps/plugin-notification");
  let permitido = await n.isPermissionGranted();
  if (!permitido) permitido = (await n.requestPermission()) === "granted";
  if (permitido) n.sendNotification({ title: titulo, body: corpo });
}
