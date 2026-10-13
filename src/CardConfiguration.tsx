import { Check, FolderOpen, GripVertical, Plus, TriangleAlert, X } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  cloneConfiguration,
  defaultDestinationDepthOrder,
  destinationDepthSegmentKey,
  destinationDepthSegmentLabel,
  MAX_CUSTOM_DIRECTORY_FIELDS,
  normalizeDestinationDepthOrder,
  resolveSortRequest,
  validateConfiguration,
  validateConfigurationStep,
  type ConfigurationDraft,
  type DestinationDepthSegment,
  type SortPreset,
} from "./configuration";
import { Modal } from "./Modal";
import "./CardConfiguration.css";

type CardConfigurationProps = {
  sourceName: string;
  initialValue: ConfigurationDraft;
  saving: boolean;
  error: string;
  sourceUnavailable: boolean;
  onSave: (draft: ConfigurationDraft) => void;
  onDismiss: () => void;
  onChooseDestination: (path: string) => Promise<string | null>;
};

const steps = ["Destination", "Organization", "Automation", "Review"] as const;
const validationSteps = ["destination", "organization", "automation"] as const;
const presets: readonly [SortPreset, string][] = [
  ["exif_day", "EXIF day"],
  ["exif_hour", "EXIF hour"],
  ["exif_custom", "Custom interval"],
  ["original_tree", "Original tree"],
];

function orderedSegments(draft: ConfigurationDraft) {
  return normalizeDestinationDepthOrder(
    draft.destinationDepthOrder,
    defaultDestinationDepthOrder(
      resolveSortRequest(draft.sortPreset, draft.customIntervalMinutes),
      draft.customDirectoryFields,
    ),
  );
}

