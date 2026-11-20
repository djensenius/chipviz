# scenes

Shared scene sketches and data-driven visual definitions.

## Local personal starfield bundle

This opt-in workflow is separate from all release and commercial packaging.
It does not download a catalog or put one in the repository. Keep the
result **local, for personal/noncommercial use only**: do not upload, publish,
redistribute, or use the bundle commercially. Commercial licensees must supply
their own catalog.

HYG v4.4 is offered upstream as
[CC BY-SA 4.0](https://creativecommons.org/licenses/by-sa/4.0/):
HYG Database, astronexus, https://codeberg.org/astronexus/hyg. The
[upstream v4.4 CSV gzip](https://codeberg.org/astronexus/hyg/media/branch/main/data/hyg/CURRENT/hyg_v44.csv.gz)
includes Hipparcos-derived data; ESA's Hipparcos terms have a CC BY-NC 3.0 IGO
caveat and require **Credit: ESA**. HYG's licence label does not resolve all
upstream terms or establish commercial/public redistribution rights. Verify
the relevant source terms for local personal use before obtaining or using the
data, and retain all accompanying copyright and attribution notices.

For the v4.4 gzip, upstream LFS metadata reports 13,636,362 bytes and SHA-256
`00b349893b9a53106dd488d8371e8d2fa586043e500bb3cdb8bff3931682197d`.
The `CURRENT` URL can change; that hash applies to the compressed file only
and is not verified by the utility.

The launcher depends on the modern renderer changes planned in
[#14](https://github.com/djensenius/chipviz/issues/14): scene 6 and the
`--scene`, `--star-catalog`, `--location`, and `--time` flags. They are not yet
part of this base; use a compatible renderer only after that work is
integrated. The utility does not run or verify the binary.

After that integration, build a renderer and supply your locally obtained HYG
v4.4 `.csv` or `.csv.gz` (no network access is performed by this utility):

```sh
cargo build --release --manifest-path renderers/modern/Cargo.toml --bin chipviz-m1
python3 scripts/personal-starfield-bundle.py \
  --catalog "/path/to/local/hyg_v44.csv.gz" \
  --binary "renderers/modern/target/release/chipviz-m1"
./build/personal-starfield/launch-starfield --location "0,0" --time 0
```

The example explicitly selects latitude/longitude `0,0` and Unix time `0`;
replace them with your chosen decimal-degree location (longitude east) and UTC
Unix time. The launcher inserts only `--scene 6` and the bundled
`--star-catalog`, forwarding every other argument unchanged. It never supplies
a location or time. The planned renderer requires a location and, if no time
is supplied, defaults to current UTC.

The default output is the ignored `build/personal-starfield/` directory, with
the executable, uncompressed CSV, launcher, and `HYG-NOTICE.txt`. Another
compatible modern profile executable may be supplied instead of `chipviz-m1`.
Use `--upstream-notice "/path/to/local/notice"` for each additional upstream
notice available locally; each is copied unchanged. The tool does not discover
or verify upstream notices, the supplied catalog's version, origin, or CSV
contents.

To choose another local destination, add `--output "/private/path/new-bundle"`.
Its parent must already exist and the output must not exist, even as a dangling
symlink. Input files themselves must be regular files, not symlinks. Output
paths cannot traverse symlinks or enter `dist/`, release paths, another Git
checkout, or tracked areas of this checkout; use a physical path such as
`/private/tmp/...` instead of macOS's symlinked `/tmp/...`. Only `build/` is
created automatically. Existing files are never overwritten; if a partial
directory cannot be safely cleaned up, the tool reports it for inspection.

