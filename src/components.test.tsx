// @vitest-environment jsdom
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { Modal } from "./components";
afterEach(cleanup);
it("retains form focus during rerenders and uses the latest close callback", () => {
  const oldClose = vi.fn(),
    currentClose = vi.fn();
  const view = render(
    <Modal title="Edit" onClose={oldClose}>
      <input aria-label="Draft" />
    </Modal>,
  );
  const field = screen.getByRole("textbox");
  field.focus();
  view.rerender(
    <Modal title="Edit" onClose={currentClose}>
      <input aria-label="Draft" />
    </Modal>,
  );
  expect(document.activeElement).toBe(field);
  fireEvent.keyDown(document, { key: "Escape" });
  expect(oldClose).not.toHaveBeenCalled();
  expect(currentClose).toHaveBeenCalledOnce();
});
it("blocks closing while an operation is active and restores it afterward", () => {
  const close = vi.fn();
  const view = render(
    <Modal title="Working" onClose={close} closeDisabled>
      Wait
    </Modal>,
  );
  fireEvent.keyDown(document, { key: "Escape" });
  fireEvent.click(screen.getByRole("button", { name: "Close" }));
  expect(close).not.toHaveBeenCalled();
  const tab = new KeyboardEvent("keydown", {
    key: "Tab",
    bubbles: true,
    cancelable: true,
  });
  document.dispatchEvent(tab);
  expect(tab.defaultPrevented).toBe(true);
  expect(document.activeElement).toBe(screen.getByRole("dialog"));
  view.rerender(
    <Modal title="Working" onClose={close}>
      Done
    </Modal>,
  );
  fireEvent.keyDown(document, { key: "Escape" });
  expect(close).toHaveBeenCalledOnce();
});
