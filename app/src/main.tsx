import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
import { api } from "./api";
import "./styles.css";

const root = createRoot(document.getElementById("root")!);
api.then((a) => {
  root.render(
    <StrictMode>
      <App api={a} />
    </StrictMode>,
  );
});
