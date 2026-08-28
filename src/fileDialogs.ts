import { open } from "@tauri-apps/plugin-dialog";

const supportedImageExtensions = [
  "png",
  "jpg",
  "jpeg",
  "webp",
  "gif",
  "bmp",
  "PNG",
  "JPG",
  "JPEG",
  "WEBP",
  "GIF",
  "BMP",
];

const supportedImageFilter = {
  name: "Supported Images",
  extensions: supportedImageExtensions,
};

export async function pickImageFiles(): Promise<string[] | null> {
  const selected = await open({
    title: "Open Image",
    multiple: true,
    directory: false,
    filters: [supportedImageFilter],
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
