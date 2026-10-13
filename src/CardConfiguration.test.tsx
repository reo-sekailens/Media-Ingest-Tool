// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import type { ComponentProps } from "react";
import { expect, test, vi } from "vitest";
import { CardConfiguration } from "./CardConfiguration";
import {
  createConfiguration,
  resolveSortRequest,
  type CaptureTimeBasis,
} from "./configuration";

type ConfigurationProps = ComponentProps<typeof CardConfiguration>;

function renderConfiguration(overrides: Partial<ConfigurationProps> = {}) {
  const onSave = vi.fn();
  const onDismiss = vi.fn();
  const onChooseDestination = vi.fn().mockResolvedValue(null);
  const props: ConfigurationProps = {
    sourceName: "A-Cam • SDXC",
    initialValue: createConfiguration({ destinationPath: "D:\\Ingest\\A-Cam" }),
    saving: false,
    error: "",
    sourceUnavailable: false,
    onSave,
    onDismiss,
    onChooseDestination,
    ...overrides,
  };
  const result = render(<CardConfiguration {...props} />);
  return { ...result, props, onSave, onDismiss, onChooseDestination };
}

function next() {
  fireEvent.click(screen.getByRole("button", { name: "Next" }));
}

function goToAutomation() {
  next();
  next();
  expect(screen.getByRole("heading", { name: "Automation" })).toBeVisible();
}

function goToReview() {
  goToAutomation();
  next();
  expect(screen.getByRole("heading", { name: "Review configuration" })).toBeVisible();
}

test("requires a destination before advancing the card configuration", () => {
  const { onSave } = renderConfiguration({
    initialValue: createConfiguration(),
  });
  const destination = screen.getByLabelText("Destination directory");
  const advance = screen.getByRole("button", { name: "Next" });
  fireEvent.click(advance);
  expect(screen.getByRole("heading", { name: "Destination" })).toBeVisible();
  expect(screen.getByRole("alert")).toHaveTextContent("Choose a destination folder.");
  expect(screen.getByRole("alert")).toHaveFocus();
  fireEvent.change(destination, { target: { value: "   " } });
  fireEvent.click(advance);
  expect(screen.getByRole("heading", { name: "Destination" })).toBeVisible();
  expect(onSave).not.toHaveBeenCalled();

  fireEvent.change(destination, { target: { value: "D:\\Ingest\\Project" } });
  expect(advance).toBeEnabled();
  next();
  expect(screen.getByRole("heading", { name: "Organization" })).toBeVisible();
});

test.each(["", "0", "1441", "1.5"])(
  "blocks organization progression for an invalid custom interval %j",
  (value) => {
    const { onSave } = renderConfiguration();
    next();
    fireEvent.click(screen.getByRole("button", { name: "Custom interval" }));
    fireEvent.change(screen.getByLabelText("Custom interval minutes"), {
      target: { value },
    });
    const advance = screen.getByRole("button", { name: "Next" });
    fireEvent.click(advance);
    expect(screen.getByRole("heading", { name: "Organization" })).toBeVisible();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Enter a whole-number interval from 1 to 1,440 minutes.",
    );
    expect(screen.getByRole("alert")).toHaveFocus();
    expect(onSave).not.toHaveBeenCalled();
  },
);

test("selecting EXIF hour explains its recorded-offset range and separate date folder", () => {
  renderConfiguration();
  next();
  expect(screen.queryByText("1300-1400+0800")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "EXIF hour" }));

  expect(screen.getByRole("button", { name: "EXIF hour" })).toHaveAttribute(
    "aria-pressed",
    "true",
  );
  expect(screen.getByText("1300-1400+0800")).toBeVisible();
  expect(screen.getByText(/EXIF hour example:/)).toHaveTextContent(
    "13:42 capture with a recorded +08:00 offset",
  );
  expect(screen.getByText(/EXIF hour example:/)).toHaveTextContent(
    "The capture date stays in a separate folder.",
  );
  expect(
    screen.getByRole("button", { name: "Move 60-minute bucket in destination order" }),
  ).toBeVisible();
  expect(screen.queryByLabelText("Custom interval minutes")).not.toBeInTheDocument();

  fireEvent.click(screen.getByRole("button", { name: "EXIF day" }));
  expect(screen.queryByText("1300-1400+0800")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", {
      name: "Move 60-minute bucket in destination order",
    }),
  ).not.toBeInTheDocument();
});

