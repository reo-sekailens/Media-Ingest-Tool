import { Channel, invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  CircleCheck,
  ChevronDown,
  ChevronUp,
  HardDrive,
  HardDriveDownload,
  ListTodo,
  RefreshCw,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Modal } from "./Modal";
import { CardConfiguration } from "./CardConfiguration";
import { CaptureTimeWarning } from "./CaptureTimeWarning";
import {
  cloneConfiguration,
  createConfiguration,
  customDirectoryFieldsAreComplete,
  defaultDestinationDepthOrder,
  destinationDepthSegmentLabel,
  normalizeDestinationDepthOrder,
  resolveSortRequest,
  validateConfiguration,
  type CaptureTimeBasis,
  type ConfigurationDraft,
  type CustomDirectoryField,
  type DestinationDepthSegment,
} from "./configuration";
import "./App.css";

type IdentityConfidence = "Verified" | "Reader-bound";
type TransferState = "Ready" | "Copying";
type VerificationState = "Verified" | "Pending";

type DeviceFixture = {
  id: string;
  name: string;
  kind: string;
  capacity: string;
  available: string;
  confidence: IdentityConfidence;
  identity: string;
  reader: string;
  slot: string;
  transfer: TransferState;
  verification: VerificationState;
  destination: string;
  media: string;
  updated: string;
  sourceRoot: string;
  nativeConfidence: string;
  connectionGeneration: number;
  ingestable: boolean;
  readerFingerprint: string | null;
  logicalUnit: number | null;
};

const devices: readonly DeviceFixture[] = [
  {
    id: "cam-a",
    name: "A-Cam • SDXC",
    kind: "Removable SDXC",
    capacity: "119.1 GB",
    available: "38.4 GB available",
    confidence: "Verified",
    identity: "SD CID • 03534453 4B4F4E47 9D0C8700",
    reader: "SanDisk Professional PRO-READER",
    slot: "SD slot",
    transfer: "Ready",
    verification: "Pending",
    destination: "D:\\Ingest\\Documentary\\Day 03",
    media: "1,284 clips · 80.7 GB",
    updated: "Seen just now",
    sourceRoot: "F:\\",
    nativeConfidence: "hardware_immutable",
    connectionGeneration: 1,
    ingestable: true,
    readerFingerprint: "preview:reader-unit-0",
    logicalUnit: 0,
  },
  {
    id: "b-cam",
    name: "B-Cam • microSDXC",
    kind: "Removable microSDXC",
    capacity: "238.3 GB",
    available: "152.6 GB available",
    confidence: "Reader-bound",
    identity: "Reader serial + slot evidence",
    reader: "SanDisk Professional PRO-READER",
    slot: "microSD slot",
    transfer: "Copying",
    verification: "Verified",
    destination: "E:\\Ingest\\Documentary\\Day 03",
    media: "846 clips · 51.2 GB",
    updated: "Copying 63% · 412 MB/s",
    sourceRoot: "M:\\",
    nativeConfidence: "hardware_stable",
    connectionGeneration: 1,
    ingestable: true,
    readerFingerprint: "preview:reader-unit-1",
    logicalUnit: 1,
  },
];

type NativeDevice = {
  state: "available" | "empty_reader" | "busy" | "removed" | "unsupported";
  connectionGeneration: number;
  identity: {
    mediaKey: string;
    confidence: string;
    evidence: { kind: string; fingerprint: string }[];
  };
  details: {
    displayName: string;
    filesystem: string | null;
    totalBytes: number | null;
    availableBytes: number | null;
    mountLocations: string[];
    readerFingerprint: string | null;
    readerFamily: "sandisk_pro_reader" | null;
    readerSlot: string | null;
  };
};

type NativeDeviceSnapshot = {
  devices: NativeDevice[];
  sequence: number;
};

const formatBytes = (value: number | null) =>
  value === null ? "Size unavailable" : `${(value / 1_000_000_000).toFixed(1)} GB`;

const formatTransferBytes = (value: number) => {
  if (value < 1_000_000) return `${Math.round(value / 1_000)} KB`;
  return `${(value / 1_000_000).toFixed(1)} MB`;
};

const formatEta = (seconds: number) => {
  if (!Number.isFinite(seconds) || seconds < 1) return "under 1 second";
  if (seconds < 60) return `${Math.ceil(seconds)} seconds`;
  return `${Math.ceil(seconds / 60)} minutes`;
};

const formatRunState = (state: IngestHistoryEntry["state"]) =>
  state
    .split("_")
    .map((word) => `${word.slice(0, 1).toUpperCase()}${word.slice(1)}`)
    .join(" ");

const asFixture = (device: NativeDevice): DeviceFixture => ({
  id: device.identity.mediaKey,
  name: device.details.displayName,
  kind: device.details.filesystem
    ? `Removable ${device.details.filesystem}`
    : "Removable storage",
  capacity: formatBytes(device.details.totalBytes),
  available:
    device.details.availableBytes === null
      ? "Availability unavailable"
      : `${formatBytes(device.details.availableBytes)} available`,
  confidence: "Reader-bound",
  identity: device.identity.evidence.some((evidence) => evidence.kind === "app_marker")
    ? "App marker observed · mutable evidence"
    : device.identity.evidence.length
      ? "Filesystem/session evidence only"
      : "No card identity exposed",
  reader:
    device.details.readerFamily === "sandisk_pro_reader"
      ? "SanDisk Professional PRO-READER family"
      : (device.details.readerFingerprint ?? "Reader topology unresolved"),
  slot: device.details.readerSlot ?? device.details.mountLocations[0] ?? "Unmounted",
  transfer: "Ready",
  verification: "Pending",
  destination: "Choose a destination before ingest",
  media: "Scan files before ingest",
  updated:
    device.state === "available"
      ? "Observed by native discovery"
      : device.state === "empty_reader"
        ? "Empty reader — insert card"
        : device.state,
  sourceRoot: device.details.mountLocations[0] ?? "",
  nativeConfidence: device.identity.confidence,
  connectionGeneration: device.connectionGeneration,
  ingestable: device.state === "available" && device.details.mountLocations.length > 0,
  readerFingerprint: device.details.readerFingerprint,
  logicalUnit: device.details.readerSlot?.startsWith("Logical unit ")
    ? Number(device.details.readerSlot.slice("Logical unit ".length))
    : null,
});

type IngestResult = {
  operationId: string;
  copiedFiles: number;
  copiedBytes: number;
  receiptName: string;
  sourceMarkerStatus: "recognized" | "created" | "unavailable";
  autoFormatStatus: "not_configured" | "skipped" | "completed" | "failed";
};

type SourceInventory = {
  fileCount: number;
  totalBytes: number;
};

type IngestPlanPreview = {
  operationId: string;
  fileCount: number;
  totalBytes: number;
  sampleDestinationPaths: string[];
};

type ScopedIngestPlanPreview = {
  sourceKey: string;
  configurationSignature: string;
  plan: IngestPlanPreview | null;
  label: string;
};

type RememberedDestination = {
  destinationPath: string | null;
  canRemember: boolean;
};

type CardRegistration = {
  registered: boolean;
  autoIngestEnabled: boolean;
  autoIngestAlreadyCompleted?: boolean;
  autoFormatEnabled: boolean;
  destinationPath: string | null;
  sortMode: "original_tree" | "camera_day" | "camera_interval" | null;
  intervalMinutes: number | null;
  captureTimeBasis?: CaptureTimeBasis;
  customDirectoryFields: CustomDirectoryField[];
  destinationDepthOrder: DestinationDepthSegment[];
  markerStatus: "recognized" | "created" | "unavailable";
};

type FormatEligibility = {
  eligible: boolean;
  reason: string;
  recommendedProfile: {
    id: string;
    filesystem: "fat" | "fat32" | "exfat";
    inferredFromCapacity: boolean;
  } | null;
};

type FormatAuthorization = {
  confirmationToken: string;
  expiresInSeconds: number;
};

type FormatExecutionResult = {
  profileId: string;
  markerRestored: boolean;
};

type PendingFormatConfirmation = {
  confirmationToken: string;
  deviceId: string;
  deviceName: string;
  media: string;
  capacity: string;
  receiptId: string;
  profile: FormatEligibility["recommendedProfile"];
};

type PendingForceReformat = {
  deviceId: string;
  deviceName: string;
  media: string;
  capacity: string;
};

type IngestHistoryEntry = {
  runId: string;
  sourceIdentityKey: string;
  sourceGeneration: number;
  state: "queued" | "copying" | "recovery_required" | "completed" | "failed";
  updatedAt: string;
  verifiedFileCount: number;
  verifiedBytes: number;
  receiptAvailable: boolean;
};

type ProgressUpdate = {
  operationId: string;
  state:
    | "queued"
    | "copying"
    | "verifying"
    | "formatting"
    | "completed"
    | "failed"
    | "cancelled";
  transferredBytes: number;
  totalBytes: number | null;
  currentFileIndex?: number | null;
  totalFiles?: number | null;
};

type ActiveOperation = {
  operationId: string;
  cancelling: boolean;
  state: ProgressUpdate["state"];
  transferredBytes: number;
  totalBytes: number | null;
  currentFileIndex: number | null;
  totalFiles: number | null;
  startedAt: number;
  bytesPerSecond: number;
  logs: readonly string[];
};

type CompletedRun = {
  runId: string;
  sourceGeneration: number;
  sourceIdentityConfidence: string;
};

function configurationKey(
  source: Pick<DeviceFixture, "id" | "connectionGeneration" | "sourceRoot">,
): string {
  return JSON.stringify([source.id, source.connectionGeneration, source.sourceRoot]);
}

function configurationFromRegistration(profile: CardRegistration): ConfigurationDraft {
  return createConfiguration({
    destinationPath: profile.destinationPath ?? "",
    sortPreset:
      profile.sortMode === "original_tree"
        ? "original_tree"
        : profile.sortMode === "camera_interval"
          ? profile.intervalMinutes === 60
            ? "exif_hour"
            : "exif_custom"
          : "exif_day",
    customIntervalMinutes: String(profile.intervalMinutes ?? 30),
    captureTimeBasis: profile.captureTimeBasis ?? "camera_recorded",
    customDirectoryFields: profile.customDirectoryFields,
    destinationDepthOrder: profile.destinationDepthOrder,
    persistOnCard: true,
    autoIngestEnabled: profile.autoIngestEnabled,
    autoFormatEnabled: profile.autoIngestEnabled && profile.autoFormatEnabled,
  });
}

function StatusDot({ tone }: { tone: "ready" | "copying" | "verified" }) {
  return <span aria-hidden="true" className={`status-dot status-dot--${tone}`} />;
}

