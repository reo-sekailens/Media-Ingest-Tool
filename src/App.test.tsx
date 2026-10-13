// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, expect, test, vi } from "vitest";
import App from "./App";

const tauri = vi.hoisted(() => ({ invoke: vi.fn() }));
const events = vi.hoisted(() => ({
  handlers: new Map<string, () => void>(),
}));

vi.mock("@tauri-apps/api/core", () => ({
  Channel: class {
    onmessage: unknown;
  },
  invoke: tauri.invoke,
  isTauri: () => "__TAURI_INTERNALS__" in window,
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (event: string, handler: () => void) => {
    events.handlers.set(event, handler);
    return () => {
      events.handlers.delete(event);
    };
  }),
}));

afterEach(() => {
  delete (window as Window & { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
  tauri.invoke.mockReset();
  events.handlers.clear();
});

async function openConfigurationDialog() {
  fireEvent.click(
    screen.getByRole("button", { name: /^(configure ingest|edit configuration)$/i }),
  );
  return await screen.findByRole("dialog", { name: "Configure card" });
}
function advance() {
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
}
async function saveSession(destination: string) {
  const dialog = await openConfigurationDialog();
  fireEvent.change(within(dialog).getByLabelText("Destination directory"), {
    target: { value: destination },
  });
  advance();
  advance();
  const remember = within(dialog).getByLabelText("Remember configuration on this card");
  if ((remember as HTMLInputElement).checked) fireEvent.click(remember);
  advance();
  fireEvent.click(within(dialog).getByRole("button", { name: "Save configuration" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
}
function nativeSource(id = "hardware:config-a", generation = 1, mount = "F:\\") {
  return {
    state: "available",
    connectionGeneration: generation,
    identity: { mediaKey: id, confidence: "hardware_immutable", evidence: [] },
    details: {
      displayName: id,
      filesystem: "exFAT",
      totalBytes: 64000000000,
      availableBytes: 32000000000,
      mountLocations: [mount],
      readerFingerprint: null,
      readerFamily: null,
      readerSlot: null,
    },
  };
}
function unregisteredProfile() {
  return {
    registered: false,
    autoIngestEnabled: false,
    autoFormatEnabled: false,
    destinationPath: null,
    sortMode: null,
    intervalMinutes: null,
    customDirectoryFields: [],
    destinationDepthOrder: [],
    markerStatus: "unavailable",
  };
}
function nativeConfigurationRuntime() {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
  const state = {
    snapshot: { sequence: 1, devices: [nativeSource()] },
    profile: unregisteredProfile() as Record<string, unknown>,
  };
  const handler = (command: string) => {
    if (command === "get_device_snapshot")
      return Promise.resolve({ data: state.snapshot });
    if (command === "get_ingest_history") return Promise.resolve({ data: [] });
    if (command === "get_auto_ingest_profile")
      return Promise.resolve({ data: state.profile });
    if (command === "watch_device_snapshots") return Promise.resolve({ data: {} });
    return Promise.resolve({ data: null, error: null });
  };
  tauri.invoke.mockImplementation(handler);
  return { state, handler };
}

test("replaces inline editors with a saved summary and a configuration dialog", async () => {
  render(<App />);
  expect(screen.queryByLabelText("Destination directory")).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "EXIF day" })).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Start Verified Ingest" })).toBeDisabled();
  expect(screen.getByRole("button", { name: "Scan Media" })).toBeVisible();
  expect(screen.getByRole("link", { name: "Skip to workspace" })).toHaveAttribute(
    "href",
    "#ingest-workspace",
  );
  const dialog = await openConfigurationDialog();
  expect(within(dialog).getByRole("heading", { name: "Destination" })).toBeVisible();
  expect(within(dialog).getByLabelText("Destination directory")).toHaveValue("");
  expect(
    within(dialog).getByRole("list", { name: "Configuration steps" }),
  ).toBeVisible();
});

test("saving applies configuration while cancelling later edits leaves it unchanged", async () => {
  render(<App />);
  await saveSession("D:\\Ingest\\A-Cam");
  expect(screen.getByText("D:\\Ingest\\A-Cam")).toBeVisible();
  expect(screen.getByRole("button", { name: "Start Verified Ingest" })).toBeEnabled();
  expect(
    screen.getByText(/preview configuration saved for this session/i),
  ).toBeVisible();
  const dialog = await openConfigurationDialog();
  fireEvent.change(within(dialog).getByLabelText("Destination directory"), {
    target: { value: "D:\\Discarded" },
  });
  fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(screen.getByText("D:\\Ingest\\A-Cam")).toBeVisible();
  expect(screen.queryByText("D:\\Discarded")).not.toBeInTheDocument();
  const reopened = await openConfigurationDialog();
  expect(within(reopened).getByLabelText("Destination directory")).toHaveValue(
    "D:\\Ingest\\A-Cam",
  );
  expect(tauri.invoke).not.toHaveBeenCalled();
});

test("draft cancellation restores trigger focus and removes modal background isolation", async () => {
  const { container } = render(<App />);
  const trigger = screen.getByRole("button", { name: "Configure ingest" });
  trigger.focus();
  const dialog = await openConfigurationDialog();
  expect(container).toHaveAttribute("inert");
  expect(container).toHaveAttribute("aria-hidden", "true");
  expect(document.body.style.overflow).toBe("hidden");
  fireEvent.keyDown(within(dialog).getByRole("button", { name: "Cancel" }), {
    key: "Escape",
  });
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(container).not.toHaveAttribute("inert");
  expect(container).not.toHaveAttribute("aria-hidden");
  expect(trigger).toHaveFocus();
});

test("configuration remains isolated between cards and fresh connections", async () => {
  const { state } = nativeConfigurationRuntime();
  state.snapshot.devices.push(nativeSource("hardware:config-b", 1, "M:\\"));
  render(<App />);
  await screen.findByRole("heading", { name: "hardware:config-a" });
  await saveSession("D:\\Ingest\\A");
  fireEvent.click(screen.getByRole("button", { name: /hardware:config-b.*64/i }));
  expect(screen.queryByText("D:\\Ingest\\A")).not.toBeInTheDocument();
  await saveSession("D:\\Ingest\\B");
  fireEvent.click(screen.getByRole("button", { name: /hardware:config-a.*64/i }));
  expect(screen.getByText("D:\\Ingest\\A")).toBeVisible();
  expect(screen.queryByText("D:\\Ingest\\B")).not.toBeInTheDocument();
  state.snapshot = { sequence: 2, devices: [nativeSource("hardware:config-a", 2)] };
  await act(async () => events.handlers.get("media-ingest://reactivated")?.());
  await waitFor(() =>
    expect(screen.queryByText("D:\\Ingest\\A")).not.toBeInTheDocument(),
  );
  const dialog = await openConfigurationDialog();
  expect(within(dialog).getByLabelText("Destination directory")).toHaveValue("");
});

test("loads saved native organization and automation before editing", async () => {
  const { state } = nativeConfigurationRuntime();
  state.profile = {
    registered: true,
    destinationPath: "D:\\Saved",
    sortMode: "camera_interval",
    intervalMinutes: 37,
    customDirectoryFields: [{ label: "", value: "Ari" }],
    destinationDepthOrder: [
      { kind: "capture_day" },
      { kind: "custom_field", index: 0 },
      { kind: "camera_model" },
      { kind: "capture_interval" },
    ],
    autoIngestEnabled: false,
    autoFormatEnabled: false,
    markerStatus: "recognized",
  };
  render(<App />);
  await screen.findByText("D:\\Saved");
  const dialog = await openConfigurationDialog();
  expect(within(dialog).getByLabelText("Destination directory")).toHaveValue(
    "D:\\Saved",
  );
  advance();
  expect(within(dialog).getByLabelText("Custom interval minutes")).toHaveValue(37);
  expect(within(dialog).getByLabelText("Custom folder 1 value")).toHaveValue("Ari");
  advance();
  expect(
    within(dialog).getByLabelText("Remember configuration on this card"),
  ).toBeChecked();
  expect(
    within(dialog).getByLabelText("Ingest automatically on mount"),
  ).not.toBeChecked();
  expect(
    tauri.invoke.mock.calls.some(([command]) => command === "register_card_marker"),
  ).toBe(false);
});

test("native save errors retain the reviewed draft and retry sends the complete settings", async () => {
  const { state, handler } = nativeConfigurationRuntime();
  let attempts = 0;
  tauri.invoke.mockImplementation(
    (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "register_card_marker") {
        attempts += 1;
        if (attempts === 1)
          return Promise.resolve({
            data: null,
            error: { message: "Destination is offline." },
          });
        state.profile = {
          ...args?.request,
          registered: true,
          markerStatus: "recognized",
        };
        return Promise.resolve({ data: state.profile, error: null });
      }
      return handler(command);
    },
  );
  render(<App />);
  await screen.findByRole("heading", { name: "hardware:config-a" });
  const dialog = await openConfigurationDialog();
  fireEvent.change(within(dialog).getByLabelText("Destination directory"), {
    target: { value: "D:\\Ingest\\Configured" },
  });
  advance();
  fireEvent.click(within(dialog).getByRole("button", { name: "Custom interval" }));
  fireEvent.change(within(dialog).getByLabelText("Custom interval minutes"), {
    target: { value: "45" },
  });
  fireEvent.click(within(dialog).getByRole("button", { name: "Add folder" }));
  fireEvent.change(within(dialog).getByLabelText("Custom folder 1 value"), {
    target: { value: "Ari" },
  });
  advance();
  fireEvent.click(within(dialog).getByLabelText("Ingest automatically on mount"));
  advance();
  fireEvent.click(within(dialog).getByRole("button", { name: "Save configuration" }));
  expect(await within(dialog).findByText("Destination is offline.")).toBeVisible();
  expect(
    within(dialog).getByRole("heading", { name: "Review configuration" }),
  ).toBeVisible();
  fireEvent.click(within(dialog).getByRole("button", { name: "Save configuration" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(tauri.invoke).toHaveBeenLastCalledWith("get_device_snapshot");
  expect(tauri.invoke).toHaveBeenCalledWith("register_card_marker", {
    request: {
      sourceMediumKey: "hardware:config-a",
      destinationPath: "D:\\Ingest\\Configured",
      sortMode: "camera_interval",
      intervalMinutes: 45,
      customDirectoryFields: [{ label: "", value: "Ari" }],
      destinationDepthOrder: [
        { kind: "custom_field", index: 0 },
        { kind: "camera_model" },
        { kind: "capture_day" },
        { kind: "capture_interval" },
      ],
      autoIngestEnabled: true,
      autoFormatEnabled: false,
    },
  });
  expect(screen.getByText("D:\\Ingest\\Configured")).toBeVisible();
  expect(
    tauri.invoke.mock.calls.some(([command]) => command === "start_verified_ingest"),
  ).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "Start Verified Ingest" }));
  await waitFor(() =>
    expect(tauri.invoke).toHaveBeenCalledWith(
      "start_verified_ingest",
      expect.objectContaining({
        request: expect.objectContaining({
          sourceMediumKey: "hardware:config-a",
          sourceGeneration: 1,
          destinationRoot: "D:\\Ingest\\Configured",
          sortMode: "camera_interval",
          intervalMinutes: 45,
          autoIngestTriggered: false,
          customDirectoryFields: [{ label: "", value: "Ari" }],
          destinationDepthOrder: [
            { kind: "custom_field", index: 0 },
            { kind: "camera_model" },
            { kind: "capture_day" },
            { kind: "capture_interval" },
          ],
        }),
      }),
    ),
  );
});

test("saved EXIF-hour settings reach native preview and manual copy as sixty-minute intervals", async () => {
  const { state, handler } = nativeConfigurationRuntime();
  const samplePath = "FX3__fixture/20260823T000000+0800/1300-1400+0800/PHOTO.JPG";
  tauri.invoke.mockImplementation(
    (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "register_card_marker") {
        state.profile = {
          ...args?.request,
          registered: true,
          markerStatus: "recognized",
        };
        return Promise.resolve({ data: state.profile, error: null });
      }
      if (command === "preview_verified_ingest") {
        return Promise.resolve({
          data: {
            operationId: args?.request?.operationId,
            fileCount: 1,
            totalBytes: 512,
            sampleDestinationPaths: [samplePath],
          },
          error: null,
        });
      }
      if (command === "start_verified_ingest")
        return Promise.resolve({
          data: null,
          error: { message: "Copy fixture intercepted." },
        });
      return handler(command);
    },
  );
  render(<App />);
  await screen.findByRole("heading", { name: "hardware:config-a" });
  const dialog = await openConfigurationDialog();
  fireEvent.change(within(dialog).getByLabelText("Destination directory"), {
    target: { value: "D:\\Ingest\\Hourly" },
  });
  advance();
  fireEvent.click(within(dialog).getByRole("button", { name: "EXIF hour" }));
  advance();
  advance();
  fireEvent.click(within(dialog).getByRole("button", { name: "Save configuration" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  const settings = {
    sortMode: "camera_interval",
    intervalMinutes: 60,
    customDirectoryFields: [],
    destinationDepthOrder: [
      { kind: "camera_model" },
      { kind: "capture_day" },
      { kind: "capture_interval" },
    ],
  };
  expect(tauri.invoke).toHaveBeenCalledWith("register_card_marker", {
    request: {
      sourceMediumKey: "hardware:config-a",
      destinationPath: "D:\\Ingest\\Hourly",
      ...settings,
      autoIngestEnabled: false,
      autoFormatEnabled: false,
    },
  });
  expect(
    tauri.invoke.mock.calls.some(([command]) => command === "start_verified_ingest"),
  ).toBe(false);
  fireEvent.click(screen.getByRole("button", { name: "Preview Organization" }));
  expect(await screen.findByText(samplePath)).toBeVisible();
  const previewRequest = tauri.invoke.mock.calls.find(
    ([command]) => command === "preview_verified_ingest",
  )?.[1].request;
  expect(previewRequest).toMatchObject({
    ...settings,
    destinationRoot: "D:\\Ingest\\Hourly",
    sourceMediumKey: "hardware:config-a",
    sourceGeneration: 1,
    autoIngestTriggered: false,
  });
  fireEvent.click(screen.getByRole("button", { name: "Start Verified Ingest" }));
  await waitFor(() =>
    expect(tauri.invoke).toHaveBeenCalledWith(
      "start_verified_ingest",
      expect.objectContaining({
        request: expect.objectContaining({ ...previewRequest }),
      }),
    ),
  );
});

test("saving automation applies to future mounts without starting a current-mount copy", async () => {
  const { state, handler } = nativeConfigurationRuntime();
  tauri.invoke.mockImplementation(
    (command: string, args?: { request?: Record<string, unknown> }) => {
      if (command === "register_card_marker") {
        state.profile = {
          ...args?.request,
          registered: true,
          markerStatus: "recognized",
        };
        return Promise.resolve({ data: state.profile });
      }
      return handler(command);
    },
  );
  render(<App />);
  await screen.findByRole("heading", { name: "hardware:config-a" });
  const dialog = await openConfigurationDialog();
  fireEvent.change(within(dialog).getByLabelText("Destination directory"), {
    target: { value: "D:\\Future" },
  });
  advance();
  advance();
  fireEvent.click(within(dialog).getByLabelText("Ingest automatically on mount"));
  advance();
  fireEvent.click(within(dialog).getByRole("button", { name: "Save configuration" }));
  await waitFor(() => expect(screen.queryByRole("dialog")).not.toBeInTheDocument());
  expect(
    tauri.invoke.mock.calls.some(([command]) => command === "start_verified_ingest"),
  ).toBe(false);
  state.snapshot = { sequence: 2, devices: [nativeSource("hardware:config-a", 2)] };
  await act(async () => events.handlers.get("media-ingest://reactivated")?.());
  await waitFor(() =>
    expect(tauri.invoke).toHaveBeenCalledWith(
      "start_verified_ingest",
      expect.objectContaining({
        request: expect.objectContaining({
          sourceGeneration: 2,
          destinationRoot: "D:\\Future",
          autoIngestTriggered: true,
        }),
      }),
    ),
  );
  await act(async () => events.handlers.get("media-ingest://reactivated")?.());
  expect(
    tauri.invoke.mock.calls.filter(([command]) => command === "start_verified_ingest"),
  ).toHaveLength(1);
});

test("a source change during an acknowledged save cannot apply its draft to another card", async () => {
  const { state, handler } = nativeConfigurationRuntime();
  let acknowledge: (() => void) | undefined;
  tauri.invoke.mockImplementation((command: string) =>
    command === "register_card_marker"
      ? new Promise((resolve) => {
          acknowledge = () => resolve({ data: { registered: true } });
        })
      : handler(command),
  );
  render(<App />);
  await screen.findByRole("heading", { name: "hardware:config-a" });
  const dialog = await openConfigurationDialog();
  fireEvent.change(within(dialog).getByLabelText("Destination directory"), {
    target: { value: "D:\\Card-A" },
  });
  advance();
  advance();
  advance();
  fireEvent.click(within(dialog).getByRole("button", { name: "Save configuration" }));
  expect(
    within(dialog).getByRole("button", { name: "Saving configuration…" }),
  ).toBeDisabled();
  fireEvent.keyDown(dialog, { key: "Escape" });
  expect(dialog).toBeInTheDocument();
  state.snapshot = { sequence: 2, devices: [nativeSource("hardware:config-b", 2)] };
  await act(async () => events.handlers.get("media-ingest://reactivated")?.());
  await act(async () => acknowledge?.());
  expect(
    await within(dialog).findByText(
      /save was confirmed, but this card disconnected or changed/i,
    ),
  ).toBeVisible();
  expect(within(dialog).getByText("D:\\Card-A")).toBeVisible();
  fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(screen.getByRole("heading", { name: "hardware:config-b" })).toBeVisible();
  expect(screen.queryByText("D:\\Card-A")).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Start Verified Ingest" })).toBeDisabled();
});

test("an unavailable native profile cannot be overwritten with an empty default", async () => {
  const { handler } = nativeConfigurationRuntime();
  let unavailable = true;
  tauri.invoke.mockImplementation((command: string) =>
    command === "get_auto_ingest_profile" && unavailable
      ? Promise.resolve({
          data: null,
          error: { message: "Saved configuration is unavailable." },
        })
      : handler(command),
  );
  render(<App />);
  await screen.findByRole("heading", { name: "hardware:config-a" });
  fireEvent.click(screen.getByRole("button", { name: "Configure ingest" }));
  expect(await screen.findByText("Saved configuration is unavailable.")).toBeVisible();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  unavailable = false;
  const dialog = await openConfigurationDialog();
  expect(within(dialog).getByLabelText("Destination directory")).toHaveValue("");
});

test("a removed source keeps its draft visible and cannot save to the next selected card", async () => {
  const { state } = nativeConfigurationRuntime();
  render(<App />);
  await screen.findByRole("heading", { name: "hardware:config-a" });
  const dialog = await openConfigurationDialog();
  fireEvent.change(within(dialog).getByLabelText("Destination directory"), {
    target: { value: "D:\\Draft" },
  });
  advance();
  advance();
  advance();
  state.snapshot = { sequence: 2, devices: [] };
  await act(async () => events.handlers.get("media-ingest://reactivated")?.());
  await waitFor(() =>
    expect(
      within(dialog).getByText(/no longer available for configuration/i),
    ).toBeVisible(),
  );
  expect(
    within(dialog).getByRole("button", { name: "Save configuration" }),
  ).toBeDisabled();
  expect(within(dialog).getByText("D:\\Draft")).toBeVisible();
  fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(
    tauri.invoke.mock.calls.some(([command]) => command === "register_card_marker"),
  ).toBe(false);
});

test("still exposes source scanning, native preview boundaries and useful identity details", () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: "Scan Media" }));
  expect(
    screen.getByText(/media scanning is available in the desktop application/i),
  ).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Preview Organization" }));
  expect(
    screen.getByText(/organization preview is available in the desktop application/i),
  ).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "View Details" }));
  expect(screen.getByText("Current mount · mutable")).toBeVisible();
  expect(screen.getByRole("button", { name: "Open Receipt" })).toBeDisabled();
});

