# PRD: Manzar Image Viewer

Triage label: `ready-for-agent`

## Problem Statement

The user wants Manzar to become a secure, concise, modular Image Viewer for Arch Linux. The current application is a default Tauri + React starter app and does not yet provide image opening, Image Sequence navigation, Supported Image validation, Sequence Ordering, Oversized Image warnings, Current Image File Actions, or a minimal image-first user interface.

The user wants the application to remain an Image Viewer rather than a photo manager or editor, while still supporting practical current-image rename and Trash Deletion actions. The implementation must keep Rust as the filesystem and security boundary, use React with shadcn/ui for a minimal frontend, and avoid broad local-file exposure to the WebView.

## Solution

Build Manzar as a local Image Viewer for Arch Linux using Tauri v2, Rust, React, and shadcn/ui. Users can open one or more Supported Images, open folders, or open a single image and navigate sibling Supported Images from the same folder. Images render through the WebView using normal image elements, but they are served by a Rust-owned custom image protocol using opaque image identifiers rather than broad asset-protocol filesystem access.

Rust owns filesystem access, path validation, Supported Image filtering, Sibling Image Discovery, Image Sequence state, Sequence Ordering, Oversized Image preflight checks, settings persistence, rename, and Trash Deletion. React owns image display, zoom/pan behavior, lightweight hover overlay controls, dialogs, keyboard shortcuts, and typed command calls into Rust.

The only persisted user setting is Sequence Ordering. The default Sequence Ordering is newest modified first. Users can also choose natural alphabetical order, file size largest first, or file size smallest first. Sequence Navigation wraps by default and is not configurable in v1.

## User Stories

