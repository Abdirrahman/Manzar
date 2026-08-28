import { invoke } from "@tauri-apps/api/core";

const rawViewerCommandNames = {
  getViewerSnapshot: "get_viewer_snapshot",
  openSingleImage: "open_single_image",
  openImageSelection: "open_image_selection",
  openFolder: "open_folder",
  navigateNext: "navigate_next",
  navigatePrevious: "navigate_previous",
  setSequenceOrdering: "set_sequence_ordering",
  renameCurrentImage: "rename_current_image",
  trashCurrentImage: "trash_current_image",
} as const;

export type SequenceOrdering =
  | "newest_modified_first"
  | "natural_name"
  | "size_largest_first"
  | "size_smallest_first";

export type ViewerSnapshot = {
  current: ViewerImage | null;
  current_position: number | null;
  count: number;
  sequence_ordering: SequenceOrdering;
};

export type ViewerImage = {
  id: string;
  url: string;
  preflight: ImagePreflight;
};

export type ImagePreflight = {
  file_size_bytes: number;
  dimensions: ImageDimensions | null;
  oversized: boolean;
  reasons: OversizedImageReason[];
};

export type ImageDimensions = {
  width: number;
  height: number;
};

export type OversizedImageReason =
  | {
      reason: "file_size";
      actual_bytes: number;
      threshold_bytes: number;
    }
  | {
      reason: "decoded_rgba_memory";
      estimated_bytes: number;
      threshold_bytes: number;
      width: number;
      height: number;
    };

export function getViewerSnapshot(): Promise<ViewerSnapshot> {
  return invokeViewer(rawViewerCommandNames.getViewerSnapshot);
}

export function openSingleImage(path: string): Promise<ViewerSnapshot> {
  return invokeViewer(rawViewerCommandNames.openSingleImage, { path });
}

export function openImageSelection(paths: string[]): Promise<ViewerSnapshot> {
  return invokeViewer(rawViewerCommandNames.openImageSelection, { paths });
}

export function openFolder(path: string): Promise<ViewerSnapshot> {
  return invokeViewer(rawViewerCommandNames.openFolder, { path });
}

export function navigateNext(): Promise<ViewerSnapshot> {
  return invokeViewer(rawViewerCommandNames.navigateNext);
}

export function navigatePrevious(): Promise<ViewerSnapshot> {
  return invokeViewer(rawViewerCommandNames.navigatePrevious);
}

export function setSequenceOrdering(
  ordering: SequenceOrdering,
): Promise<ViewerSnapshot> {
  return invokeViewer(rawViewerCommandNames.setSequenceOrdering, { ordering });
}

export function renameCurrentImage(newStem: string): Promise<ViewerSnapshot> {
  return invokeViewer(rawViewerCommandNames.renameCurrentImage, { newStem });
}

export function trashCurrentImage(): Promise<ViewerSnapshot> {
  return invokeViewer(rawViewerCommandNames.trashCurrentImage);
}

export function backendErrorMessage(error: unknown): string {
  if (isCommandError(error)) {
    return error.message;
  }

  if (typeof error === "string") {
    return error;
  }

  return "viewer command failed";
}

type CommandError = {
  message: string;
};

function invokeViewer(
  commandName: (typeof rawViewerCommandNames)[keyof typeof rawViewerCommandNames],
  args?: Record<string, unknown>,
): Promise<ViewerSnapshot> {
  return invoke<ViewerSnapshot>(commandName, args);
}

function isCommandError(error: unknown): error is CommandError {
  return (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof (error as { message: unknown }).message === "string"
  );
}