function App() {
  const [theme, setTheme] = useState<"light" | "dark">(() =>
    window.matchMedia?.("(prefers-color-scheme: dark)").matches ? "dark" : "light",
  );
  const [liveDevices, setLiveDevices] = useState<readonly DeviceFixture[]>(devices);
  const [selectedId, setSelectedId] = useState(devices[0].id);
  const [scanLabel, setScanLabel] = useState(() =>
    isTauri()
      ? "Monitoring removable storage"
      : "Preview fixture — native inventory is available in the desktop app",
  );
  const [activeOperations, setActiveOperations] = useState<
    Readonly<Record<string, ActiveOperation>>
  >({});
  const [isIngestDockOpen, setIsIngestDockOpen] = useState(false);
  const [isIdentityDetailsOpen, setIsIdentityDetailsOpen] = useState(false);
  const [ingestLabels, setIngestLabels] = useState<Readonly<Record<string, string>>>(
    {},
  );
  const [configurations, setConfigurations] = useState<
    Readonly<Record<string, ConfigurationDraft>>
  >({});
  const appliedConfigurationsRef = useRef(configurations);
  useEffect(() => {
    appliedConfigurationsRef.current = configurations;
  }, [configurations]);
  const [configuration, setConfiguration] = useState<{
    source: DeviceFixture;
    initialValue: ConfigurationDraft;
  } | null>(null);
  const [configurationError, setConfigurationError] = useState("");
  const [configurationLabels, setConfigurationLabels] = useState<
    Readonly<Record<string, string>>
  >({});
  const [isLoadingConfiguration, setIsLoadingConfiguration] = useState(false);
  const [isSavingConfiguration, setIsSavingConfiguration] = useState(false);
  const savingConfigurationSource = useRef<DeviceFixture | null>(null);
  const liveSourcesRef = useRef(liveDevices);
  useEffect(() => {
    liveSourcesRef.current = liveDevices;
  }, [liveDevices]);
  const [calibrationKind, setCalibrationKind] = useState<"sd" | "micro_sd">("sd");
  const [calibrationLabel, setCalibrationLabel] = useState("");
  const [history, setHistory] = useState<readonly IngestHistoryEntry[]>([]);
  const [activeRecoveryRuns, setActiveRecoveryRuns] = useState<ReadonlySet<string>>(
    () => new Set(),
  );
  const [recoveryLabels, setRecoveryLabels] = useState<
    Readonly<Record<string, string>>
  >({});
  const [sourceInventories, setSourceInventories] = useState<
    Readonly<Record<string, SourceInventory>>
  >({});
  const [sourceInventoryLabels, setSourceInventoryLabels] = useState<
    Readonly<Record<string, string>>
  >({});
  const [planPreviews, setPlanPreviews] = useState<
    Readonly<Record<string, ScopedIngestPlanPreview>>
  >({});
  const latestPreviewRequests = useRef(new Map<string, string>());
  const [formatReadinessLabels, setFormatReadinessLabels] = useState<
    Readonly<Record<string, string>>
  >({});
  const [formatEligibility, setFormatEligibility] = useState<
    Readonly<Record<string, FormatEligibility>>
  >({});
  const [pendingFormatConfirmation, setPendingFormatConfirmation] =
    useState<PendingFormatConfirmation | null>(null);
  const [pendingForceReformat, setPendingForceReformat] =
    useState<PendingForceReformat | null>(null);
  const [forceReformatPhrase, setForceReformatPhrase] = useState("");
  const [forceReformatError, setForceReformatError] = useState("");
  const [isExecutingFormat, setIsExecutingFormat] = useState(false);
  const [completedRuns, setCompletedRuns] = useState<
    Readonly<Record<string, CompletedRun>>
  >({});
  const [ejectLabels, setEjectLabels] = useState<Readonly<Record<string, string>>>({});
  const [isCloseConfirmationOpen, setIsCloseConfirmationOpen] = useState(false);
  const [isCancellingForClose, setIsCancellingForClose] = useState(false);
  const storageWatchStarted = useRef(false);
  const autoIngestAttempted = useRef(new Set<string>());
  const hydratedProfiles = useRef(new Set<string>());
  // A mounted card can become registered while the app is already observing
  // it (for example, immediately after a managed format restores its marker).
  // Keep a short-lived lookup guard separate from the terminal attempt guard:
  // an unregistered lookup must never consume this mount's one auto-ingest.
  const autoIngestProfileChecks = useRef(new Set<string>());
  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    document
      .querySelector('meta[name="theme-color"]')
      ?.setAttribute("content", theme === "dark" ? "#020617" : "#f8fafc");
  }, [theme]);
  const selected = useMemo(
    () => liveDevices.find((device) => device.id === selectedId) ?? liveDevices[0],
    [liveDevices, selectedId],
  );
  const selectedConfigurationKey = selected ? configurationKey(selected) : "";
  const selectedConfiguration =
    configurations[selectedConfigurationKey] ??
    createConfiguration({
      persistOnCard: isTauri(),
    });
  const destination = selectedConfiguration.destinationPath;
  const {
    sortPreset,
    captureTimeBasis,
    customIntervalMinutes,
    customDirectoryFields,
    destinationDepthOrder,
  } = selectedConfiguration;
  const selectedSortRequest = resolveSortRequest(sortPreset, customIntervalMinutes);
  const selectedDestinationDepthOrder = normalizeDestinationDepthOrder(
    destinationDepthOrder,
    defaultDestinationDepthOrder(selectedSortRequest, customDirectoryFields),
  );
  const selectedSortLevels = selectedDestinationDepthOrder.map((segment) =>
    destinationDepthSegmentLabel(segment, customDirectoryFields, selectedSortRequest),
  );
  const selectedConfigurationLabel =
    configurationLabels[selectedConfigurationKey] ?? "";
  const selectedInventory = selected ? sourceInventories[selected.id] : undefined;
  const selectedInventoryLabel = selected ? sourceInventoryLabels[selected.id] : "";
  const selectedConfigurationSignature = JSON.stringify(selectedConfiguration);
  const cachedPreview = selected ? planPreviews[selected.id] : undefined;
  const selectedPreviewState =
    cachedPreview?.sourceKey === selectedConfigurationKey &&
    cachedPreview.configurationSignature === selectedConfigurationSignature
      ? cachedPreview
      : undefined;
  const selectedPlanPreview = selectedPreviewState?.plan;
  const selectedPlanPreviewLabel = selectedPreviewState?.label ?? "";
  const selectedFormatReadinessLabel = selected
    ? formatReadinessLabels[selected.id]
    : "";
  const selectedFormatEligibility = selected
    ? formatEligibility[selected.id]
    : undefined;
  const selectedCompletedRun = selected
    ? (completedRuns[selected.id] ??
      (() => {
        const entry = history.find(
          (candidate) =>
            candidate.state === "completed" &&
            candidate.receiptAvailable &&
            candidate.sourceIdentityKey === selected.id &&
            candidate.sourceGeneration === selected.connectionGeneration,
        );
        return entry
          ? {
              runId: entry.runId,
              sourceGeneration: entry.sourceGeneration,
              sourceIdentityConfidence: selected.nativeConfidence,
            }
          : undefined;
      })())
    : undefined;
  const selectedCompletedRunId = selectedCompletedRun?.runId;
  const selectedCompletedRunGeneration = selectedCompletedRun?.sourceGeneration;
  const selectedCompletedRunConfidence = selectedCompletedRun?.sourceIdentityConfidence;
  const selectedEjectLabel = selected ? ejectLabels[selected.id] : "";
  const selectedOperation = selected ? activeOperations[selected.id] : undefined;
  const selectedIngestLabel = selected ? ingestLabels[selected.id] : "";
  const dockOperations = useMemo(
    () =>
      Object.entries(activeOperations).map(([deviceId, operation]) => ({
        ...operation,
        device: liveDevices.find((device) => device.id === deviceId),
      })),
    [activeOperations, liveDevices],
  );
  const dockKnownTotalBytes = dockOperations.reduce(
    (total, operation) => total + (operation.totalBytes ?? 0),
    0,
  );
  const dockTransferredBytes = dockOperations.reduce(
    (total, operation) => total + operation.transferredBytes,
    0,
  );
  const dockBytesPerSecond = dockOperations.reduce(
    (total, operation) => total + operation.bytesPerSecond,
    0,
  );
  const dockPercent =
    dockKnownTotalBytes > 0
      ? Math.min(100, Math.round((dockTransferredBytes / dockKnownTotalBytes) * 100))
      : 0;
  const dockEta =
    dockKnownTotalBytes > dockTransferredBytes && dockBytesPerSecond > 0
      ? formatEta((dockKnownTotalBytes - dockTransferredBytes) / dockBytesPerSecond)
      : "Calculating…";
  const trackOperationProgress = useCallback(
    (deviceId: string, update: ProgressUpdate) => {
      setActiveOperations((current) => {
        const existing = current[deviceId];
        if (!existing) return current;
        const transferredBytes = Math.max(
          existing.transferredBytes,
          update.transferredBytes,
        );
        const elapsedSeconds = Math.max(
          0.001,
          (Date.now() - existing.startedAt) / 1_000,
        );
        const bytesPerSecond = transferredBytes / elapsedSeconds;
        const stateChanged = existing.state !== update.state;
        const fileChanged = existing.currentFileIndex !== update.currentFileIndex;
        const log = `${update.state.replace(/_/g, " ")} · ${formatTransferBytes(transferredBytes)}${update.totalBytes === null ? "" : ` / ${formatTransferBytes(update.totalBytes)}`}`;
        return {
          ...current,
          [deviceId]: {
            ...existing,
            state: update.state,
            transferredBytes,
            totalBytes: update.totalBytes,
            currentFileIndex: update.currentFileIndex ?? null,
            totalFiles: update.totalFiles ?? null,
            bytesPerSecond: Number.isFinite(bytesPerSecond) ? bytesPerSecond : 0,
            logs:
              stateChanged || fileChanged
                ? [...existing.logs, log].slice(-6)
                : existing.logs,
          },
        };
      });
    },
    [],
  );
  const chooseConfigurationDestination = useCallback(async (currentPath: string) => {
    if (!isTauri())
      throw new Error(
        "Native folder selection is available in the desktop application. You can type a path in this preview.",
      );
    try {
      return await open({
        title: "Select ingest destination",
        directory: true,
        multiple: false,
        defaultPath: currentPath || undefined,
      });
    } catch {
      throw new Error(
        "The native folder picker could not be opened. Type a destination path instead.",
      );
    }
  }, []);
  const applyNativeSnapshot = useCallback((snapshot: NativeDeviceSnapshot) => {
    const nextDevices = snapshot.devices.map(asFixture);
    for (const source of liveSourcesRef.current) {
      if (
        !nextDevices.some(
          (device) =>
            configurationKey(device) === configurationKey(source) && device.ingestable,
        )
      ) {
        latestPreviewRequests.current.delete(source.id);
      }
    }
    liveSourcesRef.current = nextDevices;
    setPlanPreviews((current) =>
      Object.fromEntries(
        Object.entries(current).filter(([, preview]) =>
          nextDevices.some(
            (device) =>
              configurationKey(device) === preview.sourceKey && device.ingestable,
          ),
        ),
      ),
    );
    setLiveDevices(nextDevices);
    setSelectedId((current) =>
      nextDevices.some((device) => device.id === current)
        ? current
        : (nextDevices[0]?.id ?? ""),
    );
    setScanLabel(
      nextDevices.length
        ? "Native storage inventory refreshed"
        : "No mounted removable media detected",
    );
  }, []);
  const refreshDevices = useCallback(async () => {
    if (!isTauri()) {
      return;
    }
    try {
      const response = await invoke<{
        data: NativeDeviceSnapshot | null;
      }>("get_device_snapshot");
      if (!response.data) return;
      applyNativeSnapshot(response.data);
    } catch {
      setScanLabel(
        "Preview fixture — native inventory is available in the desktop app",
      );
    }
  }, [applyNativeSnapshot]);
  const refreshHistory = useCallback(async () => {
    if (!isTauri()) return;
    try {
      const response = await invoke<{
        data: IngestHistoryEntry[] | null;
      }>("get_ingest_history");
      if (!response.data) return;
      setHistory(response.data);
      const completed = new Map(
        response.data
          .filter((run) => run.state === "completed" && run.receiptAvailable)
          .map((run) => [run.runId, run]),
      );
      if (!completed.size) return;
      setActiveOperations((current) => {
        const next = { ...current };
        for (const [deviceId, operation] of Object.entries(current)) {
          if (completed.has(operation.operationId)) delete next[deviceId];
        }
        return next;
      });
      setIngestLabels((current) => {
        const next = { ...current };
        for (const [deviceId, operation] of Object.entries(activeOperations)) {
          const run = completed.get(operation.operationId);
          if (run) {
            next[deviceId] = `${run.verifiedFileCount} files verified · receipt sealed`;
          }
        }
        return next;
      });
      setCompletedRuns((current) => {
        const next = { ...current };
        for (const [deviceId, operation] of Object.entries(activeOperations)) {
          const run = completed.get(operation.operationId);
          if (run) {
            next[deviceId] = {
              runId: run.runId,
              sourceGeneration: run.sourceGeneration,
              sourceIdentityConfidence: "unresolved",
            };
          }
        }
        return next;
      });
    } catch {
      // History is supplementary; failure must not hide live media controls.
    }
  }, [activeOperations]);
  useEffect(() => {
    if (!Object.keys(activeOperations).length) return;
    const interval = window.setInterval(() => void refreshHistory(), 2_000);
    return () => window.clearInterval(interval);
  }, [activeOperations, refreshHistory]);
  const resumeHistoryRun = useCallback(
    async (runId: string) => {
      if (!isTauri()) {
        setRecoveryLabels((current) => ({
          ...current,
          [runId]: "Recovery is available in the desktop application.",
        }));
        return;
      }
      setActiveRecoveryRuns((current) => new Set(current).add(runId));
      setRecoveryLabels((current) => ({
        ...current,
        [runId]: "Rechecking published files against the frozen plan…",
      }));
      const progress = new Channel<ProgressUpdate>();
      progress.onmessage = (update) => {
        if (update.operationId !== runId) return;
        const filePosition =
          update.currentFileIndex !== null &&
          update.currentFileIndex !== undefined &&
          update.totalFiles !== null &&
          update.totalFiles !== undefined
            ? ` · file ${update.currentFileIndex} of ${update.totalFiles}`
            : "";
        setRecoveryLabels((current) => ({
          ...current,
          [runId]: `${update.state.replace(/_/g, " ")} ${formatTransferBytes(update.transferredBytes)}${update.totalBytes === null ? "" : ` / ${formatTransferBytes(update.totalBytes)}`}${filePosition}`,
        }));
      };
      try {
        const response = await invoke<{
          data: IngestResult | null;
          error: { message: string } | null;
        }>("resume_verified_ingest", {
          request: { runId, maxWorkers: 2 },
          channel: progress,
        });
        setRecoveryLabels((current) => ({
          ...current,
          [runId]: response.data
            ? `${response.data.copiedFiles} files verified and receipt sealed.`
            : (response.error?.message ?? "Recovery did not complete."),
        }));
        await refreshHistory();
      } catch {
        setRecoveryLabels((current) => ({
          ...current,
          [runId]: "The desktop service could not resume this ingest.",
        }));
      } finally {
        setActiveRecoveryRuns((current) => {
          const next = new Set(current);
          next.delete(runId);
          return next;
        });
      }
    },
    [refreshHistory],
  );
  const scanSourceMedia = useCallback(async () => {
    if (!selected) return;
    if (!isTauri()) {
      setSourceInventoryLabels((current) => ({
        ...current,
        [selected.id]: "Media scanning is available in the desktop application.",
      }));
      return;
    }
    if (!selected.ingestable || !selected.sourceRoot) {
      setSourceInventoryLabels((current) => ({
        ...current,
        [selected.id]: "Insert mounted media before scanning its files.",
      }));
      return;
    }
    setSourceInventoryLabels((current) => ({
      ...current,
      [selected.id]: "Scanning regular media files…",
    }));
    try {
      const response = await invoke<{
        data: SourceInventory | null;
        error: { message: string } | null;
      }>("scan_source_inventory", {
        request: {
          sourceRoot: selected.sourceRoot,
          sourceMediumKey: selected.id,
        },
      });
      if (!response.data) {
        setSourceInventoryLabels((current) => ({
          ...current,
          [selected.id]: response.error?.message ?? "The media scan did not complete.",
        }));
        return;
      }
      const inventory = response.data;
      setSourceInventories((current) => ({
        ...current,
        [selected.id]: inventory,
      }));
      setSourceInventoryLabels((current) => ({
        ...current,
        [selected.id]: `${inventory.fileCount} files · ${formatTransferBytes(inventory.totalBytes)}`,
      }));
    } catch {
      setSourceInventoryLabels((current) => ({
        ...current,
        [selected.id]: "The desktop service could not scan this card.",
      }));
    }
  }, [selected]);
  const previewIngestPlan = useCallback(async () => {
    if (!selected) return;
    const sourceKey = configurationKey(selected);
    const signature = JSON.stringify(selectedConfiguration);
    const requestId = crypto.randomUUID();
    latestPreviewRequests.current.set(selected.id, requestId);
    const isCurrentRequest = () =>
      latestPreviewRequests.current.get(selected.id) === requestId &&
      liveSourcesRef.current.some(
        (device) => configurationKey(device) === sourceKey && device.ingestable,
      ) &&
      JSON.stringify(
        appliedConfigurationsRef.current[sourceKey] ??
          createConfiguration({ persistOnCard: isTauri() }),
      ) === signature;
    const showPreview = (label: string, plan: IngestPlanPreview | null = null) => {
      if (!isCurrentRequest()) return;
      setPlanPreviews((current) => ({
        ...current,
        [selected.id]: {
          sourceKey,
          configurationSignature: signature,
          plan,
          label,
        },
      }));
    };
    const sortRequest = resolveSortRequest(sortPreset, customIntervalMinutes);
    if (!sortRequest) {
      showPreview("Enter a whole custom interval from 1 to 1,440 minutes.");
      return;
    }
    if (!customDirectoryFieldsAreComplete(customDirectoryFields)) {
      showPreview("Complete or remove each custom folder field before previewing.");
      return;
    }
    if (!isTauri()) {
      showPreview("Organization preview is available in the desktop application.");
      return;
    }
    if (!selected.ingestable || !selected.sourceRoot) {
      showPreview("Insert mounted media before planning its organization.");
      return;
    }
    if (!destination.trim()) {
      showPreview("Enter a destination directory before previewing organization.");
      return;
    }
    showPreview("Planning destination paths without copying files…");
    try {
      const response = await invoke<{
        data: IngestPlanPreview | null;
        error: { message: string } | null;
      }>("preview_verified_ingest", {
        request: {
          operationId: requestId,
          sourceRoot: selected.sourceRoot,
          destinationRoot: destination.trim(),
          sourceMediumKey: selected.id,
          sourceIdentityConfidence: selected.nativeConfidence,
          sourceGeneration: selected.connectionGeneration,
          maxWorkers: 2,
          sortMode: sortRequest.sortMode,
          intervalMinutes: sortRequest.intervalMinutes,
          captureTimeBasis,
          customDirectoryFields,
          destinationDepthOrder: selectedDestinationDepthOrder,
          autoIngestTriggered: false,
        },
      });
      if (!response.data) {
        showPreview(
          response.error?.message ?? "The organization plan did not complete.",
        );
        return;
      }
      showPreview("", response.data);
    } catch {
      showPreview("The desktop service could not preview this organization.");
    }
  }, [
    customDirectoryFields,
    customIntervalMinutes,
    destination,
    selected,
    selectedConfiguration,
    selectedDestinationDepthOrder,
    sortPreset,
    captureTimeBasis,
  ]);
  const openConfiguration = useCallback(async () => {
    if (
      !selected ||
      !selected.ingestable ||
      activeOperations[selected.id] ||
      isLoadingConfiguration ||
      savingConfigurationSource.current
    )
      return;
    const source = { ...selected };
    const key = configurationKey(source);
    let initialValue = cloneConfiguration(selectedConfiguration);
    setConfigurationError("");
    setConfigurationLabels((current) => ({ ...current, [key]: "" }));
    if (isTauri() && !configurations[key]) {
      setIsLoadingConfiguration(true);
      try {
        const response = await invoke<{
          data: CardRegistration | null;
          error: { message: string } | null;
        }>("get_auto_ingest_profile", { sourceMediumKey: source.id });
        if (response.error) throw new Error(response.error.message);
        if (response.data?.registered)
          initialValue = configurationFromRegistration(response.data);
        else if (
          source.nativeConfidence === "hardware_immutable" ||
          source.nativeConfidence === "hardware_stable"
        ) {
          const remembered = await invoke<{
            data: RememberedDestination | null;
            error: { message: string } | null;
          }>("get_remembered_destination", { sourceMediumKey: source.id });
          if (remembered.error) throw new Error(remembered.error.message);
          if (remembered.data?.destinationPath)
            initialValue.destinationPath = remembered.data.destinationPath;
        }
      } catch (error) {
        setConfigurationError(
          error instanceof Error
            ? error.message
            : "Saved settings could not be loaded. Close and retry before replacing them.",
        );
        // Never replace unknown saved options with defaults after a failed lookup.
        setConfigurationLabels((current) => ({
          ...current,
          [key]:
            error instanceof Error
              ? error.message
              : "Saved configuration could not be loaded. Retry before changing settings.",
        }));
        setIsLoadingConfiguration(false);
        return;
      }
      setIsLoadingConfiguration(false);
    }
    setConfiguration({ source, initialValue });
  }, [
    selected,
    selectedConfiguration,
    configurations,
    activeOperations,
    isLoadingConfiguration,
  ]);

  const saveConfiguration = useCallback(
    async (draft: ConfigurationDraft) => {
      if (!configuration || savingConfigurationSource.current) return;
      const source = configuration.source;
      const isCurrentSource = () =>
        liveSourcesRef.current.some(
          (device) =>
            configurationKey(device) === configurationKey(source) && device.ingestable,
        );
      if (!isCurrentSource()) {
        setConfigurationError(
          "This card is no longer connected. Close this dialog and select the current card.",
        );
        return;
      }
      if (activeOperations[source.id]) {
        setConfigurationError(
          "Wait for this card's ingest to finish before saving settings.",
        );
        return;
      }
      const error = validateConfiguration(draft);
      if (error) {
        setConfigurationError(error);
        return;
      }
      if (draft.persistOnCard && !isTauri()) {
        setConfigurationError(
          "Saving settings on a card is available in the desktop application. Turn off card persistence to save preview settings for this session.",
        );
        return;
      }
      const saved = cloneConfiguration(draft);
      saved.destinationPath = saved.destinationPath.trim();
      const request = resolveSortRequest(
        saved.sortPreset,
        saved.customIntervalMinutes,
      )!;
      saved.destinationDepthOrder = normalizeDestinationDepthOrder(
        saved.destinationDepthOrder,
        defaultDestinationDepthOrder(request, saved.customDirectoryFields),
      );
      setConfigurationError("");
      savingConfigurationSource.current = source;
      setIsSavingConfiguration(true);
      try {
        if (saved.persistOnCard) {
          const response = await invoke<{
            data: CardRegistration | null;
            error: { message: string } | null;
          }>("register_card_marker", {
            request: {
              sourceMediumKey: source.id,
              destinationPath: saved.destinationPath,
              sortMode: request.sortMode,
              intervalMinutes: request.intervalMinutes,
              captureTimeBasis: saved.captureTimeBasis,
              customDirectoryFields: saved.customDirectoryFields,
              destinationDepthOrder: saved.destinationDepthOrder,
              autoIngestEnabled: saved.autoIngestEnabled,
              autoFormatEnabled: saved.autoIngestEnabled && saved.autoFormatEnabled,
            },
          });
          if (!response.data?.registered) {
            setConfigurationError(
              response.error?.message ??
                "Configuration could not be saved. Your draft is still available.",
            );
            return;
          }
          // Saving enables future mounts; it must not itself start a copy now.
          hydratedProfiles.current.add(configurationKey(source));
          autoIngestAttempted.current.add(
            source.id + ":" + source.connectionGeneration,
          );
        }
        if (!isCurrentSource()) {
          setConfigurationError(
            "The save was confirmed, but this card disconnected or changed. Close this dialog and reload the current card's settings.",
          );
          return;
        }
        // Invalidate pending previews before applying a new configuration, even
        // when a later save restores the same settings.
        latestPreviewRequests.current.delete(source.id);
        appliedConfigurationsRef.current = {
          ...appliedConfigurationsRef.current,
          [configurationKey(source)]: saved,
        };
        setConfigurations((current) => ({
          ...current,
          [configurationKey(source)]: saved,
        }));
        setPlanPreviews((current) => {
          const next = { ...current };
          delete next[source.id];
          return next;
        });
        setConfigurationLabels((current) => ({
          ...current,
          [configurationKey(source)]: saved.persistOnCard
            ? "Configuration saved for this card. Automation applies on future mounts."
            : isTauri()
              ? "Configuration saved for this session."
              : "Preview configuration saved for this session.",
        }));
        if (saved.persistOnCard) {
          try {
            const snapshot = await invoke<{ data: NativeDeviceSnapshot | null }>(
              "get_device_snapshot",
            );
            if (snapshot.data) {
              // A newly written marker may change the mutable discovery key.
              const matching = snapshot.data.devices.filter(
                (device) =>
                  device.state === "available" &&
                  device.details.mountLocations.includes(source.sourceRoot),
              );
              if (matching.length === 1) {
                const refreshed = asFixture(matching[0]);
                autoIngestAttempted.current.add(
                  refreshed.id + ":" + refreshed.connectionGeneration,
                );
              }
              applyNativeSnapshot(snapshot.data);
            }
          } catch {
            /* The save is confirmed; the inventory watcher will reconcile. */
          }
        }
        setConfiguration(null);
      } catch {
        setConfigurationError(
          "The desktop service could not confirm the save. Your draft is still available; retry when connected.",
        );
      } finally {
        savingConfigurationSource.current = null;
        setIsSavingConfiguration(false);
      }
    },
    [configuration, activeOperations, applyNativeSnapshot],
  );

  const checkFormatReadiness = useCallback(
    async (request: {
      runId: string;
      sourceMediumKey: string;
      sourceGeneration: number;
      sourceIdentityConfidence: string;
    }) => {
      try {
        const response = await invoke<{
          data: FormatEligibility | null;
          error: { message: string } | null;
        }>("get_format_eligibility", { request });
        if (response.data) {
          setFormatEligibility((current) => ({
            ...current,
            [request.sourceMediumKey]: response.data!,
          }));
        }
        setFormatReadinessLabels((current) => ({
          ...current,
          [request.sourceMediumKey]:
            response.data?.reason ??
            response.error?.message ??
            "Format readiness could not be checked.",
        }));
      } catch {
        setFormatReadinessLabels((current) => ({
          ...current,
          [request.sourceMediumKey]: "Format readiness could not be checked.",
        }));
      }
    },
    [],
  );
  const requestFormatAuthorization = useCallback(async () => {
    if (!selected || !selectedCompletedRun) {
      return;
    }
    if (!isTauri()) {
      setFormatReadinessLabels((current) => ({
        ...current,
        [selected.id]: "Quick format is available only in the desktop application.",
      }));
      return;
    }
    setFormatReadinessLabels((current) => ({
      ...current,
      [selected.id]: "Checking the current quick-format safety gate…",
    }));
    try {
      const eligibility = await invoke<{
        data: FormatEligibility | null;
        error: { message: string } | null;
      }>("get_format_eligibility", {
        request: {
          runId: selectedCompletedRun.runId,
          sourceMediumKey: selected.id,
          sourceGeneration: selectedCompletedRun.sourceGeneration,
          sourceIdentityConfidence: selectedCompletedRun.sourceIdentityConfidence,
        },
      });
      if (!eligibility.data?.eligible) {
        setFormatEligibility((current) => ({
          ...current,
          [selected.id]: eligibility.data ?? {
            eligible: false,
            reason:
              eligibility.error?.message ?? "Format readiness could not be checked.",
            recommendedProfile: null,
          },
        }));
        setFormatReadinessLabels((current) => ({
          ...current,
          [selected.id]:
            eligibility.data?.reason ??
            eligibility.error?.message ??
            "Format readiness could not be checked.",
        }));
        return;
      }
      setFormatEligibility((current) => ({
        ...current,
        [selected.id]: eligibility.data!,
      }));
      setFormatReadinessLabels((current) => ({
        ...current,
        [selected.id]: "Preparing a one-time quick-format confirmation…",
      }));
      const response = await invoke<{
        data: FormatAuthorization | null;
        error: { message: string } | null;
      }>("request_format_authorization", {
        request: {
          runId: selectedCompletedRun.runId,
          sourceMediumKey: selected.id,
          sourceGeneration: selectedCompletedRun.sourceGeneration,
          sourceIdentityConfidence: selectedCompletedRun.sourceIdentityConfidence,
        },
      });
      if (!response.data) {
        setFormatReadinessLabels((current) => ({
          ...current,
          [selected.id]:
            response.error?.message ?? "Quick-format authorization was not issued.",
        }));
        return;
      }
      setPendingFormatConfirmation({
        confirmationToken: response.data.confirmationToken,
        deviceId: selected.id,
        deviceName: selected.name,
        media: selected.media,
        capacity: selected.capacity,
        receiptId: selectedCompletedRun.runId,
        profile: eligibility.data.recommendedProfile,
      });
      setFormatReadinessLabels((current) => ({ ...current, [selected.id]: "" }));
    } catch {
      setFormatReadinessLabels((current) => ({
        ...current,
        [selected.id]: "The desktop service could not prepare quick format.",
      }));
    }
  }, [selected, selectedCompletedRun]);
  const executeFormatAuthorization = useCallback(async () => {
    const confirmation = pendingFormatConfirmation;
    if (!confirmation || isExecutingFormat) return;
    setIsExecutingFormat(true);
    try {
      const response = await invoke<{
        data: FormatExecutionResult | null;
        error: { message: string } | null;
      }>("execute_format_authorization", {
        request: { confirmationToken: confirmation.confirmationToken },
      });
      if (!response.data) {
        setFormatReadinessLabels((current) => ({
          ...current,
          [confirmation.deviceId]:
            response.error?.message ?? "Quick format did not complete.",
        }));
        return;
      }
      setFormatReadinessLabels((current) => ({
        ...current,
        [confirmation.deviceId]: `Quick format completed: ${response.data!.profileId}; formatted and writable${response.data!.markerRestored ? "; card registration restored" : "; card registration was not restored"}.`,
      }));
      setPendingFormatConfirmation(null);
      await Promise.all([refreshDevices(), refreshHistory()]);
    } catch (error) {
      setFormatReadinessLabels((current) => ({
        ...current,
        [confirmation.deviceId]:
          error instanceof Error
            ? `The desktop service could not complete quick format: ${error.message}`
            : "The desktop service could not complete quick format.",
      }));
    } finally {
      setIsExecutingFormat(false);
    }
  }, [isExecutingFormat, pendingFormatConfirmation, refreshDevices, refreshHistory]);
  const executeForceReformat = useCallback(async () => {
    const confirmation = pendingForceReformat;
    if (!confirmation || !selected || isExecutingFormat) return;
    setIsExecutingFormat(true);
    try {
      const authorization = await invoke<{
        data: FormatAuthorization | null;
        error: { message: string } | null;
      }>("request_force_format_authorization", {
        request: {
          sourceMediumKey: selected.id,
          sourceGeneration: selected.connectionGeneration,
          confirmationPhrase: forceReformatPhrase,
        },
      });
      if (!authorization.data) {
        const message =
          authorization.error?.message ?? "Force reformat was not authorized.";
        setFormatReadinessLabels((current) => ({
          ...current,
          [selected.id]: message,
        }));
        setForceReformatError(message);
        return;
      }
      const response = await invoke<{
        data: FormatExecutionResult | null;
        error: { message: string } | null;
      }>("execute_format_authorization", {
        request: { confirmationToken: authorization.data.confirmationToken },
      });
      const message = response.data
        ? `Force reformat completed: ${response.data.profileId}; formatted and writable${response.data.markerRestored ? "; card registration restored" : ""}.`
        : (response.error?.message ?? "Force reformat did not complete.");
      setFormatReadinessLabels((current) => ({
        ...current,
        [selected.id]: message,
      }));
      if (!response.data) {
        setForceReformatError(message);
      }
      if (response.data) {
        setPendingForceReformat(null);
        setForceReformatPhrase("");
        await Promise.all([refreshDevices(), refreshHistory()]);
      }
    } catch (error) {
      const message =
        error instanceof Error
          ? `The desktop service could not complete force reformat: ${error.message}`
          : "The desktop service could not complete force reformat.";
      setFormatReadinessLabels((current) => ({
        ...current,
        [selected.id]: message,
      }));
      setForceReformatError(message);
    } finally {
      setIsExecutingFormat(false);
    }
  }, [
    forceReformatPhrase,
    isExecutingFormat,
    pendingForceReformat,
    refreshDevices,
    refreshHistory,
    selected,
  ]);

  useEffect(() => {
    if (
      !selected?.id ||
      !selectedCompletedRunId ||
      selectedCompletedRunGeneration === undefined ||
      !selectedCompletedRunConfidence ||
      !isTauri()
    ) {
      return;
    }
    void checkFormatReadiness({
      runId: selectedCompletedRunId,
      sourceMediumKey: selected.id,
      sourceGeneration: selectedCompletedRunGeneration,
      sourceIdentityConfidence: selectedCompletedRunConfidence,
    });
  }, [
    checkFormatReadiness,
    selected?.id,
    selectedCompletedRunId,
    selectedCompletedRunGeneration,
    selectedCompletedRunConfidence,
  ]);

  useEffect(() => {
    if (!isTauri()) return;
    if (storageWatchStarted.current) {
      return;
    }
    storageWatchStarted.current = true;
    const updates = new Channel<NativeDeviceSnapshot>();
    updates.onmessage = applyNativeSnapshot;
    void invoke<{ error: { message: string } | null }>("watch_device_snapshots", {
      channel: updates,
    })
      .then((response) => {
        if (response.error) setScanLabel(response.error.message);
      })
      .catch(() => {
        setScanLabel("Native storage change monitoring could not start.");
      })
      .finally(() => {
        // Native subscriptions are registered before this first explicit
        // snapshot, so a card inserted during startup cannot race the watcher.
        void refreshDevices();
        void refreshHistory();
      });
  }, [applyNativeSnapshot, refreshDevices, refreshHistory]);
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    void listen("media-ingest://close-requested", () => {
      setIsCloseConfirmationOpen(true);
    }).then((unsubscribe) => {
      unlisten = unsubscribe;
    });
    return () => unlisten?.();
  }, []);
  useEffect(() => {
    if (!isTauri()) return;
    let unlistenReactivated: (() => void) | undefined;
    void listen("media-ingest://reactivated", () => {
      setScanLabel("App reactivated; rechecking removable storage and local history…");
      void refreshDevices();
      void refreshHistory();
    }).then((unsubscribe) => {
      unlistenReactivated = unsubscribe;
    });
    return () => {
      unlistenReactivated?.();
    };
  }, [refreshDevices, refreshHistory]);
  const startIngest = useCallback(async () => {
    if (!selected) return;
    const sortRequest = resolveSortRequest(sortPreset, customIntervalMinutes);
    if (!sortRequest) {
      setIngestLabels((current) => ({
        ...current,
        [selected.id]: "Enter a whole custom interval from 1 to 1,440 minutes.",
      }));
      return;
    }
    if (!customDirectoryFieldsAreComplete(customDirectoryFields)) {
      setIngestLabels((current) => ({
        ...current,
        [selected.id]: "Complete or remove each custom folder field before ingesting.",
      }));
      return;
    }
    if (!isTauri()) {
      setIngestLabels((current) => ({
        ...current,
        [selected.id]: "Verified ingest is available in the desktop application.",
      }));
      return;
    }
    if (!selected.ingestable || !selected.sourceRoot) {
      setIngestLabels((current) => ({
        ...current,
        [selected.id]: "Insert mounted media before starting an ingest.",
      }));
      return;
    }
    if (!destination.trim()) {
      setIngestLabels((current) => ({
        ...current,
        [selected.id]: "Enter a destination directory before starting an ingest.",
      }));
      return;
    }
    if (activeOperations[selected.id]) {
      return;
    }
    const operationId = selectedPlanPreview?.operationId ?? crypto.randomUUID();
    const progress = new Channel<ProgressUpdate>();
    let copyStartedAt: number | null = null;
    let latestTransferred = 0;
    progress.onmessage = (update) => {
      if (update.operationId !== operationId) return;
      trackOperationProgress(selected.id, update);
      const total = update.totalBytes;
      latestTransferred = Math.max(latestTransferred, update.transferredBytes);
      if (update.state === "copying") {
        if (latestTransferred > 0) copyStartedAt ??= performance.now();
        const elapsedSeconds =
          copyStartedAt === null ? 0 : (performance.now() - copyStartedAt) / 1_000;
        const bytesPerSecond = latestTransferred / elapsedSeconds;
        const detail =
          total === null
            ? ` ${formatTransferBytes(latestTransferred)}`
            : ` ${formatTransferBytes(latestTransferred)} / ${formatTransferBytes(total)}`;
        const rate =
          Number.isFinite(bytesPerSecond) && bytesPerSecond > 0
            ? ` · average ${(bytesPerSecond / 1_000_000).toFixed(1)} MB/s`
            : "";
        const eta =
          total !== null && Number.isFinite(bytesPerSecond) && bytesPerSecond > 0
            ? ` · ETA ${formatEta(Math.max(0, total - latestTransferred) / bytesPerSecond)}`
            : "";
        const filePosition =
          update.currentFileIndex !== null &&
          update.currentFileIndex !== undefined &&
          update.totalFiles !== null &&
          update.totalFiles !== undefined
            ? ` · file ${update.currentFileIndex} of ${update.totalFiles}`
            : "";
        setIngestLabels((current) => ({
          ...current,
          [selected.id]: `Copying${detail}${filePosition}${rate}${eta}`,
        }));
        return;
      }
      const detail =
        total === null
          ? ""
          : ` ${formatTransferBytes(latestTransferred)} / ${formatTransferBytes(total)}`;
      const filePosition =
        update.currentFileIndex !== null &&
        update.currentFileIndex !== undefined &&
        update.totalFiles !== null &&
        update.totalFiles !== undefined
          ? ` · file ${update.currentFileIndex} of ${update.totalFiles}`
          : "";
      setIngestLabels((current) => ({
        ...current,
        [selected.id]: `${update.state.replace(/_/g, " ")}${detail}${filePosition}`,
      }));
    };
    setActiveOperations((current) => ({
      ...current,
      [selected.id]: {
        operationId,
        cancelling: false,
        state: "queued",
        transferredBytes: 0,
        totalBytes: null,
        currentFileIndex: null,
        totalFiles: null,
        startedAt: Date.now(),
        bytesPerSecond: 0,
        logs: ["Queued · waiting for the native ingest worker"],
      },
    }));
    setIngestLabels((current) => ({
      ...current,
      [selected.id]: "Copying with independent destination readback…",
    }));
    try {
      const response = await invoke<{
        data: IngestResult | null;
        error: { message: string } | null;
      }>("start_verified_ingest", {
        request: {
          operationId,
          sourceRoot: selected.sourceRoot,
          destinationRoot: destination.trim(),
          sourceMediumKey: selected.id,
          sourceIdentityConfidence: selected.nativeConfidence,
          sourceGeneration: selected.connectionGeneration,
          maxWorkers: 2,
          sortMode: sortRequest.sortMode,
          intervalMinutes: sortRequest.intervalMinutes,
          captureTimeBasis,
          customDirectoryFields,
          destinationDepthOrder: selectedDestinationDepthOrder,
          autoIngestTriggered: false,
        },
        channel: progress,
      });
      if (response.data) {
        const result = response.data;
        setCompletedRuns((current) => ({
          ...current,
          [selected.id]: {
            runId: result.operationId,
            sourceGeneration: selected.connectionGeneration,
            sourceIdentityConfidence: selected.nativeConfidence,
          },
        }));
        const markerLabel =
          result.sourceMarkerStatus === "created"
            ? " · card marker created"
            : result.sourceMarkerStatus === "recognized"
              ? " · card marker recognized"
              : " · card marker unavailable";
        setIngestLabels((current) => ({
          ...current,
          [selected.id]: `${result.copiedFiles} files verified · receipt ${result.receiptName}${markerLabel}`,
        }));
        void checkFormatReadiness({
          runId: result.operationId,
          sourceMediumKey: selected.id,
          sourceGeneration: selected.connectionGeneration,
          sourceIdentityConfidence: selected.nativeConfidence,
        });
        void refreshHistory();
        // Marker creation can change the non-authoritative continuity key.
        // Re-read native inventory before another card action uses a stale
        // key/generation pair from the pre-marker snapshot.
        void refreshDevices();
      } else {
        setIngestLabels((current) => ({
          ...current,
          [selected.id]: response.error?.message ?? "The ingest did not complete.",
        }));
      }
    } catch {
      setIngestLabels((current) => ({
        ...current,
        [selected.id]: "The desktop service could not start the ingest.",
      }));
    } finally {
      setActiveOperations((current) => {
        const next = { ...current };
        delete next[selected.id];
        return next;
      });
    }
  }, [
    activeOperations,
    checkFormatReadiness,
    destination,
    refreshDevices,
    refreshHistory,
    trackOperationProgress,
    selected,
    selectedPlanPreview,
    customDirectoryFields,
    customIntervalMinutes,
    selectedDestinationDepthOrder,
    sortPreset,
    captureTimeBasis,
  ]);
  useEffect(() => {
    if (!isTauri()) return;
    for (const device of liveDevices) {
      const attemptKey = `${device.id}:${device.connectionGeneration}`;
      if (
        !device.ingestable ||
        savingConfigurationSource.current?.sourceRoot === device.sourceRoot ||
        (autoIngestAttempted.current.has(attemptKey) &&
          hydratedProfiles.current.has(configurationKey(device))) ||
        autoIngestProfileChecks.current.has(attemptKey)
      )
        continue;
      autoIngestProfileChecks.current.add(attemptKey);
      void invoke<{
        data: CardRegistration | null;
        error: { message: string } | null;
      }>("get_auto_ingest_profile", { sourceMediumKey: device.id })
        .then(async (response) => {
          const profile = response.data;
          if (response.error) {
            setConfigurationLabels((current) => ({
              ...current,
              [configurationKey(device)]: response.error!.message,
            }));
            return;
          }
          if (savingConfigurationSource.current?.sourceRoot === device.sourceRoot)
            return;
          if (profile?.registered)
            hydratedProfiles.current.add(configurationKey(device));
          if (profile?.registered)
            setConfigurations((current) =>
              current[configurationKey(device)]
                ? current
                : {
                    ...current,
                    [configurationKey(device)]: configurationFromRegistration(profile),
                  },
            );
          if (
            autoIngestAttempted.current.has(attemptKey) ||
            !profile?.registered ||
            !profile.autoIngestEnabled ||
            !profile.destinationPath
          )
            return;
          // From this point onward the native profile has authorized exactly
          // one attempt for this observed insertion.  In particular, an
          // already-completed result is terminal too.
          autoIngestAttempted.current.add(attemptKey);
          if (profile.autoIngestAlreadyCompleted) {
            setIngestLabels((current) => ({
              ...current,
              [device.id]: "Registered card already auto-ingested for this mount.",
            }));
            return;
          }
          if (activeOperations[device.id]) return;
          const operationId = crypto.randomUUID();
          const progress = new Channel<ProgressUpdate>();
          progress.onmessage = (update) => {
            if (update.operationId !== operationId) return;
            trackOperationProgress(device.id, update);
            const filePosition =
              update.currentFileIndex !== null &&
              update.currentFileIndex !== undefined &&
              update.totalFiles !== null &&
              update.totalFiles !== undefined
                ? ` · file ${update.currentFileIndex} of ${update.totalFiles}`
                : "";
            setIngestLabels((current) => ({
              ...current,
              [device.id]: `Auto-ingest ${update.state.replace(/_/g, " ")} ${formatTransferBytes(update.transferredBytes)}${update.totalBytes === null ? "" : ` / ${formatTransferBytes(update.totalBytes)}`}${filePosition}`,
            }));
          };
          setActiveOperations((current) => ({
            ...current,
            [device.id]: {
              operationId,
              cancelling: false,
              state: "queued",
              transferredBytes: 0,
              totalBytes: null,
              currentFileIndex: null,
              totalFiles: null,
              startedAt: Date.now(),
              bytesPerSecond: 0,
              logs: ["Queued · registered media detected"],
            },
          }));
          setIngestLabels((current) => ({
            ...current,
            [device.id]: "Registered card detected; starting verified auto-ingest…",
          }));
          try {
            const response = await invoke<{
              data: IngestResult | null;
              error: { message: string } | null;
            }>("start_verified_ingest", {
              request: {
                operationId,
                sourceRoot: device.sourceRoot,
                destinationRoot: profile.destinationPath,
                sourceMediumKey: device.id,
                sourceIdentityConfidence: device.nativeConfidence,
                sourceGeneration: device.connectionGeneration,
                maxWorkers: 2,
                sortMode: profile.sortMode ?? "camera_day",
                intervalMinutes:
                  profile.sortMode === "camera_interval"
                    ? profile.intervalMinutes
                    : null,
                captureTimeBasis: profile.captureTimeBasis ?? "camera_recorded",
                customDirectoryFields: profile.customDirectoryFields ?? [],
                destinationDepthOrder: profile.destinationDepthOrder?.length
                  ? profile.destinationDepthOrder
                  : undefined,
                autoIngestTriggered: true,
              },
              channel: progress,
            });
            setIngestLabels((current) => ({
              ...current,
              [device.id]: response.data
                ? `${response.data.copiedFiles} files verified by auto-ingest · receipt ${response.data.receiptName}${response.data.autoFormatStatus === "completed" ? " · card formatted and marker restored" : response.data.autoFormatStatus === "failed" ? " · format or marker restoration failed" : response.data.autoFormatStatus === "skipped" ? " · auto-format skipped by the native safety gate" : ""}`
                : (response.error?.message ?? "Auto-ingest did not complete."),
            }));
            if (response.data) void refreshHistory();
            if (response.data) {
              setCompletedRuns((current) => ({
                ...current,
                [device.id]: {
                  runId: response.data!.operationId,
                  sourceGeneration: device.connectionGeneration,
                  sourceIdentityConfidence: device.nativeConfidence,
                },
              }));
            }
          } catch {
            setIngestLabels((current) => ({
              ...current,
              [device.id]: "The desktop service could not start auto-ingest.",
            }));
          } finally {
            setActiveOperations((current) => {
              const next = { ...current };
              delete next[device.id];
              return next;
            });
          }
        })
        .catch(() => {
          setConfigurationLabels((current) => ({
            ...current,
            [configurationKey(device)]:
              "Saved card settings are unavailable. Retry configuration when connected.",
          }));
        })
        .finally(() => {
          autoIngestProfileChecks.current.delete(attemptKey);
        });
    }
  }, [activeOperations, liveDevices, refreshHistory, trackOperationProgress]);
  const requestSafeEject = useCallback(async () => {
    if (!selected || !selectedCompletedRun) return;
    if (!isTauri()) {
      setEjectLabels((current) => ({
        ...current,
        [selected.id]: "Safe eject is available in the desktop application.",
      }));
      return;
    }
    setEjectLabels((current) => ({
      ...current,
      [selected.id]: "Requesting safe eject…",
    }));
    try {
      const response = await invoke<{
        data: { sourceMediumKey: string; sourceGeneration: number } | null;
        error: { message: string } | null;
      }>("safe_eject", {
        request: {
          runId: selectedCompletedRun.runId,
          sourceMediumKey: selected.id,
          sourceGeneration: selectedCompletedRun.sourceGeneration,
          sourceIdentityConfidence: selectedCompletedRun.sourceIdentityConfidence,
        },
      });
      setEjectLabels((current) => ({
        ...current,
        [selected.id]: response.data
          ? "The operating system confirmed the eject request. You may remove the card."
          : (response.error?.message ??
            "Safe eject was not confirmed; leave the card connected."),
      }));
      if (response.data) await refreshDevices();
    } catch {
      setEjectLabels((current) => ({
        ...current,
        [selected.id]: "Safe eject was not confirmed; leave the card connected.",
      }));
    }
  }, [refreshDevices, selected, selectedCompletedRun]);
  const cancelIngest = useCallback(async () => {
    if (!selected || !selectedOperation) return;
    setActiveOperations((current) => ({
      ...current,
      [selected.id]: { ...selectedOperation, cancelling: true },
    }));
    setIngestLabels((current) => ({
      ...current,
      [selected.id]: "Stopping after the current safe copy or verification chunk…",
    }));
    try {
      const response = await invoke<{ error: { message: string } | null }>(
        "cancel_verified_ingest",
        { operationId: selectedOperation.operationId },
      );
      if (response.error) {
        setIngestLabels((current) => ({
          ...current,
          [selected.id]: response.error!.message,
        }));
        setActiveOperations((current) => ({
          ...current,
          [selected.id]: { ...selectedOperation, cancelling: false },
        }));
      }
    } catch {
      setIngestLabels((current) => ({
        ...current,
        [selected.id]: "The cancellation request could not reach the native worker.",
      }));
      setActiveOperations((current) => ({
        ...current,
        [selected.id]: { ...selectedOperation, cancelling: false },
      }));
    }
  }, [selected, selectedOperation]);
  const cancelActiveIngestsForClose = useCallback(async () => {
    const operations = Object.entries(activeOperations);
    if (!operations.length) {
      setIsCloseConfirmationOpen(false);
      return;
    }
    setIsCancellingForClose(true);
    await Promise.all(
      operations.map(async ([deviceId, operation]) => {
        setActiveOperations((current) => ({
          ...current,
          [deviceId]: { ...operation, cancelling: true },
        }));
        setIngestLabels((current) => ({
          ...current,
          [deviceId]:
            "Stopping for close after the current safe copy or verification chunk…",
        }));
        try {
          const response = await invoke<{ error: { message: string } | null }>(
            "cancel_verified_ingest",
            { operationId: operation.operationId },
          );
          if (response.error) {
            setIngestLabels((current) => ({
              ...current,
              [deviceId]: response.error!.message,
            }));
          }
        } catch {
          setIngestLabels((current) => ({
            ...current,
            [deviceId]: "The cancellation request could not reach the native worker.",
          }));
        }
      }),
    );
    setIsCancellingForClose(false);
    setIsCloseConfirmationOpen(false);
  }, [activeOperations]);
  const calibrateReaderSlot = useCallback(async () => {
    if (!selected?.readerFingerprint || selected.logicalUnit === null) return;
    if (!isTauri()) {
      setCalibrationLabel(
        "Reader-slot calibration is available in the desktop application.",
      );
      return;
    }
    setCalibrationLabel("Saving controlled-insertion calibration…");
    try {
      const response = await invoke<{ data: null; error: { message: string } | null }>(
        "calibrate_reader_slot",
        {
          request: {
            readerFingerprint: selected.readerFingerprint,
            logicalUnit: selected.logicalUnit,
            slotKind: calibrationKind,
            evidenceNote: `Operator confirmed ${calibrationKind === "sd" ? "SD" : "microSD"} card in this logical unit`,
          },
        },
      );
      if (response.error) {
        setCalibrationLabel(response.error.message);
        return;
      }
      setCalibrationLabel("Calibration saved. Inventory is refreshing…");
      await refreshDevices();
    } catch {
      setCalibrationLabel("The desktop service could not save this calibration.");
    }
  }, [calibrationKind, refreshDevices, selected]);
  const configurationDialog = configuration ? (
    <CardConfiguration
      key={configurationKey(configuration.source)}
      sourceName={configuration.source.name}
      initialValue={configuration.initialValue}
      saving={isSavingConfiguration}
      error={configurationError}
      sourceUnavailable={
        !liveDevices.some(
          (device) =>
            configurationKey(device) === configurationKey(configuration.source) &&
            device.ingestable,
        ) || Boolean(activeOperations[configuration.source.id])
      }
      onSave={(draft) => void saveConfiguration(draft)}
      onDismiss={() => {
        if (!savingConfigurationSource.current) {
          setConfiguration(null);
          setConfigurationError("");
        }
      }}
      onChooseDestination={chooseConfigurationDestination}
    />
  ) : null;
  if (!selected) {
    return (
      <main className="grid min-h-screen place-content-center gap-4 bg-slate-50 p-8 text-center text-slate-950 dark:bg-slate-950 dark:text-slate-50">
        <h1 className="text-xl font-semibold tracking-tight">Ingest Station</h1>
        <p className="text-sm text-slate-500 dark:text-slate-400">
          No removable media is mounted. Insert a card and refresh inventory.
        </p>
        <button
          className="rounded-lg bg-blue-600 px-4 py-2 text-sm font-semibold text-white shadow-sm transition hover:bg-blue-500"
          onClick={() => void refreshDevices()}
          type="button"
        >
          Refresh Inventory
        </button>
        {isCloseConfirmationOpen ? (
          <Modal
            labelledBy="close-confirmation-title"
            className="fixed inset-0 z-50 grid place-items-center overscroll-contain bg-slate-950/40 p-4 backdrop-blur-sm"
            dismissible={!isCancellingForClose}
            onDismiss={() => setIsCloseConfirmationOpen(false)}
          >
            <section className="w-full max-w-md rounded-2xl border border-slate-200 bg-white p-6 shadow-2xl dark:border-slate-700 dark:bg-slate-900">
              <p className="mb-1 text-[10px] font-bold tracking-[0.14em] text-amber-600 dark:text-amber-300">
                ACTIVE VERIFIED INGEST
              </p>
              <h2
                id="close-confirmation-title"
                className="text-2xl font-semibold tracking-tight"
              >
                Keep This App Open Until Copying Stops
              </h2>
              <p className="mt-3 text-sm leading-6 text-slate-500 dark:text-slate-400">
                Closing now could interrupt a copy or verification. You can keep the app
                open, or request a safe cancellation; the affected run will stay
                recoverable and must not be treated as transferred.
              </p>
              <div className="mt-6 flex justify-end gap-3">
                <button
                  className="rounded-lg px-3 py-2 text-xs font-semibold text-slate-600 transition hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800"
                  disabled={isCancellingForClose}
                  onClick={() => setIsCloseConfirmationOpen(false)}
                  type="button"
                >
                  Keep Ingesting
                </button>
                <button
                  className="rounded-lg bg-amber-600 px-3 py-2 text-xs font-semibold text-white shadow-sm transition hover:bg-amber-500 disabled:cursor-wait disabled:opacity-60"
                  disabled={isCancellingForClose}
                  onClick={() => void cancelActiveIngestsForClose()}
                  type="button"
                >
                  {isCancellingForClose ? "Requesting Cancellation…" : "Cancel Ingests"}
                </button>
              </div>
            </section>
          </Modal>
        ) : null}
        {configurationDialog}
      </main>
    );
  }
  const isCopying = Boolean(selectedOperation) || selected.transfer === "Copying";

  return (
    <main
      className={`ingest-shell min-h-screen bg-slate-50 text-slate-950 transition-colors duration-200 dark:bg-slate-950 dark:text-slate-50 ${dockOperations.length ? "ingest-shell--dock" : ""} ${dockOperations.length && isIngestDockOpen ? "ingest-shell--dock-expanded" : ""}`}
    >
      <a className="skip-to-workspace" href="#ingest-workspace">
        Skip to workspace
      </a>
      <header className="ingest-topbar flex h-[72px] items-center justify-between border-b border-slate-200 px-5 sm:px-8 dark:border-slate-800">
        <div className="flex items-center gap-3">
          <div
            className="grid size-8 place-items-center rounded-lg bg-blue-600 text-sm font-bold text-white shadow-sm"
            aria-hidden="true"
          >
            <HardDriveDownload className="size-5" strokeWidth={2.25} />
          </div>
          <h1 className="text-base font-semibold tracking-tight">Media Ingest Tool</h1>
        </div>
        <div className="flex items-center gap-3">
          <span className="hidden items-center gap-2 text-xs text-slate-500 sm:flex dark:text-slate-400">
            <StatusDot tone="ready" /> Host Ready
          </span>
          <button
            aria-label={`Switch to ${theme === "light" ? "dark" : "light"} mode`}
            className="rounded-lg border border-slate-200 bg-white px-3 py-2 text-xs font-semibold text-slate-700 shadow-sm transition hover:border-blue-300 hover:text-blue-700 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200 dark:hover:border-blue-600 dark:hover:text-blue-300"
            onClick={() =>
              setTheme((current) => (current === "light" ? "dark" : "light"))
            }
            type="button"
          >
            {theme === "light" ? "Dark Mode" : "Light Mode"}
          </button>
        </div>
      </header>
      <section
        className="grid min-h-[calc(100vh-114px)] grid-cols-1 lg:grid-cols-[19rem_minmax(0,1fr)]"
        aria-label="Media ingest workspace"
      >
        <aside
          className="source-rail border-b border-slate-200 bg-white px-4 py-6 dark:border-slate-800 dark:bg-slate-900/40 lg:border-r lg:border-b-0"
          aria-labelledby="connected-media-title"
        >
          <div className="flex items-start justify-between gap-4">
            <div>
              <p className="mb-1 text-[10px] font-bold tracking-[0.14em] text-slate-400 dark:text-slate-500">
                LIVE INVENTORY
              </p>
              <h2
                id="connected-media-title"
                className="text-base font-semibold tracking-tight"
              >
                Connected Media
              </h2>
            </div>
            <span className="grid size-6 place-items-center rounded-md bg-blue-50 text-xs font-semibold text-blue-700 dark:bg-blue-500/10 dark:text-blue-300">
              {liveDevices.length}
            </span>
          </div>
          <div className="mt-5 flex items-start gap-2 text-xs leading-5 text-slate-500 dark:text-slate-400">
            <StatusDot tone="ready" />
            <span>{scanLabel}</span>
          </div>
          <ul
            className="mt-4 grid list-none gap-2 p-0 sm:grid-cols-2 lg:grid-cols-1"
            aria-label="Connected media"
          >
            {liveDevices.map((device) => {
              const selectedCard = device.id === selected.id;
              const transferTone = device.transfer === "Copying" ? "copying" : "ready";
              return (
                <li key={device.id} className="min-w-0">
                  <button
                    aria-pressed={selectedCard}
                    className={`flex w-full gap-3 rounded-xl border p-3 text-left text-xs transition ${selectedCard ? "border-blue-200 bg-blue-50 shadow-sm dark:border-blue-500/40 dark:bg-blue-500/10" : "border-transparent hover:border-slate-200 hover:bg-slate-50 dark:hover:border-slate-700 dark:hover:bg-slate-800/70"}`}
                    onClick={() => setSelectedId(device.id)}
                    type="button"
                  >
                    <span
                      className="grid size-8 place-items-center rounded-lg bg-blue-100 text-blue-700 dark:bg-blue-500/15 dark:text-blue-300"
                      aria-hidden="true"
                    >
                      <HardDrive className="size-4" strokeWidth={2.25} />
                    </span>
                    <span className="grid min-w-0 flex-1 gap-1">
                      <span className="flex items-center justify-between gap-2 text-[13px] text-slate-900 dark:text-slate-100">
                        <strong>{device.name}</strong>
                        <StatusDot tone={transferTone} />
                      </span>
                      <span className="text-slate-500 dark:text-slate-400">
                        {device.capacity} · {device.available}
                      </span>
                      <span className="text-slate-500 dark:text-slate-400">
                        {device.sourceRoot
                          ? `${device.sourceRoot} · ${device.kind.replace("Removable ", "")}`
                          : device.kind}
                      </span>
                      <span className="text-slate-500 dark:text-slate-400">
                        {device.slot}
                      </span>
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
          <button
            className="mt-5 w-full border-t border-slate-200 px-1 py-4 text-left text-xs font-medium text-slate-500 transition hover:text-blue-700 dark:border-slate-800 dark:text-slate-400 dark:hover:text-blue-300"
            onClick={() => void refreshDevices()}
            type="button"
          >
            <RefreshCw
              aria-hidden="true"
              className="mr-2 inline-block size-3.5 text-blue-600 dark:text-blue-400"
              strokeWidth={2.25}
            />
            Rescan Connected Media
          </button>
        </aside>
        <section
          className="workspace-content mx-auto w-full max-w-7xl px-5 py-8 sm:px-8 sm:py-12"
          id="ingest-workspace"
          aria-labelledby="device-title"
        >
          <div className="flex flex-col justify-between gap-6 pb-8 md:flex-row">
            <div>
              <div className="mb-3 flex items-center gap-3">
                <span
                  className={`flex items-center gap-2 rounded-md px-2 py-1 text-[11px] font-semibold ${selected.confidence === "Verified" ? "bg-emerald-50 text-emerald-700 dark:bg-emerald-500/10 dark:text-emerald-300" : "bg-blue-50 text-blue-700 dark:bg-blue-500/10 dark:text-blue-300"}`}
                >
                  <StatusDot
                    tone={selected.confidence === "Verified" ? "verified" : "ready"}
                  />
                  {selected.confidence} Identity
                </span>
                <span className="text-xs text-slate-500 dark:text-slate-400">
                  {selected.updated === "Seen just now"
                    ? "Seen Just Now"
                    : selected.updated}
                </span>
              </div>
              <h2
                id="device-title"
                className="text-3xl font-semibold tracking-tight sm:text-4xl"
              >
                {selected.name}
              </h2>
              <p className="mt-2 text-sm text-slate-500 dark:text-slate-400">
                {selected.kind} · {selected.capacity}
              </p>
              <p className="mt-1 text-xs font-medium text-slate-600 dark:text-slate-300">
                {selected.sourceRoot
                  ? `Drive ${selected.sourceRoot} · ${selected.kind.replace("Removable ", "")}`
                  : "Drive Mount Unavailable"}
              </p>
            </div>
            <div className="flex flex-col items-start gap-2 md:items-end">
              <button
                className="rounded-lg border border-slate-200 bg-white px-3 py-2 text-xs font-semibold text-slate-700 disabled:cursor-not-allowed disabled:opacity-60 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200"
                disabled={!selectedCompletedRun || isCopying}
                onClick={() => void requestSafeEject()}
                title={
                  selectedCompletedRun
                    ? "Requires a sealed receipt, the current card mount, and no active ingest."
                    : "Complete a verified ingest for this card before requesting safe eject."
                }
                type="button"
              >
                Eject safely
              </button>
              {selectedEjectLabel ? (
                <span className="max-w-xs text-xs leading-5 text-slate-500 md:text-right dark:text-slate-400">
                  {selectedEjectLabel}
                </span>
              ) : null}
              <button
                className="rounded-lg border border-rose-200 bg-rose-50 px-3 py-2 text-xs font-semibold text-rose-700 disabled:cursor-not-allowed disabled:opacity-60 dark:border-rose-500/25 dark:bg-rose-500/10 dark:text-rose-300"
                type="button"
                disabled={!selectedCompletedRun || isCopying}
                onClick={() => void requestFormatAuthorization()}
                title={
                  selectedFormatEligibility?.eligible
                    ? "Opens a one-time confirmation for this verified card."
                    : "Rechecks the sealed receipt, current card, and native format provider before opening confirmation."
                }
              >
                {selectedFormatEligibility?.eligible
                  ? "Quick Format"
                  : "Check Quick Format"}
              </button>
              <span className="max-w-xs text-xs leading-5 text-slate-500 md:text-right dark:text-slate-400">
                {selectedFormatReadinessLabel ||
                  (selectedFormatEligibility?.eligible
                    ? "Eligible for a one-time quick-format confirmation."
                    : "Native format provider not installed.")}
              </span>
              <button
                className="rounded-lg border border-rose-300 bg-white px-3 py-2 text-xs font-semibold text-rose-800 disabled:cursor-not-allowed disabled:opacity-60 dark:border-rose-500/40 dark:bg-slate-900 dark:text-rose-200"
                disabled={!selected || isCopying}
                onClick={() => {
                  if (!selected) return;
                  setPendingForceReformat({
                    deviceId: selected.id,
                    deviceName: selected.name,
                    media: selected.media,
                    capacity: selected.capacity,
                  });
                  setForceReformatPhrase("");
                  setForceReformatError("");
                }}
                title="Destructive recovery action. It bypasses receipt, registration, and identity continuity, but still revalidates the removable target."
                type="button"
              >
                Force Reformat
              </button>
              <span className="max-w-xs text-xs leading-5 text-slate-500 md:text-right dark:text-slate-400">
                Bypasses receipt, registration, and identity continuity only after
                explicit confirmation; native removable-target checks still apply.
              </span>
            </div>
          </div>
          <div className="my-8 grid divide-y divide-slate-200 border-y border-slate-200 sm:grid-cols-3 sm:divide-x sm:divide-y-0 dark:divide-slate-800 dark:border-slate-800">
            <article className="py-4 sm:px-5 sm:first:pl-0">
              <p className="text-[11px] font-semibold uppercase tracking-[0.08em] text-slate-500 dark:text-slate-400">
                Media
              </p>
              <strong className="mt-2 block text-base font-semibold">
                {selectedInventory
                  ? `${selectedInventory.fileCount} files · ${formatTransferBytes(selectedInventory.totalBytes)}`
                  : selected.media}
              </strong>
              <span className="mt-2 block text-xs leading-5 text-slate-500 dark:text-slate-400">
                Read-only while copying; marker only after verification
              </span>
            </article>
            <article className="py-4 sm:px-5">
              <p className="text-[11px] font-semibold uppercase tracking-[0.08em] text-slate-500 dark:text-slate-400">
                Transfer
              </p>
              <strong className="mt-2 flex items-center gap-2 text-base font-semibold">
                <StatusDot tone={isCopying ? "copying" : "ready"} />{" "}
                {isCopying ? "Copying" : selected.transfer}
              </strong>
              <span className="mt-2 block text-xs leading-5 text-slate-500 dark:text-slate-400">
                Bounded device-aware workers
              </span>
            </article>
            <article className="py-4 sm:px-5">
              <p className="text-[11px] font-semibold uppercase tracking-[0.08em] text-slate-500 dark:text-slate-400">
                Verification
              </p>
              <strong className="mt-2 flex items-center gap-2 text-base font-semibold">
                <StatusDot
                  tone={selected.verification === "Verified" ? "verified" : "ready"}
                />{" "}
                {selected.verification}
              </strong>
              <span className="mt-2 block text-xs leading-5 text-slate-500 dark:text-slate-400">
                Fresh destination readback required
              </span>
            </article>
          </div>
          <div className="grid gap-8 lg:grid-cols-[minmax(0,1.2fr)_minmax(17rem,0.8fr)]">
            <article>
              <div>
                <div>
                  <p className="mb-1 text-[10px] font-bold tracking-[0.14em] text-slate-400 dark:text-slate-500">
                    INGEST PLAN
                  </p>
                  <h3 className="text-lg font-semibold tracking-tight">
                    Destination & Organization
                  </h3>
                </div>
              </div>
              <div
                className="mt-5 rounded-xl bg-slate-50 p-4 dark:bg-slate-800/60"
                aria-label="Saved configuration"
              >
                <p className="text-xs font-semibold text-slate-500 dark:text-slate-400">
                  Destination
                </p>
                <p className="mt-1 break-all font-mono text-sm text-slate-800 dark:text-slate-100">
                  {destination || "Choose a destination in Configure ingest."}
                </p>
                <div className="mt-4 border-t border-slate-200 pt-3 dark:border-slate-700">
                  <p className="text-xs font-semibold text-slate-500 dark:text-slate-400">
                    Folder organization
                  </p>
                  <p className="mt-1 text-sm leading-6 text-slate-700 dark:text-slate-200">
                    {selectedSortLevels.join(" / ")} / Filename
                  </p>
                  {sortPreset !== "original_tree" ? (
                    <p className="mt-2 text-xs text-slate-600 dark:text-slate-300">
                      Time basis:{" "}
                      {captureTimeBasis === "computer_local"
                        ? "Computer local time"
                        : "Camera time"}
                    </p>
                  ) : null}
                </div>
                <p className="mt-3 text-xs leading-5 text-slate-500 dark:text-slate-400">
                  {selectedConfiguration.autoIngestEnabled
                    ? "Automatic ingest on future mounts"
                    : "Manual ingest"}{" "}
                  ·{" "}
                  {selectedConfiguration.persistOnCard && destination
                    ? "Saved card settings"
                    : "Session settings"}
                </p>
              </div>
              <button
                className="mt-4 w-full rounded-lg border border-blue-200 bg-blue-50 px-4 py-3 text-sm font-semibold text-blue-700 transition hover:bg-blue-100 disabled:opacity-45 dark:border-blue-500/30 dark:bg-blue-500/10 dark:text-blue-300"
                disabled={isCopying || !selected.ingestable || isLoadingConfiguration}
                onClick={() => void openConfiguration()}
                type="button"
              >
                {isLoadingConfiguration
                  ? "Loading configuration…"
                  : destination
                    ? "Edit configuration"
                    : "Configure ingest"}
              </button>
              <p
                className="mt-2 min-h-5 text-xs leading-5 text-slate-500 dark:text-slate-400"
                aria-live="polite"
              >
                {selectedConfigurationLabel}
              </p>
              <div className="flex items-center justify-between gap-4 border-t border-slate-200 py-3 text-xs dark:border-slate-800">
                <span className="text-slate-500 dark:text-slate-400">
                  Camera identity
                </span>
                <strong className="font-semibold">Serial evidence required</strong>
              </div>
              <div className="mt-5 flex items-center gap-3">
                <button
                  className="rounded-lg border border-slate-200 bg-white px-3 py-2 text-xs font-semibold text-slate-700 shadow-sm transition hover:border-blue-300 hover:text-blue-700 disabled:cursor-not-allowed disabled:opacity-40 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200"
                  disabled={isCopying || !selected.ingestable}
                  onClick={() => void scanSourceMedia()}
                  type="button"
                >
                  Scan Media
                </button>
                <span
                  className="text-xs text-slate-500 dark:text-slate-400"
                  aria-live="polite"
                >
                  {selectedInventoryLabel}
                </span>
              </div>
              <div className="mt-3 flex items-center gap-3">
                <button
                  className="rounded-lg border border-slate-200 bg-white px-3 py-2 text-xs font-semibold text-slate-700 shadow-sm transition hover:border-blue-300 hover:text-blue-700 disabled:cursor-not-allowed disabled:opacity-40 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200"
                  disabled={isCopying || !selected.ingestable}
                  onClick={() => void previewIngestPlan()}
                  type="button"
                >
                  Preview Organization
                </button>
                <span
                  className="text-xs text-slate-500 dark:text-slate-400"
                  aria-live="polite"
                >
                  {selectedPlanPreviewLabel}
                </span>
              </div>
              {selectedPlanPreview ? (
                <div
                  className="mt-3 grid gap-1 rounded-lg border border-blue-100 bg-blue-50 p-3 text-xs text-slate-600 dark:border-blue-500/20 dark:bg-blue-500/10 dark:text-slate-300"
                  aria-live="polite"
                >
                  <strong className="font-semibold text-slate-900 dark:text-slate-100">
                    {selectedPlanPreview.fileCount} files ·{" "}
                    {formatTransferBytes(selectedPlanPreview.totalBytes)}
                  </strong>
                  <span>Preview only — no files have been copied.</span>
                  {selectedPlanPreview.sampleDestinationPaths.map((path) => (
                    <code
                      className="overflow-hidden text-ellipsis whitespace-nowrap text-blue-700 dark:text-blue-300"
                      key={path}
                    >
                      {path}
                    </code>
                  ))}
                </div>
              ) : null}
              {sortPreset !== "original_tree" ? <CaptureTimeWarning /> : null}
              <button
                className="mt-3 w-full rounded-lg bg-blue-600 px-4 py-3 text-sm font-semibold text-white shadow-sm transition hover:bg-blue-500 disabled:cursor-not-allowed disabled:opacity-45 dark:text-white"
                disabled={
                  isCopying ||
                  !selected.ingestable ||
                  !destination.trim() ||
                  isSavingConfiguration
                }
                onClick={() => void startIngest()}
                type="button"
              >
                {isCopying ? "View Active Ingest" : "Start Verified Ingest"}
              </button>
              {selectedOperation ? (
                <button
                  className="mt-3 rounded-lg border border-slate-200 bg-white px-3 py-2 text-xs font-semibold text-slate-700 shadow-sm transition hover:border-blue-300 hover:text-blue-700 disabled:cursor-not-allowed disabled:opacity-40 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200"
                  disabled={selectedOperation.cancelling}
                  onClick={() => void cancelIngest()}
                  type="button"
                >
                  {selectedOperation.cancelling ? "Stopping safely…" : "Stop ingest"}
                </button>
              ) : null}
              <p
                className="mt-3 min-h-5 text-xs leading-5 text-slate-500 dark:text-slate-400"
                aria-live="polite"
              >
                {selectedIngestLabel}
              </p>
            </article>
            <article className="border-t border-slate-200 pt-8 dark:border-slate-800 lg:border-t-0 lg:border-l lg:pt-0 lg:pl-8">
              <div className="flex items-start justify-between gap-4">
                <div>
                  <p className="mb-1 text-[10px] font-bold tracking-[0.14em] text-slate-400 dark:text-slate-500">
                    HARDWARE EVIDENCE
                  </p>
                  <h3 className="text-lg font-semibold tracking-tight">
                    Identity & Reader
                  </h3>
                </div>
                <button
                  aria-controls="identity-details"
                  aria-expanded={isIdentityDetailsOpen}
                  className="text-xs font-semibold text-blue-600 hover:text-blue-700 dark:text-blue-400"
                  onClick={() => setIsIdentityDetailsOpen((open) => !open)}
                  type="button"
                >
                  {isIdentityDetailsOpen ? "Hide Details" : "View Details"}
                </button>
              </div>
              <dl className="mt-5">
                <div className="border-t border-slate-200 py-3 dark:border-slate-800">
                  <dt className="mb-1 text-[11px] text-slate-500 dark:text-slate-400">
                    Identity evidence
                  </dt>
                  <dd className="m-0 font-mono text-sm leading-5 text-slate-700 dark:text-slate-200">
                    {selected.identity}
                  </dd>
                </div>
                <div className="border-t border-slate-200 py-3 dark:border-slate-800">
                  <dt className="mb-1 text-[11px] text-slate-500 dark:text-slate-400">
                    Reader
                  </dt>
                  <dd className="m-0 font-mono text-sm leading-5 text-slate-700 dark:text-slate-200">
                    {selected.reader}
                  </dd>
                </div>
                <div className="border-t border-slate-200 py-3 dark:border-slate-800">
                  <dt className="mb-1 text-[11px] text-slate-500 dark:text-slate-400">
                    Physical location
                  </dt>
                  <dd className="m-0 font-mono text-sm leading-5 text-slate-700 dark:text-slate-200">
                    {selected.slot}
                  </dd>
                </div>
                <div className="border-t border-slate-200 py-3 dark:border-slate-800">
                  <dt className="mb-1 text-[11px] text-slate-500 dark:text-slate-400">
                    Current connection
                  </dt>
                  <dd className="m-0 font-mono text-sm leading-5 text-slate-700 dark:text-slate-200">
                    Insertion {selected.connectionGeneration} · changes after this
                    medium is absent
                  </dd>
                </div>
                <div className="border-t border-slate-200 py-3 dark:border-slate-800">
                  <dt className="mb-1 text-[11px] text-slate-500 dark:text-slate-400">
                    Volume reference
                  </dt>
                  <dd className="m-0 font-mono text-sm leading-5 text-slate-700 dark:text-slate-200">
                    Observed only · never used as identity
                  </dd>
                </div>
              </dl>
              {isIdentityDetailsOpen ? (
                <dl
                  id="identity-details"
                  className="mt-3 grid gap-3 rounded-lg border border-slate-200 bg-slate-50 p-3 text-xs dark:border-slate-700 dark:bg-slate-950/40"
                >
                  <div>
                    <dt className="text-slate-500 dark:text-slate-400">
                      Native identity confidence
                    </dt>
                    <dd className="m-0 mt-1 break-words font-mono">
                      {selected.nativeConfidence.replace(/_/g, " ")}
                    </dd>
                  </div>
                  <div>
                    <dt className="text-slate-500 dark:text-slate-400">
                      Media reference
                    </dt>
                    <dd className="m-0 mt-1 break-all font-mono">{selected.id}</dd>
                  </div>
                  <div>
                    <dt className="text-slate-500 dark:text-slate-400">
                      Current mount · mutable
                    </dt>
                    <dd className="m-0 mt-1 break-all font-mono">
                      {selected.sourceRoot || "No mounted source"}
                    </dd>
                  </div>
                  {selected.readerFingerprint ? (
                    <div>
                      <dt className="text-slate-500 dark:text-slate-400">
                        Reader reference · identifies the reader
                      </dt>
                      <dd className="m-0 mt-1 break-all font-mono">
                        {selected.readerFingerprint}
                      </dd>
                    </div>
                  ) : null}
                  {selected.logicalUnit !== null ? (
                    <div>
                      <dt className="text-slate-500 dark:text-slate-400">
                        Reader logical unit · not card identity
                      </dt>
                      <dd className="m-0 mt-1 font-mono">{selected.logicalUnit}</dd>
                    </div>
                  ) : null}
                </dl>
              ) : null}
              {selected.readerFingerprint && selected.logicalUnit !== null ? (
                <div className="mt-4 grid gap-3 border-t border-slate-200 pt-4 dark:border-slate-800">
                  <p className="m-0 text-xs text-slate-500 dark:text-slate-400">
                    Controlled slot calibration
                  </p>
                  <div className="flex items-center justify-between gap-3">
                    <label>
                      <span className="sr-only">Reader slot type</span>
                      <select
                        aria-label="Reader slot type"
                        onChange={(event) =>
                          setCalibrationKind(event.target.value as "sd" | "micro_sd")
                        }
                        value={calibrationKind}
                        className="rounded-md border border-slate-200 bg-white px-2 py-1.5 text-xs font-semibold text-slate-700 outline-none focus:border-blue-500 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200"
                      >
                        <option value="sd">SD card</option>
                        <option value="micro_sd">microSD card</option>
                      </select>
                    </label>
                    <button
                      className="text-xs font-semibold text-blue-600 hover:text-blue-700 dark:text-blue-400"
                      onClick={() => void calibrateReaderSlot()}
                      type="button"
                    >
                      Calibrate This Unit
                    </button>
                  </div>
                  <span
                    className="text-xs text-slate-500 dark:text-slate-400"
                    aria-live="polite"
                  >
                    {calibrationLabel}
                  </span>
                </div>
              ) : null}
            </article>
          </div>
          <article className="mt-8 flex flex-wrap items-center gap-4 rounded-xl border border-blue-100 bg-blue-50 p-5 dark:border-blue-500/20 dark:bg-blue-500/10">
            <div
              className="grid size-9 place-items-center rounded-full bg-blue-600 text-sm font-bold text-white"
              aria-hidden="true"
            >
              <CircleCheck className="size-5" strokeWidth={2.25} />
            </div>
            <div>
              <p className="mb-1 text-[10px] font-bold tracking-[0.14em] text-blue-700 dark:text-blue-300">
                VERIFICATION GATE
              </p>
              <h3 className="text-lg font-semibold tracking-tight">
                {selected.verification === "Verified"
                  ? "All copied files are verified"
                  : "Verification Begins After Every Copy"}
              </h3>
              <p className="mb-0 mt-1 text-xs leading-5 text-slate-600 dark:text-slate-300">
                Each file must match a fresh destination read before this card can be
                formatted.
              </p>
            </div>
            <button
              aria-describedby="receipt-access-explanation"
              className="ml-auto text-xs font-semibold text-blue-700 hover:text-blue-800 dark:text-blue-300"
              disabled
              type="button"
            >
              Open Receipt
            </button>
            <p
              id="receipt-access-explanation"
              className="m-0 basis-full text-xs leading-5 text-slate-600 dark:text-slate-300"
            >
              Receipt opening is not available in this app yet. Completed native ingests
              save JSON receipts in the destination&apos;s{" "}
              <code>.media-ingest-receipts</code> folder.
            </p>
          </article>
          <article
            className="mt-8 border-t border-slate-200 pt-8 dark:border-slate-800"
            aria-labelledby="history-title"
          >
            <div className="flex items-start justify-between gap-4">
              <div>
                <p className="mb-1 text-[10px] font-bold tracking-[0.14em] text-slate-400 dark:text-slate-500">
                  LOCAL HISTORY
                </p>
                <h3 id="history-title" className="text-lg font-semibold tracking-tight">
                  Recent Ingests
                </h3>
              </div>
              <button
                className="text-xs font-semibold text-blue-600 hover:text-blue-700 dark:text-blue-400"
                onClick={() => void refreshHistory()}
                type="button"
              >
                Refresh
              </button>
            </div>
            {history.length ? (
              <ul className="mt-4 grid list-none gap-0 p-0">
                {history.map((run) => (
                  <li
                    className="flex items-center justify-between gap-4 border-t border-slate-200 py-3 text-xs dark:border-slate-800"
                    key={run.runId}
                  >
                    <span className="grid gap-0.5">
                      <strong className="text-sm">{formatRunState(run.state)}</strong>
                      <small className="text-[11px] text-slate-500 dark:text-slate-400">
                        {run.receiptAvailable ? "Receipt sealed" : "No receipt"}
                      </small>
                    </span>
                    <span>
                      {run.verifiedFileCount} files ·{" "}
                      {formatTransferBytes(run.verifiedBytes)}
                    </span>
                    {run.state === "recovery_required" ? (
                      <span className="grid justify-items-end gap-1 text-right">
                        <button
                          className="text-xs font-semibold text-blue-600 disabled:cursor-wait disabled:opacity-50 dark:text-blue-400"
                          disabled={activeRecoveryRuns.has(run.runId)}
                          onClick={() => void resumeHistoryRun(run.runId)}
                          type="button"
                        >
                          {activeRecoveryRuns.has(run.runId)
                            ? "Recovering…"
                            : "Resume safely"}
                        </button>
                        {recoveryLabels[run.runId] ? (
                          <small className="text-[11px] text-slate-500 dark:text-slate-400">
                            {recoveryLabels[run.runId]}
                          </small>
                        ) : null}
                      </span>
                    ) : null}
                  </li>
                ))}
              </ul>
            ) : (
              <p className="mt-4 text-xs text-slate-500 dark:text-slate-400">
                No completed native ingest history yet.
              </p>
            )}
          </article>
        </section>
      </section>
      {pendingFormatConfirmation ? (
        <Modal
          labelledBy="format-confirmation-title"
          className="fixed inset-0 z-50 grid place-items-center overscroll-contain bg-slate-950/55 p-4 backdrop-blur-sm"
          closeOnBackdrop
          dismissible={!isExecutingFormat}
          onDismiss={() => setPendingFormatConfirmation(null)}
        >
          <section
            className="w-full max-w-lg rounded-2xl border border-rose-200 bg-white p-6 shadow-2xl dark:border-rose-500/25 dark:bg-slate-900"
            onClick={(event) => event.stopPropagation()}
          >
            <p className="mb-1 text-[10px] font-bold tracking-[0.14em] text-rose-700 dark:text-rose-300">
              DESTRUCTIVE ACTION
            </p>
            <h2
              id="format-confirmation-title"
              className="text-2xl font-semibold tracking-tight"
            >
              Quick Format This Verified Card?
            </h2>
            <p className="mt-3 text-sm leading-6 text-slate-600 dark:text-slate-300">
              This removes the card&apos;s filesystem structures. It is not secure
              erasure: old file data may remain recoverable until overwritten.
            </p>
            <dl className="mt-5 grid gap-3 rounded-xl border border-slate-200 bg-slate-50 p-4 text-xs dark:border-slate-700 dark:bg-slate-950/40">
              <div className="grid gap-1 sm:grid-cols-[8rem_1fr] sm:gap-3">
                <dt className="text-slate-500 dark:text-slate-400">Selected media</dt>
                <dd className="m-0 font-semibold">
                  {pendingFormatConfirmation.deviceName}
                </dd>
              </div>
              <div className="grid gap-1 sm:grid-cols-[8rem_1fr] sm:gap-3">
                <dt className="text-slate-500 dark:text-slate-400">Media contents</dt>
                <dd className="m-0 font-semibold">{pendingFormatConfirmation.media}</dd>
              </div>
              <div className="grid gap-1 sm:grid-cols-[8rem_1fr] sm:gap-3">
                <dt className="text-slate-500 dark:text-slate-400">Capacity</dt>
                <dd className="m-0 font-semibold">
                  {pendingFormatConfirmation.capacity}
                </dd>
              </div>
              <div className="grid gap-1 sm:grid-cols-[8rem_1fr] sm:gap-3">
                <dt className="text-slate-500 dark:text-slate-400">Sealed receipt</dt>
                <dd className="m-0 break-all font-mono text-[11px]">
                  {pendingFormatConfirmation.receiptId}
                </dd>
              </div>
              <div className="grid gap-1 sm:grid-cols-[8rem_1fr] sm:gap-3">
                <dt className="text-slate-500 dark:text-slate-400">Approved profile</dt>
                <dd className="m-0 font-semibold">
                  {pendingFormatConfirmation.profile
                    ? `${pendingFormatConfirmation.profile.id} (${pendingFormatConfirmation.profile.filesystem.toUpperCase()})${pendingFormatConfirmation.profile.inferredFromCapacity ? " — inferred from capacity" : ""}`
                    : "No profile approved"}
                </dd>
              </div>
            </dl>
            <p className="mt-4 text-xs leading-5 text-slate-500 dark:text-slate-400">
              The native service will re-check the exact current medium before it
              writes. If the card changed, was removed, or the confirmation expired,
              formatting is refused.
            </p>
            <div className="mt-6 flex flex-wrap justify-end gap-3">
              <button
                className="rounded-lg px-3 py-2 text-xs font-semibold text-slate-600 transition hover:bg-slate-100 disabled:cursor-not-allowed disabled:opacity-60 dark:text-slate-300 dark:hover:bg-slate-800"
                disabled={isExecutingFormat}
                onClick={() => setPendingFormatConfirmation(null)}
                type="button"
              >
                Cancel
              </button>
              <button
                className="rounded-lg bg-rose-700 px-3 py-2 text-xs font-semibold text-white shadow-sm transition hover:bg-rose-600 disabled:cursor-wait disabled:opacity-60"
                disabled={isExecutingFormat}
                onClick={() => void executeFormatAuthorization()}
                type="button"
              >
                {isExecutingFormat ? "Quick Formatting…" : "Quick Format Card"}
              </button>
            </div>
          </section>
        </Modal>
      ) : null}
      {pendingForceReformat ? (
        <Modal
          labelledBy="force-reformat-title"
          className="fixed inset-0 z-50 grid place-items-center overscroll-contain bg-slate-950/55 p-4 backdrop-blur-sm"
          dismissible={!isExecutingFormat}
          onDismiss={() => {
            setPendingForceReformat(null);
            setForceReformatPhrase("");
            setForceReformatError("");
          }}
          initialFocus="button"
        >
          <section className="w-full max-w-lg rounded-2xl border border-rose-200 bg-white p-6 shadow-2xl dark:border-rose-500/25 dark:bg-slate-900">
            <p className="mb-1 text-[10px] font-bold tracking-[0.14em] text-rose-700 dark:text-rose-300">
              RECOVERY DESTRUCTIVE ACTION
            </p>
            <h2
              id="force-reformat-title"
              className="text-2xl font-semibold tracking-tight"
            >
              Force Reformat This Registered Card?
            </h2>
            <p className="mt-3 text-sm leading-6 text-slate-600 dark:text-slate-300">
              This bypasses receipt, registration, and identity continuity checks and
              removes all filesystem structures. It still requires an exact current
              removable target and post-format validation; it is not secure erasure.
            </p>
            <dl className="mt-5 grid gap-3 rounded-xl border border-slate-200 bg-slate-50 p-4 text-xs dark:border-slate-700 dark:bg-slate-950/40">
              <div className="grid gap-1 sm:grid-cols-[8rem_1fr] sm:gap-3">
                <dt className="text-slate-500 dark:text-slate-400">Selected media</dt>
                <dd className="m-0 font-semibold">{pendingForceReformat.deviceName}</dd>
              </div>
              <div className="grid gap-1 sm:grid-cols-[8rem_1fr] sm:gap-3">
                <dt className="text-slate-500 dark:text-slate-400">Media contents</dt>
                <dd className="m-0 font-semibold">{pendingForceReformat.media}</dd>
              </div>
              <div className="grid gap-1 sm:grid-cols-[8rem_1fr] sm:gap-3">
                <dt className="text-slate-500 dark:text-slate-400">Capacity</dt>
                <dd className="m-0 font-semibold">{pendingForceReformat.capacity}</dd>
              </div>
            </dl>
            <label className="mt-5 grid gap-2 text-xs font-semibold text-slate-700 dark:text-slate-200">
              Type <code>FORCE REFORMAT</code> to enable this action
              <input
                aria-label="Force reformat confirmation"
                className="rounded-lg border border-slate-300 bg-white px-3 py-2 font-mono text-sm font-normal dark:border-slate-600 dark:bg-slate-950"
                value={forceReformatPhrase}
                onChange={(event) => setForceReformatPhrase(event.target.value)}
              />
            </label>
            {forceReformatError ? (
              <p
                className="mt-3 rounded-lg border border-rose-300 bg-rose-50 px-3 py-2 text-xs leading-5 text-rose-800 dark:border-rose-500/40 dark:bg-rose-500/10 dark:text-rose-200"
                role="alert"
              >
                {forceReformatError}
              </p>
            ) : null}
            <div className="mt-6 flex flex-wrap justify-end gap-3">
              <button
                className="rounded-lg px-3 py-2 text-xs font-semibold text-slate-600 transition hover:bg-slate-100 disabled:cursor-not-allowed disabled:opacity-60 dark:text-slate-300 dark:hover:bg-slate-800"
                disabled={isExecutingFormat}
                onClick={() => {
                  setPendingForceReformat(null);
                  setForceReformatPhrase("");
                  setForceReformatError("");
                }}
                type="button"
              >
                Cancel
              </button>
              <button
                className="rounded-lg bg-rose-700 px-3 py-2 text-xs font-semibold text-white shadow-sm transition hover:bg-rose-600 disabled:cursor-not-allowed disabled:opacity-60"
                disabled={isExecutingFormat || forceReformatPhrase !== "FORCE REFORMAT"}
                onClick={() => void executeForceReformat()}
                type="button"
              >
                {isExecutingFormat ? "Force Reformatting…" : "Force Reformat Card"}
              </button>
            </div>
          </section>
        </Modal>
      ) : null}
      {isCloseConfirmationOpen ? (
        <Modal
          labelledBy="close-confirmation-title"
          className="fixed inset-0 z-50 grid place-items-center overscroll-contain bg-slate-950/40 p-4 backdrop-blur-sm"
          dismissible={!isCancellingForClose}
          onDismiss={() => setIsCloseConfirmationOpen(false)}
        >
          <section className="w-full max-w-md rounded-2xl border border-slate-200 bg-white p-6 shadow-2xl dark:border-slate-700 dark:bg-slate-900">
            <p className="mb-1 text-[10px] font-bold tracking-[0.14em] text-amber-600 dark:text-amber-300">
              ACTIVE VERIFIED INGEST
            </p>
            <h2
              id="close-confirmation-title"
              className="text-2xl font-semibold tracking-tight"
            >
              Keep This App Open Until Copying Stops
            </h2>
            <p className="mt-3 text-sm leading-6 text-slate-500 dark:text-slate-400">
              Closing now could interrupt a copy or verification. You can keep the app
              open, or request a safe cancellation; the affected run will stay
              recoverable and must not be treated as transferred.
            </p>
            <div className="mt-6 flex justify-end gap-3">
              <button
                className="rounded-lg px-3 py-2 text-xs font-semibold text-slate-600 transition hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800"
                disabled={isCancellingForClose}
                onClick={() => setIsCloseConfirmationOpen(false)}
                type="button"
              >
                Keep Ingesting
              </button>
              <button
                className="rounded-lg bg-amber-600 px-3 py-2 text-xs font-semibold text-white shadow-sm transition hover:bg-amber-500 disabled:cursor-wait disabled:opacity-60"
                disabled={isCancellingForClose}
                onClick={() => void cancelActiveIngestsForClose()}
                type="button"
              >
                {isCancellingForClose ? "Requesting Cancellation…" : "Cancel Ingests"}
              </button>
            </div>
          </section>
        </Modal>
      ) : null}
      {dockOperations.length ? (
        <section
          aria-label="All ingest progress"
          className="ingest-progress-dock fixed bottom-4 z-40 mx-auto max-w-5xl rounded-2xl border border-slate-200 bg-white/95 shadow-2xl backdrop-blur dark:border-slate-700 dark:bg-slate-900/95"
        >
          <button
            aria-expanded={isIngestDockOpen}
            className="flex w-full items-center gap-3 px-4 py-2.5 text-left sm:px-5"
            onClick={() => setIsIngestDockOpen((current) => !current)}
            type="button"
          >
            <span className="grid size-8 shrink-0 place-items-center rounded-lg bg-blue-600 text-white">
              <ListTodo aria-hidden="true" className="size-4" strokeWidth={2.25} />
            </span>
            <span className="min-w-0 flex-1">
              <span className="flex items-center justify-between gap-3">
                <strong className="truncate text-sm">
                  All Ingest Work · {dockOperations.length} active
                </strong>
                <span className="shrink-0 text-sm font-semibold tabular-nums">
                  {dockPercent}%
                </span>
              </span>
              <span className="mt-1.5 block h-1.5 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-700">
                <span
                  className="block h-full rounded-full bg-blue-600 transition-[width] duration-300"
                  style={{ width: `${dockPercent}%` }}
                />
              </span>
              <span className="mt-1 flex justify-between gap-3 text-[11px] text-slate-500 dark:text-slate-400">
                <span className="truncate">
                  {formatTransferBytes(dockTransferredBytes)}
                  {dockKnownTotalBytes
                    ? ` / ${formatTransferBytes(dockKnownTotalBytes)}`
                    : ""}
                </span>
                <span className="shrink-0">ETA {dockEta}</span>
              </span>
            </span>
            {isIngestDockOpen ? (
              <ChevronDown
                aria-hidden="true"
                className="size-5 text-slate-500 dark:text-slate-400"
              />
            ) : (
              <ChevronUp
                aria-hidden="true"
                className="size-5 text-slate-500 dark:text-slate-400"
              />
            )}
          </button>
          {isIngestDockOpen ? (
            <div className="max-h-[42vh] overflow-y-auto border-t border-slate-200 px-4 py-3 dark:border-slate-700 sm:px-5">
              <ul className="grid gap-3" aria-label="Per-operation ingest progress">
                {dockOperations.map((operation) => {
                  const percent =
                    operation.totalBytes && operation.totalBytes > 0
                      ? Math.min(
                          100,
                          Math.round(
                            (operation.transferredBytes / operation.totalBytes) * 100,
                          ),
                        )
                      : 0;
                  return (
                    <li
                      className="rounded-xl border border-slate-200 p-3 dark:border-slate-700"
                      key={operation.operationId}
                    >
                      <div className="flex items-start justify-between gap-3">
                        <div>
                          <strong className="block text-sm">
                            {operation.device?.name ?? "Disconnected source"}
                          </strong>
                          <span className="mt-0.5 block text-xs text-slate-500 dark:text-slate-400">
                            {operation.cancelling
                              ? "Cancelling"
                              : formatRunState(
                                  operation.state as IngestHistoryEntry["state"],
                                )}
                            {operation.currentFileIndex && operation.totalFiles
                              ? ` · file ${operation.currentFileIndex} of ${operation.totalFiles}`
                              : ""}
                          </span>
                        </div>
                        <span className="text-sm font-semibold tabular-nums">
                          {percent}%
                        </span>
                      </div>
                      <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-slate-200 dark:bg-slate-700">
                        <div
                          className="h-full rounded-full bg-blue-600"
                          style={{ width: `${percent}%` }}
                        />
                      </div>
                      <p className="mt-2 text-xs text-slate-600 dark:text-slate-300">
                        {formatTransferBytes(operation.transferredBytes)}
                        {operation.totalBytes
                          ? ` / ${formatTransferBytes(operation.totalBytes)}`
                          : ""}
                        {operation.bytesPerSecond > 0
                          ? ` · ${(operation.bytesPerSecond / 1_000_000).toFixed(1)} MB/s`
                          : ""}
                      </p>
                      <ol className="mt-2 grid gap-1 border-l-2 border-slate-200 pl-3 text-[11px] text-slate-500 dark:border-slate-700 dark:text-slate-400">
                        {operation.logs.map((log, index) => (
                          <li key={`${operation.operationId}-${index}`}>{log}</li>
                        ))}
                      </ol>
                    </li>
                  );
                })}
              </ul>
            </div>
          ) : null}
        </section>
      ) : null}
      <footer className="flex min-h-11 flex-col justify-between gap-2 border-t border-slate-200 px-5 py-3 text-[11px] text-slate-500 sm:flex-row sm:items-center sm:px-8 dark:border-slate-800 dark:text-slate-400">
        <span>Local-first · native removable-media inventory</span>
        <span>Desktop-only · local-first</span>
      </footer>
      {configurationDialog}
    </main>
  );
}

export default App;
