// Ersteinrichtung: Ordner wählen, Wohnort, dann weiter zur Spracheinrichtung.
import "../../../shared/tokens.css";
import "../../../shared/avatar/panco.css";
import "./settings.css";
import { invoke } from "@tauri-apps/api/core";
import { PancoAvatar } from "../../../shared/avatar/panco";
import { initChrome } from "./chrome";
import { folderForm, type FileSettings } from "./folders";

interface SetupData {
  files: FileSettings;
  city: string;
  done: boolean;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;

(async () => {
  initChrome();
  new PancoAvatar($("avatar"));
  const data = await invoke<SetupData>("setup_state");
  const read = folderForm($("folders"), data.files);
  const city = $<HTMLInputElement>("city");
  city.value = data.city;

  $("save").addEventListener("click", async () => {
    const err = $("error");
    try {
      await invoke("setup_save", { files: read(), city: city.value });
      $("step-folders").hidden = true;
      $("step-done").hidden = false;
      $("step-done").classList.add("is-entering");
    } catch (e) {
      err.hidden = false;
      err.textContent = String(e);
    }
  });
  $("finish").addEventListener("click", () => invoke("setup_finish"));
})();
