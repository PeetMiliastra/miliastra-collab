import { initializeAppearance } from "./appearance";

const overlay = new URLSearchParams(location.search).has("overlay");
if (overlay) document.documentElement.classList.add("overlay-body");
const stopAppearance = initializeAppearance();
let disposed = false;
let stopWindowAppearance = () => {};
if (!overlay && "__TAURI_INTERNALS__" in window) {
  void import("./nativeAppearance").then(({ connectWindowAppearance }) => {
    if (!disposed) stopWindowAppearance = connectWindowAppearance();
  });
}
if (import.meta.hot)
  import.meta.hot.dispose(() => {
    disposed = true;
    stopWindowAppearance();
    stopAppearance();
  });
