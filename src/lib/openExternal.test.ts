import { describe, expect, it } from "vitest";
import { externalLinkOf } from "./openExternal";

/** Um clique de mentira: o suficiente para a regra, sem DOM. */
function clique(
  link: { href: string; target?: string } | null,
  extra: Partial<MouseEvent> = {},
): MouseEvent {
  const caminho = link ? [{ nodeName: "SPAN" }, { nodeName: "A", target: "", ...link }] : [];
  return {
    defaultPrevented: false,
    button: 0,
    metaKey: false,
    altKey: false,
    ctrlKey: false,
    shiftKey: false,
    composedPath: () => caminho,
    ...extra,
  } as unknown as MouseEvent;
}

describe("externalLinkOf", () => {
  it("takes the links that leave the app", () => {
    expect(externalLinkOf(clique({ href: "https://huggingface.co/x", target: "_blank" }))).toBe(
      "https://huggingface.co/x",
    );
    expect(externalLinkOf(clique({ href: "mailto:a@b.c", target: "_blank" }))).toBe("mailto:a@b.c");
    expect(externalLinkOf(clique({ href: "https://a.b/" }, { ctrlKey: true }))).toBe("https://a.b/");
    expect(externalLinkOf(clique({ href: "https://a.b/" }, { shiftKey: true }))).toBe("https://a.b/");
  });

  it("leaves the rest to the webview", () => {
    // Link interno, sem _blank nem modificador: navegação normal da interface.
    expect(externalLinkOf(clique({ href: "https://a.b/" }))).toBeNull();
    expect(externalLinkOf(clique({ href: "file:///etc/passwd", target: "_blank" }))).toBeNull();
    expect(externalLinkOf(clique({ href: "javascript:alert(1)", target: "_blank" }))).toBeNull();
    expect(externalLinkOf(clique({ href: "", target: "_blank" }))).toBeNull();
    expect(externalLinkOf(clique(null))).toBeNull();
    const alvo = { href: "https://a.b/", target: "_blank" };
    expect(externalLinkOf(clique(alvo, { defaultPrevented: true }))).toBeNull();
    expect(externalLinkOf(clique(alvo, { button: 1 }))).toBeNull();
    expect(externalLinkOf(clique(alvo, { metaKey: true }))).toBeNull();
    expect(externalLinkOf(clique(alvo, { altKey: true }))).toBeNull();
  });
});