1. As an Arch Linux user, I want to open a PNG image, so that I can view it locally.
2. As an Arch Linux user, I want to open a JPEG image, so that I can view common camera and web images.
3. As an Arch Linux user, I want to open a JPG image, so that files using either JPEG extension are accepted.
4. As an Arch Linux user, I want to open a WebP image, so that I can view modern web images.
5. As an Arch Linux user, I want to open a GIF image, so that I can view common animated or static GIF files.
6. As an Arch Linux user, I want to open a BMP image, so that I can view simple bitmap files.
7. As an Arch Linux user, I want unsupported formats to be rejected clearly, so that I understand why a file did not open.
8. As an Arch Linux user, I want to open one image and immediately see that image, so that opening a file feels direct.
9. As an Arch Linux user, I want opening one image to create an Image Sequence from sibling Supported Images, so that previous and next navigation works naturally.
10. As an Arch Linux user, I want to open multiple image files explicitly, so that I can view only the Image Selection I chose.
11. As an Arch Linux user, I want to open a folder explicitly, so that I can browse Supported Images in that folder.
12. As an Arch Linux user, I want hidden dotfiles to be skipped from Image Sequences, so that hidden files do not unexpectedly appear while browsing.
13. As an Arch Linux user, I want previous and next controls, so that I can navigate an Image Sequence without reopening files.
14. As an Arch Linux user, I want Sequence Navigation to wrap by default, so that pressing next at the end returns to the first image.
15. As an Arch Linux user, I want pressing previous at the first image to wrap to the last image, so that navigation is continuous.
16. As an Arch Linux user, I want the default Sequence Ordering to be newest modified first, so that recent images appear first.
17. As an Arch Linux user, I want natural alphabetical Sequence Ordering, so that numbered filenames sort in the order humans expect.
18. As an Arch Linux user, I want file size largest-first Sequence Ordering, so that I can review large images first.
19. As an Arch Linux user, I want file size smallest-first Sequence Ordering, so that I can review small images first.
20. As an Arch Linux user, I want my Sequence Ordering preference to persist, so that I do not need to choose it every time.
21. As a privacy-conscious user, I do not want recent files or image history persisted, so that Manzar does not create an unnecessary usage trail.
22. As a privacy-conscious user, I want the frontend to avoid routine full path exposure, so that local directory structure is not unnecessarily present in frontend state.
23. As a power user, I want an explicit way to copy or reveal the current image path later, so that I can still access full paths when I ask for them.
24. As a security-conscious user, I want Rust to own filesystem access, so that the frontend cannot freely read arbitrary local files.
25. As a security-conscious user, I want images served through opaque approved identifiers, so that the WebView cannot load arbitrary local files through broad asset scopes.
26. As a user opening a large image, I want Manzar to warn me before displaying an Oversized Image, so that I understand the risk of slowdown or freezing.
27. As a user opening a large but intentional image, I want to open an Oversized Image anyway, so that Manzar does not block expert workflows.
28. As a user opening Oversized Images rarely, I want Manzar to warn every time, so that no hidden remembered state is needed.
29. As an Arch Linux user, I want Manzar to warn when file size exceeds 200 MB, so that unusually heavy files are treated carefully.
30. As an Arch Linux user, I want Manzar to warn when estimated decoded display memory exceeds 512 MB, so that compressed files that expand massively are treated carefully.
31. As an Arch Linux user, I want images to render efficiently, so that normal viewing does not copy entire image files through command IPC.
32. As an Arch Linux user, I want zoom in and zoom out controls, so that I can inspect image details.
33. As an Arch Linux user, I want actual-size view, so that I can inspect images at 100% scale.
34. As an Arch Linux user, I want fit-to-window view, so that I can see the whole image at once.
35. As an Arch Linux user, I want pan behavior when zoomed in, so that I can inspect different parts of a large image.
36. As an Arch Linux user, I want fullscreen support, so that I can view images without window distractions.
37. As an Arch Linux user, I want a minimal image-first interface, so that the image is the focus.
38. As an Arch Linux user, I want lightweight overlay controls that appear only on hover over the image area, so that controls are discoverable but unobtrusive.
39. As an Arch Linux user, I want keyboard navigation with ArrowRight and ArrowLeft, so that I can browse without using the mouse.
40. As an Arch Linux user, I want keyboard shortcuts for zoom, fit, actual size, fullscreen, open, rename, and delete, so that common actions are fast.
41. As an Arch Linux user, I do not want Space or Backspace to navigate images, so that those keys are not overloaded.
42. As an Arch Linux user, I want to rename the current image, so that I can correct or improve a filename while viewing it.
43. As an Arch Linux user, I want rename to preserve the original file extension, so that renaming does not imply format conversion.
44. As an Arch Linux user, I want rename to stay in the same folder, so that rename does not become a move operation.
45. As an Arch Linux user, I want invalid rename inputs rejected, so that empty names, hidden names, path separators, and collisions do not cause unsafe behavior.
46. As an Arch Linux user, I want the Image Sequence to update after rename, so that navigation and ordering remain accurate.
47. As an Arch Linux user, I want to delete the current image by moving it to trash, so that accidental deletion can be recovered through the desktop environment.
48. As an Arch Linux user, I want delete to apply only to the current image, so that Manzar does not become a bulk file manager.
49. As an Arch Linux user, I want delete confirmation, so that I do not accidentally move an image to trash.
50. As an Arch Linux user, I want no extra confirmation after submitting a valid rename, so that rename remains lightweight.
51. As an Arch Linux user, I want Manzar to show the next image after successful Trash Deletion, so that browsing flow continues.
52. As an Arch Linux user, I want Manzar to show an empty state if the deleted image was the only image, so that the app clearly represents that nothing remains in the sequence.
53. As an Arch Linux user, I want Manzar to stay on the current image if Trash Deletion fails, so that the UI does not pretend the file was removed.
54. As an Arch Linux user, I want clear errors for filesystem failures, so that permission problems and missing files are understandable.
55. As a developer, I want a concise modular Rust backend, so that security-sensitive logic is testable and maintainable.
56. As a developer, I want thin command handlers, so that Tauri command glue does not contain business logic.
57. As a developer, I want deep modules for Image Sequence construction, Sequence Ordering, metadata preflight, path security, settings, and file actions, so that each can be tested independently.
58. As a developer, I want typed frontend command wrappers, so that React code does not scatter raw command names and untyped payload assumptions.
59. As a developer, I want a small React state model without an external state manager, so that the frontend remains concise.
60. As a developer, I want shadcn/ui components added selectively, so that the UI remains accessible without adding unnecessary component surface area.
61. As a maintainer, I want a strict content security posture, so that the app does not ship with unrestricted frontend security settings.
62. As a maintainer, I want the Rust-owned image protocol decision documented, so that future contributors do not replace it with broad asset protocol access without understanding the trade-off.

