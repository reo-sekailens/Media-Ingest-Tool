import { describe, expect, test } from "vitest";
import {
  cloneConfiguration,
  createConfiguration,
  customDirectoryFieldsAreComplete,
  defaultDestinationDepthOrder,
  destinationDepthSegmentKey,
  destinationDepthSegmentLabel,
  normalizeDestinationDepthOrder,
  resolveSortRequest,
  validateConfiguration,
  validateConfigurationStep,
  type DestinationDepthSegment,
} from "./configuration";

describe("configuration drafts", () => {
  test("starts with safe opt-in defaults and no destination", () => {
    expect(createConfiguration()).toEqual({
      destinationPath: "",
      sortPreset: "exif_day",
      customIntervalMinutes: "30",
      customDirectoryFields: [],
      destinationDepthOrder: [],
      persistOnCard: false,
      autoIngestEnabled: false,
      autoFormatEnabled: false,
    });
    expect(validateConfiguration(createConfiguration())).toMatch(/destination/i);
    expect(
      validateConfiguration(createConfiguration({ destinationPath: "D:/Ingest" })),
    ).toBeNull();
  });

  test("independent defaults and clones keep canceled edits out of saved state", () => {
    const first = createConfiguration();
    const second = createConfiguration();
    first.customDirectoryFields.push({ label: "", value: "Event" });
    first.destinationDepthOrder.push({ kind: "custom_field", index: 0 });
    expect(second.customDirectoryFields).toEqual([]);
    expect(second.destinationDepthOrder).toEqual([]);

    const saved = createConfiguration({
      destinationPath: "D:/Ingest",
      customDirectoryFields: first.customDirectoryFields,
      destinationDepthOrder: first.destinationDepthOrder,
      persistOnCard: true,
    });
    first.customDirectoryFields[0].value = "Not saved";
    expect(saved.customDirectoryFields[0].value).toBe("Event");
    const draft = cloneConfiguration(saved);
    draft.destinationPath = "E:/Changed";
    draft.customDirectoryFields[0].value = "Changed";
    const segment = draft.destinationDepthOrder[0];
    if (segment.kind === "custom_field") segment.index = 2;
    draft.autoIngestEnabled = true;
    expect(saved.destinationPath).toBe("D:/Ingest");
    expect(saved.customDirectoryFields[0].value).toBe("Event");
    expect(saved.destinationDepthOrder[0]).toEqual({ kind: "custom_field", index: 0 });
    expect(saved.autoIngestEnabled).toBe(false);
  });

  test("step validation leaves later steps editable and full validation catches them", () => {
    const draft = createConfiguration({
      destinationPath: "D:/Ingest",
      sortPreset: "exif_custom",
      customIntervalMinutes: "0",
      autoFormatEnabled: true,
    });
    expect(validateConfigurationStep(draft, "destination")).toBeNull();
    expect(validateConfigurationStep(draft, "organization")).toMatch(/interval/);
    expect(validateConfigurationStep(draft, "automation")).toMatch(/automatic ingest/);
    expect(validateConfiguration(draft)).toMatch(/interval/);
    const emptyDestination = createConfiguration({ destinationPath: " \t " });
    expect(validateConfigurationStep(emptyDestination, "destination")).toMatch(
      /destination/,
    );
    expect(validateConfigurationStep(emptyDestination, "organization")).toBeNull();
  });
});

