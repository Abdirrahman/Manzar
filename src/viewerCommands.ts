import { invoke } from "@tauri-apps/api/core";

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

export type MediaKind = "image" | "video";

export type ViewerImage = {
  id: string;
  filename: string;
  url: string;
  kind: MediaKind;
  preflight: ImagePreflight;
};

export type ImagePreflight = {
  file_size_bytes: number;
  dimensions: ImageDimensions | null;
  oversized: boolean;
  crop_supported: boolean;
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

export type CropRect = { x: number; y: number; width: number; height: number };

export const getViewerSnapshot = () =>
  invoke<ViewerSnapshot>("get_viewer_snapshot");
export const openSingleImage = (path: string) =>
  invoke<ViewerSnapshot>("open_single_image", { path });
export const openImageSelection = (paths: string[]) =>
  invoke<ViewerSnapshot>("open_image_selection", { paths });
export const openFolder = (path: string) =>
  invoke<ViewerSnapshot>("open_folder", { path });
export const navigateNext = () => invoke<ViewerSnapshot>("navigate_next");
export const navigatePrevious = () =>
  invoke<ViewerSnapshot>("navigate_previous");
export const setSequenceOrdering = (ordering: SequenceOrdering) =>
  invoke<ViewerSnapshot>("set_sequence_ordering", { ordering });
export const renameCurrentImage = (newStem: string) =>
  invoke<ViewerSnapshot>("rename_current_image", { newStem });
export const trashCurrentImage = () =>
  invoke<ViewerSnapshot>("trash_current_image");
export const cropCurrentImage = (imageId: string, rect: CropRect) =>
  invoke<ViewerSnapshot>("crop_current_image", { imageId, rect });

export function backendErrorMessage(error: unknown): string {
  if (
    typeof error === "object" &&
    error !== null &&
    "message" in error &&
    typeof error.message === "string"
  )
    return error.message;
  return typeof error === "string"
    ? error
    : "The action could not finish. Please try again.";
}