## Implementation Decisions

- Build Manzar as a local Image Viewer, not a photo manager, gallery library, or image editor.
- Start from the existing Tauri v2 + React + TypeScript starter app and replace the starter greeting UI and command with Image Viewer functionality.
- Use React with shadcn/ui for the frontend. Add only the components needed for v1, such as buttons, dialogs, menus or popovers, and a Sequence Ordering selector.
- Use WebView image rendering for v1 instead of Rust-side pixel decoding. This avoids full image-byte transfer through command IPC and avoids sending decoded bitmaps to the frontend.
- Serve displayable images through a Rust-owned custom image protocol backed by approved Image Sequence state, as recorded in the accepted ADR.
- Do not expose broad local filesystem scopes to the WebView for arbitrary user-selected image locations.
- Rust owns filesystem access, path validation, Supported Image filtering, Sibling Image Discovery, Image Sequence state, metadata preflight, settings persistence, rename, and Trash Deletion.
- React owns the image-first UI, hover overlay, dialogs, keyboard shortcuts, zoom, pan, fit-to-window, actual-size view, and typed command calls.
- Use opaque image identifiers for frontend display URLs. The frontend should not routinely receive absolute filesystem paths.
- Provide explicit commands later for actions that require full paths, such as copy path or reveal in file manager.
- Supported Image formats for v1 are PNG, JPEG/JPG, WebP, GIF, and BMP.
- Defer SVG, TIFF, AVIF, HEIC, RAW formats, multipage image behavior, and explicit animation controls.
- Opening a single image displays that image and creates an Image Sequence from sibling Supported Images in the same folder.
- Opening multiple files creates an Image Sequence from the explicit Image Selection.
- Opening a folder creates an Image Sequence from Supported Images in that folder.
- Skip hidden dotfiles when building Image Sequences.
- Sequence Navigation wraps by default and is not configurable in v1.
- Persist only Sequence Ordering as a user setting.
- Default Sequence Ordering is newest modified first.
- Additional Sequence Ordering options are natural case-insensitive alphabetical, file size largest first, and file size smallest first.
- Alphabetical ordering should be natural and case-insensitive, with human-friendly numeric ordering.
- Sequence Ordering tie-breakers should prefer natural filename order, then a stable backend-owned final ordering.
- Do not persist recent files, image history, approved paths, oversized-warning decisions, or full paths.
- Warn for an Oversized Image when file size exceeds 200 MB or estimated decoded RGBA display memory exceeds 512 MB.
- Allow users to open Oversized Images anyway.
- Do not remember Oversized Image approval across session or image visits; warn every time.
- Rename is a Current Image File Action only.
- Rename is stem-only, preserves the original extension, and stays in the same folder.
- Reject rename inputs that are empty, contain path separators, start with a dot, or collide with an existing file.
- After rename, update the current image descriptor and rebuild or reorder the Image Sequence according to the current Sequence Ordering.
- Delete is a Current Image File Action only.
- Delete means Trash Deletion, not permanent delete.
- Trash Deletion requires confirmation.
- Rename does not require a second confirmation after valid submission.
- After successful Trash Deletion, remove the trashed image from the Image Sequence and show the next image according to current sequence direction and wraparound behavior.
- If the trashed image was the only image, show an empty state.
- If Trash Deletion fails, keep the current image visible and show an error.
- The minimal UI should be image-first with a dark viewing background.
- Overlay controls should be lightweight and appear only when hovering over the image area.
- Keyboard shortcuts are ArrowRight for next, ArrowLeft for previous, plus or equals for zoom in, minus for zoom out, zero for actual size, F for fit to window, F11 for fullscreen, O for open image files, Shift+O for open folder, R for rename, Delete for Trash Deletion, and Escape for closing dialogs, exiting fullscreen, or clearing transient UI state.
- Space and Backspace must not be image navigation shortcuts.
- Use concise vertical modules rather than a heavy layered framework.
- Deep Rust modules should include Image Sequence construction, Sequence Ordering, metadata preflight and Oversized Image warning calculation, image protocol registry/serving, path security, settings persistence, and Current Image File Actions.
- Thin Rust command modules should expose open, navigate, settings, and file-action operations without embedding core behavior in command glue.
- Frontend modules should include viewer composition, image canvas behavior, hover overlay controls, keyboard shortcuts, typed Tauri command wrappers, dialogs, and Sequence Ordering selection.
- The error model should map backend errors into frontend-safe messages without leaking unnecessary path details.
- The application security configuration should move away from unrestricted content security settings and toward a narrow policy compatible with the custom image protocol and Tauri IPC.