test("confirms an eligible quick format with an opaque native token", async () => {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
  let formatEligibilityChecks = 0;
  const snapshot = {
    sequence: 1,
    devices: [
      {
        state: "available",
        connectionGeneration: 4,
        identity: {
          mediaKey: "hardware:format-fixture",
          confidence: "hardware_immutable",
          evidence: [],
        },
        details: {
          displayName: "Sacrificial SDXC",
          filesystem: "exFAT",
          totalBytes: 64_000_000_000,
          availableBytes: 32_000_000_000,
          mountLocations: ["F:\\"],
          readerFingerprint: null,
          readerFamily: null,
          readerSlot: null,
        },
      },
    ],
  };
  tauri.invoke.mockImplementation((command: string) => {
    if (command === "get_device_snapshot") return Promise.resolve({ data: snapshot });
    if (command === "get_ingest_history") return Promise.resolve({ data: [] });
    if (command === "watch_device_snapshots") return Promise.resolve({ data: {} });
    if (command === "get_auto_ingest_profile") {
      return Promise.resolve({
        data: {
          registered: false,
          autoIngestEnabled: false,
          autoFormatEnabled: false,
          destinationPath: null,
          sortMode: null,
          intervalMinutes: null,
          markerStatus: "unavailable",
        },
      });
    }
    if (command === "start_verified_ingest") {
      return Promise.resolve({
        data: {
          operationId: "sealed-format-run",
          copiedFiles: 5,
          copiedBytes: 100,
          receiptName: "sealed-format-run.json",
          sourceMarkerStatus: "created",
          autoFormatStatus: "not_configured",
        },
      });
    }
    if (command === "get_format_eligibility") {
      formatEligibilityChecks += 1;
      return Promise.resolve({
        data: {
          eligible: formatEligibilityChecks > 1,
          reason:
            formatEligibilityChecks > 1
              ? "Eligible for the allowlisted generic profile."
              : "A stale webview eligibility result must not block a fresh check.",
          recommendedProfile:
            formatEligibilityChecks > 1
              ? {
                  id: "sdxc-default",
                  filesystem: "exfat",
                  inferredFromCapacity: true,
                }
              : null,
        },
      });
    }
    if (command === "request_format_authorization") {
      return Promise.resolve({
        data: { confirmationToken: "opaque-format-token", expiresInSeconds: 60 },
      });
    }
    if (command === "request_force_format_authorization") {
      return Promise.resolve({
        data: null,
        error: {
          message: "Force reformat needs a hardware-stable current card identity",
        },
      });
    }
    if (command === "execute_format_authorization") {
      return Promise.resolve({
        data: { profileId: "sdxc-default", markerRestored: true },
      });
    }
    return Promise.resolve({ data: null });
  });

  render(<App />);
  await screen.findByRole("heading", { name: /sacrificial sdxc/i });
  await saveSession("D:\\Ingest\\Format");
  fireEvent.click(screen.getByRole("button", { name: /start verified ingest/i }));
  const quickFormat = await screen.findByRole("button", {
    name: /^check quick format$/i,
  });
  await waitFor(() => expect(quickFormat).toBeEnabled());

  fireEvent.click(screen.getByRole("button", { name: /^force reformat$/i }));
  const forceDialog = await screen.findByRole("dialog", {
    name: /force reformat this registered card/i,
  });
  const forceSubmit = within(forceDialog).getByRole("button", {
    name: /force reformat card/i,
  });
  expect(within(forceDialog).getByRole("button", { name: /cancel/i })).toHaveFocus();
  expect(forceSubmit).toBeDisabled();
  fireEvent.change(within(forceDialog).getByLabelText(/force reformat confirmation/i), {
    target: { value: "FORCE REFORMAT" },
  });
  expect(forceSubmit).toBeEnabled();
  fireEvent.click(forceSubmit);
  expect(
    await within(forceDialog).findByText(
      /force reformat needs a hardware-stable current card identity/i,
    ),
  ).toBeInTheDocument();
  expect(forceSubmit).toBeEnabled();
  fireEvent.click(within(forceDialog).getByRole("button", { name: /cancel/i }));

  fireEvent.click(quickFormat);
  expect(
    await screen.findByRole("dialog", { name: /quick format this verified card/i }),
  ).toBeInTheDocument();
  const formatDialog = screen.getByRole("dialog", {
    name: /quick format this verified card/i,
  });
  expect(within(formatDialog).getByRole("button", { name: /cancel/i })).toHaveFocus();
  expect(within(formatDialog).getByText("Sacrificial SDXC")).toBeInTheDocument();
  expect(within(formatDialog).getByText("sealed-format-run")).toBeInTheDocument();
  expect(
    within(formatDialog).getByText(/old file data may remain recoverable/i),
  ).toBeInTheDocument();
  expect(tauri.invoke).toHaveBeenCalledWith("request_format_authorization", {
    request: {
      runId: "sealed-format-run",
      sourceMediumKey: "hardware:format-fixture",
      sourceGeneration: 4,
      sourceIdentityConfidence: "hardware_immutable",
    },
  });
  expect(formatEligibilityChecks).toBeGreaterThanOrEqual(2);

  fireEvent.click(screen.getByRole("button", { name: /quick format card/i }));
  await waitFor(() => {
    expect(tauri.invoke).toHaveBeenCalledWith("execute_format_authorization", {
      request: { confirmationToken: "opaque-format-token" },
    });
  });
  expect(
    await screen.findByText(
      /quick format completed: sdxc-default; formatted and writable; card registration restored/i,
    ),
  ).toBeInTheDocument();
});

