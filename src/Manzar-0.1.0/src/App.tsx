import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./App.css";
import { pickImageFiles, pickImageFolder } from "./fileDialogs";
import { useImagePresentation } from "./useImagePresentation";
import {
  backendErrorMessage,
  getViewerSnapshot,
  navigateNext,
  navigatePrevious,
  openFolder,
  openImageSelection,
  openSingleImage,
  renameCurrentImage,
  setSequenceOrdering,
  trashCurrentImage,
  type ImagePreflight,
  type OversizedImageReason,
  type SequenceOrdering,
  type ViewerSnapshot,
} from "./viewerCommands";

type NavigationDirection = "next" | "previous";

type SequenceOrderingOption = {
  value: SequenceOrdering;
  label: string;
};

type ControlIconName =
  | "actual-size"
  | "chevron-left"
  | "chevron-right"
  | "fit"
  | "folder"
  | "fullscreen-enter"
  | "fullscreen-exit"
  | "image"
  | "rename"
  | "trash"
  | "zoom-in"
  | "zoom-out";

const defaultSequenceOrdering: SequenceOrdering = "newest_modified_first";

const sequenceOrderingOptions: SequenceOrderingOption[] = [
  { value: "newest_modified_first", label: "Newest modified" },
  { value: "natural_name", label: "Name" },
  { value: "size_largest_first", label: "Largest first" },
  { value: "size_smallest_first", label: "Smallest first" },
];

