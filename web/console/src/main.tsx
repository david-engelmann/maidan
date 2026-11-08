import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./styles.css";
import { startPwa } from "./pwa/install.ts";
import { startTheme } from "./core/theme.ts";

if (import.meta.env.PROD) startPwa();
startTheme();

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
