import {
  useCallback,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
} from "react";
import type { ImageDimensions } from "./viewerCommands";

const initial = { mode: "fit" as "fit" | "manual", scale: 1, x: 0, y: 0 };

export function useImagePresentation(
  imageId: string | null,
  dimensions: ImageDimensions | null,
) {
  const [state, setState] = useState(initial);
  const [isPanning, setPanning] = useState(false);
  const mediaRef = useRef<HTMLImageElement | HTMLVideoElement | null>(null);
  const drag = useRef<{
    id: number;
    x: number;
    y: number;
    panX: number;
    panY: number;
  } | null>(null);
  const fitToWindow = useCallback(() => {
    setState(initial);
    drag.current = null;
    setPanning(false);
  }, []);
  useLayoutEffect(fitToWindow, [imageId, fitToWindow]);

  function zoom(factor: number) {
    const media = mediaRef.current;
    // Start zooming from the visible fit scale, not a hidden 100% scale.
    const width =
      dimensions?.width ??
      (media instanceof HTMLVideoElement
        ? media.videoWidth
        : media?.naturalWidth);
    const fitScale =
      media && width ? media.getBoundingClientRect().width / width : 1;
    setState((current) => ({
      ...current,
      mode: "manual",
      scale: Math.min(
        8,
        Math.max(
          0.01,
          (current.mode === "fit" ? fitScale : current.scale) * factor,
        ),
      ),
    }));
  }
  const imageStyle: CSSProperties | undefined =
    state.mode === "manual"
      ? {
          width: dimensions?.width,
          height: dimensions?.height,
          transform: `translate(${state.x}px, ${state.y}px) scale(${state.scale})`,
        }
      : undefined;

  return {
    mode: state.mode,
    scale: state.scale,
    mediaRef,
    imageStyle,
    isPanning,
    zoomIn: () => zoom(1.2),
    zoomOut: () => zoom(1 / 1.2),
    fitToWindow,
    resetActualSize: () => {
      fitToWindow();
      setState({ ...initial, mode: "manual" });
    },
    startPan: (id: number, x: number, y: number) => {
      drag.current = { id, x, y, panX: state.x, panY: state.y };
      setPanning(true);
    },
    updatePan: (id: number, x: number, y: number) => {
      const start = drag.current;
      if (start?.id === id)
        setState((current) => ({
          ...current,
          x: start.panX + x - start.x,
          y: start.panY + y - start.y,
        }));
    },
    endPan: () => {
      drag.current = null;
      setPanning(false);
    },
  };
}