test("saves EXIF hour as a sixty-minute request with every ordered folder level", () => {
  const { onSave } = renderConfiguration({
    initialValue: createConfiguration({
      destinationPath: "D:\\Ingest\\Documentary\\Day 13",
      sortPreset: "exif_day",
      customIntervalMinutes: "37",
      customDirectoryFields: [{ label: "", value: "Photographer - Ari" }],
      destinationDepthOrder: [
        { kind: "capture_day" },
        { kind: "custom_field", index: 0 },
        { kind: "camera_model" },
      ],
    }),
  });
  next();
  fireEvent.click(screen.getByRole("button", { name: "EXIF hour" }));
  next();
  next();
  expect(screen.getByRole("heading", { name: "Review configuration" })).toBeVisible();
  expect(onSave).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Save configuration" }));

  expect(onSave).toHaveBeenCalledOnce();
  const saved: ConfigurationProps["initialValue"] = onSave.mock.calls[0][0];
  expect(saved.sortPreset).toBe("exif_hour");
  expect(resolveSortRequest(saved.sortPreset, saved.customIntervalMinutes)).toEqual({
    sortMode: "camera_interval",
    intervalMinutes: 60,
  });
  expect(saved.destinationDepthOrder).toEqual([
    { kind: "capture_day" },
    { kind: "custom_field", index: 0 },
    { kind: "camera_model" },
    { kind: "capture_interval" },
  ]);
  expect(saved.customDirectoryFields).toEqual([
    { label: "", value: "Photographer - Ari" },
  ]);
});

test("time-based sorting defaults to camera time and explains clocks, timezone, and missing offsets", () => {
  renderConfiguration();
  next();
  expect(screen.getByRole("group", { name: "Capture time basis" })).toBeVisible();
  expect(screen.getByRole("radio", { name: "Camera time" })).toBeChecked();
  expect(screen.getByRole("radio", { name: "Computer local time" })).not.toBeChecked();
  const warning = screen.getByRole("note", {
    name: "Synchronize camera and computer clocks",
  });
  expect(warning).toHaveTextContent(
    "correct date, time, time zone, and daylight saving time settings",
  );
  expect(warning).toHaveTextContent(
    "An incorrect camera clock can place files in the wrong folders.",
  );
  expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();
  expect(screen.getByText(/If the camera offset is missing/)).toHaveTextContent(
    "recorded wall clock",
  );
  expect(screen.getByText("offset-unknown")).toBeVisible();
  expect(screen.getByText(/If the camera offset is missing/)).toHaveTextContent(
    "cannot convert that capture",
  );
});

test("computer time survives switching to original folders, review, and the final save", () => {
  const { onSave } = renderConfiguration();
  next();
  fireEvent.click(screen.getByRole("radio", { name: "Computer local time" }));
  expect(screen.getByRole("radio", { name: "Computer local time" })).toBeChecked();
  expect(
    screen.getByText(/Convert a capture with a recorded offset/),
  ).toHaveTextContent(
    "local time zone at the capture date, including daylight saving time",
  );
  fireEvent.click(screen.getByRole("button", { name: "Original tree" }));
  expect(
    screen.queryByRole("group", { name: "Capture time basis" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("note", { name: "Synchronize camera and computer clocks" }),
  ).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "EXIF hour" }));
  expect(screen.getByRole("radio", { name: "Computer local time" })).toBeChecked();
  expect(screen.queryByText("1300-1400+0800")).not.toBeInTheDocument();
  next();
  next();
  expect(
    screen.getByText(
      "Computer local time · use the local time zone at the capture date",
    ),
  ).toBeVisible();
  expect(
    screen.getByRole("note", { name: "Synchronize camera and computer clocks" }),
  ).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Save configuration" }));
  expect(onSave).toHaveBeenCalledOnce();
  expect(onSave.mock.calls[0][0]).toEqual(
    expect.objectContaining({
      captureTimeBasis: "computer_local",
      sortPreset: "exif_hour",
      destinationDepthOrder: [
        { kind: "camera_model" },
        { kind: "capture_day" },
        { kind: "capture_interval" },
      ],
    }),
  );
});

