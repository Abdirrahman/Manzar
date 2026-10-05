import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ButtonHTMLAttributes,
  type ReactNode,
} from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { isTauri } from "@tauri-apps/api/core";
import { pickImageFiles, pickImageFolder } from "./fileDialogs";
import { useImagePresentation } from "./useImagePresentation";
import { CropEditor } from "./CropEditor";
import {
  backendErrorMessage,
  cropCurrentImage,
  getViewerSnapshot,
  navigateNext,
  navigatePrevious,
  openFolder,
  openImageSelection,
  openSingleImage,
  renameCurrentImage,
  setSequenceOrdering,
  trashCurrentImage,
  type SequenceOrdering,
  type ViewerSnapshot,
} from "./viewerCommands";
import "./App.css";

const orderings: [SequenceOrdering, string][] = [
  ["newest_modified_first", "Newest modified"],
  ["natural_name", "Name"],
  ["size_largest_first", "Largest first"],
  ["size_smallest_first", "Smallest first"],
];
const shortcuts = [
  ["Open images", "O"],
  ["Open folder", "Shift O"],
  ["Previous / next", "← →"],
  ["Zoom", "− +"],
  ["Fit to window", "F"],
  ["Actual size", "0"],
  ["Crop image", "C"],
  ["Save crop", "Ctrl Enter"],
  ["Rename", "R"],
  ["Move to trash", "Delete"],
  ["Fullscreen", "F11"],
  ["Cancel / close", "Esc"],
];
type DialogName = "rename" | "trash" | "large" | "shortcuts" | null;

function viewportSize() {
  const quantise = (value: number) =>
    Math.ceil((value * (window.devicePixelRatio || 1)) / 256) * 256;
  return {
    width: quantise(window.innerWidth),
    height: quantise(window.innerHeight),
  };
}

