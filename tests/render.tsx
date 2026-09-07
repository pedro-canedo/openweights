import React, { Profiler, useState } from "react";
import { createRoot } from "react-dom/client";
import MessageList, { type UiMessage } from "../src/components/chat/MessageList";
import "../src/i18n";
import "../src/styles.css";

const samples: number[] = [];
const history: UiMessage[] = Array.from({ length: 500 }, (_, i) => ({
  role: i % 2 ? "assistant" : "user", content: `Mensagem ${i}\n\nTexto de referência para uma conversa extensa.`, tokensPerSec: null,
}));
function Fixture() {
  const [messages, setMessages] = useState(history);
  const [active, setActive] = useState(false);
  (window as any).startRenderBench = (batch = 1) => {
    samples.length = 0;
    setActive(true);
    let n = 0;
    let content = "```typescript\n";
    const timer = setInterval(() => {
      content += `const value${n} = ${n};\n`;
      n++;
      if (n % batch === 0 || n === 500) setMessages([...history, { role: "assistant", content, tokensPerSec: null }]);
      if (n === 500) { clearInterval(timer); setActive(false); (window as any).benchDone = true; }
    }, 10);
  };
  (window as any).renderSamples = samples;
  return <div style={{ height: "100vh", display: "flex", flexDirection: "column" }}>
    <input aria-label="Typing probe" />
    <Profiler id="messages" onRender={(_, __, duration) => samples.push(duration)}>
      <MessageList messages={messages} generating={active} loadingModel={false} />
    </Profiler>
  </div>;
}
createRoot(document.getElementById("root")!).render(<Fixture />);
