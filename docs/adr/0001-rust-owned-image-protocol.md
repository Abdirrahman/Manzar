# Use a Rust-owned image protocol for approved image display

Manzar will serve viewed images through a Rust-owned custom protocol using opaque image identifiers instead of exposing filesystem paths through Tauri's general asset protocol. This keeps Rust as the authorization boundary for supported images and sequence membership while still allowing the React frontend to render images with normal WebView image elements.

## Considered Options

- Use Tauri's built-in asset protocol with broad filesystem scope.
- Use Tauri's built-in asset protocol with narrow static scopes.
- Use a Rust-owned custom image protocol backed by approved image state.

## Consequences

The custom protocol reduces accidental local-file exposure and avoids broad asset scopes, but it requires MIME handling, protocol tests, and careful mapping from opaque image identifiers to approved files.
