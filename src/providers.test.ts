import { expect, it } from "vitest";
import {
  defaultCloudProviders,
  isCloudProvider,
  providerName,
  storageProviders,
} from "./providers";

it("shares unique provider metadata and availability with the native adapters", () => {
  expect(new Set(storageProviders.map((provider) => provider.id)).size).toBe(
    storageProviders.length,
  );
  expect(defaultCloudProviders).toEqual(["onedrive"]);
  expect(providerName("folder")).toBe("Shared folder");
  expect(providerName("google")).toBe("Google Drive");
  expect(isCloudProvider("google")).toBe(true);
  expect(isCloudProvider("folder")).toBe(false);
});
it("does not mislabel unknown or locked storage as a supported provider", () => {
  expect(providerName("future-provider")).toBe("future-provider");
  expect(isCloudProvider("future-provider")).toBe(false);
  expect(isCloudProvider("locked")).toBe(false);
  expect(isCloudProvider(undefined)).toBe(false);
});
