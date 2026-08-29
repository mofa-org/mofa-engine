import { Component, type ReactNode } from "react";
import { Result, Typography } from "antd";

// Page-level error boundary: a render error would otherwise unmount the whole tree
// and (on the near-black background) look like the screen "going black". Shows the
// error instead, so one bad page doesn't take down the app.
export default class ErrorBoundary extends Component<
  { children: ReactNode },
  { error: Error | null }
> {
  state = { error: null as Error | null };

  static getDerivedStateFromError(error: Error) {
    return { error };
  }

  render() {
    if (this.state.error) {
      return (
        <div style={{ padding: 32 }}>
          <Result
            status="error"
            title="This page hit an error"
            subTitle="The rest of the app is still fine — switch sections or reload."
          />
          <Typography.Paragraph>
            <pre
              style={{
                whiteSpace: "pre-wrap",
                background: "#161a22",
                border: "1px solid #232833",
                borderRadius: 8,
                padding: 12,
                color: "#ff7875",
                overflow: "auto",
              }}
            >
              {String(this.state.error?.stack || this.state.error)}
            </pre>
          </Typography.Paragraph>
        </div>
      );
    }
    return this.props.children;
  }
}
