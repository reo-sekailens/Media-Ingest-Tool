export type SortPreset = "original_tree" | "exif_day" | "exif_hour" | "exif_custom";

export type SortRequest = {
  sortMode: "original_tree" | "camera_day" | "camera_interval";
  intervalMinutes: number | null;
};

export type CustomDirectoryField = {
  label: string;
  value: string;
};

export type DestinationDepthSegment =
  | { kind: "custom_field"; index: number }
  | { kind: "camera_model" }
  | { kind: "capture_day" }
  | { kind: "capture_interval" }
  | { kind: "original_tree" };

export const CUSTOM_INTERVAL_MINUTES_MIN = 1;
export const CUSTOM_INTERVAL_MINUTES_MAX = 1_440;
export const MAX_CUSTOM_DIRECTORY_FIELDS = 8;
const MAX_PORTABLE_FOLDER_BYTES = 120;

export function resolveSortRequest(
  preset: SortPreset,
  customIntervalMinutes: string,
): SortRequest | null {
  if (preset === "original_tree") {
    return { sortMode: "original_tree", intervalMinutes: null };
  }
  if (preset === "exif_day") {
    return { sortMode: "camera_day", intervalMinutes: null };
  }
  if (preset === "exif_hour") {
    return { sortMode: "camera_interval", intervalMinutes: 60 };
  }
  const interval = Number(customIntervalMinutes);
  return Number.isInteger(interval) &&
    interval >= CUSTOM_INTERVAL_MINUTES_MIN &&
    interval <= CUSTOM_INTERVAL_MINUTES_MAX
    ? { sortMode: "camera_interval", intervalMinutes: interval }
    : null;
}

export function defaultDestinationDepthOrder(
  request: SortRequest | null,
  customDirectoryFields: readonly CustomDirectoryField[],
): DestinationDepthSegment[] {
  const customFields = customDirectoryFields.map((_, index) => ({
    kind: "custom_field" as const,
    index,
  }));
  if (!request || request.sortMode === "original_tree") {
    return [...customFields, { kind: "original_tree" }];
  }
  if (request.sortMode === "camera_day") {
    return [...customFields, { kind: "camera_model" }, { kind: "capture_day" }];
  }
  return [
    ...customFields,
    { kind: "camera_model" },
    { kind: "capture_day" },
    { kind: "capture_interval" },
  ];
}

export function destinationDepthSegmentKey(segment: DestinationDepthSegment): string {
  return segment.kind === "custom_field"
    ? `${segment.kind}:${segment.index}`
    : segment.kind;
}

export function normalizeDestinationDepthOrder(
  requested: readonly DestinationDepthSegment[],
  available: readonly DestinationDepthSegment[],
): DestinationDepthSegment[] {
  const allowed = new Map(
    available.map((segment) => [destinationDepthSegmentKey(segment), segment]),
  );
  const selected = requested.flatMap((segment) => {
    const key = destinationDepthSegmentKey(segment);
    const availableSegment = allowed.get(key);
    if (!availableSegment) return [];
    allowed.delete(key);
    return [availableSegment];
  });
  return [...selected, ...allowed.values()];
}

export function destinationDepthSegmentLabel(
  segment: DestinationDepthSegment,
  customDirectoryFields: readonly CustomDirectoryField[],
  request: SortRequest | null,
): string {
  if (segment.kind === "custom_field") {
    const field = customDirectoryFields[segment.index];
    return field?.value.trim() || `Custom folder ${segment.index + 1}`;
  }
  if (segment.kind === "camera_model") return "Camera model";
  if (segment.kind === "capture_day") return "EXIF capture day";
  if (segment.kind === "capture_interval") {
    return `${request?.intervalMinutes ?? "Custom"}-minute bucket`;
  }
  return "Original folders";
}

export function customDirectoryFieldsAreComplete(
  fields: readonly CustomDirectoryField[],
): boolean {
  return fields.every((field) => field.value.trim());
}