export default function App() {
  const [snapshot, setSnapshot] = useState<ViewerSnapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const busyRef = useRef(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [dialog, setDialog] = useState<DialogName>(null);
  const [renameDraft, setRenameDraft] = useState("");
  const [cropping, setCropping] = useState(false);
  const [approved, setApproved] = useState<string | null>(null);
  const [fullscreen, setFullscreen] = useState(false);
  const [viewport, setViewport] = useState(viewportSize);
  const [loadedUrl, setLoadedUrl] = useState<string | null>(null);
  const shell = useRef<HTMLElement>(null);
  const stageRef = useRef<HTMLElement>(null);
  const video = useRef<HTMLVideoElement | null>(null);
  const current = snapshot?.current;
  const gated = !!current?.preflight.oversized && approved !== current.id;
  const displayable = !!current && !gated;
  const locked = busy || cropping;
  const dimensions = current?.preflight.dimensions;
  const presentation = useImagePresentation(
    displayable ? current.id : null,
    dimensions ?? null,
  );
  const {
    mode,
    scale,
    mediaRef,
    imageStyle,
    isPanning,
    zoomIn,
    zoomOut,
    fitToWindow,
    resetActualSize,
    startPan,
    updatePan,
    endPan,
  } = presentation;
  const displayUrl =
    current && current.kind === "image" && mode === "fit"
      ? `${current.url}?w=${viewport.width}&h=${viewport.height}`
      : current?.url;
  const canCrop =
    displayable && !!current?.preflight.crop_supported && !!dimensions;
  const renameError = !renameDraft.trim()
    ? "Enter a filename."
    : renameDraft.trim().startsWith(".")
      ? "The name cannot start with a dot."
      : /[/\\]/.test(renameDraft)
        ? "The name cannot contain / or \\."
        : null;

  // One action boundary handles errors and prevents duplicate destructive actions,
  // including two key events that arrive before React has rendered the busy state.
  const run = useCallback(
    async (
      operation: () => Promise<ViewerSnapshot | null>,
      success?: string,
    ) => {
      if (busyRef.current) return;
      busyRef.current = true;
      setBusy(true);
      setError(null);
      setNotice(null);
      try {
        const next = await operation();
        if (next) {
          setSnapshot(next);
          setDialog(null);
          setCropping(false);
          if (success) setNotice(success);
        }
      } catch (error) {
        setError(backendErrorMessage(error));
      } finally {
        busyRef.current = false;
        setBusy(false);
      }
    },
    [],
  );

  useEffect(() => {
    void run(getViewerSnapshot);
  }, [run]);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const resize = () => {
      clearTimeout(timer);
      timer = setTimeout(() => setViewport(viewportSize()), 120);
    };
    const changed = () => setFullscreen(!!document.fullscreenElement);
    window.addEventListener("resize", resize);
    document.addEventListener("fullscreenchange", changed);
    return () => {
      clearTimeout(timer);
      window.removeEventListener("resize", resize);
      document.removeEventListener("fullscreenchange", changed);
    };
  }, []);
  useEffect(() => {
    if (gated) setDialog("large");
  }, [current?.id, gated]);
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(null), 4000);
    return () => clearTimeout(timer);
  }, [notice]);

  function openFiles(folder = false) {
    if (cropping) return;
    void run(async () => {
      if (folder) {
        const path = await pickImageFolder();
        return path ? openFolder(path) : null;
      }
      const paths = await pickImageFiles();
      return paths
        ? paths.length === 1
          ? openSingleImage(paths[0])
          : openImageSelection(paths)
        : null;
    });
  }
  function navigate(next: boolean) {
    if (!locked && current && (snapshot?.count ?? 0) > 1)
      void run(next ? navigateNext : navigatePrevious);
  }
  function beginCrop() {
    if (!locked && canCrop && loadedUrl === displayUrl) {
      setError(null);
      setNotice(null);
      fitToWindow();
      setCropping(true);
    }
  }
  function rename() {
    if (!current || locked) return;
    setRenameDraft(current.filename.replace(/\.[^.]+$/, ""));
    setDialog("rename");
  }
  function closeDialog() {
    if (!busyRef.current) {
      setDialog(null);
      setError(null);
    }
  }
  function toggleFullscreen() {
    void (
      document.fullscreenElement
        ? document.exitFullscreen()
        : shell.current?.requestFullscreen()
    )?.catch((error) => setError(backendErrorMessage(error)));
  }
  function windowAction(action: "minimize" | "toggleMaximize" | "close") {
    if (isTauri())
      void getCurrentWindow()
        [action]()
        .catch((error) => setError(backendErrorMessage(error)));
  }

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.altKey) return;
      if (event.key === "Escape") {
        if (busyRef.current) {
          event.preventDefault();
          return;
        }
        if (dialog) closeDialog();
        else if (cropping) {
          setCropping(false);
          setError(null);
        } else if (document.fullscreenElement) void document.exitFullscreen();
        else setError(null);
        event.preventDefault();
        return;
      }
      if (dialog || cropping || busyRef.current) return;
      if (
        (event.target as HTMLElement)?.closest(
          "input, select, textarea, video, [contenteditable=true]",
        )
      )
        return;
      const key = event.key.toLowerCase();
      if (event.ctrlKey || event.metaKey) {
        if (key === "o") {
          event.preventDefault();
          openFiles(event.shiftKey);
        }
        return;
      }
      const actions: Record<string, () => void> = {
        o: () => openFiles(event.shiftKey),
        arrowleft: () => navigate(false),
        arrowright: () => navigate(true),
        c: beginCrop,
        r: rename,
        delete: () => {
          if (current) setDialog("trash");
        },
        f: () => {
          if (displayable) fitToWindow();
        },
        "0": () => {
          if (displayable) resetActualSize();
        },
        "+": () => {
          if (displayable) zoomIn();
        },
        "=": () => {
          if (displayable) zoomIn();
        },
        "-": () => {
          if (displayable) zoomOut();
        },
        f11: toggleFullscreen,
        "?": () => setDialog("shortcuts"),
      };
      if (
        key === " " &&
        video.current &&
        !(event.target as HTMLElement)?.closest("button")
      ) {
        event.preventDefault();
        if (video.current.paused)
          void video.current
            .play()
            .catch((error) => setError(backendErrorMessage(error)));
        else video.current.pause();
      } else if (actions[key]) {
        event.preventDefault();
        actions[key]();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  useEffect(() => {
    const stage = stageRef.current;
    const wheel = (event: WheelEvent) => {
      if (!event.ctrlKey || !displayable || dialog || busy) return;
      event.preventDefault();
      if (event.deltaY < 0) zoomIn();
      else if (event.deltaY > 0) zoomOut();
    };
    stage?.addEventListener("wheel", wheel, { passive: false });
    return () => stage?.removeEventListener("wheel", wheel);
  });

  const errorBanner = error && (
    <div className="message message--error" role="alert">
      <span>{error}</span>
      <button aria-label="Dismiss error" onClick={() => setError(null)}>
        ×
      </button>
    </div>
  );
  return (
    <main
      ref={shell}
      className={`viewer-shell${cropping ? " is-cropping" : ""}`}
      aria-busy={busy}
    >
      <header className="window-titlebar">
        <div className="window-brand" data-tauri-drag-region>
          <img src="/manzar-logo.svg" alt="" />
          <span data-tauri-drag-region>Manzar</span>
        </div>
        <div
          className="window-filename"
          title={current?.filename}
          data-tauri-drag-region
        >
          {current?.filename ?? "Image viewer"}
        </div>
        <div className="window-actions">
          <button
            aria-label="Minimize window"
            onClick={() => windowAction("minimize")}
          >
            −
          </button>
          <button
            aria-label="Maximize or restore window"
            onClick={() => windowAction("toggleMaximize")}
          >
            □
          </button>
          <button
            className="window-close"
            aria-label="Close window"
            disabled={busy}
            onClick={() => windowAction("close")}
          >
            ×
          </button>
        </div>
      </header>
      <nav className="file-toolbar" aria-label="File controls">
        <div className="button-group">
          <Tool
            icon="image"
            label="Open images"
            shortcut="O"
            disabled={locked}
            onClick={() => openFiles()}
            text="Open"
          />
          <Tool
            icon="folder"
            label="Open folder"
            shortcut="Shift+O"
            disabled={locked}
            onClick={() => openFiles(true)}
          />
          <span className="separator" />
          <label className="sort-control">
            <span>Sort</span>
            <select
              aria-label="Sequence ordering"
              value={snapshot?.sequence_ordering ?? "newest_modified_first"}
              disabled={locked || !current}
              onChange={(event) =>
                void run(() =>
                  setSequenceOrdering(event.target.value as SequenceOrdering),
                )
              }
            >
              {orderings.map(([value, label]) => (
                <option value={value} key={value}>
                  {label}
                </option>
              ))}
            </select>
          </label>
        </div>
        <div className="button-group file-actions">
          <Tool
            icon="crop"
            label="Crop image"
            shortcut="C"
            text="Crop"
            aria-pressed={cropping}
            disabled={locked || !canCrop || loadedUrl !== displayUrl}
            title={
              !canCrop && current
                ? "Cropping is available for still PNG, JPEG, WebP and BMP images"
                : undefined
            }
            onClick={beginCrop}
          />
          <Tool
            icon="rename"
            label="Rename"
            shortcut="R"
            disabled={locked || !current}
            onClick={rename}
          />
          <Tool
            icon="trash"
            label="Move to trash"
            shortcut="Delete"
            disabled={locked || !current}
            onClick={() => setDialog("trash")}
          />
        </div>
      </nav>
      {cropping && current && dimensions ? (
        <CropEditor
          key={current.id}
          url={`${current.url}?w=${viewport.width}&h=${viewport.height}`}
          size={dimensions}
          busy={busy}
          onCancel={() => {
            setCropping(false);
            setError(null);
          }}
          onError={() =>
            setError(
              "The image could not be displayed. Reopen it to try again.",
            )
          }
          onSave={(rect) =>
            void run(
              () => cropCurrentImage(current.id, rect),
              "Crop saved · original image replaced",
            )
          }
        />
      ) : (
        <>
          <section
            ref={stageRef}
            className={`viewer-stage${mode === "manual" && current?.kind === "image" ? " is-pannable" : ""}${isPanning ? " is-panning" : ""}`}
            aria-label="Image viewer"
            onDoubleClick={(event) => {
              if (
                displayable &&
                current.kind === "image" &&
                event.target === mediaRef.current
              ) {
                if (mode === "fit") resetActualSize();
                else fitToWindow();
              }
            }}
            onPointerDown={(event) => {
              if (
                !displayable ||
                mode !== "manual" ||
                current.kind === "video" ||
                event.button !== 0 ||
                dialog
              )
                return;
              event.preventDefault();
              event.currentTarget.setPointerCapture(event.pointerId);
              startPan(event.pointerId, event.clientX, event.clientY);
            }}
            onPointerMove={(event) =>
              updatePan(event.pointerId, event.clientX, event.clientY)
            }
            onPointerUp={endPan}
            onPointerCancel={endPan}
          >
            {displayable ? (
              <div className="viewer-image-frame">
                {current.kind === "video" ? (
                  <video
                    key={current.id}
                    ref={(element) => {
                      video.current = element;
                      mediaRef.current = element;
                    }}
                    className={`viewer-image viewer-image--${mode}`}
                    style={imageStyle}
                    src={displayUrl}
                    controls
                    autoPlay
                    preload="metadata"
                    aria-label={current.filename}
                    onError={() =>
                      setError(
                        "This video could not be played. Check that its codec is supported.",
                      )
                    }
                  />
                ) : (
                  <img
                    key={current.id}
                    ref={(element) => {
                      mediaRef.current = element;
                    }}
                    className={`viewer-image viewer-image--${mode}`}
                    style={imageStyle}
                    src={displayUrl}
                    alt={current.filename}
                    draggable={false}
                    decoding="async"
                    onLoad={() => setLoadedUrl(displayUrl ?? null)}
                    onError={() => {
                      setLoadedUrl(null);
                      setError(
                        "This image could not be displayed. Check that the file is still available and is a valid image.",
                      );
                    }}
                  />
                )}
              </div>
            ) : gated ? (
              <div className="empty-state">
                <p className="eyebrow">Large image</p>
                <h1>Take a moment.</h1>
                <p>This image may need extra time and memory to display.</p>
                <button
                  className="primary-button"
                  onClick={() => setDialog("large")}
                >
                  Review image
                </button>
              </div>
            ) : (
              <div className="empty-state">
                <div className="empty-viewfinder">
                  <img src="/manzar-logo.svg" alt="" />
                  <i />
                  <i />
                  <i />
                  <i />
                </div>
                <p className="eyebrow">Your files. A little closer.</p>
                <h1>A closer look.</h1>
                <p>
                  Open an image, a video, or a whole folder.
                  <br />
                  Just your files, ready to view.
                </p>
                <div className="empty-actions">
                  <button
                    className="primary-button"
                    disabled={busy}
                    onClick={() => openFiles()}
                  >
                    <Icon name="image" />
                    Open images<kbd>O</kbd>
                  </button>
                  <button disabled={busy} onClick={() => openFiles(true)}>
                    <Icon name="folder" />
                    Open folder
                  </button>
                </div>
                <div className="empty-formats">
                  PNG · JPEG · WEBP · GIF · BMP
                  <span>MP4 · MOV · MKV · WEBM</span>
                </div>
              </div>
            )}
            {current && displayable && (snapshot?.count ?? 0) > 1 && (
              <div className="stage-navigation">
                <Tool
                  icon="left"
                  label="Previous image"
                  shortcut="ArrowLeft"
                  disabled={busy}
                  onClick={() => navigate(false)}
                />
                <Tool
                  icon="right"
                  label="Next image"
                  shortcut="ArrowRight"
                  disabled={busy}
                  onClick={() => navigate(true)}
                />
              </div>
            )}
          </section>
          <footer className="viewer-footer">
            <div className="file-details">
              {current ? (
                <>
                  <span className="file-extension">
                    {current.filename.split(".").pop()?.toUpperCase()}
                  </span>
                  {dimensions && (
                    <span>
                      {dimensions.width.toLocaleString()} ×{" "}
                      {dimensions.height.toLocaleString()}
                    </span>
                  )}
                  <span>{formatBytes(current.preflight.file_size_bytes)}</span>
                </>
              ) : (
                <span>Local files, in focus.</span>
              )}
            </div>
            <div className="view-controls">
              <Tool
                icon="minus"
                label="Zoom out"
                shortcut="-"
                disabled={!displayable}
                onClick={zoomOut}
              />
              <button
                className="zoom-value"
                aria-label="Actual size"
                title="Actual size (0)"
                disabled={!displayable}
                onClick={resetActualSize}
              >
                {mode === "fit" ? "Fit" : `${Math.round(scale * 100)}%`}
              </button>
              <Tool
                icon="plus"
                label="Zoom in"
                shortcut="+"
                disabled={!displayable}
                onClick={zoomIn}
              />
              <span className="separator" />
              <Tool
                icon="fit"
                label="Fit to window"
                shortcut="F"
                aria-pressed={mode === "fit" && displayable}
                disabled={!displayable}
                onClick={fitToWindow}
              />
              <Tool
                icon="fullscreen"
                label={fullscreen ? "Exit fullscreen" : "Fullscreen"}
                shortcut="F11"
                onClick={toggleFullscreen}
              />
            </div>
            <div className="sequence-controls">
              <span className="sequence-position" aria-live="polite">
                {current
                  ? `${snapshot?.current_position} / ${snapshot?.count}`
                  : "No file open"}
              </span>
              <Tool
                icon="help"
                label="Keyboard shortcuts"
                shortcut="?"
                onClick={() => setDialog("shortcuts")}
              />
            </div>
          </footer>
        </>
      )}
      {!dialog && (
        <div className="messages">
          {errorBanner}
          {notice && (
            <div className="message" role="status">
              <Icon name="check" />
              {notice}
            </div>
          )}
          {busy && (
            <div className="message" role="status">
              <span className="spinner" />
              {cropping ? "Saving crop…" : "Opening…"}
            </div>
          )}
        </div>
      )}
      {dialog && (
        <Dialog
          title={
            dialog === "rename"
              ? "Rename file"
              : dialog === "trash"
                ? "Move to trash?"
                : dialog === "large"
                  ? "Display this image?"
                  : "Keyboard shortcuts"
          }
          onClose={closeDialog}
          busy={busy}
        >
          {dialog === "rename" && (
            <form
              onSubmit={(event) => {
                event.preventDefault();
                if (!renameError)
                  void run(
                    () => renameCurrentImage(renameDraft.trim()),
                    "File renamed",
                  );
              }}
            >
              <p className="dialog-description">
                The file stays in the same folder with its current extension.
              </p>
              <label className="text-field">
                Filename
                <input
                  autoFocus
                  value={renameDraft}
                  disabled={busy}
                  onFocus={(event) => event.target.select()}
                  onChange={(event) => setRenameDraft(event.target.value)}
                  aria-invalid={!!renameError}
                  aria-describedby={renameError ? "rename-error" : undefined}
                />
              </label>
              {renameError && (
                <p id="rename-error" className="validation">
                  {renameError}
                </p>
              )}
              {errorBanner}
              <div className="dialog-actions">
                <button type="button" disabled={busy} onClick={closeDialog}>
                  Cancel
                </button>
                <button
                  className="primary-button"
                  disabled={busy || !!renameError}
                >
                  {busy ? "Renaming…" : "Rename"}
                </button>
              </div>
            </form>
          )}
          {dialog === "trash" && (
            <>
              <p className="dialog-description">
                <strong>{current?.filename}</strong> will move to your desktop
                trash. You can restore it from there.
              </p>
              {errorBanner}
              <div className="dialog-actions">
                <button autoFocus disabled={busy} onClick={closeDialog}>
                  Cancel
                </button>
                <button
                  className="danger-button"
                  disabled={busy}
                  onClick={() =>
                    void run(trashCurrentImage, "File moved to trash")
                  }
                >
                  {busy ? "Moving…" : "Move to trash"}
                </button>
              </div>
            </>
          )}
          {dialog === "large" && (
            <>
              <p className="dialog-description">
                This image may take extra time and memory to display.
              </p>
              <p className="large-details">
                {dimensions
                  ? `${dimensions.width.toLocaleString()} × ${dimensions.height.toLocaleString()} pixels · `
                  : ""}
                {formatBytes(current?.preflight.file_size_bytes ?? 0)}
              </p>
              <div className="dialog-actions">
                <button autoFocus onClick={closeDialog}>
                  Keep paused
                </button>
                <button
                  className="primary-button"
                  onClick={() => {
                    setApproved(current?.id ?? null);
                    setDialog(null);
                  }}
                >
                  Display image
                </button>
              </div>
            </>
          )}
          {dialog === "shortcuts" && (
            <>
              <dl className="shortcuts">
                {shortcuts.map(([label, key]) => (
                  <div key={label}>
                    <dt>{label}</dt>
                    <dd>
                      <kbd>{key}</kbd>
                    </dd>
                  </div>
                ))}
              </dl>
              <p className="dialog-description">
                Double-click an image to toggle actual size. Ctrl + scroll to
                zoom.
              </p>
              <div className="dialog-actions">
                <button autoFocus onClick={closeDialog}>
                  Done
                </button>
              </div>
            </>
          )}
        </Dialog>
      )}
    </main>
  );
}