function App() {
  const [snapshot, setSnapshot] = useState<ViewerSnapshot | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [isFullscreen, setIsFullscreen] = useState(false);
  const [approvedOversizedImageId, setApprovedOversizedImageId] = useState<
    string | null
  >(null);
  const [isOversizedDialogOpen, setIsOversizedDialogOpen] = useState(false);
  const [isRenameDialogOpen, setIsRenameDialogOpen] = useState(false);
  const [renameDraft, setRenameDraft] = useState("");
  const [isTrashDialogOpen, setIsTrashDialogOpen] = useState(false);
  const viewerShellRef = useRef<HTMLElement | null>(null);

  const current = snapshot?.current ?? null;
  const isCurrentOversized = current?.preflight.oversized ?? false;
  const shouldGateOversizedImage = Boolean(
    current && isCurrentOversized && approvedOversizedImageId !== current.id,
  );
  const canDisplayCurrent = Boolean(current && !shouldGateOversizedImage);
  const hasOpenDialog =
    isOversizedDialogOpen || isRenameDialogOpen || isTrashDialogOpen;
  const activeSequenceOrdering =
    snapshot?.sequence_ordering ?? defaultSequenceOrdering;
  const {
    mode,
    scale,
    isPannable,
    isPanning,
    imageClassName,
    imageStyle,
    zoomIn,
    zoomOut,
    resetActualSize,
    fitToWindow,
    startPan,
    updatePan,
    endPan,
  } = useImagePresentation(canDisplayCurrent ? (current?.id ?? null) : null);
  const sequenceLabel = useMemo(() => {
    if (!snapshot?.current_position || snapshot.count === 0) {
      return "No image open";
    }

    return `${snapshot.current_position} / ${snapshot.count}`;
  }, [snapshot]);
  const renameValidationMessage = useMemo(
    () => validateRenameStem(renameDraft),
    [renameDraft],
  );

  useEffect(() => {
    let cancelled = false;

    setIsLoading(true);
    setErrorMessage(null);

    getViewerSnapshot()
      .then((initialSnapshot) => {
        if (!cancelled) {
          setSnapshot((currentSnapshot) => currentSnapshot ?? initialSnapshot);
        }
      })
      .catch((error) => {
        if (!cancelled) {
          setErrorMessage(backendErrorMessage(error));
        }
      })
      .finally(() => {
        if (!cancelled) {
          setIsLoading(false);
        }
      });

    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!current) {
      setApprovedOversizedImageId(null);
      setIsOversizedDialogOpen(false);
      return;
    }

    if (!current.preflight.oversized) {
      setIsOversizedDialogOpen(false);
      return;
    }

    if (approvedOversizedImageId !== current.id) {
      setIsOversizedDialogOpen(true);
    }
  }, [approvedOversizedImageId, current]);

  const openImagesFromDialog = useCallback(async () => {
    if (isLoading) {
      return;
    }

    setIsLoading(true);
    setErrorMessage(null);

    try {
      const paths = await pickImageFiles();

      if (paths === null) {
        return;
      }

      const nextSnapshot =
        paths.length === 1
          ? await openSingleImage(paths[0])
          : await openImageSelection(paths);
      setSnapshot(nextSnapshot);
      setApprovedOversizedImageId(null);
    } catch (error) {
      setErrorMessage(backendErrorMessage(error));
    } finally {
      setIsLoading(false);
    }
  }, [isLoading]);

  const openFolderFromDialog = useCallback(async () => {
    if (isLoading) {
      return;
    }

    setIsLoading(true);
    setErrorMessage(null);

    try {
      const path = await pickImageFolder();

      if (path === null) {
        return;
      }

      setSnapshot(await openFolder(path));
      setApprovedOversizedImageId(null);
    } catch (error) {
      setErrorMessage(backendErrorMessage(error));
    } finally {
      setIsLoading(false);
    }
  }, [isLoading]);

  const navigate = useCallback(
    async (direction: NavigationDirection) => {
      if (!current || isLoading) {
        return;
      }

      setIsLoading(true);
      setErrorMessage(null);

      try {
        const nextSnapshot =
          direction === "next"
            ? await navigateNext()
            : await navigatePrevious();
        setSnapshot(nextSnapshot);
        setApprovedOversizedImageId(null);
      } catch (error) {
        setErrorMessage(backendErrorMessage(error));
      } finally {
        setIsLoading(false);
      }
    },
    [current, isLoading],
  );

  const changeSequenceOrdering = useCallback(
    async (ordering: SequenceOrdering) => {
      if (ordering === activeSequenceOrdering || isLoading) {
        return;
      }

      setIsLoading(true);
      setErrorMessage(null);

      try {
        setSnapshot(await setSequenceOrdering(ordering));
        setApprovedOversizedImageId(null);
      } catch (error) {
        setErrorMessage(backendErrorMessage(error));
      } finally {
        setIsLoading(false);
      }
    },
    [activeSequenceOrdering, isLoading],
  );

  const openRenameDialog = useCallback(() => {
    if (!current || isLoading) {
      return;
    }

    setRenameDraft("");
    setIsRenameDialogOpen(true);
  }, [current, isLoading]);

  const submitRename = useCallback(async () => {
    if (!current || isLoading) {
      return;
    }

    const validationMessage = validateRenameStem(renameDraft);
    if (validationMessage) {
      setErrorMessage(validationMessage);
      return;
    }

    setIsLoading(true);
    setErrorMessage(null);

    try {
      setSnapshot(await renameCurrentImage(renameDraft.trim()));
      setApprovedOversizedImageId(null);
      setIsRenameDialogOpen(false);
      setRenameDraft("");
    } catch (error) {
      setErrorMessage(backendErrorMessage(error));
    } finally {
      setIsLoading(false);
    }
  }, [current, isLoading, renameDraft]);

  const openTrashDialog = useCallback(() => {
    if (!current || isLoading) {
      return;
    }

    setIsTrashDialogOpen(true);
  }, [current, isLoading]);

  const confirmTrashDeletion = useCallback(async () => {
    if (!current || isLoading) {
      return;
    }

    setIsLoading(true);
    setErrorMessage(null);

    try {
      setSnapshot(await trashCurrentImage());
      setApprovedOversizedImageId(null);
      setIsTrashDialogOpen(false);
    } catch (error) {
      setErrorMessage(backendErrorMessage(error));
    } finally {
      setIsLoading(false);
    }
  }, [current, isLoading]);

  const confirmOversizedImage = useCallback(() => {
    if (!current) {
      return;
    }

    setApprovedOversizedImageId(current.id);
    setIsOversizedDialogOpen(false);
  }, [current]);

  const minimizeWindow = useCallback(() => {
    void getCurrentWindow().minimize();
  }, []);

  const toggleMaximizeWindow = useCallback(() => {
    void getCurrentWindow().toggleMaximize();
  }, []);

  const closeWindow = useCallback(() => {
    void getCurrentWindow().close();
  }, []);

  const toggleFullscreen = useCallback(async () => {
    const viewerShell = viewerShellRef.current;

    if (!viewerShell) {
      return;
    }

    if (document.fullscreenElement) {
      await document.exitFullscreen();
      return;
    }

    await viewerShell.requestFullscreen();
  }, []);

  const closeTransientUi = useCallback(() => {
    if (isRenameDialogOpen) {
      setIsRenameDialogOpen(false);
      return true;
    }

    if (isTrashDialogOpen) {
      setIsTrashDialogOpen(false);
      return true;
    }

    if (isOversizedDialogOpen) {
      setIsOversizedDialogOpen(false);
      return true;
    }

    if (document.fullscreenElement) {
      void document.exitFullscreen();
      return true;
    }

    if (errorMessage) {
      setErrorMessage(null);
      return true;
    }

    return false;
  }, [
    errorMessage,
    isOversizedDialogOpen,
    isRenameDialogOpen,
    isTrashDialogOpen,
  ]);

  useEffect(() => {
    const onFullscreenChange = () => {
      setIsFullscreen(document.fullscreenElement === viewerShellRef.current);
    };

    document.addEventListener("fullscreenchange", onFullscreenChange);
    return () =>
      document.removeEventListener("fullscreenchange", onFullscreenChange);
  }, []);

  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) {
        return;
      }

      if (event.key === "Escape") {
        if (closeTransientUi()) {
          event.preventDefault();
        }
        return;
      }

      if (isEditableTarget(event.target)) {
        return;
      }

      if (hasOpenDialog) {
        return;
      }

      if (event.key === "ArrowRight") {
        event.preventDefault();
        void navigate("next");
        return;
      }

      if (event.key === "ArrowLeft") {
        event.preventDefault();
        void navigate("previous");
        return;
      }

      if (event.key === " " || event.key === "Backspace") {
        event.preventDefault();
        return;
      }

      if (event.key === "F11") {
        event.preventDefault();
        void toggleFullscreen();
        return;
      }

      if (
        !event.altKey &&
        !event.metaKey &&
        (event.key === "+" || event.key === "=" || event.key === "-")
      ) {
        if (canDisplayCurrent) {
          event.preventDefault();
          if (event.key === "-") {
            zoomOut();
          } else {
            zoomIn();
          }
        }
        return;
      }

      if (event.altKey || event.ctrlKey || event.metaKey) {
        return;
      }

      if (event.key === "0") {
        if (canDisplayCurrent) {
          event.preventDefault();
          resetActualSize();
        }
        return;
      }

      if (event.key.toLowerCase() === "f") {
        if (canDisplayCurrent) {
          event.preventDefault();
          fitToWindow();
        }
        return;
      }

      if (event.key.toLowerCase() === "o") {
        event.preventDefault();
        void (event.shiftKey ? openFolderFromDialog() : openImagesFromDialog());
        return;
      }

      if (event.key.toLowerCase() === "r") {
        if (current) {
          event.preventDefault();
          openRenameDialog();
        }
        return;
      }

      if (event.key === "Delete") {
        if (current) {
          event.preventDefault();
          openTrashDialog();
        }
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [
    canDisplayCurrent,
    closeTransientUi,
    current,
    fitToWindow,
    hasOpenDialog,
    navigate,
    openFolderFromDialog,
    openImagesFromDialog,
    openRenameDialog,
    openTrashDialog,
    resetActualSize,
    toggleFullscreen,
    zoomIn,
    zoomOut,
  ]);

  return (
    <main ref={viewerShellRef} className="viewer-shell" aria-busy={isLoading}>
      <header className="window-titlebar" aria-label="Window controls">
        <div className="window-titlebar__drag-region" data-tauri-drag-region>
          <span className="window-titlebar__title" data-tauri-drag-region>
            Manzar
          </span>
        </div>
        <div className="window-titlebar__controls">
          <button
            type="button"
            className="window-titlebar__button"
            onClick={minimizeWindow}
            aria-label="Minimize window"
            title="Minimize"
          >
            <span aria-hidden="true">−</span>
          </button>
          <button
            type="button"
            className="window-titlebar__button"
            onClick={toggleMaximizeWindow}
            aria-label="Maximize or restore window"
            title="Maximize or restore"
          >
            <span aria-hidden="true">□</span>
          </button>
          <button
            type="button"
            className="window-titlebar__button window-titlebar__button--close"
            onClick={closeWindow}
            aria-label="Close window"
            title="Close"
          >
            <span aria-hidden="true">×</span>
          </button>
        </div>
      </header>

      <section
        className={`viewer-stage${isPannable ? " viewer-stage--pannable" : ""}${isPanning ? " viewer-stage--panning" : ""}`}
        aria-label="Image viewer"
        onWheel={(event) => {
          if (!event.ctrlKey || !canDisplayCurrent || hasOpenDialog) {
            return;
          }

          event.preventDefault();
          if (event.deltaY < 0) {
            zoomIn();
          } else if (event.deltaY > 0) {
            zoomOut();
          }
        }}
        onPointerDown={(event) => {
          if (!canDisplayCurrent || !isPannable || event.button !== 0) {
            return;
          }

          event.preventDefault();
          event.currentTarget.setPointerCapture(event.pointerId);
          startPan(event.pointerId, event.clientX, event.clientY);
        }}
        onPointerMove={(event) => {
          updatePan(event.pointerId, event.clientX, event.clientY);
        }}
        onPointerUp={(event) => {
          endPan(event.pointerId);
        }}
        onPointerCancel={(event) => {
          endPan(event.pointerId);
        }}
      >
        {canDisplayCurrent && current ? (
          <div
            className={`viewer-image-frame${mode === "fit" ? " viewer-image-frame--fit" : ""}`}
          >
            <img
              className={imageClassName}
              style={imageStyle}
              src={current.url}
              alt="Current image"
              draggable={false}
            />
          </div>
        ) : current && shouldGateOversizedImage ? (
          <div className="empty-state">
            <p className="eyebrow">Large image</p>
            <h1>Display paused</h1>
            <p>This image may be slow to display.</p>
            <button
              type="button"
              className="inline-action"
              onClick={() => setIsOversizedDialogOpen(true)}
              disabled={isLoading}
            >
              Review Warning
            </button>
          </div>
        ) : (
          <div className="empty-state">
            <img
              className="app-logo"
              src="/manzar-logo.svg"
              alt=""
              aria-hidden="true"
            />
            <p className="eyebrow">Manzar image viewer</p>
            <h1>No image open</h1>
            <p>Open an image or folder to start viewing.</p>
          </div>
        )}

        {isLoading ? <div className="status-pill">Loading…</div> : null}

        {errorMessage ? (
          <div className="error-banner" role="alert">
            {errorMessage}
          </div>
        ) : null}

        {canDisplayCurrent && current?.preflight.oversized ? (
          <div className="warning-banner" role="status">
            Large image: display may be slow.
          </div>
        ) : null}
      </section>

      <footer className="viewer-controls" aria-label="Viewer controls">
        <div className="viewer-controls__group viewer-controls__group--start">
          <button
            type="button"
            className="viewer-icon-button"
            onClick={() => void openImagesFromDialog()}
            disabled={isLoading}
            aria-label="Open images"
            title="Open images… (O)"
            aria-keyshortcuts="O"
          >
            <ControlIcon name="image" />
          </button>
          <button
            type="button"
            className="viewer-icon-button"
            onClick={() => void openFolderFromDialog()}
            disabled={isLoading}
            aria-label="Open folder"
            title="Open folder… (Shift+O)"
            aria-keyshortcuts="Shift+O"
          >
            <ControlIcon name="folder" />
          </button>
          <label className="ordering-control">
            <span>Sort</span>
            <select
              value={activeSequenceOrdering}
              onChange={(event) =>
                void changeSequenceOrdering(
                  event.currentTarget.value as SequenceOrdering,
                )
              }
              disabled={isLoading}
              aria-label="Sequence ordering"
              title="Sequence ordering"
            >
              {sequenceOrderingOptions.map((option) => (
                <option key={option.value} value={option.value}>
                  {option.label}
                </option>
              ))}
            </select>
          </label>
        </div>

        <div className="viewer-controls__group viewer-controls__group--center">
          <button
            type="button"
            className="viewer-icon-button viewer-icon-button--nav"
            onClick={() => void navigate("previous")}
            disabled={!current || isLoading}
            aria-label="Previous"
            title="Previous (←)"
            aria-keyshortcuts="ArrowLeft"
          >
            <ControlIcon name="chevron-left" />
          </button>
          <span className="sequence-position">{sequenceLabel}</span>
          <button
            type="button"
            className="viewer-icon-button viewer-icon-button--nav"
            onClick={() => void navigate("next")}
            disabled={!current || isLoading}
            aria-label="Next"
            title="Next (→)"
            aria-keyshortcuts="ArrowRight"
          >
            <ControlIcon name="chevron-right" />
          </button>
        </div>

        <div className="viewer-controls__group viewer-controls__group--end">
          <button
            type="button"
            className="viewer-icon-button"
            onClick={zoomOut}
            disabled={!canDisplayCurrent}
            aria-label="Zoom out"
            title="Zoom out (-)"
            aria-keyshortcuts="-"
          >
            <ControlIcon name="zoom-out" />
          </button>
          <button
            type="button"
            className="viewer-icon-button"
            onClick={resetActualSize}
            disabled={!canDisplayCurrent}
            aria-label="Actual size"
            title="Actual size (0)"
            aria-keyshortcuts="0"
          >
            <ControlIcon name="actual-size" />
          </button>
          <button
            type="button"
            className="viewer-icon-button"
            onClick={fitToWindow}
            disabled={!canDisplayCurrent}
            aria-label="Fit"
            title="Fit (F)"
            aria-keyshortcuts="F"
          >
            <ControlIcon name="fit" />
          </button>
          <button
            type="button"
            className="viewer-icon-button"
            onClick={zoomIn}
            disabled={!canDisplayCurrent}
            aria-label="Zoom in"
            title="Zoom in (+)"
            aria-keyshortcuts="+ ="
          >
            <ControlIcon name="zoom-in" />
          </button>
          <span className="zoom-status" aria-live="polite">
            {mode === "fit" ? "Fit" : `${Math.round(scale * 100)}%`}
          </span>
          <button
            type="button"
            className="viewer-icon-button"
            onClick={() => void toggleFullscreen()}
            aria-label={isFullscreen ? "Exit fullscreen" : "Fullscreen"}
            title={isFullscreen ? "Exit fullscreen (F11)" : "Fullscreen (F11)"}
            aria-keyshortcuts="F11"
          >
            <ControlIcon
              name={isFullscreen ? "fullscreen-exit" : "fullscreen-enter"}
            />
          </button>
          <button
            type="button"
            className="viewer-icon-button"
            onClick={openRenameDialog}
            disabled={!current || isLoading}
            aria-label="Rename"
            title="Rename… (R)"
            aria-keyshortcuts="R"
          >
            <ControlIcon name="rename" />
          </button>
          <button
            type="button"
            className="viewer-icon-button viewer-icon-button--danger danger-button"
            onClick={openTrashDialog}
            disabled={!current || isLoading}
            aria-label="Trash"
            title="Trash… (Delete)"
            aria-keyshortcuts="Delete"
          >
            <ControlIcon name="trash" />
          </button>
        </div>
      </footer>

      {isOversizedDialogOpen && current && shouldGateOversizedImage ? (
        <div className="dialog-backdrop" role="presentation">
          <section
            className="viewer-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="oversized-dialog-title"
          >
            <p className="eyebrow">Large image</p>
            <h2 id="oversized-dialog-title">Display this image?</h2>
            <p>
              This image may slow down the viewer while it is being displayed.
            </p>
            <OversizedDetails preflight={current.preflight} />
            <div className="dialog-actions">
              <button
                type="button"
                onClick={() => setIsOversizedDialogOpen(false)}
              >
                Keep paused
              </button>
              <button
                type="button"
                className="primary-button"
                onClick={confirmOversizedImage}
              >
                Display image
              </button>
            </div>
          </section>
        </div>
      ) : null}

      {isRenameDialogOpen ? (
        <div className="dialog-backdrop" role="presentation">
          <form
            className="viewer-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="rename-dialog-title"
            onSubmit={(event) => {
              event.preventDefault();
              void submitRename();
            }}
          >
            <p className="eyebrow">Current image file</p>
            <h2 id="rename-dialog-title">Rename current image</h2>
            <p>
              Enter only the new filename stem. Manzar preserves the extension
              and keeps the file in the same folder.
            </p>
            <label className="text-field">
              <span>New filename stem</span>
              <input
                autoFocus
                value={renameDraft}
                onChange={(event) => setRenameDraft(event.currentTarget.value)}
                placeholder="new-image-name"
              />
            </label>
            {renameValidationMessage ? (
              <p className="dialog-validation">{renameValidationMessage}</p>
            ) : null}
            <div className="dialog-actions">
              <button
                type="button"
                onClick={() => setIsRenameDialogOpen(false)}
                disabled={isLoading}
              >
                Cancel
              </button>
              <button
                type="submit"
                className="primary-button"
                disabled={isLoading || renameValidationMessage !== null}
              >
                Rename
              </button>
            </div>
          </form>
        </div>
      ) : null}

      {isTrashDialogOpen ? (
        <div className="dialog-backdrop" role="presentation">
          <section
            className="viewer-dialog"
            role="dialog"
            aria-modal="true"
            aria-labelledby="trash-dialog-title"
          >
            <p className="eyebrow">Trash deletion</p>
            <h2 id="trash-dialog-title">Move current image to trash?</h2>
            <p>
              This applies only to the image currently being viewed. If it
              succeeds, Manzar will show the next image or an empty state.
            </p>
            <div className="dialog-actions">
              <button
                type="button"
                onClick={() => setIsTrashDialogOpen(false)}
                disabled={isLoading}
              >
                Cancel
              </button>
              <button
                type="button"
                className="danger-button"
                onClick={() => void confirmTrashDeletion()}
                disabled={isLoading}
              >
                Move to trash
              </button>
            </div>
          </section>
        </div>
      ) : null}
    </main>
  );
}

