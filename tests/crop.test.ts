import { test, expect } from "bun:test";
import { adjustCrop, centeredCrop } from "../src/CropEditor";

test("crop selections stay inside the source when moved, resized or ratio constrained", () => {
  for (const size of [
    { width: 1600, height: 1000 },
    { width: 17, height: 400 },
    { width: 1, height: 1 },
  ]) {
    for (const ratio of [0, 1, 4 / 3, 16 / 9, size.width / size.height]) {
      const initial = centeredCrop(size, ratio);
      for (const handle of ["move", "nw", "ne", "sw", "se"] as const) {
        for (const delta of [-9999, -21, 0, 34, 9999]) {
          const rect = adjustCrop(initial, delta, -delta, handle, size, ratio);
          expect(rect.x).toBeGreaterThanOrEqual(0);
          expect(rect.y).toBeGreaterThanOrEqual(0);
          expect(rect.width).toBeGreaterThanOrEqual(1);
          expect(rect.height).toBeGreaterThanOrEqual(1);
          expect(rect.x + rect.width).toBeLessThanOrEqual(size.width);
          expect(rect.y + rect.height).toBeLessThanOrEqual(size.height);
          if (ratio && handle !== "move")
            expect(
              Math.abs(rect.width - rect.height * ratio),
            ).toBeLessThanOrEqual(Math.max(1, ratio));
        }
      }
    }
  }
  expect(
    adjustCrop({ x: 10, y: 20, width: 100, height: 80 }, 5, -10, "move", {
      width: 500,
      height: 400,
    }),
  ).toEqual({ x: 15, y: 10, width: 100, height: 80 });
});
