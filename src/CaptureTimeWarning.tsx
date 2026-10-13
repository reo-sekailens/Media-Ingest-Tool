import { TriangleAlert } from "lucide-react";

export function CaptureTimeWarning() {
  return (
    <aside
      role="note"
      aria-label="Synchronize camera and computer clocks"
      className="flex items-start gap-2 rounded-lg border border-amber-300 bg-amber-50 p-3 text-xs leading-5 text-amber-950 dark:border-amber-800 dark:bg-amber-950/70 dark:text-amber-200"
    >
      <TriangleAlert aria-hidden="true" className="mt-0.5 size-4 shrink-0" />
      <p className="m-0">
        <strong className="mb-1 block font-semibold">
          Synchronize clocks before using capture-time folders
        </strong>
        Check that the camera and computer have the correct date, time, time zone, and
        daylight saving time settings. An incorrect camera clock can place files in the
        wrong folders.
      </p>
    </aside>
  );
}