test("starts one registered auto-ingest per observed connection generation", async () => {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
  let autoIngestAlreadyCompleted = false;
  let completeAutoIngest: (() => void) | undefined;
  let autoProgressChannel:
    | {
        onmessage?: (update: {
          operationId: string;
          state: "copying" | "verifying";
          transferredBytes: number;
          totalBytes: number;
          currentFileIndex: number;
          totalFiles: number;
        }) => void;
      }
    | undefined;
  const snapshot = {
    sequence: 1,
    devices: [
      {
        state: "available",
        connectionGeneration: 7,
        identity: {
          mediaKey: "hardware:fixture-card",
          confidence: "hardware_immutable",
          evidence: [],
        },
        details: {
          displayName: "Registered card",
          filesystem: "exFAT",
          totalBytes: 64_000_000_000,
          availableBytes: 32_000_000_000,
          mountLocations: ["F:\\"],
          readerFingerprint: null,
          readerFamily: null,
          readerSlot: null,
        },
      },
    ],
  };
  tauri.invoke.mockImplementation(
    (
      command: string,
      args?: {
        sourceMediumKey?: string;
        request?: { operationId?: string };
        channel?: {
          onmessage?: (update: {
            operationId: string;
            state: "copying" | "verifying";
            transferredBytes: number;
            totalBytes: number;
            currentFileIndex: number;
            totalFiles: number;
          }) => void;
        };
      },
    ) => {
      if (command === "get_device_snapshot") return Promise.resolve({ data: snapshot });
      if (command === "get_ingest_history") return Promise.resolve({ data: [] });
      if (command === "watch_device_snapshots") return Promise.resolve({ data: {} });
      if (command === "get_auto_ingest_profile") {
        return Promise.resolve({
          data:
            args?.sourceMediumKey === "hardware:fixture-card"
              ? {
                  registered: true,
                  autoIngestEnabled: true,
                  autoIngestAlreadyCompleted: autoIngestAlreadyCompleted,
                  autoFormatEnabled: false,
                  destinationPath: "D:\\Ingest\\Registered",
                  sortMode: "camera_interval",
                  intervalMinutes: 1,
                  markerStatus: "recognized",
                }
              : {
                  registered: false,
                  autoIngestEnabled: false,
                  autoFormatEnabled: false,
                  destinationPath: null,
                  sortMode: null,
                  intervalMinutes: null,
                  markerStatus: "unavailable",
                },
        });
      }
      if (command === "start_verified_ingest") {
        autoProgressChannel = args?.channel;
        args?.channel?.onmessage?.({
          operationId: args.request?.operationId ?? "",
          state: "copying",
          transferredBytes: 50,
          totalBytes: 100,
          currentFileIndex: 1,
          totalFiles: 3,
        });
        return new Promise((resolve) => {
          completeAutoIngest = () =>
            resolve({
              data: {
                operationId: "auto-run",
                copiedFiles: 3,
                copiedBytes: 100,
                receiptName: "auto-run.json",
                sourceMarkerStatus: "recognized",
                autoFormatStatus: "skipped",
              },
            });
        });
      }
      if (command === "safe_eject") {
        return Promise.resolve({
          data: {
            sourceMediumKey: "hardware:fixture-card",
            sourceGeneration: 7,
          },
        });
      }
      return Promise.resolve({ data: null });
    },
  );

  const firstApp = render(<App />);

  await waitFor(() => {
    expect(tauri.invoke).toHaveBeenCalledWith(
      "start_verified_ingest",
      expect.objectContaining({
        request: expect.objectContaining({
          sourceMediumKey: "hardware:fixture-card",
          sourceGeneration: 7,
          destinationRoot: "D:\\Ingest\\Registered",
          sortMode: "camera_interval",
          intervalMinutes: 1,
          destinationDepthOrder: undefined,
          autoIngestTriggered: true,
        }),
      }),
    );
  });
  expect(screen.getByText(/auto-ingest copying.*file 1 of 3/i)).toBeInTheDocument();
  await act(async () => {
    autoProgressChannel?.onmessage?.({
      operationId:
        tauri.invoke.mock.calls.find(
          ([command]) => command === "start_verified_ingest",
        )?.[1]?.request?.operationId ?? "",
      state: "verifying",
      transferredBytes: 50,
      totalBytes: 100,
      currentFileIndex: 1,
      totalFiles: 3,
    });
  });
  await waitFor(() =>
    expect(screen.getByText(/auto-ingest verifying.*file 1 of 3/i)).toBeInTheDocument(),
  );
  await act(async () => {
    completeAutoIngest?.();
  });
  await waitFor(() =>
    expect(screen.getByText(/3 files verified by auto-ingest/i)).toBeInTheDocument(),
  );

  fireEvent.click(screen.getByRole("button", { name: /eject safely/i }));
  await waitFor(() => {
    expect(tauri.invoke).toHaveBeenCalledWith("safe_eject", {
      request: {
        runId: "auto-run",
        sourceMediumKey: "hardware:fixture-card",
        sourceGeneration: 7,
        sourceIdentityConfidence: "hardware_immutable",
      },
    });
  });
  expect(
    screen.getByText(/operating system confirmed the eject request/i),
  ).toBeInTheDocument();

  // A fresh webview session must defer to the native persisted decision for
  // this insertion instead of replaying the completed automatic copy.
  firstApp.unmount();
  autoIngestAlreadyCompleted = true;
  render(<App />);
  await waitFor(() =>
    expect(
      screen.getByText(/already auto-ingested for this mount/i),
    ).toBeInTheDocument(),
  );
  expect(
    tauri.invoke.mock.calls.filter(([command]) => command === "start_verified_ingest"),
  ).toHaveLength(1);
  await waitFor(() => {
    expect(
      tauri.invoke.mock.calls.filter(([command]) => command === "get_device_snapshot")
        .length,
    ).toBeGreaterThanOrEqual(2);
  });

  fireEvent.click(screen.getByRole("button", { name: /refresh/i }));
  await waitFor(() => {
    expect(
      tauri.invoke.mock.calls.filter(
        ([command]) => command === "start_verified_ingest",
      ),
    ).toHaveLength(1);
  });
});

