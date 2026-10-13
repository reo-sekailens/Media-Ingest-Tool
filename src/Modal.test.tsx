// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { Modal } from "./Modal";

test("refuses Escape and backdrop dismissal while an operation is busy", () => {
  const onDismiss = vi.fn();
  const modal = (dismissible: boolean) => (
    <Modal
      labelledBy="modal-title"
      className="test-modal"
      onDismiss={onDismiss}
      dismissible={dismissible}
      closeOnBackdrop
    >
      <section>
        <h2 id="modal-title">Native operation</h2>
        <button disabled={!dismissible}>Cancel</button>
      </section>
    </Modal>
  );
  const { rerender } = render(modal(false));
  const dialog = screen.getByRole("dialog", { name: "Native operation" });
  fireEvent.keyDown(dialog, { key: "Escape" });
  fireEvent.click(dialog);
  expect(onDismiss).not.toHaveBeenCalled();
  fireEvent.keyDown(dialog, { key: "Tab" });
  expect(dialog).toHaveFocus();

  rerender(modal(true));
  fireEvent.keyDown(dialog, { key: "Escape" });
  expect(onDismiss).toHaveBeenCalledOnce();
});

test("keeps the page isolated when a lower modal unmounts first", () => {
  const background = document.createElement("div");
  background.setAttribute("aria-hidden", "false");
  document.body.append(background);
  const onDismiss = vi.fn();
  const first = render(
    <Modal labelledBy="first-title" className="test-modal" onDismiss={onDismiss}>
      <section>
        <h2 id="first-title">First modal</h2>
        <button>Keep open</button>
      </section>
    </Modal>,
  );
  const second = render(
    <Modal labelledBy="second-title" className="test-modal" onDismiss={onDismiss}>
      <section>
        <h2 id="second-title">Second modal</h2>
        <button>Cancel</button>
      </section>
    </Modal>,
  );
  first.unmount();
  expect(background).toHaveAttribute("inert");
  expect(background).toHaveAttribute("aria-hidden", "true");
  expect(document.body.style.overflow).toBe("hidden");
  expect(screen.getByRole("button", { name: "Cancel" })).toHaveFocus();
  second.unmount();
  expect(background).not.toHaveAttribute("inert");
  expect(background).toHaveAttribute("aria-hidden", "false");
  expect(document.body.style.overflow).toBe("");
  background.remove();
});