function Dialog({
  title,
  children,
  onClose,
  busy,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  busy: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    ref.current?.showModal();
    return () => {
      previous?.focus();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      className="viewer-dialog"
      aria-labelledby="dialog-title"
      onCancel={(event) => {
        event.preventDefault();
        if (!busy) onClose();
      }}
    >
      <h2 id="dialog-title">{title}</h2>
      {children}
    </dialog>
  );
}

function Tool({
  icon,
  label,
  shortcut,
  text,
  title,
  ...props
}: {
  icon: keyof typeof icons;
  label: string;
  shortcut?: string;
  text?: string;
} & ButtonHTMLAttributes<HTMLButtonElement>) {
  return (
    <button
      type="button"
      className={`tool${text ? " tool--text" : ""}`}
      aria-label={label}
      aria-keyshortcuts={shortcut}
      title={title ?? `${label}${shortcut ? ` (${shortcut})` : ""}`}
      {...props}
    >
      <Icon name={icon} />
      {text && <span>{text}</span>}
    </button>
  );
}
const icons = {
  image: "M4 4h16v16H4z M4 15l5-5 5 6 3-3 3 3 M15 8h.01",
  folder: "M3 7V5h6l2 2h10v13H3z",
  crop: "M7 3v14h14 M3 7h14v14 M7 7l10 10",
  rename: "m4 16-1 5 5-1L20 8l-4-4z M14 6l4 4 M12 21h9",
  trash: "M4 7h16 M9 7V4h6v3 M6 7l1 14h10l1-14 M10 11v6 M14 11v6",
  left: "m14 6-6 6 6 6",
  right: "m10 6 6 6-6 6",
  plus: "M5 12h14 M12 5v14",
  minus: "M5 12h14",
  fit: "M8 3H3v5 M16 3h5v5 M21 16v5h-5 M8 21H3v-5 M8 8h8v8H8z",
  fullscreen: "M8 3H3v5 M16 3h5v5 M21 16v5h-5 M8 21H3v-5",
  help: "M12 21a9 9 0 1 0 0-18 9 9 0 0 0 0 18 M9.5 9a2.5 2.5 0 1 1 4 2c-1 .7-1.5 1-1.5 2 M12 17h.01",
  check: "m5 12 4 4L19 6",
};
function Icon({ name }: { name: keyof typeof icons }) {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d={icons[name]} />
    </svg>
  );
}
function formatBytes(bytes: number) {
  const unit =
    bytes >= 1024 ** 3 ? 3 : bytes >= 1024 ** 2 ? 2 : bytes >= 1024 ? 1 : 0;
  return `${(bytes / 1024 ** unit).toFixed(unit ? 1 : 0)} ${["B", "KB", "MB", "GB"][unit]}`;
}
