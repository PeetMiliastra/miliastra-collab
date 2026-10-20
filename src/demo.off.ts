// Replaces demo.ts in production builds (see vite.config.ts).
export const isDemo = () => false;
export const beginDemo = () => {};
export const endDemo = () => {};
export async function demoCall<T>(
  _command: string,
  _args?: unknown,
): Promise<T> {
  throw new Error("The browser preview is only available in development.");
}
