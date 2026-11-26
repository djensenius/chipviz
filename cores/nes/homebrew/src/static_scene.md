# NES Static model (unlinked)

This deterministic model is not linked into the ROM or selectable. It does
not change the existing scene selector, controls, scene-7 gates or cartridge
layout. The three caller-supplied looks (pixel, glyph and mixed) are internal
model inputs, not catalog-v1 variants or wire values. Catalog Static has
variant 0; no mapping to these looks is specified.

Initialize the state before use. Each valid step observes the beat level: a
rising edge perturbs the generator, including while waiting, but a held beat
does not retrigger or force a patch. When a patch is due, energy selects the
wait until the next patch: values 0–63, 64–127, 128–191 and 192–255 give
periods of 4, 3, 2 and 1 steps respectively, if energy remains constant.
The band byte affects tile choices. A look change affects subsequent patches.

`NES_STATIC_PATCH_READY` supplies eight tile indices and an address in the
32-by-30 nametable range. The cursor advances in raster order and wraps after
120 patches. `NES_STATIC_NO_PATCH` advances state but leaves the patch
untouched. An invalid look returns `NES_STATIC_INVALID_LOOK` without changing
state or patch. The caller must provide valid state and patch pointers.

The supplied CHR data occupies 17 tiles (272 bytes): 16 distinct nonblank
tiles each have eight set bits in the first plane and a zero second plane;
tile 16 is blank. The illustrative palette is four bytes. A hypothetical
consumer would need to allocate or relocate the CHR indices, reserve the
palette, initialize the nametable and attributes, and manage its own patch
storage and PPU uploads. The current NROM uses CHR ROM. No resource placement,
upload schedule or integration interface is established by this model.

Run the host-only tests with
`python3 -m unittest discover -s tests -p test_nes_static.py`. They compile
and load this model as a host library and compare its output with an independent
oracle. They do not execute a ROM or demonstrate target placement, displayed
pixels, appearance, timing, transitions or physical safety. Selection,
PPU/VBlank timing, target presentation and hardware evidence remain future
work under #153 and its related issues.
