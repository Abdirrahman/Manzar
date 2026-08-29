import { open } from "@tauri-apps/plugin-dialog";

// Mirrors SUPPORTED_MEDIA in src-tauri/src/core/supported_image.rs. Both cases
// are listed because the dialog filter is case-sensitive on Linux.
const supportedMediaExtensions = [
  "png",
  "jpg",
  "jpeg",
  "webp",
  "gif",
  "bmp",
  "mp4",
  "mov",
  "mkv",
  "webm",
].flatMap((extension) => [extension, extension.toUpperCase()]);

const supportedMediaFilter = {
  name: "Supported Images and Videos",
  extensions: supportedMediaExtensions,
};

export async function pickImageFiles(): Promise<string[] | null> {
  const selected = await open({
    title: "Open Images or Videos",
    multiple: true,
    directory: false,
    filters: [supportedMediaFilter],
  });

  if (selected === null) {
    return null;
  }

  const paths = Array.isArray(selected) ? selected : [selected];
  return paths.length > 0 ? paths : null;
}

export async function pickImageFolder(): Promise<string | null> {
  const selected = await open({
    title: "Open Folder",
    multiple: false,
    directory: true,
  });

  if (selected === null) {
    return null;
  }

  if (Array.isArray(selected)) {
    return selected[0] ?? null;
  }

  return selected;
}