test("does not consume a mounted auto-ingest attempt before its marker profile is available", async () => {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
  let profileLookups = 0;
  const snapshot = {
    sequence: 1,
    devices: [
      {
        state: "available",
        connectionGeneration: 11,
        identity: {
          mediaKey: "marker:restored-in-place",
          confidence: "unresolved",
          evidence: [],
        },
        details: {
          displayName: "Restored managed card",
          filesystem: "exFAT",
          totalBytes: 64_000_000_000,
          availableBytes: 32_000_000_000,
          mountLocations: ["F:\\"],
          readerFingerprint: "reader:fixture",
          readerFamily: null,
          readerSlot: "microSD slot (calibrated)",
        },
      },
    ],
  };
  tauri.invoke.mockImplementation((command: string) => {
    if (command === "get_device_snapshot") return Promise.resolve({ data: snapshot });
    if (command === "get_ingest_history") return Promise.resolve({ data: [] });
    if (command === "watch_device_snapshots") return Promise.resolve({ data: {} });
    if (command === "get_auto_ingest_profile") {
      profileLookups += 1;
      return Promise.resolve({
        data:
          profileLookups === 1
            ? {
                registered: false,
                autoIngestEnabled: false,
                autoFormatEnabled: false,
                destinationPath: null,
                sortMode: null,
                intervalMinutes: null,
                markerStatus: "unavailable",
              }
            : {
                registered: true,
                autoIngestEnabled: true,
                autoIngestAlreadyCompleted: false,
                autoFormatEnabled: true,
                destinationPath: "D:\\Ingest\\Restored",
                sortMode: "camera_day",
                intervalMinutes: null,
                markerStatus: "recognized",
              },
      });
    }
    if (command === "start_verified_ingest") {
      return Promise.resolve({
        data: {
          operationId: "restored-marker-run",
          copiedFiles: 1,
          copiedBytes: 1,
          receiptName: "restored-marker-run.json",
          sourceMarkerStatus: "recognized",
          autoFormatStatus: "completed",
        },
      });
    }
    return Promise.resolve({ data: null });
  });

  render(<App />);
  await waitFor(() =>
    expect(
      tauri.invoke.mock.calls.filter(
        ([command]) => command === "get_auto_ingest_profile",
      ),
    ).toHaveLength(2),
  );
  await waitFor(() =>
    expect(
      tauri.invoke.mock.calls.some(
        ([command, args]) =>
          command === "start_verified_ingest" &&
          args?.request?.sourceMediumKey === "marker:restored-in-place",
      ),
    ).toBe(true),
  );
  expect(
    tauri.invoke.mock.calls.find(
      ([command, args]) =>
        command === "start_verified_ingest" &&
        args?.request?.sourceMediumKey === "marker:restored-in-place",
    )?.[1],
  ).toEqual(
    expect.objectContaining({
      request: expect.objectContaining({
        sourceMediumKey: "marker:restored-in-place",
        sourceGeneration: 11,
        autoIngestTriggered: true,
      }),
    }),
  );
});