describe("sort and folder order", () => {
  test("keeps fixed preset semantics and bounded custom minute intervals", () => {
    expect(resolveSortRequest("original_tree", "invalid")).toEqual({
      sortMode: "original_tree",
      intervalMinutes: null,
    });
    expect(resolveSortRequest("exif_day", "invalid")).toEqual({
      sortMode: "camera_day",
      intervalMinutes: null,
    });
    expect(resolveSortRequest("exif_hour", "invalid")).toEqual({
      sortMode: "camera_interval",
      intervalMinutes: 60,
    });
    for (const interval of ["1", "30", "37", "1440"]) {
      expect(resolveSortRequest("exif_custom", interval)).toEqual({
        sortMode: "camera_interval",
        intervalMinutes: Number(interval),
      });
    }
    for (const interval of ["", "0", "-1", "1441", "1.5", "NaN", "Infinity"]) {
      expect(resolveSortRequest("exif_custom", interval)).toBeNull();
      expect(
        validateConfiguration(
          createConfiguration({
            destinationPath: "D:/Ingest",
            sortPreset: "exif_custom",
            customIntervalMinutes: interval,
          }),
        ),
      ).toMatch(/interval/);
    }
  });

  test("provides the complete mode-specific folder set without merging custom folders", () => {
    const fields = [
      { label: "Event", value: "Southpaw" },
      { label: "", value: "Ari" },
    ];
    const original = defaultDestinationDepthOrder(
      resolveSortRequest("original_tree", "30"),
      fields,
    );
    expect(original.map(destinationDepthSegmentKey)).toEqual([
      "custom_field:0",
      "custom_field:1",
      "original_tree",
    ]);
    const day = defaultDestinationDepthOrder(
      resolveSortRequest("exif_day", "30"),
      fields,
    );
    expect(day.map(destinationDepthSegmentKey)).toEqual([
      "custom_field:0",
      "custom_field:1",
      "camera_model",
      "capture_day",
    ]);
    const interval = defaultDestinationDepthOrder(
      resolveSortRequest("exif_custom", "37"),
      fields,
    );
    expect(interval.map(destinationDepthSegmentKey)).toEqual([
      "custom_field:0",
      "custom_field:1",
      "camera_model",
      "capture_day",
      "capture_interval",
    ]);
  });

  test("normalizes stale and duplicate depths while preserving the surviving drag order", () => {
    const available = defaultDestinationDepthOrder(
      resolveSortRequest("exif_day", "30"),
      [{ label: "", value: "Event" }],
    );
    const requested: DestinationDepthSegment[] = [
      { kind: "capture_day" },
      { kind: "capture_interval" },
      { kind: "capture_day" },
      { kind: "custom_field", index: 4 },
      { kind: "camera_model" },
    ];
    const original = structuredClone(requested);
    const normalized = normalizeDestinationDepthOrder(requested, available);
    expect(normalized.map(destinationDepthSegmentKey)).toEqual([
      "capture_day",
      "camera_model",
      "custom_field:0",
    ]);
    expect(requested).toEqual(original);
    expect(available.map(destinationDepthSegmentKey)).toEqual([
      "custom_field:0",
      "camera_model",
      "capture_day",
    ]);
  });

  test("folder labels retain configured values and explicit interval minutes", () => {
    const fields = [{ label: "Legacy label", value: "Ari" }];
    const request = resolveSortRequest("exif_custom", "37");
    expect(
      destinationDepthSegmentLabel({ kind: "custom_field", index: 0 }, fields, request),
    ).toBe("Ari");
    expect(
      destinationDepthSegmentLabel({ kind: "custom_field", index: 1 }, fields, request),
    ).toBe("Custom folder 2");
    expect(
      destinationDepthSegmentLabel({ kind: "capture_interval" }, fields, request),
    ).toBe("37-minute bucket");
    expect(destinationDepthSegmentLabel({ kind: "capture_day" }, fields, request)).toBe(
      "EXIF capture day",
    );
    expect(
      destinationDepthSegmentLabel({ kind: "camera_model" }, fields, request),
    ).toBe("Camera model");
    expect(
      destinationDepthSegmentLabel({ kind: "original_tree" }, fields, request),
    ).toBe("Original folders");
  });
});

describe("portable custom folder values", () => {
  test("requires nonblank folder values and allows eight folders", () => {
    expect(customDirectoryFieldsAreComplete([])).toBe(true);
    expect(customDirectoryFieldsAreComplete([{ label: "Ignored", value: " " }])).toBe(
      false,
    );
    const fields = Array.from({ length: 8 }, (_, index) => ({
      label: "",
      value: `Folder ${index + 1}`,
    }));
    expect(
      validateConfiguration(
        createConfiguration({
          destinationPath: "D:/Ingest",
          customDirectoryFields: fields,
        }),
      ),
    ).toBeNull();
    fields.push({ label: "", value: "Folder 9" });
    expect(
      validateConfiguration(
        createConfiguration({
          destinationPath: "D:/Ingest",
          customDirectoryFields: fields,
        }),
      ),
    ).toMatch(/eight/);
  });

  test.each([
    "",
    " ",
    ".",
    "..",
    " Event",
    "Event ",
    "Event.",
    "a/b",
    "a\\b",
    "a:b",
    "a?b",
    "a*b",
    'a"b',
    "a<b",
    "a>b",
    "a|b",
    "a\nb",
    "a\u0000b",
    "a\u0085b",
    "CON",
    "con.jpg",
    "COM1",
    "LPT9.raw",
    "CLOCK$",
  ])("rejects nonportable folder %j", (value) => {
    const draft = createConfiguration({
      destinationPath: "D:/Ingest",
      customDirectoryFields: [{ label: "", value }],
    });
    expect(validateConfiguration(draft)).toMatch(/Custom folder 1/);
  });

  test("accepts Unicode names and checks length by encoded bytes", () => {
    for (const value of [
      "Mañana",
      "撮影",
      "Event 2027",
      "a".repeat(120),
      "é".repeat(60),
    ]) {
      expect(
        validateConfiguration(
          createConfiguration({
            destinationPath: "D:/Ingest",
            customDirectoryFields: [{ label: "", value }],
          }),
        ),
      ).toBeNull();
    }
    for (const value of ["a".repeat(121), "é".repeat(61)]) {
      expect(
        validateConfiguration(
          createConfiguration({
            destinationPath: "D:/Ingest",
            customDirectoryFields: [{ label: "", value }],
          }),
        ),
      ).toMatch(/too long/);
    }
  });
});

describe("automation dependencies", () => {
  test.each([
    [false, false, false, true],
    [false, false, true, false],
    [false, true, false, false],
    [false, true, true, false],
    [true, false, false, true],
    [true, false, true, false],
    [true, true, false, true],
    [true, true, true, true],
  ])(
    "validates persistence=%s, auto-ingest=%s, auto-format=%s",
    (persistOnCard, autoIngestEnabled, autoFormatEnabled, valid) => {
      const draft = createConfiguration({
        destinationPath: "D:/Ingest",
        persistOnCard,
        autoIngestEnabled,
        autoFormatEnabled,
      });
      expect(validateConfiguration(draft) === null).toBe(valid);
    },
  );
});
