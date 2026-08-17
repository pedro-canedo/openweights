import { Component, type ErrorInfo, type ReactNode } from "react";
import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./i18n";
import "./styles.css";

const theme = localStorage.getItem("theme");
if (theme === "light") {
  document.documentElement.dataset.theme = "light";
}

class ErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(error, info.componentStack);
  }

  render() {
    if (!this.state.error) return this.props.children;
    return (
      <div className="flex h-full flex-col items-center justify-center gap-3 p-8 text-ink">
        <p className="text-sm font-medium">A interface falhou ao renderizar.</p>
        <pre className="max-w-xl overflow-auto rounded-xl border border-edge bg-panel p-4 text-xs text-bad whitespace-pre-wrap">
          {this.state.error.stack ?? this.state.error.message}
        </pre>
      </div>
    );
  }
}

window.addEventListener("error", (e) => {
  console.error(e.error ?? e.message);
});
window.addEventListener("unhandledrejection", (e) => {
  console.error(e.reason);
});

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
