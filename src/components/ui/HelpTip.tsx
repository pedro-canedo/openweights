// O "?" ao lado de um termo técnico.
//
// Quantização, janela de contexto, VRAM, tok/s, MoE: o app usa esses termos
// porque são os certos, e quem está começando não tem onde perguntar. O "?"
// abre uma explicação curta num popover — clicável e alcançável pelo Tab, ao
// contrário do `title` — e some no Esc ou no clique fora.

import type { ReactNode } from "react";
import { IconButton } from "./Button";
import { Popover, usePopover } from "./Popover";

export function HelpTip({
  label,
  children,
  align = "left",
}: {
  /** A pergunta que o botão responde: "O que é quantização?". */
  label: string;
  children: ReactNode;
  align?: "left" | "right";
}) {
  const p = usePopover();
  return (
    <span className="relative inline-flex align-middle">
      <IconButton icon="help" size="sm" label={label} {...p.triggerProps} />
      <Popover
        {...p.popoverProps}
        label={label}
        className={`absolute top-full mt-1 w-72 p-3 text-[12px] leading-relaxed text-ink ${
          align === "right" ? "right-0" : "left-0"
        }`}
      >
        {children}
      </Popover>
    </span>
  );
}
