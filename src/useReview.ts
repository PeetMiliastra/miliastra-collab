import { useEffect, useState } from "react";
import { friendlyError } from "./domain";
/** Loads a review. A slow response for an older review never replaces a newer one. */
export function useReview<T>(load: () => Promise<T>, enabled = true) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    setData(null);
    setError("");
    if (!enabled) return;
    let active = true;
    void load()
      .then((value) => {
        if (active) setData(value);
      })
      .catch((e) => {
        if (active) setError(friendlyError(e));
      });
    return () => {
      active = false;
    };
  }, [load, enabled]);
  return { data, error };
}
