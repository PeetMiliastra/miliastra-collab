import { isDemo, demoCall } from "./demo";
import { invoke } from "@tauri-apps/api/core";
import type { CommandMap } from "./commands.generated";
export const desktop =
  typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
type Args<K extends keyof CommandMap> = CommandMap[K]["args"];
export async function call<K extends keyof CommandMap>(
  command: K,
  ...params: {} extends Args<K> ? [args?: Args<K>] : [args: Args<K>]
): Promise<CommandMap[K]["result"]> {
  const args = params[0];
  if (!desktop && isDemo())
    return demoCall<CommandMap[K]["result"]>(command, args);
  if (!desktop)
    throw new Error(
      "Open the Windows app to connect a project. Browser preview cannot access your files.",
    );
  return invoke<CommandMap[K]["result"]>(command, args);
}
