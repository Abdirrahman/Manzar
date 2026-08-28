import {
  useCallback,
  useLayoutEffect,
  useMemo,
  useState,
  type CSSProperties,
} from "react";

const zoomStep = 1.2;
const minScale = 0.1;
const maxScale = 8;

type PresentationMode = "fit" | "manual";

type PanOffset = {
  x: number;
  y: number;
};

type PresentationState = {
  mode: PresentationMode;
  scale: number;
  pan: PanOffset;
  dragStart: DragStart | null;
};

type DragStart = {
  pointerId: number;
  clientX: number;
  clientY: number;
  pan: PanOffset;
};

const initialPresentationState: PresentationState = {
  mode: "fit",
  scale: 1,
  pan: { x: 0, y: 0 },
  dragStart: null,
};

export function useImagePresentation(imageId: string | null) {
  const [state, setState] = useState<PresentationState>(
    initialPresentationState,
  );

  useLayoutEffect(() => {
    setState(initialPresentationState);
  }, [imageId]);

  const zoomIn = useCallback(() => {
    setState((current) => ({
      ...current,
      mode: "manual",
      scale: clampScale(current.scale * zoomStep),
    }));
  }, []);

  const zoomOut = useCallback(() => {
    setState((current) => ({
      ...current,
      mode: "manual",
      scale: clampScale(current.scale / zoomStep),
    }));
  }, []);

  const resetActualSize = useCallback(() => {
    setState({
      ...initialPresentationState,
      mode: "manual",
      scale: 1,
    });
  }, []);

  const fitToWindow = useCallback(() => {
    setState(initialPresentationState);
  }, []);

  const startPan = useCallback(
    (pointerId: number, clientX: number, clientY: number) => {
      setState((current) => {
        if (!canPan(current)) {
          return current;
        }

        return {
          ...current,
          dragStart: {
            pointerId,
            clientX,
            clientY,
            pan: current.pan,
          },
        };
      });
    },
    [],
  );

  const updatePan = useCallback(
    (pointerId: number, clientX: number, clientY: number) => {
      setState((current) => {
        if (current.dragStart?.pointerId !== pointerId) {
          return current;
        }

        return {
          ...current,
          pan: {
            x: current.dragStart.pan.x + clientX - current.dragStart.clientX,
            y: current.dragStart.pan.y + clientY - current.dragStart.clientY,
          },
        };
      });
    },
    [],
  );

  const endPan = useCallback((pointerId: number) => {
    setState((current) => {
      if (current.dragStart?.pointerId !== pointerId) {
        return current;
      }

      return {
        ...current,
        dragStart: null,
      };
    });
  }, []);

  const imageClassName =
    state.mode === "fit"
      ? "viewer-image viewer-image--fit"
      : "viewer-image viewer-image--manual";
  const imageStyle = useMemo<CSSProperties | undefined>(() => {
    if (state.mode === "fit") {
      return undefined;
    }

    return {
      transform: `translate(${state.pan.x}px, ${state.pan.y}px) scale(${state.scale})`,
    };
  }, [state.mode, state.pan.x, state.pan.y, state.scale]);

  return {
    mode: state.mode,
    scale: state.scale,
    isPannable: canPan(state),
    isPanning: state.dragStart !== null,
    imageClassName,
    imageStyle,
    zoomIn,
    zoomOut,
    resetActualSize,
    fitToWindow,
    startPan,
    updatePan,
    endPan,
  };
}

function canPan(state: PresentationState): boolean {
  return state.mode === "manual";
}

function clampScale(scale: number): number {
  return Math.min(maxScale, Math.max(minScale, scale));
}
