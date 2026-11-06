import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./styles.css";
import { startPwa } from "./pwa/install.ts";

if (import.meta.env.PROD) startPwa();

const root = document.getElementById("root");
if (!root) {
  console.error("Maidan console: #root is missing; the shell cannot mount.");
} else {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
