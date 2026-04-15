import { createMMKV } from "react-native-mmkv";

export const storage = createMMKV({ id: "stormscope" });

export function getString(key: string, fallback: string): string {
  return storage.getString(key) ?? fallback;
}

export function setString(key: string, value: string): void {
  storage.set(key, value);
}

export function getBoolean(key: string, fallback: boolean): boolean {
  const val = storage.getBoolean(key);
  return val !== undefined ? val : fallback;
}

export function setBoolean(key: string, value: boolean): void {
  storage.set(key, value);
}