## Testing Decisions

- Good tests should assert externally visible behavior and security boundaries, not private implementation details.
- Because the current codebase is a starter app with no meaningful test suite, new tests should establish the first testing patterns for both Rust and frontend behavior.
- Test the Image Sequence module with temporary directories and files to verify Supported Image filtering, hidden-file skipping, Sibling Image Discovery, explicit Image Selection behavior, folder-open behavior, and current-image positioning.
- Test the Sequence Ordering module with controlled file metadata to verify newest modified first, natural case-insensitive alphabetical ordering, file size largest first, file size smallest first, and tie-breakers.
- Test metadata preflight behavior with representative image headers and file sizes to verify Oversized Image warning thresholds without decoding full images into pixels.
- Test path security behavior to verify canonicalization, same-folder rename constraints, rejection of path separators, rejection of hidden rename targets, collision handling, and unsupported file rejection.
- Test settings persistence to verify only Sequence Ordering is saved and restored, and no recent files, image history, approved paths, or full paths are persisted.
- Test Current Image File Actions using temporary files and a test double or isolated implementation for trash behavior where needed, verifying current-image-only scope and sequence updates after rename or Trash Deletion.
- Test custom image protocol authorization to verify approved opaque image IDs can be served and unknown, stale, unsupported, or non-sequence IDs are rejected.
- Test command handlers at the boundary level to verify frontend-safe DTOs, error messages, and no routine full-path exposure.
- Test frontend keyboard behavior to verify the agreed shortcuts trigger the right actions and Space/Backspace do not navigate.
- Test frontend dialogs to verify Oversized Image warning, delete confirmation, and rename validation behavior from the user's perspective.
- Test viewer behavior to verify overlay controls appear only on hover/focus conditions intended for the image area and do not become a permanent toolbar.
- Prefer Rust unit tests for pure deep modules and integration-style tests for filesystem behavior.
- Prefer React component tests for user-visible interaction behavior if a frontend test harness is added.
- Avoid snapshot-heavy frontend tests for the minimal UI; behavior and accessibility queries are more valuable.

## Out of Scope

- Photo library management, catalogs, albums, tagging, ratings, search, or indexing.
- Image editing, crop, rotate-and-save, color adjustment, metadata editing, or format conversion.
- Bulk rename, bulk delete, arbitrary file management, folder management, or recursive library scans.
- Permanent delete in v1.
- Persistent recent files, browsing history, approved path history, or remembered Oversized Image approvals.
- SVG, TIFF, AVIF, HEIC, RAW formats, multipage image behavior, and explicit animation controls.
- Thumbnails, filmstrip UI, contact sheets, or gallery grid views.
- Tiled/deep-zoom rendering for extremely large images.
- Database-backed state.
- Cloud, network, or remote image loading.
- Configurable wraparound, configurable warning thresholds, configurable confirmations, theming, or extensive preferences beyond Sequence Ordering.
- Space or Backspace navigation shortcuts.
- Broad Tauri asset protocol filesystem access for user-selected image locations.

## Further Notes

- The current app is still the default starter UI and starter Rust command, so implementation will be a replacement rather than an incremental feature addition.
- The accepted ADR requires a Rust-owned image protocol with opaque identifiers. Future implementation should respect that decision unless a new ADR supersedes it.
- The glossary defines the canonical language for this work. PRD and implementation should consistently use Image Viewer, Supported Image, Image Sequence, Sibling Image Discovery, Sequence Ordering, Sequence Navigation, Oversized Image, Current Image File Action, and Trash Deletion.
- The custom protocol is intentionally chosen over direct path-to-asset conversion to reduce accidental local-file exposure.
- WebView rendering is chosen for v1 because it is concise and avoids unnecessary IPC copies, while Rust metadata preflight provides a safety warning for unusually expensive images.
- The implementation should prioritize small, deep, testable backend modules and a minimal frontend without introducing global state management unless the app grows beyond v1 needs.
