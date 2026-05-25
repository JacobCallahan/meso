# Meso Copilot Instructions

Meso is a professional Rust/GTK4 desktop weather app for Fedora Linux. It targets weather enthusiasts and professionals who already know what they want; keep the UX dense, direct, and operational.

## Build & Run

```bash
cargo build
./target/debug/meso

cargo build --release
./target/release/meso

RUST_LOG=info ./target/debug/meso
RUST_LOG=debug ./target/debug/meso

cargo check -p meso-app
cargo check -p meso-data
cargo check -p meso-render
cargo check -p meso-updraft
```

There are no automated tests in this repo; verify behavior by running the app.

Fedora system deps:

```bash
sudo dnf install gtk4-devel libadwaita-devel
```

Config is persisted to `~/.config/Meso/config.toml` on exit. It is safe to hand-edit while the app is closed.

## Workspace

- `meso-data` — fetching, decoding, caching, and domain types
- `meso-render` — Cairo/wgpu rendering, no GTK dependency
- `meso-app` — GTK4/libadwaita UI
- `meso-updraft` — optional background caching daemon

## Current shipped features

- Radar: Level 2 and Level 3 radar, animation, storm-track overlay, warnings, right-click gate inspection
- Satellite: GOES sector/band browser with animation
- Alerts: active NWS alerts with area selection and detail view
- Forecast: 7-day forecast, hourly forecast, and current conditions
- Soundings: Skew-T viewer with search and favorites
- National: WPC, NHC, and other national product panes
- SPC: outlooks, storm reports, and mesoanalysis
- Models: SREF/NCEP model viewer with search, favorites, and local timestamps
- Text: NWS text product browser
- Settings: palettes, rendering, cache retention, locations, and updraft settings
- Updraft: optional background cache daemon and systemd user service

## Core conventions

- All async work goes through `runtime::spawn(future, callback)`. Do not `.await` inside GTK signal handlers.
- Always use `meso_data::http::wx_client()`. The NWS API rejects reqwest's default user agent.
- Shared config flows through `Rc<RefCell<Config>>` and is saved from the app close handler.
- New config fields need `#[serde(default = "fn_name")]` or `#[serde(default)]` as appropriate.
- Radar uses a geographic viewport; satellite uses image-space zoom/pan.
- Radar azimuth is stored clockwise from north. Do not convert to the old wX `450 - az` convention.
- `geometry.rs::emit_quad_km` assumes the clockwise-from-north convention.
- Level 2 range requests must send `Accept-Encoding: identity`.
- `fetch_level2_decompressed()` is the preferred entry point for animation frames because it reuses the decompression cache.
- Keep radar animation vectors parallel: `anim_pixbufs`, `anim_timestamps`, `anim_l2_frames`, and `anim_l3_frames`.
- `viewport.width` and `viewport.height` must track the actual DrawingArea size.

## Repo hygiene for first git history

- Track source, docs, manifests, and `Cargo.lock`.
- Ignore build output, cache directories, and local editor junk.
- Keep generated or machine-specific files out of the repository unless they are intentionally part of the product.

## Known future work

Treat items below as unimplemented until the code proves otherwise:

- radar dual-pol products
- radar mosaic / blended radar views
- satellite overlays and RGB composites
- RTMA and upper-air observation products
- NHC / WPC dashboard expansion
- dark-mode polish, status bar, and accessibility work
- Flatpak / package distribution work

