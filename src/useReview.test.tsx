// @vitest-environment jsdom
import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, it } from "vitest";
import { useReview } from "./useReview";
it("ignores an obsolete inspection and resets on retry", async () => {
  let finish!: (value: string) => void;
  const first = () =>
    new Promise<string>((resolve) => {
      finish = resolve;
    });
  const second = async () => "new slice";
  const { result, rerender } = renderHook(({ load }) => useReview(load), {
    initialProps: { load: first },
  });
  rerender({ load: second });
  await waitFor(() => expect(result.current.data).toBe("new slice"));
  await act(async () => {
    finish("old slice");
  });
  expect(result.current.data).toBe("new slice");
  rerender({
    load: async () => {
      throw new Error("read failed");
    },
  });
  await waitFor(() => expect(result.current.error).toBe("read failed"));
  expect(result.current.data).toBeNull();
  rerender({ load: second });
  await waitFor(() => expect(result.current.data).toBe("new slice"));
  expect(result.current.error).toBe("");
});
it("waits until a review is opened", async () => {
  let reads = 0;
  const load = async () => {
    reads++;
    return "assets";
  };
  const { result, rerender } = renderHook(
    ({ enabled }) => useReview(load, enabled),
    { initialProps: { enabled: false } },
  );
  expect(reads).toBe(0);
  rerender({ enabled: true });
  await waitFor(() => expect(result.current.data).toBe("assets"));
  expect(reads).toBe(1);
  rerender({ enabled: false });
  expect(result.current.data).toBeNull();
});