test("restores safe-eject eligibility from a matching sealed receipt after restart", async () => {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
  const snapshot = {
    sequence: 1,
    devices: [
      {
        state: "available",
        connectionGeneration: 9,
        identity: {
          mediaKey: "marker:fixture-card",
          confidence: "unresolved",
          evidence: [],
        },
        details: {
          displayName: "Marker-backed card",
          filesystem: "exFAT",
          totalBytes: 64_000_000_000,
          availableBytes: 32_000_000_000,
          mountLocations: ["F:\\"],
          readerFingerprint: null,
          readerFamily: null,
          readerSlot: null,
        },
      },
    ],
  };
  tauri.invoke.mockImplementation((command: string) => {
    if (command === "get_device_snapshot") return Promise.resolve({ data: snapshot });
    if (command === "get_ingest_history") {
      return Promise.resolve({
        data: [
          {
            runId: "sealed-run",
            sourceIdentityKey: "marker:fixture-card",
            sourceGeneration: 9,
            state: "completed",
            updatedAt: "2026-08-26T00:00:00Z",
            verifiedFileCount: 3,
            verifiedBytes: 100,
            receiptAvailable: true,
          },
        ],
      });
    }
    if (command === "watch_device_snapshots") return Promise.resolve({ data: {} });
    if (command === "get_auto_ingest_profile") {
      return Promise.resolve({
        data: {
          registered: false,
          autoIngestEnabled: false,
          autoFormatEnabled: false,
          destinationPath: null,
          sortMode: null,
          intervalMinutes: null,
          markerStatus: "unavailable",
        },
      });
    }
    if (command === "safe_eject") {
      return Promise.resolve({
        data: { sourceMediumKey: "marker:fixture-card", sourceGeneration: 9 },
      });
    }
    return Promise.resolve({ data: null });
  });

  render(<App />);
  expect(await screen.findByText("Completed")).toBeInTheDocument();
  expect(screen.queryByText(/^completed$/)).not.toBeInTheDocument();
  const eject = await screen.findByRole("button", { name: /eject safely/i });
  await waitFor(() => expect(eject).toBeEnabled());
  fireEvent.click(eject);
  await waitFor(() => {
    expect(tauri.invoke).toHaveBeenCalledWith("safe_eject", {
      request: {
        runId: "sealed-run",
        sourceMediumKey: "marker:fixture-card",
        sourceGeneration: 9,
        sourceIdentityConfidence: "unresolved",
      },
    });
  });
});

