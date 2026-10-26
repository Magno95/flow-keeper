import { mount } from "svelte";
import "./app.css";
import Companion from "./companion/Companion.svelte";
import Panel from "./panel/Panel.svelte";

const view = new URLSearchParams(location.search).get("view");
const target = document.getElementById("app")!;

export default view === "companion" ? mount(Companion, { target }) : mount(Panel, { target });
