import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import App from "./App";
import "./join.css";
import "./orchestra.css";

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
