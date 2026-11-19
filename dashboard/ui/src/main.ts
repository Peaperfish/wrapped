import { mount } from "svelte";
import "./app.css";
import App from "./App.svelte";
import { initAppearance } from "./lib/theme";
import { initFrameMeter } from "./lib/frameMeter";

initAppearance();
initFrameMeter();

const target = document.getElementById("app");
if (!target) throw new Error("#app missing");
export default mount(App, { target });
