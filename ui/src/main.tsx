import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { Toaster } from "./Toast";
import "./app.css";

const root = document.getElementById("root");
if (!root) throw new Error("index.html is missing #root");

createRoot(root).render(
  <StrictMode>
    <Toaster>
      <App />
    </Toaster>
  </StrictMode>,
);
