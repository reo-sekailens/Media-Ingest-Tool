// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test } from "vitest";
import App from "./App";

test("switches the selected storage fixture", () => {
  render(<App />);
  fireEvent.change(screen.getByLabelText("Destination directory"), {
    target: { value: "D:\\Ingest\\A-Cam" },
  });
  fireEvent.click(screen.getByRole("button", { name: /b-cam/i }));
  expect(
    screen.getByRole("heading", { name: /b-cam.*microsdxc/i }),
  ).toBeInTheDocument();
  expect(
    screen.getByPlaceholderText("E:\\Ingest\\Documentary\\Day 03"),
  ).toBeInTheDocument();
  expect(screen.getByRole("button", { name: /format unavailable/i })).toBeDisabled();
  expect(screen.getByText(/native format provider not installed/i)).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Destination directory"), {
    target: { value: "E:\\Ingest\\B-Cam" },
  });
  fireEvent.click(screen.getByRole("button", { name: /a-cam/i }));
  expect(screen.getByLabelText("Destination directory")).toHaveValue(
    "D:\\Ingest\\A-Cam",
  );
});

test("explains that source scanning needs the desktop runtime in fixture mode", () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: /scan media/i }));
  expect(
    screen.getByText(/media scanning is available in the desktop application/i),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /b-cam/i }));
  expect(
    screen.queryByText(/media scanning is available in the desktop application/i),
  ).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /a-cam/i }));
  expect(
    screen.getByText(/media scanning is available in the desktop application/i),
  ).toBeInTheDocument();
});

test("keeps remembered destinations inside the desktop trust boundary", () => {
  render(<App />);
  fireEvent.change(screen.getByLabelText("Destination directory"), {
    target: { value: "D:\\Ingest\\A-Cam" },
  });
  fireEvent.click(screen.getByRole("button", { name: /remember for this card/i }));
  expect(
    screen.getByText(/destination memory is available in the desktop application/i),
  ).toBeInTheDocument();
});

test("keeps organization previews inside the desktop boundary", () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: /preview organization/i }));
  expect(
    screen.getByText(/organization preview is available in the desktop application/i),
  ).toBeInTheDocument();
});

test("opens the auto-ingest setup modal", () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: /set up auto-ingest/i }));
  expect(screen.getByRole("dialog", { name: /set up auto-ingest/i })).toBeInTheDocument();
  expect(screen.getByLabelText("Auto-ingest destination directory")).toHaveValue(
    "D:\\Ingest\\Documentary\\Day 03",
  );
  expect(screen.getByRole("button", { name: /save setup/i })).toBeEnabled();
  fireEvent.click(screen.getByLabelText(/ingest automatically on mount/i));
  expect(screen.getByLabelText(/format after a verified auto-ingest/i)).not.toBeDisabled();
});

test("keeps typed destination entry as a browser-preview fallback", () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "Choose destination folder" }));
  expect(screen.getByText(/native folder selection is available/i)).toBeInTheDocument();
  fireEvent.change(screen.getByLabelText("Destination directory"), {
    target: { value: "D:\\Ingest\\Manual" },
  });
  expect(screen.getByLabelText("Destination directory")).toHaveValue(
    "D:\\Ingest\\Manual",
  );
});
