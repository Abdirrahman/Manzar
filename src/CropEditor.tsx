import { useEffect, useRef, useState, type PointerEvent } from "react";
import type { CropRect, ImageDimensions } from "./viewerCommands";

type Handle = "move" | "nw" | "ne" | "sw" | "se";
const clamp = (n: number, min: number, max: number) =>
  Math.min(max, Math.max(min, n));

export function adjustCrop(
  rect: CropRect,
  dx: number,
  dy: number,
  handle: Handle,
  size: ImageDimensions,
  ratio = 0,
): CropRect {
  if (handle === "move")
    return {
      ...rect,
      x: clamp(rect.x + dx, 0, size.width - rect.width),
      y: clamp(rect.y + dy, 0, size.height - rect.height),
    };
  const left = handle.includes("w"),
    top = handle.includes("n");
  const anchorX = left ? rect.x + rect.width : rect.x;
  const anchorY = top ? rect.y + rect.height : rect.y;
  const maxWidth = left ? anchorX : size.width - anchorX;
  const maxHeight = top ? anchorY : size.height - anchorY;
  let width = clamp(rect.width + (left ? -dx : dx), 1, maxWidth);
  let height = clamp(rect.height + (top ? -dy : dy), 1, maxHeight);
  if (ratio) {
    width = Math.min(
      Math.max(width, height * ratio),
      maxWidth,
      maxHeight * ratio,
    );
    height = width / ratio;
  }
  width = Math.max(1, Math.round(width));
  height = Math.max(1, Math.round(height));
  return {
    x: left ? anchorX - width : anchorX,
    y: top ? anchorY - height : anchorY,
    width,
    height,
  };
}

export function centeredCrop(size: ImageDimensions, ratio: number): CropRect {
  const width = Math.max(
    1,
    Math.round(
      Math.min(size.width, ratio ? size.height * ratio : size.width) * 0.8,
    ),
  );
  const height = clamp(
    Math.round(ratio ? width / ratio : size.height * 0.8),
    1,
    size.height,
  );
  return {
    x: Math.floor((size.width - width) / 2),
    y: Math.floor((size.height - height) / 2),
    width,
    height,
  };
}

