# Uploads out of a small BAR heap

Source: crates.io `gpu-allocator` 0.28.0, with the original MIT/Apache licences.

Local patch: `src/vulkan/mod.rs`.

- `CpuToGpu` allocations first look for a type outside host-visible VRAM on a
  heap of at most 512 MiB (`find_memorytype_index_outside_small_bar`), then
  fall back to the original preferred and required lookups.
- Resizable BAR and unified-memory heaps are larger and keep the original
  device-local preference.

wgpu 28+ stages every `Queue::write_buffer` in `CpuToGpu` memory
(gfx-rs/wgpu#10484). On the NVIDIA proprietary driver without Resizable BAR
that is the shared 256 MiB BAR, mapped uncached: glibc's `rep movsb` memcpy
into it ran at ~14 MB/s and Bevy's 1.5 MB instance-buffer upload took 110 ms
a frame. Upstream fix pending as Traverse-Research/gpu-allocator#315; drop
this copy once a release carries it.