export function CardConfiguration({
  sourceName,
  initialValue,
  saving,
  error,
  sourceUnavailable,
  onSave,
  onDismiss,
  onChooseDestination,
}: CardConfigurationProps) {
  const [draft, setDraft] = useState(() => cloneConfiguration(initialValue));
  const [step, setStep] = useState(0);
  const [stepError, setStepError] = useState("");
  const [pickerError, setPickerError] = useState("");
  const [choosingDestination, setChoosingDestination] = useState(false);
  const [fieldIds, setFieldIds] = useState(() =>
    initialValue.customDirectoryFields.map((_, index) => index),
  );
  const nextFieldId = useRef(initialValue.customDirectoryFields.length);
  const draggedKey = useRef<string | null>(null);
  const [draggedSegment, setDraggedSegment] = useState<string | null>(null);
  const [orderAnnouncement, setOrderAnnouncement] = useState("");
  const stepHeading = useRef<HTMLHeadingElement>(null);
  const errorMessage = useRef<HTMLParagraphElement>(null);
  const busy = saving || choosingDestination;
  const displayedError = stepError || error;
  const sortRequest = resolveSortRequest(draft.sortPreset, draft.customIntervalMinutes);
  const order = orderedSegments(draft);
  const folderLabels = order.map((segment) =>
    destinationDepthSegmentLabel(segment, draft.customDirectoryFields, sortRequest),
  );

  useLayoutEffect(() => {
    stepHeading.current?.focus();
  }, [step]);

  useLayoutEffect(() => {
    if (displayedError) errorMessage.current?.focus();
  }, [displayedError]);

  useEffect(() => {
    const clearDrag = () => {
      draggedKey.current = null;
      setDraggedSegment(null);
    };
    document.addEventListener("pointerup", clearDrag);
    document.addEventListener("pointercancel", clearDrag);
    return () => {
      document.removeEventListener("pointerup", clearDrag);
      document.removeEventListener("pointercancel", clearDrag);
    };
  }, []);

  const updateDraft = (patch: Partial<ConfigurationDraft>) => {
    if (busy) return;
    setStepError("");
    setDraft((current) => {
      const next = { ...current, ...patch };
      const hasExplicitOrder =
        current.destinationDepthOrder.length > 0 ||
        patch.destinationDepthOrder !== undefined;
      return {
        ...next,
        destinationDepthOrder: hasExplicitOrder
          ? orderedSegments(next)
          : next.destinationDepthOrder,
      };
    });
  };

  const moveFolder = (key: string, targetKey: string) => {
    if (busy || key === targetKey) return;
    const from = order.findIndex(
      (segment) => destinationDepthSegmentKey(segment) === key,
    );
    const to = order.findIndex(
      (segment) => destinationDepthSegmentKey(segment) === targetKey,
    );
    if (from < 0 || to < 0) return;
    const next = [...order];
    const [moved] = next.splice(from, 1);
    next.splice(to, 0, moved);
    updateDraft({ destinationDepthOrder: next });
    setOrderAnnouncement(
      `Moved ${folderLabels[from]} to position ${to + 1} of ${order.length}.`,
    );
  };

  const removeFolder = (index: number) => {
    if (busy) return;
    const nextOrder = order.flatMap((segment): DestinationDepthSegment[] => {
      if (segment.kind !== "custom_field") return [segment];
      if (segment.index === index) return [];
      return [
        {
          ...segment,
          index: segment.index > index ? segment.index - 1 : segment.index,
        },
      ];
    });
    setFieldIds((current) => current.filter((_, fieldIndex) => fieldIndex !== index));
    updateDraft({
      customDirectoryFields: draft.customDirectoryFields.filter(
        (_, fieldIndex) => fieldIndex !== index,
      ),
      destinationDepthOrder: nextOrder,
    });
  };

  const chooseDestination = async () => {
    if (busy) return;
    setChoosingDestination(true);
    setPickerError("");
    try {
      const selected = await onChooseDestination(draft.destinationPath);
      if (selected !== null) {
        setDraft((current) => ({ ...current, destinationPath: selected }));
        setStepError("");
      }
    } catch (failure) {
      setPickerError(
        failure instanceof Error && failure.message
          ? failure.message
          : "Folder selection could not be completed. You can enter a destination directory below.",
      );
    } finally {
      setChoosingDestination(false);
    }
  };

  const nextStep = () => {
    if (busy) return;
    const validationStep = validationSteps[step];
    if (!validationStep) return;
    const problem = validateConfigurationStep(draft, validationStep);
    if (problem) {
      setStepError(problem);
      return;
    }
    setStepError("");
    setStep((current) => Math.min(current + 1, steps.length - 1));
  };

  const save = () => {
    if (busy || sourceUnavailable) return;
    const reviewedDraft = cloneConfiguration({
      ...draft,
      destinationDepthOrder: order,
    });
    const problem = validateConfiguration(reviewedDraft);
    if (problem) {
      setStepError(problem);
      return;
    }
    onSave(reviewedDraft);
  };

  return (
    <Modal
      labelledBy="card-configuration-title"
      className="card-configuration-overlay fixed inset-0 z-50 grid place-items-center bg-slate-950/45 p-4 backdrop-blur-sm"
      dismissible={!busy}
      onDismiss={onDismiss}
      initialFocus="#card-configuration-title"
    >
      <section className="card-configuration" aria-busy={busy}>
        <header className="card-configuration__header">
          <div>
            <p className="card-configuration__eyebrow">CARD SETUP</p>
            <h2 id="card-configuration-title" tabIndex={-1}>
              Configure card
            </h2>
            <p className="card-configuration__source">{sourceName}</p>
          </div>
          <button
            className="card-configuration__close"
            aria-label="Close card configuration"
            disabled={busy}
            onClick={onDismiss}
            type="button"
          >
            <X aria-hidden="true" size={18} />
          </button>
        </header>
        <ol className="card-configuration__steps" aria-label="Configuration steps">
          {steps.map((label, index) => (
            <li
              className={
                index === step ? "is-current" : index < step ? "is-complete" : ""
              }
              key={label}
              aria-current={index === step ? "step" : undefined}
            >
              <span className="card-configuration__step-number" aria-hidden="true">
                {index < step ? <Check size={14} /> : index + 1}
              </span>
              <span>{label}</span>
              <span className="sr-only">
                {index < step ? ", completed" : index === step ? ", current step" : ""}
              </span>
            </li>
          ))}
        </ol>
        <div className="card-configuration__body">
          <h3 ref={stepHeading} tabIndex={-1}>
            {step === 3 ? "Review configuration" : steps[step]}
          </h3>
          {sourceUnavailable ? (
            <p className="card-configuration__warning" role="alert">
              This card is no longer available for configuration. You can review your
              draft or cancel; saving is disabled.
            </p>
          ) : null}
          {displayedError ? (
            <p
              className="card-configuration__error"
              ref={errorMessage}
              role="alert"
              tabIndex={-1}
            >
              {displayedError}
            </p>
          ) : null}
          <fieldset className="card-configuration__fields" disabled={busy}>
            <legend className="sr-only">{steps[step]} settings</legend>
            {step === 0 ? (
              <div className="card-configuration__stack">
                <p className="card-configuration__description">
                  Choose where this card&apos;s verified copies will be saved.
                </p>
                <label
                  className="card-configuration__label"
                  htmlFor="card-configuration-destination"
                >
                  Destination directory
                </label>
                <div className="card-configuration__destination">
                  <input
                    id="card-configuration-destination"
                    className="card-configuration__input card-configuration__input--path"
                    autoComplete="off"
                    spellCheck={false}
                    placeholder="Choose or enter a destination folder"
                    value={draft.destinationPath}
                    aria-describedby="card-configuration-destination-help"
                    aria-invalid={Boolean(stepError && !draft.destinationPath.trim())}
                    onChange={(event) =>
                      updateDraft({ destinationPath: event.target.value })
                    }
                  />
                  <button
                    className="card-configuration__button"
                    onClick={() => void chooseDestination()}
                    type="button"
                  >
                    <FolderOpen aria-hidden="true" size={16} />
                    {choosingDestination ? "Choosing folder…" : "Choose folder"}
                  </button>
                </div>
                <p
                  id="card-configuration-destination-help"
                  className="card-configuration__hint"
                >
                  You can also type the full path. The desktop service checks the
                  destination before an ingest starts.
                </p>
                {pickerError ? (
                  <p className="card-configuration__error" role="alert">
                    {pickerError}
                  </p>
                ) : null}
              </div>
            ) : null}
            {step === 1 ? (
              <div className="card-configuration__stack">
                <p className="card-configuration__description">
                  Organize by capture metadata, or retain the card&apos;s original
                  folders.
                </p>
                <div className="card-configuration__presets" aria-label="Sort rule">
                  {presets.map(([value, label]) => (
                    <button
                      className={`card-configuration__preset ${draft.sortPreset === value ? "is-selected" : ""}`}
                      aria-pressed={draft.sortPreset === value}
                      key={value}
                      onClick={() => updateDraft({ sortPreset: value })}
                      type="button"
                    >
                      {label}
                    </button>
                  ))}
                </div>
                {draft.sortPreset === "exif_custom" ? (
                  <label className="card-configuration__interval">
                    Every{" "}
                    <input
                      aria-label="Custom interval minutes"
                      className="card-configuration__input"
                      type="number"
                      min={1}
                      max={1440}
                      step={1}
                      value={draft.customIntervalMinutes}
                      aria-invalid={Boolean(stepError && !sortRequest)}
                      onChange={(event) =>
                        updateDraft({ customIntervalMinutes: event.target.value })
                      }
                    />{" "}
                    minutes{" "}
                    <span className="card-configuration__hint">1–1,440 minutes</span>
                  </label>
                ) : null}
                {draft.sortPreset === "exif_hour" ? (
                  <p className="card-configuration__hint">
                    EXIF hour example: a 13:42 capture with a recorded +08:00 offset
                    uses <code>1300-1400+0800</code>. The capture date stays in a
                    separate folder.
                  </p>
                ) : null}
                <p className="card-configuration__hint">
                  EXIF capture time is used when available. Missing or invalid metadata
                  uses the source file timestamp; unknown camera identities remain
                  separate.
                </p>
                <div className="card-configuration__section-heading">
                  <div>
                    <h4>Custom folders</h4>
                    <p className="card-configuration__hint">
                      Each entry adds one folder, such as Photographer — Ari.
                    </p>
                  </div>
                  <button
                    className="card-configuration__button"
                    disabled={
                      busy ||
                      draft.customDirectoryFields.length >= MAX_CUSTOM_DIRECTORY_FIELDS
                    }
                    onClick={() => {
                      const id = nextFieldId.current++;
                      setFieldIds((current) => [...current, id]);
                      updateDraft({
                        customDirectoryFields: [
                          ...draft.customDirectoryFields,
                          { label: "", value: "" },
                        ],
                      });
                    }}
                    type="button"
                  >
                    <Plus aria-hidden="true" size={14} />
                    Add folder
                  </button>
                </div>
                {draft.customDirectoryFields.map((field, index) => (
                  <div className="card-configuration__folder-row" key={fieldIds[index]}>
                    <input
                      className="card-configuration__input"
                      aria-label={`Custom folder ${index + 1} value`}
                      value={field.value}
                      placeholder="Folder name"
                      onChange={(event) =>
                        updateDraft({
                          customDirectoryFields: draft.customDirectoryFields.map(
                            (current, currentIndex) =>
                              currentIndex === index
                                ? { ...current, value: event.target.value }
                                : current,
                          ),
                        })
                      }
                    />
                    <button
                      className="card-configuration__button"
                      aria-label={`Remove custom folder ${index + 1}`}
                      onClick={() => removeFolder(index)}
                      type="button"
                    >
                      Remove
                    </button>
                  </div>
                ))}
                <div className="card-configuration__layout">
                  <h4>Folder order</h4>
                  <p
                    className="card-configuration__hint"
                    id="card-configuration-order-help"
                  >
                    Drag tags, or focus a tag and use Left/Right arrows, Home, or End.
                    Filename stays last.
                  </p>
                  <ol
                    className="card-configuration__folders"
                    aria-label="Destination folder order"
                  >
                    {order.map((segment, index) => {
                      const key = destinationDepthSegmentKey(segment);
                      const label = folderLabels[index];
                      return (
                        <li
                          key={
                            segment.kind === "custom_field"
                              ? `custom-${fieldIds[segment.index]}`
                              : key
                          }
                        >
                          <button
                            className={`card-configuration__folder-tag ${draggedSegment === key ? "is-dragging" : ""}`}
                            draggable={!busy}
                            aria-label={`Move ${label} in destination order`}
                            aria-describedby="card-configuration-order-help"
                            aria-keyshortcuts="ArrowLeft ArrowRight Home End"
                            onDragStart={(event) => {
                              if (busy) return;
                              draggedKey.current = key;
                              setDraggedSegment(key);
                              event.dataTransfer.effectAllowed = "move";
                              event.dataTransfer.setData("text/plain", key);
                            }}
                            onDragOver={(event) => event.preventDefault()}
                            onDrop={(event) => {
                              event.preventDefault();
                              const moved =
                                event.dataTransfer.getData("text/plain") ||
                                draggedKey.current;
                              if (moved) moveFolder(moved, key);
                              draggedKey.current = null;
                              setDraggedSegment(null);
                            }}
                            onDragEnd={() => {
                              draggedKey.current = null;
                              setDraggedSegment(null);
                            }}
                            onPointerDown={() => {
                              if (busy) return;
                              draggedKey.current = key;
                              setDraggedSegment(key);
                            }}
                            onPointerEnter={() => {
                              if (draggedKey.current)
                                moveFolder(draggedKey.current, key);
                            }}
                            onKeyDown={(event) => {
                              const target =
                                event.key === "ArrowLeft"
                                  ? index - 1
                                  : event.key === "ArrowRight"
                                    ? index + 1
                                    : event.key === "Home"
                                      ? 0
                                      : event.key === "End"
                                        ? order.length - 1
                                        : null;
                              if (target === null) return;
                              event.preventDefault();
                              if (order[target])
                                moveFolder(
                                  key,
                                  destinationDepthSegmentKey(order[target]),
                                );
                            }}
                            type="button"
                          >
                            <GripVertical aria-hidden="true" size={12} />
                            {label}
                          </button>
                          <span aria-hidden="true">/</span>
                        </li>
                      );
                    })}
                    <li>
                      <span className="card-configuration__filename">Filename</span>
                    </li>
                  </ol>
                  <p className="sr-only" role="status" aria-live="polite">
                    {orderAnnouncement}
                  </p>
                </div>
              </div>
            ) : null}
            {step === 2 ? (
              <div className="card-configuration__stack">
                <p className="card-configuration__description">
                  Keep this setup for the session, or register this card for future
                  matching mounts.
                </p>
                <label className="card-configuration__option">
                  <input
                    type="checkbox"
                    aria-label="Remember configuration on this card"
                    aria-describedby="card-configuration-persistence-help"
                    checked={draft.persistOnCard}
                    onChange={(event) =>
                      updateDraft({
                        persistOnCard: event.target.checked,
                        ...(!event.target.checked
                          ? { autoIngestEnabled: false, autoFormatEnabled: false }
                          : {}),
                      })
                    }
                  />
                  <span>
                    <strong>Remember configuration on this card</strong>
                    <span
                      id="card-configuration-persistence-help"
                      className="card-configuration__hint"
                    >
                      Register a marker on the card and save its setup in this app. When
                      off, this setup stays in the current session; previously saved
                      card settings remain available for future mounts.
                    </span>
                  </span>
                </label>
                <label className="card-configuration__option">
                  <input
                    type="checkbox"
                    aria-label="Ingest automatically on mount"
                    aria-describedby="card-configuration-auto-ingest-help"
                    checked={draft.autoIngestEnabled}
                    disabled={busy || !draft.persistOnCard}
                    onChange={(event) =>
                      updateDraft({
                        autoIngestEnabled: event.target.checked,
                        ...(!event.target.checked ? { autoFormatEnabled: false } : {}),
                      })
                    }
                  />
                  <span>
                    <strong>Ingest automatically on mount</strong>
                    <span
                      id="card-configuration-auto-ingest-help"
                      className="card-configuration__hint"
                    >
                      Requires a remembered configuration, a fresh matching marker, and
                      an available destination.
                    </span>
                  </span>
                </label>
                <label className="card-configuration__option">
                  <input
                    type="checkbox"
                    aria-label="Format after a verified auto-ingest"
                    aria-describedby="card-configuration-auto-format-help"
                    checked={draft.autoFormatEnabled}
                    disabled={busy || !draft.persistOnCard || !draft.autoIngestEnabled}
                    onChange={(event) =>
                      updateDraft({ autoFormatEnabled: event.target.checked })
                    }
                  />
                  <span>
                    <strong>Format after a verified auto-ingest</strong>
                    <span
                      id="card-configuration-auto-format-help"
                      className="card-configuration__hint"
                    >
                      Quick formatting removes filesystem structures. It runs only when
                      native safety checks and the platform format provider allow it.
                    </span>
                  </span>
                </label>
                <div className="card-configuration__warning">
                  <TriangleAlert aria-hidden="true" size={17} />
                  <p>
                    The marker can be changed, copied, or removed by other software. It
                    is not proof of immutable card identity. Quick formatting is not
                    secure erasure; old data may remain recoverable.
                  </p>
                </div>
              </div>
            ) : null}
            {step === 3 ? (
              <div className="card-configuration__stack">
                <p className="card-configuration__description">
                  Check this card&apos;s setup before saving. Back lets you make
                  changes.
                </p>
                <dl className="card-configuration__review">
                  <div>
                    <dt>Source card</dt>
                    <dd>{sourceName}</dd>
                  </div>
                  <div>
                    <dt>Destination directory</dt>
                    <dd className="card-configuration__review-path">
                      {draft.destinationPath}
                    </dd>
                  </div>
                  <div>
                    <dt>Organization</dt>
                    <dd>
                      {presets.find(([preset]) => preset === draft.sortPreset)?.[1]}
                      {draft.sortPreset === "exif_custom"
                        ? ` · every ${sortRequest?.intervalMinutes} minutes`
                        : ""}
                    </dd>
                  </div>
                  <div>
                    <dt>Folder order</dt>
                    <dd className="card-configuration__review-path">
                      {[...folderLabels, "Filename"].join(" / ")}
                    </dd>
                  </div>
                  <div>
                    <dt>Saved configuration</dt>
                    <dd>
                      {draft.persistOnCard
                        ? "Write or update the mutable card marker and save this setup for future matching mounts"
                        : "Session only · previously saved card settings stay in place for future mounts"}
                    </dd>
                  </div>
                  <div>
                    <dt>Automatic ingest</dt>
                    <dd>
                      {draft.autoIngestEnabled
                        ? "On · begin on a matching mount"
                        : "Off"}
                    </dd>
                  </div>
                  <div>
                    <dt>Automatic format</dt>
                    <dd>
                      {draft.autoFormatEnabled
                        ? "On · after a verified auto-ingest and native safety checks"
                        : "Off"}
                    </dd>
                  </div>
                </dl>
                {draft.persistOnCard ? (
                  <p className="card-configuration__hint">
                    The saved card marker is mutable. Every ingest and format remains
                    subject to the native service&apos;s checks.
                  </p>
                ) : null}
              </div>
            ) : null}
          </fieldset>
        </div>
        <footer className="card-configuration__footer">
          <button
            className="card-configuration__button card-configuration__cancel"
            disabled={busy}
            onClick={onDismiss}
            type="button"
          >
            Cancel
          </button>
          <span className="card-configuration__footer-step">
            Step {step + 1} of {steps.length}
          </span>
          {step > 0 ? (
            <button
              className="card-configuration__button"
              disabled={busy}
              onClick={() => {
                setStepError("");
                setStep((current) => current - 1);
              }}
              type="button"
            >
              Back
            </button>
          ) : null}
          <button
            className="card-configuration__button card-configuration__button--primary"
            disabled={busy || (step === 3 && sourceUnavailable)}
            onClick={step === 3 ? save : nextStep}
            type="button"
          >
            {saving
              ? "Saving configuration…"
              : step === 3
                ? "Save configuration"
                : "Next"}
          </button>
        </footer>
      </section>
    </Modal>
  );
}

export default CardConfiguration;
