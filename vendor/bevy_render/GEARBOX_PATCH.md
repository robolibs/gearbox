# Sparse buffer page bookkeeping

Source: crates.io `bevy_render` 0.19.1, with the original MIT/Apache licences.

Local patch: `src/render_resource/sparse_buffer_vec.rs`.

- `grow` and `truncate` round the number of pages up, retaining the dirty word for a partially occupied page.
- `grow` marks existing dirty words even when growth starts in page zero of that word.
- Tests cover sub-page buffers, page/word boundaries, truncation, clean-to-dirty growth and thread-safe writes.

Gearbox's globe-site transition exposed the original out-of-bounds panic in `note_changed_index`. GPU preprocessing remains enabled. Run `oslo make test-render-pages`; native terrain streaming remains a separate acceptance gate.
