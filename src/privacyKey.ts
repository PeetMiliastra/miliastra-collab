// Format check only; the app checks the key itself.
export function validPrivacyKey(value: string): boolean {
  const key = value.trim();
  return key.length <= 40000 && /^mcollab-key-v2:[A-Za-z0-9_-]+$/.test(key);
}