export type ConfigurationDraft = {
  destinationPath: string;
  sortPreset: SortPreset;
  customIntervalMinutes: string;
  customDirectoryFields: CustomDirectoryField[];
  destinationDepthOrder: DestinationDepthSegment[];
  persistOnCard: boolean;
  autoIngestEnabled: boolean;
  autoFormatEnabled: boolean;
};

export function createConfiguration(
  partial: Partial<ConfigurationDraft> = {},
): ConfigurationDraft {
  return {
    destinationPath: partial.destinationPath ?? "",
    sortPreset: partial.sortPreset ?? "exif_day",
    customIntervalMinutes: partial.customIntervalMinutes ?? "30",
    customDirectoryFields: (partial.customDirectoryFields ?? []).map((field) => ({
      ...field,
    })),
    destinationDepthOrder: (partial.destinationDepthOrder ?? []).map((segment) => ({
      ...segment,
    })),
    persistOnCard: partial.persistOnCard ?? false,
    autoIngestEnabled: partial.autoIngestEnabled ?? false,
    autoFormatEnabled: partial.autoFormatEnabled ?? false,
  };
}

export function cloneConfiguration(
  configuration: ConfigurationDraft,
): ConfigurationDraft {
  return createConfiguration(configuration);
}

export type ConfigurationValidationStep = "destination" | "organization" | "automation";

function folderValidationError(value: string, index: number): string | null {
  const folder = `Custom folder ${index + 1}`;
  if (!value.trim()) return `${folder} needs a name.`;
  if (
    value !== value.trim() ||
    value === "." ||
    value === ".." ||
    value.endsWith(".") ||
    Array.from(value).some((character) => {
      const code = character.codePointAt(0)!;
      return (
        '<>:"/\\|?*'.includes(character) || code < 32 || (code >= 127 && code <= 159)
      );
    })
  ) {
    return `${folder} must use a portable name without reserved characters, surrounding spaces, or trailing periods.`;
  }
  const stem = value.split(".")[0].trimEnd();
  if (/^(CON|PRN|AUX|NUL|CLOCK\$|COM[1-9]|LPT[1-9])$/i.test(stem)) {
    return `${folder} uses a reserved filename.`;
  }
  if (new TextEncoder().encode(value).length > MAX_PORTABLE_FOLDER_BYTES) {
    return `${folder} is too long; use a shorter name.`;
  }
  return null;
}

// These checks keep incomplete drafts out of the save flow. Native commands
// still own current-card identity, actual destination access, and format safety.
export function validateConfigurationStep(
  configuration: ConfigurationDraft,
  step: ConfigurationValidationStep,
): string | null {
  if (step === "destination") {
    return configuration.destinationPath.trim() ? null : "Choose a destination folder.";
  }
  if (step === "organization") {
    if (
      !resolveSortRequest(configuration.sortPreset, configuration.customIntervalMinutes)
    ) {
      return "Enter a whole-number interval from 1 to 1,440 minutes.";
    }
    if (configuration.customDirectoryFields.length > MAX_CUSTOM_DIRECTORY_FIELDS) {
      return "Use at most eight custom folders.";
    }
    for (const [index, field] of configuration.customDirectoryFields.entries()) {
      const error = folderValidationError(field.value, index);
      if (error) return error;
    }
    return null;
  }
  if (configuration.autoFormatEnabled && !configuration.autoIngestEnabled) {
    return "Enable automatic ingest before enabling automatic format.";
  }
  if (
    !configuration.persistOnCard &&
    (configuration.autoIngestEnabled || configuration.autoFormatEnabled)
  ) {
    return "Save this configuration on the card before enabling automation.";
  }
  return null;
}

export function validateConfiguration(
  configuration: ConfigurationDraft,
): string | null {
  for (const step of ["destination", "organization", "automation"] as const) {
    const error = validateConfigurationStep(configuration, step);
    if (error) return error;
  }
  return null;
}
