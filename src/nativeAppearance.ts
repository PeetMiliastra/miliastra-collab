import { getCurrentWindow } from "@tauri-apps/api/window";
import { getAppearance, subscribeAppearance } from "./appearance";

export function connectWindowAppearance(): () => void {
  let active = true;
  let pending = Promise.resolve();
  const update = () => {
    pending = pending
      .then(async () => {
        if (!active) return;
        const preference = getAppearance();
        await getCurrentWindow().setTheme(
          preference === "system" ? null : preference,
        );
      })
      .catch((error: unknown) => {
        console.warn("Could not update the window appearance", error);
      });
  };
  const unsubscribe = subscribeAppearance(update);
  update();
  return () => {
    active = false;
    unsubscribe();
  };
}