test("busy configuration locks the time-basis controls and keeps its current selection", () => {
  const { props, rerender } = renderConfiguration();
  next();
  rerender(<CardConfiguration {...props} saving />);
  const camera = screen.getByRole("radio", { name: "Camera time" });
  const computer = screen.getByRole("radio", { name: "Computer local time" });
  expect(camera).toBeDisabled();
  expect(computer).toBeDisabled();
  fireEvent.click(computer);
  expect(camera).toBeChecked();
  expect(computer).not.toBeChecked();
});

test("a malformed saved time basis needs an explicit correction before progression", () => {
  const { onSave } = renderConfiguration({
    initialValue: createConfiguration({
      destinationPath: "D:/Ingest",
      captureTimeBasis: "system_now" as CaptureTimeBasis,
    }),
  });
  next();
  expect(screen.getByRole("radio", { name: "Camera time" })).not.toBeChecked();
  expect(screen.getByRole("radio", { name: "Computer local time" })).not.toBeChecked();
  next();
  expect(screen.getByRole("heading", { name: "Organization" })).toBeVisible();
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Choose camera time or computer local time.",
  );
  expect(onSave).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("radio", { name: "Camera time" }));
  next();
  expect(screen.getByRole("heading", { name: "Automation" })).toBeVisible();
  expect(screen.queryByRole("alert")).not.toBeInTheDocument();
});

test("original-folder review omits the time-basis choice and clock warning", () => {
  renderConfiguration({
    initialValue: createConfiguration({
      destinationPath: "D:/Ingest",
      sortPreset: "original_tree",
      captureTimeBasis: "computer_local",
    }),
  });
  goToReview();
  expect(screen.queryByText("Capture time basis")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("note", { name: "Synchronize camera and computer clocks" }),
  ).not.toBeInTheDocument();
});

test("original-folder settings expose a malformed time basis for correction without changing sorting", () => {
  const { onSave } = renderConfiguration({
    initialValue: createConfiguration({
      destinationPath: "D:/Ingest",
      sortPreset: "original_tree",
      captureTimeBasis: "system_now" as CaptureTimeBasis,
    }),
  });
  next();
  expect(screen.getByRole("group", { name: "Capture time basis" })).toBeVisible();
  expect(
    screen.queryByRole("note", { name: "Synchronize camera and computer clocks" }),
  ).not.toBeInTheDocument();
  next();
  expect(screen.getByRole("alert")).toHaveTextContent(
    "Choose camera time or computer local time.",
  );
  fireEvent.click(screen.getByRole("radio", { name: "Camera time" }));
  expect(
    screen.queryByRole("group", { name: "Capture time basis" }),
  ).not.toBeInTheDocument();
  next();
  next();
  expect(screen.getByRole("heading", { name: "Review configuration" })).toBeVisible();
  expect(
    screen.queryByRole("note", { name: "Synchronize camera and computer clocks" }),
  ).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Save configuration" }));
  expect(onSave).toHaveBeenCalledWith(
    expect.objectContaining({
      sortPreset: "original_tree",
      captureTimeBasis: "camera_recorded",
    }),
  );
});

