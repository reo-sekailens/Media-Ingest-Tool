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

test("renders named, visible controls for the core ingest workflow", () => {
  render(<App />);

  expect(screen.getByRole("button", { name: "Rescan Connected Media" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Start Verified Ingest" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Set Up Auto-Ingest" })).toBeVisible();
  expect(screen.getByLabelText("Destination directory")).toHaveAttribute(
    "name",
    "destination-directory",
  );
  expect(screen.getByRole("link", { name: "Skip to workspace" })).toHaveAttribute(
    "href",
    "#ingest-workspace",
  );
  expect(screen.getByText("F:\\ · SDXC")).toBeInTheDocument();
  expect(screen.getByText("Drive F:\\ · SDXC")).toBeInTheDocument();
});

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
  expect(
    screen.getByText(/insertion 1.*changes after this medium is absent/i),
  ).toBeInTheDocument();
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

test("selects EXIF sort tags and shows the resulting destination depth", () => {
  render(<App />);

  expect(screen.getByText("Destination depth · 3 levels")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Custom interval" }));
  expect(screen.getByLabelText("Custom interval minutes")).toHaveValue(30);
  expect(screen.getByText("Destination depth · 4 levels")).toBeInTheDocument();
  expect(screen.getByText("30-minute bucket")).toBeInTheDocument();

  fireEvent.change(screen.getByLabelText("Custom interval minutes"), {
    target: { value: "45" },
  });
  expect(screen.getByText("45-minute bucket")).toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "Original tree" }));
  expect(screen.getByText("Destination depth · 2 levels")).toBeInTheDocument();
});

test("opens the auto-ingest setup modal", () => {
  render(<App />);
  fireEvent.click(screen.getByRole("button", { name: /set up auto-ingest/i }));
  expect(
    screen.getByRole("dialog", { name: /set up auto-ingest/i }),
  ).toBeInTheDocument();
  expect(screen.getByLabelText("Auto-ingest destination directory")).toHaveValue(
    "D:\\Ingest\\Documentary\\Day 03",
  );
  expect(screen.getByRole("button", { name: /save setup/i })).toBeEnabled();
  fireEvent.click(screen.getByLabelText(/ingest automatically on mount/i));
  expect(
    screen.getByLabelText(/format after a verified auto-ingest/i),
  ).not.toBeDisabled();
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

test("confirms an eligible quick format with an opaque native token", async () => {
  Object.assign(window, { __TAURI_INTERNALS__: {} });
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
      return Promise.resolve({
        data: {
          eligible: true,
          reason: "Eligible for the allowlisted generic profile.",
          recommendedProfile: {
            id: "sdxc-default",
            filesystem: "exfat",
            inferredFromCapacity: true,
          },
        },
      });
    }
    if (command === "request_format_authorization") {
      return Promise.resolve({
        data: { confirmationToken: "opaque-format-token", expiresInSeconds: 60 },
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
  fireEvent.change(screen.getByLabelText("Destination directory"), {
    target: { value: "D:\\Ingest\\Format" },
  });
  fireEvent.click(screen.getByRole("button", { name: /start verified ingest/i }));
  const quickFormat = await screen.findByRole("button", { name: /^quick format$/i });
  await waitFor(() => expect(quickFormat).toBeEnabled());

  fireEvent.click(quickFormat);
  expect(
    await screen.findByRole("dialog", { name: /quick format this verified card/i }),
  ).toBeInTheDocument();
  const formatDialog = screen.getByRole("dialog", {
    name: /quick format this verified card/i,
  });
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
