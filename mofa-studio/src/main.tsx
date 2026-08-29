import React from "react";
import ReactDOM from "react-dom/client";
import { ConfigProvider, theme } from "antd";
import { HashRouter } from "react-router-dom";
import App from "./App";
import "./styles.css";

// Hash routing suits a desktop app (no server for deep links); antd's dark
// algorithm themes the whole UI.
ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <ConfigProvider
      theme={{
        algorithm: theme.darkAlgorithm,
        token: {
          colorPrimary: "#6d5efc",
          borderRadius: 10,
          colorBgLayout: "#0b0d12",
        },
      }}
    >
      <HashRouter>
        <App />
      </HashRouter>
    </ConfigProvider>
  </React.StrictMode>,
);