function ControlIcon({ name }: { name: ControlIconName }) {
  switch (name) {
    case "actual-size":
      return (
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="M7 7h10v10H7z" />
          <path d="M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5" />
        </svg>
      );
    case "chevron-left":
      return (
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="m14.5 6-6 6 6 6" />
        </svg>
      );
    case "chevron-right":
      return (
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="m9.5 6 6 6-6 6" />
        </svg>
      );
    case "fit":
      return (
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="M9 4H4v5M15 4h5v5M20 15v5h-5M9 20H4v-5" />
          <path d="M9 9h6v6H9z" />
        </svg>
      );
    case "folder":
      return (
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="M3.5 7.5h6l1.7 2h9.3v8a2 2 0 0 1-2 2h-13a2 2 0 0 1-2-2z" />
          <path d="M3.5 7.5v-1a2 2 0 0 1 2-2h3.2l1.7 2h5.1a2 2 0 0 1 2 2v1" />
        </svg>
      );
    case "fullscreen-enter":
      return (
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="M9 4H4v5M15 4h5v5M20 15v5h-5M9 20H4v-5" />
        </svg>
      );
    case "fullscreen-exit":
      return (
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="M9 4v5H4M15 4v5h5M20 15h-5v5M4 15h5v5" />
        </svg>
      );
    case "image":
      return (
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="M5 5h14v14H5z" />
          <path d="m7.5 16 3.2-3.4 2.4 2.4 1.9-2.1 1.5 3.1" />
          <path d="M15.5 8.5h.01" />
        </svg>
      );
    case "rename":
      return (
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="m5 16.8-.6 2.8 2.8-.6L18.5 7.7l-2.2-2.2z" />
          <path d="m14.8 7 2.2 2.2" />
          <path d="M11 19.5h8" />
        </svg>
      );
    case "trash":
      return (
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="M5 7h14" />
          <path d="M9 7V5h6v2" />
          <path d="m7 7 .8 12h8.4L17 7" />
          <path d="M10.5 10.5v5M13.5 10.5v5" />
        </svg>
      );
    case "zoom-in":
      return (
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="M10.5 17a6.5 6.5 0 1 1 0-13 6.5 6.5 0 0 1 0 13Z" />
          <path d="m15.5 15.5 4 4" />
          <path d="M10.5 8v5M8 10.5h5" />
        </svg>
      );
    case "zoom-out":
      return (
        <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
          <path d="M10.5 17a6.5 6.5 0 1 1 0-13 6.5 6.5 0 0 1 0 13Z" />
          <path d="m15.5 15.5 4 4" />
          <path d="M8 10.5h5" />
        </svg>
      );
  }
}

