# Manzar

Manzar is a local image-viewing application for opening and navigating image files without becoming a photo manager or editor.

## Language

**Image Viewer**:
A local application for opening and navigating image files without cataloging, editing, or managing a photo library.
_Avoid_: Photo manager, image editor, gallery app

**Image Selection**:
An explicit set of image files chosen by the user for viewing.
_Avoid_: Upload, import

**Image Sequence**:
An ordered set of image files that Manzar can navigate with previous and next controls.
_Avoid_: Album, playlist, gallery

**Sibling Image Discovery**:
The creation of an image sequence from the other supported image files in the same folder as an opened image.
_Avoid_: Folder import, library scan

**Sequence Ordering**:
The user preference that arranges an image sequence for previous and next navigation.
_Avoid_: Sort hack, file listing order

**Sequence Navigation**:
Movement through an image sequence using previous and next controls.
_Avoid_: Back/forward, history navigation

**Supported Image**:
An image file whose format Manzar intentionally accepts for viewing.
_Avoid_: Any file, media file, asset

**Oversized Image**:
A supported image whose estimated display cost is high enough that Manzar should warn before showing it.
_Avoid_: Bad image, broken image, unsupported image

**Current Image File Action**:
A file operation that applies only to the image currently being viewed.
_Avoid_: Bulk file management, photo management, folder management

**Trash Deletion**:
A current image file action that removes the viewed image by moving it to the desktop trash rather than permanently deleting it.
_Avoid_: Permanent delete, unlink
