# Patched dependencies

## femtovg 0.27.0 + #366

`femtovg/` is the published femtovg 0.27.0 (MIT OR Apache-2.0, licences
included) with one change in `src/renderer/wgpu.rs`: upstream's
[#366](https://github.com/femtovg/femtovg/commit/8d092f2) "Keep WGPU
pipelines across flushes that don't use them", ported by hand (it does not
apply to 0.27 as is). Book, examples and tests of the crate are left out.

Why: 0.27 dropped every cached render pipeline a flush did not use, so a
streaming chat (whose frames differ in what they draw) recompiled pipelines and
their shaders every few frames; that was ~70% of the main thread in a real
window. With the backport, a 30 s stream into a 1000-message thread in a GPU
window took 9.6-10.2 s of process CPU instead of 16.4-23.6 s, at 114-115 fps
instead of 103-107 (FemtoVG-WGPU, Intel UHD, Wayland).

Remove it (and the `[patch.crates-io]` entry in the root `Cargo.toml`) when
Slint depends on femtovg 0.28 or later: Slint's main branch already does
(`internal/renderers/femtovg/Cargo.toml`), the 1.18.1 release does not.