function OversizedDetails({ preflight }: { preflight: ImagePreflight }) {
  if (preflight.reasons.length === 0) {
    return null;
  }

  return (
    <ul className="warning-details">
      {preflight.reasons.map((reason, index) => (
        <li key={index}>{describeOversizedReason(reason)}</li>
      ))}
    </ul>
  );
}

function describeOversizedReason(reason: OversizedImageReason): string {
  if (reason.reason === "file_size") {
    return `File size ${formatBytes(reason.actual_bytes)} exceeds ${formatBytes(
      reason.threshold_bytes,
    )}.`;
  }

  return `Estimated decoded RGBA memory ${formatBytes(
    reason.estimated_bytes,
  )} exceeds ${formatBytes(reason.threshold_bytes)} (${reason.width} × ${
    reason.height
  }).`;
}

function formatBytes(bytes: number): string {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unitIndex = 0;

  while (value >= 1024 && unitIndex < units.length - 1) {
    value /= 1024;
    unitIndex += 1;
  }

  return `${value >= 10 || unitIndex === 0 ? value.toFixed(0) : value.toFixed(1)} ${
    units[unitIndex]
  }`;
}

function validateRenameStem(stem: string): string | null {
  const trimmed = stem.trim();

  if (trimmed.length === 0) {
    return "Enter a new filename stem.";
  }

  if (trimmed.startsWith(".")) {
    return "The new name cannot start with a dot.";
  }

  if (trimmed.includes("/") || trimmed.includes("\\")) {
    return "The new name cannot contain path separators.";
  }

  return null;
}

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) {
    return false;
  }

  return (
    target.isContentEditable ||
    target instanceof HTMLInputElement ||
    target instanceof HTMLTextAreaElement ||
    target instanceof HTMLSelectElement
  );
}

export default App;
