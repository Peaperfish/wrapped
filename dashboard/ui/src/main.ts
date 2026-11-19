import { mount } from "svelte";
import { invoke } from "@tauri-apps/api/core";
import "./app.css";
import App from "./App.svelte";
import { initAppearance } from "./lib/theme";
import { initFrameMeter } from "./lib/frameMeter";

initAppearance();
void invoke<boolean>("debug_enabled").then((debug) => {
  if (debug) initFrameMeter();
}, () => {
  // The dashboard can run without a native backend. Diagnostics stay off.
});

const target = document.getElementById("app");
if (!target) throw new Error("#app missing");
export default mount(App, { target });
