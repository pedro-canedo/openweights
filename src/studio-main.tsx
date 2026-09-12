import React from "react";
import { createRoot } from "react-dom/client";
import Studio from "./screens/Studio";
import "./i18n";
import "./styles.css";

async function start() {
  const token = new URLSearchParams(location.hash.slice(1)).get("session");
  if (token) {
    history.replaceState(null, "", location.pathname);
    const result = await fetch("/session", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ token }) });
    if (!result.ok) throw new Error("Reabra o endereço informado pelo Studio para conectar.");
  }
  createRoot(document.getElementById("root")!).render(<React.StrictMode><div className="h-full overflow-auto"><Studio /></div></React.StrictMode>);
}
void start().catch(error => { document.getElementById("root")!.textContent = String(error); });