export function CropEditor({
  url,
  size,
  busy,
  onSave,
  onCancel,
  onError,
}: {
  url: string;
  size: ImageDimensions;
  busy: boolean;
  onSave: (rect: CropRect) => void;
  onCancel: () => void;
  onError: () => void;
}) {
  const [rect, setRect] = useState(() => centeredCrop(size, 0));
  const [ratio, setRatio] = useState(0);
  const [available, setAvailable] = useState({ width: 0, height: 0 });
  const [ready, setReady] = useState(false);
  const stage = useRef<HTMLElement>(null);
  const surface = useRef<HTMLDivElement>(null);
  const drag = useRef<{
    id: number;
    x: number;
    y: number;
    rect: CropRect;
    handle: Handle;
  } | null>(null);
  const scale = Math.min(
    available.width / size.width,
    available.height / size.height,
    1,
  );

  useEffect(() => {
    const observer = new ResizeObserver(([entry]) =>
      setAvailable({
        width: Math.max(1, entry.contentRect.width - 56),
        height: Math.max(1, entry.contentRect.height - 56),
      }),
    );
    observer.observe(stage.current!);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const save = (event: KeyboardEvent) => {
      if (
        (event.ctrlKey || event.metaKey) &&
        event.key === "Enter" &&
        !event.repeat
      ) {
        event.preventDefault();
        if (!busy && ready) onSave(rect);
      }
    };
    window.addEventListener("keydown", save);
    return () => window.removeEventListener("keydown", save);
  });

  function point(event: PointerEvent) {
    const bounds = surface.current!.getBoundingClientRect();
    return {
      x: Math.round(
        ((event.clientX - bounds.left) / bounds.width) * size.width,
      ),
      y: Math.round(
        ((event.clientY - bounds.top) / bounds.height) * size.height,
      ),
    };
  }

  return (
    <>
      <section
        ref={stage}
        className="viewer-stage crop-stage"
        aria-label="Crop image"
      >
        <div
          ref={surface}
          className="crop-surface"
          style={{ width: size.width * scale, height: size.height * scale }}
          onPointerDown={(event) => {
            if (busy || !ready || event.button !== 0) return;
            const target = event.target as HTMLElement;
            const handle = target.dataset.handle as Handle | undefined;
            if (!handle && !target.closest(".crop-selection")) return;
            event.preventDefault();
            (handle
              ? target
              : surface.current?.querySelector<HTMLElement>(".crop-selection")
            )?.focus();
            event.currentTarget.setPointerCapture(event.pointerId);
            drag.current = {
              id: event.pointerId,
              ...point(event),
              rect,
              handle: handle ?? "move",
            };
          }}
          onPointerMove={(event) => {
            const start = drag.current;
            if (busy || !start || start.id !== event.pointerId) return;
            const next = point(event);
            setRect(
              adjustCrop(
                start.rect,
                next.x - start.x,
                next.y - start.y,
                start.handle,
                size,
                ratio,
              ),
            );
          }}
          onPointerUp={() => {
            drag.current = null;
          }}
          onPointerCancel={() => {
            drag.current = null;
          }}
        >
          <img
            src={url}
            alt="Image to crop"
            draggable={false}
            onLoad={() => setReady(true)}
            onError={() => {
              setReady(false);
              onError();
            }}
          />
          {ready && (
            <div
              className="crop-selection"
              role="group"
              aria-label="Crop selection"
              aria-describedby="crop-instructions"
              tabIndex={0}
              style={{
                left: `${(rect.x / size.width) * 100}%`,
                top: `${(rect.y / size.height) * 100}%`,
                width: `${(rect.width / size.width) * 100}%`,
                height: `${(rect.height / size.height) * 100}%`,
              }}
              onKeyDown={(event) => {
                if (
                  busy ||
                  !["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown"].includes(
                    event.key,
                  )
                )
                  return;
                event.preventDefault();
                event.stopPropagation();
                const step = event.shiftKey ? 10 : 1;
                const handle =
                  ((event.target as HTMLElement).dataset.handle as Handle) ||
                  "move";
                setRect(
                  adjustCrop(
                    rect,
                    event.key === "ArrowLeft"
                      ? -step
                      : event.key === "ArrowRight"
                        ? step
                        : 0,
                    event.key === "ArrowUp"
                      ? -step
                      : event.key === "ArrowDown"
                        ? step
                        : 0,
                    handle,
                    size,
                    ratio,
                  ),
                );
              }}
            >
              <div className="crop-grid" aria-hidden="true" />
              <span className="crop-size">
                {rect.width} × {rect.height}
              </span>
              {(["nw", "ne", "sw", "se"] as const).map((handle, index) => (
                <button
                  key={handle}
                  type="button"
                  className={`crop-handle crop-handle--${handle}`}
                  data-handle={handle}
                  disabled={busy}
                  aria-label={`Resize ${["top left", "top right", "bottom left", "bottom right"][index]} corner`}
                />
              ))}
            </div>
          )}
        </div>
      </section>
      <footer className="crop-controls">
        <div className="crop-options">
          <label>
            Aspect ratio{" "}
            <select
              aria-label="Crop aspect ratio"
              value={ratio}
              disabled={busy}
              onChange={(event) => {
                const next = Number(event.target.value);
                setRatio(next);
                setRect(centeredCrop(size, next));
              }}
            >
              <option value={0}>Freeform</option>
              <option value={1}>Square · 1:1</option>
              <option value={4 / 3}>4:3</option>
              <option value={16 / 9}>16:9</option>
              <option value={size.width / size.height}>Original</option>
            </select>
          </label>
          <button
            disabled={busy}
            onClick={() => {
              setRatio(0);
              setRect({ x: 0, y: 0, ...size });
            }}
          >
            Reset
          </button>
          <div className="crop-fields">
            {(["x", "y", "width", "height"] as const).map((field) => (
              <label key={field}>
                {{ x: "X", y: "Y", width: "W", height: "H" }[field]}
                <input
                  type="number"
                  aria-label={`Crop ${field}`}
                  disabled={busy}
                  min={field === "x" || field === "y" ? 0 : 1}
                  max={
                    field === "x"
                      ? size.width - rect.width
                      : field === "y"
                        ? size.height - rect.height
                        : field === "width"
                          ? size.width - rect.x
                          : size.height - rect.y
                  }
                  value={rect[field]}
                  onChange={(event) => {
                    if (!event.target.value) return;
                    const value = Math.round(Number(event.target.value));
                    if (!Number.isFinite(value)) return;
                    const next = {
                      ...rect,
                      [field]: clamp(
                        value,
                        Number(event.target.min),
                        Number(event.target.max),
                      ),
                    };
                    setRatio(0);
                    setRect(next);
                  }}
                />
              </label>
            ))}
          </div>
        </div>
        <div className="crop-save-row">
          <p id="crop-instructions">
            Drag corners to crop. Arrow keys fine-tune.
            <span>Saving replaces the original file.</span>
          </p>
          <div className="button-group">
            <button disabled={busy} onClick={onCancel}>
              Cancel
            </button>
            <button
              className="primary-button"
              disabled={busy || !ready}
              onClick={() => onSave(rect)}
            >
              {busy ? "Saving…" : "Save crop"}
            </button>
          </div>
        </div>
      </footer>
    </>
  );
}
