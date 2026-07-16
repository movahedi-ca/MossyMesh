import { Component, type ErrorInfo, type ReactNode } from "react";

interface Props { children?: ReactNode; }
interface State { hasError: boolean; }

export class ErrorBoundary extends Component<Props, State> {
  public state: State = { hasError: false };
  public static getDerivedStateFromError(_: Error): State { return { hasError: true }; }
  public componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error("Uncaught error:", error, errorInfo);
  }
  public render() {
    if (this.state.hasError) {
      return (
        <div className="glass-panel" style={{ margin: "2rem auto", maxWidth: 480 }}>
          <h1 style={{ fontSize: "1.75rem", WebkitTextFillColor: "#fb2d7f" }}>Fatal React Crash</h1>
          <p style={{ color: "var(--text-muted)" }}>Please reload the Captive Portal.</p>
          <button type="button" className="mesh-btn primary" style={{ marginTop: "1rem" }}
            onClick={() => window.location.reload()}>
            Reload portal
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}