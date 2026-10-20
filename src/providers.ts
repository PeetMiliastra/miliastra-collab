import catalog from "../src-tauri/src/providers/catalog.json";

export const storageProviders = catalog;
export const defaultCloudProviders = catalog
  .filter((provider) => provider.cloud && provider.enabled)
  .map((provider) => provider.id);

export function providerName(id: string | null | undefined): string {
  return (
    catalog.find((provider) => provider.id === id)?.name ?? id ?? "Storage"
  );
}
export function isCloudProvider(id: string | null | undefined): id is string {
  return catalog.some((provider) => provider.id === id && provider.cloud);
}
