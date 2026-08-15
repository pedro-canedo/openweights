// Renderização de Markdown (GFM) para mensagens do assistente.
// O preflight do Tailwind zera os estilos, então mapeamos os elementos
// principais para classes do projeto. Blocos de código usam o CodeBlock.

import { memo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import CodeBlock from "./CodeBlock";

const components: Components = {
  // O CodeBlock já renderiza o próprio <pre>; evita <div> dentro de <pre>.
  pre: ({ children }) => <>{children}</>,
  code: ({ className, children }) => {
    const match = /language-([\w+-]+)/.exec(className ?? "");
    const text = String(children ?? "");
    if (match || text.includes("\n")) {
      return (
        <CodeBlock code={text.replace(/\n$/, "")} lang={match?.[1] ?? "text"} />
      );
    }
    return (
      <code className="rounded bg-panel2 px-1.5 py-0.5 text-[0.9em]">
        {children}
      </code>
    );
  },
  p: ({ children }) => <p className="my-2 leading-relaxed">{children}</p>,
  a: ({ href, children }) => (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="text-accent underline underline-offset-2"
    >
      {children}
    </a>
  ),
  ul: ({ children }) => (
    <ul className="my-2 list-disc space-y-1 pl-6">{children}</ul>
  ),
  ol: ({ children }) => (
    <ol className="my-2 list-decimal space-y-1 pl-6">{children}</ol>
  ),
  li: ({ children }) => <li className="leading-relaxed">{children}</li>,
  h1: ({ children }) => (
    <h1 className="mt-4 mb-2 text-lg font-semibold">{children}</h1>
  ),
  h2: ({ children }) => (
    <h2 className="mt-4 mb-2 text-base font-semibold">{children}</h2>
  ),
  h3: ({ children }) => (
    <h3 className="mt-3 mb-1 text-sm font-semibold">{children}</h3>
  ),
  h4: ({ children }) => (
    <h4 className="mt-3 mb-1 text-sm font-semibold">{children}</h4>
  ),
  blockquote: ({ children }) => (
    <blockquote className="my-2 border-l-2 border-accent/60 pl-3 text-dim">
      {children}
    </blockquote>
  ),
  hr: () => <hr className="my-3 border-edge" />,
  table: ({ children }) => (
    <div className="my-2 overflow-x-auto">
      <table className="min-w-max border-collapse text-[13px]">
        {children}
      </table>
    </div>
  ),
  th: ({ children }) => (
    <th className="border border-edge bg-panel2/60 px-3 py-1.5 text-left font-medium">
      {children}
    </th>
  ),
  td: ({ children }) => (
    <td className="border border-edge px-3 py-1.5">{children}</td>
  ),
};

/** memo: evita re-renderizar mensagens antigas a cada delta do streaming. */
const Markdown = memo(function Markdown({ text }: { text: string }) {
  return (
    <div className="text-sm">
      <ReactMarkdown remarkPlugins={[remarkGfm]} components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
});

export default Markdown;