test("shows an explicit keep-or-cancel choice when the native close gate fires", async () => {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
  tauri.invoke.mockImplementation((command: string) => {
    if (command === "get_device_snapshot")
      return Promise.resolve({ data: { sequence: 1, devices: [] } });
    if (command === "get_ingest_history") return Promise.resolve({ data: [] });
    if (command === "watch_device_snapshots") return Promise.resolve({ data: {} });
    return Promise.resolve({ data: null });
  });
  render(<App />);
  await waitFor(() =>
    expect(events.handlers.get("media-ingest://close-requested")).toBeTypeOf(
      "function",
    ),
  );
  await act(async () => {
    events.handlers.get("media-ingest://close-requested")?.();
  });
  expect(
    screen.getByRole("dialog", { name: /keep this app open until copying stops/i }),
  ).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: /keep ingesting/i }));
  expect(
    screen.queryByRole("dialog", { name: /keep this app open until copying stops/i }),
  ).not.toBeInTheDocument();
});

test("rechecks inventory and history when the app regains focus", async () => {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
  tauri.invoke.mockImplementation((command: string) => {
    if (command === "get_device_snapshot")
      return Promise.resolve({ data: { sequence: 1, devices: [] } });
    if (command === "get_ingest_history") return Promise.resolve({ data: [] });
    if (command === "watch_device_snapshots") return Promise.resolve({ data: {} });
    return Promise.resolve({ data: null });
  });
  render(<App />);
  await waitFor(() =>
    expect(events.handlers.get("media-ingest://reactivated")).toBeTypeOf("function"),
  );
  const before = tauri.invoke.mock.calls.length;
  await act(async () => {
    events.handlers.get("media-ingest://reactivated")?.();
  });
  await waitFor(() => expect(tauri.invoke.mock.calls.length).toBeGreaterThan(before));
  expect(
    tauri.invoke.mock.calls.filter(([command]) => command === "get_device_snapshot")
      .length,
  ).toBeGreaterThanOrEqual(2);
  expect(
    tauri.invoke.mock.calls.filter(([command]) => command === "get_ingest_history")
      .length,
  ).toBeGreaterThanOrEqual(2);
});
