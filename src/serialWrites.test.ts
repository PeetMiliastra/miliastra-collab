import { expect, it } from "vitest";
import { SerialWrites } from "./serialWrites";
it("orders writes, reports failure, and allows later recovery", async () => {
  const queue = new SerialWrites(),
    order: number[] = [];
  let release!: () => void;
  const first = queue.enqueue(async () => {
    await new Promise<void>((resolve) => {
      release = resolve;
    });
    order.push(1);
  });
  const failed = queue.enqueue(async () => {
    order.push(2);
    throw new Error("disk full");
  });
  await Promise.resolve();
  await Promise.resolve();
  expect(order).toEqual([]);
  release();
  await first;
  await expect(failed).rejects.toThrow("disk full");
  await expect(queue.flush()).rejects.toThrow("disk full");
  await queue.enqueue(async () => {
    order.push(3);
  });
  await queue.flush();
  expect(order).toEqual([1, 2, 3]);
});
