/* @refresh reload */
import { render } from "solid-js/web";
import App from "./App";
import { endUiMeasure, startUiMeasure } from "./utils/performance";

const root = document.getElementById("root");
if (!root) throw new Error("Root element not found");

const renderStart = startUiMeasure("app-render");
render(() => <App />, root);
endUiMeasure("app-render", renderStart);
