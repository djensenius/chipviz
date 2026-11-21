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

The launcher requires a compatible modern renderer with scene 6 and the
`--scene`, `--star-catalog`, `--location`, and `--time` flags; see
[#14](https://github.com/djensenius/chipviz/issues/14). The utility does not
run or verify the binary.

Build a compatible renderer and supply your locally obtained HYG
v4.4 `.csv` or `.csv.gz` (no network access is performed by this utility).
Before creating the destination, ensure the checkout, `build/`, and their
ancestors are trusted and contain no symlinks:

```sh
mise exec -- cargo build --release --manifest-path renderers/modern/Cargo.toml --bin chipviz-m1 &&
mkdir -p build &&
(
  umask 077 &&
  mkdir -m 700 build/personal-starfield &&
  mise exec -- python scripts/personal-starfield-bundle.py \
    --catalog "/path/to/local/hyg_v44.csv.gz" \
    --binary "renderers/modern/target/release/chipviz-m1"
) &&
./build/personal-starfield/launch-starfield --location "0,0" --time 946728000
```

The example explicitly selects latitude/longitude `0,0` and
`2000-01-01 12:00:00 UTC` (`946728000` Unix seconds), not the current sky.
Replace them with your chosen decimal-degree location (longitude east) and UTC
Unix time. The launcher inserts only `--scene 6` and the bundled
`--star-catalog`, forwarding every other argument unchanged. It never supplies
a location or time. The compatible renderer requires a location and, if no
time is supplied, defaults to current UTC.

This native launch is not a [catalog-v1](../../shared/specs/scene-catalog-v1.md)
session or recording. That future format uses Unix milliseconds, requires a
2000–2099 UTC epoch and separately validated provenance; an in-range timestamp
alone does not enable it.

The default output is the ignored `build/personal-starfield/` directory, with
the executable, uncompressed CSV, launcher, and `HYG-NOTICE.txt`. Another
compatible modern profile executable may be supplied instead of `chipviz-m1`.
Use `--upstream-notice "/path/to/local/notice"` for each additional upstream
notice available locally; each is copied unchanged. The tool does not discover
or verify upstream notices, the supplied catalog's version, origin, or CSV
contents. Both plain CSV and decompressed gzip are limited to 64 MiB
(67,108,864 bytes); the known HYG v4.4 CSV is 33,929,696 bytes.

Create the destination yourself before running the utility. It must be an
empty, private directory owned by you and below this checkout's ignored
`build/`. For another destination, create it similarly and add, for example,
`--output build/my-personal-bundle`. Input files must be regular files, not
symlinks. Output paths cannot traverse symlinks or enter release paths or
another Git checkout.

The destination and all its ancestors, including their ACLs, must be trusted
against concurrent changes. Permissions do **not** isolate processes using
the same user account. The utility holds an open directory descriptor: if the
directory is renamed while it runs, writes continue into the opened directory
and the printed path may no longer locate it.

The utility never changes permissions, deletes files, or overwrites entries.
Failures can leave partial files (including an incomplete launcher); inspect
them and manually remove only your own unwanted files, or prepare another
empty directory before retrying. It reports an error if the process umask
strips the launcher's required owner permissions.

