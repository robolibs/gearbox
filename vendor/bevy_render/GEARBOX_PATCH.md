# Renderer patches

Source: crates.io `bevy_render` 0.20.1, with the original MIT/Apache licences.

Local patch: `src/mesh/allocator.rs` clamps the mesh slab size and large-allocation
threshold to the render device's maximum buffer size.

Bevy 0.20 replaced page-based sparse-buffer tracking with per-element dirty bits
and a summary bitset. The old page rounding patch is superseded. Gearbox's
boundary, clean-to-dirty growth, and concurrent-write regressions exercise the
new implementation in `src/render_resource/sparse_buffer_vec.rs`.

Run `oslo make test-render-pages`; native terrain streaming remains a separate
acceptance gate. GPU preprocessing remains enabled.