test("cancelling a draft leaves the original destination and nested folders untouched", () => {
  const initialValue = createConfiguration({
    destinationPath: "D:\\Ingest\\Original",
    customDirectoryFields: [{ label: "Folder1", value: "Original folder" }],
    destinationDepthOrder: [
      { kind: "custom_field", index: 0 },
      { kind: "camera_model" },
      { kind: "capture_day" },
    ],
  });
  const original = structuredClone(initialValue);
  const { onSave, onDismiss } = renderConfiguration({ initialValue });
  fireEvent.change(screen.getByLabelText("Destination directory"), {
    target: { value: "E:\\Changed draft" },
  });
  next();
  fireEvent.click(screen.getByRole("radio", { name: "Computer local time" }));
  fireEvent.change(screen.getByLabelText("Custom folder 1 value"), {
    target: { value: "Changed folder" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onDismiss).toHaveBeenCalledOnce();
  expect(onSave).not.toHaveBeenCalled();
  expect(initialValue).toEqual(original);
  expect(initialValue.captureTimeBasis).toBe("camera_recorded");
});

test("Back retains destination, interval, and folder edits within the draft", () => {
  renderConfiguration();
  fireEvent.change(screen.getByLabelText("Destination directory"), {
    target: { value: "E:\\Ingest\\Revised" },
  });
  next();
  fireEvent.click(screen.getByRole("button", { name: "Custom interval" }));
  fireEvent.click(screen.getByRole("radio", { name: "Computer local time" }));
  fireEvent.change(screen.getByLabelText("Custom interval minutes"), {
    target: { value: "37" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Add folder" }));
  fireEvent.change(screen.getByLabelText("Custom folder 1 value"), {
    target: { value: "Day 03" },
  });
  next();
  fireEvent.click(screen.getByRole("button", { name: "Back" }));
  expect(screen.getByLabelText("Custom interval minutes")).toHaveValue(37);
  expect(screen.getByLabelText("Custom folder 1 value")).toHaveValue("Day 03");
  expect(screen.getByRole("radio", { name: "Computer local time" })).toBeChecked();
  fireEvent.click(screen.getByRole("button", { name: "Back" }));
  expect(screen.getByLabelText("Destination directory")).toHaveValue(
    "E:\\Ingest\\Revised",
  );
  next();
  expect(screen.getByLabelText("Custom interval minutes")).toHaveValue(37);
  expect(screen.getByLabelText("Custom folder 1 value")).toHaveValue("Day 03");
});

test("saves the full reviewed configuration only after the final confirmation", () => {
  const initialValue = createConfiguration({
    destinationPath: "D:\\Ingest\\A-Cam",
    sortPreset: "exif_custom",
    customIntervalMinutes: "37",
    customDirectoryFields: [{ label: "Folder1", value: "Day 03" }],
    destinationDepthOrder: [
      { kind: "capture_day" },
      { kind: "custom_field", index: 0 },
      { kind: "camera_model" },
      { kind: "capture_interval" },
    ],
    persistOnCard: true,
    autoIngestEnabled: true,
    autoFormatEnabled: true,
  });
  const original = structuredClone(initialValue);
  const { onSave, onDismiss } = renderConfiguration({ initialValue });
  goToReview();
  expect(onSave).not.toHaveBeenCalled();
  expect(screen.getByText("D:\\Ingest\\A-Cam", { exact: true })).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Save configuration" }));
  expect(onSave).toHaveBeenCalledExactlyOnceWith(original);
  expect(onDismiss).not.toHaveBeenCalled();
  expect(initialValue).toEqual(original);
});

test("requires persistence for automation and clears dependent automation choices", () => {
  const { onSave } = renderConfiguration();
  goToAutomation();
  const persistence = screen.getByLabelText(/Remember configuration on this card/);
  const ingest = screen.getByLabelText(/Ingest automatically on mount/);
  const format = screen.getByLabelText(/Format after a verified auto-ingest/);
  expect(ingest).toBeDisabled();
  expect(format).toBeDisabled();

  fireEvent.click(persistence);
  expect(ingest).toBeEnabled();
  expect(format).toBeDisabled();
  fireEvent.click(ingest);
  expect(format).toBeEnabled();
  fireEvent.click(format);
  expect(format).toBeChecked();
  fireEvent.click(ingest);
  expect(format).not.toBeChecked();
  expect(format).toBeDisabled();

  fireEvent.click(ingest);
  fireEvent.click(format);
  fireEvent.click(persistence);
  expect(ingest).not.toBeChecked();
  expect(format).not.toBeChecked();
  expect(ingest).toBeDisabled();
  expect(format).toBeDisabled();
  next();
  fireEvent.click(screen.getByRole("button", { name: "Save configuration" }));
  expect(onSave).toHaveBeenCalledWith(
    expect.objectContaining({
      persistOnCard: false,
      autoIngestEnabled: false,
      autoFormatEnabled: false,
    }),
  );
});

test("keeps a failed save visible in the review and preserves the retryable draft", () => {
  const { props, rerender, onSave, onDismiss } = renderConfiguration();
  goToReview();
  fireEvent.click(screen.getByRole("button", { name: "Save configuration" }));
  rerender(
    <CardConfiguration {...props} error="Card registration could not be saved." />,
  );
  expect(screen.getByText("Card registration could not be saved.")).toBeVisible();
  expect(screen.getByRole("alert")).toHaveFocus();
  expect(screen.getByRole("heading", { name: "Review configuration" })).toBeVisible();
  expect(screen.getByRole("button", { name: "Save configuration" })).toBeEnabled();
  expect(onDismiss).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Save configuration" }));
  expect(onSave).toHaveBeenCalledTimes(2);
  expect(onSave.mock.calls[1][0]).toEqual(onSave.mock.calls[0][0]);
});

test("blocks saving, Back, Cancel, and Escape while persistence is busy", () => {
  const { props, rerender, onSave, onDismiss } = renderConfiguration();
  goToReview();
  rerender(<CardConfiguration {...props} saving />);
  const save = screen.getByRole("button", {
    name: "Saving configuration…",
  });
  const back = screen.getByRole("button", { name: "Back" });
  const cancel = screen.getByRole("button", { name: "Cancel" });
  const close = screen.getByRole("button", { name: "Close card configuration" });
  expect(save).toBeDisabled();
  expect(back).toBeDisabled();
  expect(cancel).toBeDisabled();
  expect(close).toBeDisabled();
  fireEvent.click(save);
  fireEvent.click(back);
  fireEvent.click(cancel);
  fireEvent.click(close);
  fireEvent.keyDown(screen.getByRole("dialog", { name: "Configure card" }), {
    key: "Escape",
  });
  expect(onSave).not.toHaveBeenCalled();
  expect(onDismiss).not.toHaveBeenCalled();
  expect(screen.getByRole("heading", { name: "Review configuration" })).toBeVisible();
});

test("source loss blocks saving while keeping the draft dismissible", () => {
  const { props, rerender, onSave, onDismiss } = renderConfiguration();
  goToReview();
  rerender(<CardConfiguration {...props} sourceUnavailable />);
  const save = screen.getByRole("button", { name: "Save configuration" });
  expect(save).toBeDisabled();
  expect(screen.getByRole("alert")).toHaveTextContent(
    /this card is no longer available for configuration/i,
  );
  expect(screen.getByRole("button", { name: "Cancel" })).toBeEnabled();
  fireEvent.click(save);
  expect(onSave).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(onDismiss).toHaveBeenCalledOnce();
});

test("a successful native folder choice updates the draft destination", async () => {
  const onChooseDestination = vi.fn().mockResolvedValue("E:\\Chosen destination");
  renderConfiguration({ onChooseDestination });
  fireEvent.click(screen.getByRole("button", { name: "Choose folder" }));
  await waitFor(() =>
    expect(screen.getByLabelText("Destination directory")).toHaveValue(
      "E:\\Chosen destination",
    ),
  );
  expect(onChooseDestination).toHaveBeenCalledExactlyOnceWith("D:\\Ingest\\A-Cam");
});

test("cancelling the native folder picker preserves the destination draft", async () => {
  const onChooseDestination = vi.fn().mockResolvedValue(null);
  renderConfiguration({ onChooseDestination });
  fireEvent.click(screen.getByRole("button", { name: "Choose folder" }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Choose folder" })).toBeEnabled(),
  );
  expect(onChooseDestination).toHaveBeenCalledOnce();
  expect(screen.getByLabelText("Destination directory")).toHaveValue(
    "D:\\Ingest\\A-Cam",
  );
});

test("a native folder picker failure stays visible without discarding the draft", async () => {
  const onChooseDestination = vi
    .fn()
    .mockRejectedValue(new Error("Folder picker unavailable"));
  const { onSave, onDismiss } = renderConfiguration({ onChooseDestination });
  fireEvent.click(screen.getByRole("button", { name: "Choose folder" }));
  const error = await screen.findByRole("alert");
  expect(error).toBeVisible();
  expect(error).toHaveTextContent(/folder.*(unavailable|could not be completed)/i);
  expect(screen.getByLabelText("Destination directory")).toHaveValue(
    "D:\\Ingest\\A-Cam",
  );
  expect(screen.getByRole("button", { name: "Choose folder" })).toBeEnabled();
  expect(onSave).not.toHaveBeenCalled();
  expect(onDismiss).not.toHaveBeenCalled();
});

test("typing a custom folder retains the same focused input across draft updates", () => {
  renderConfiguration();
  next();
  fireEvent.click(screen.getByRole("button", { name: "Add folder" }));
  const folder = screen.getByLabelText("Custom folder 1 value");
  folder.focus();
  for (const value of ["D", "Da", "Day", "Day 03"]) {
    fireEvent.change(folder, { target: { value } });
    expect(screen.getByLabelText("Custom folder 1 value")).toBe(folder);
    expect(folder).toHaveFocus();
    expect(folder).toHaveValue(value);
  }
});

test("keyboard folder ordering retains tag focus and reaches the saved configuration", () => {
  const { onSave } = renderConfiguration();
  next();
  const camera = screen.getByRole("button", {
    name: "Move Camera model in destination order",
  });
  camera.focus();
  fireEvent.keyDown(camera, { key: "ArrowRight" });
  expect(camera).toHaveFocus();
  expect(
    screen
      .getAllByRole("button", { name: /move .* in destination order/i })
      .map((button) => button.getAttribute("aria-label")),
  ).toEqual([
    "Move EXIF capture day in destination order",
    "Move Camera model in destination order",
  ]);
  fireEvent.keyDown(camera, { key: "Home" });
  expect(camera).toHaveFocus();
  expect(
    screen
      .getAllByRole("button", { name: /move .* in destination order/i })
      .map((button) => button.getAttribute("aria-label")),
  ).toEqual([
    "Move Camera model in destination order",
    "Move EXIF capture day in destination order",
  ]);
  fireEvent.keyDown(camera, { key: "End" });
  expect(camera).toHaveFocus();
  expect(
    screen
      .getAllByRole("button", { name: /move .* in destination order/i })
      .map((button) => button.getAttribute("aria-label")),
  ).toEqual([
    "Move EXIF capture day in destination order",
    "Move Camera model in destination order",
  ]);
  fireEvent.keyDown(camera, { key: "ArrowLeft" });
  expect(camera).toHaveFocus();
  expect(
    screen
      .getAllByRole("button", { name: /move .* in destination order/i })
      .map((button) => button.getAttribute("aria-label")),
  ).toEqual([
    "Move Camera model in destination order",
    "Move EXIF capture day in destination order",
  ]);
  fireEvent.keyDown(camera, { key: "ArrowRight" });
  next();
  next();
  fireEvent.click(screen.getByRole("button", { name: "Save configuration" }));
  expect(onSave).toHaveBeenCalledWith(
    expect.objectContaining({
      destinationDepthOrder: [{ kind: "capture_day" }, { kind: "camera_model" }],
    }),
  );
});
