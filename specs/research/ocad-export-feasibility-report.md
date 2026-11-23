# OCAD export feasibility report

Date: 2026-11-23

## Executive summary

Writing an OCAD file (`.ocd`, format version 12) directly from `cassini batch` is feasible. It is a medium-to-large feature, and the file format is not the hardest part:

1. **The OCD format is tractable.** The [OCAD 12 wiki page](https://www.ocad.com/wiki/ocad/en/index.php?title=OCAD_12_File_Format) gives byte layouts but leaves important gaps and contains at least one documented error. [OpenOrienteering Mapper](https://github.com/OpenOrienteering/mapper) (OOM) has a working OCD 8–12 writer under GPL-3.0-or-later, which is compatible with Cassini's GPL-3 licence. It settles every ambiguity found so far.
2. **Symbols are the riskiest part of the format.** Encoding ISOM line and area symbols from scratch is laborious and error-prone. Copying them from a template `.ocd` file avoids most of that work.
3. **Cassini needs vector geometry for every layer.** Contours, form lines and OSM features already exist as vectors. Vegetation and cliffs only exist as rasters.
4. **Vegetation vectorization works in a prototype.** Tracing isobands on the density fields Cassini already computes, after applying the ISOM minimum areas, turns one 1 km² test tile into about 740 clean area objects in about 1 s.
5. **The renderer draws features as soon as it classifies them.** A small "map feature" model has to sit between classification and output so that the Skia renderer and the OCD writer can share it. The [architecture report](architecture-and-maintainability-report.md) (§6) already recommends this split.

Recommended order: build the OCD writer first, using only the layers that are already vectors. That settles the biggest unknown (whether OCAD opens the file) early and cheaply. Then add vegetation polygons, merge tiles in batch mode, and do cliffs last.

## Current state per layer

| Layer | Current form | Vector-ready? | ISOM 2017-2 target |
| --- | --- | --- | --- |
| Contours | Smoothed polylines in memory (`pullautin_smooth_contours`) and `contours/contours.shp`. The contour class is derived from elevation in the renderer. | Yes | 101 contour, 102 index contour |
| Form lines | Culled segments written to `formlines/formlines.shp` | Yes | 103 form line |
| Depression contours | Coloured purple in the raster (`is_contour_depression`) | Flag available | No ISOM equivalent. Use 101/102, optionally with a non-printing helper symbol |
| OSM features | Shapefiles (EPSG:2154), classified in `vectors.rs` and drawn immediately by `MapRenderer` | Yes, but classification is interleaved with drawing | 301, 304, 306, 308, 502, 503, 505, 509, 510, 520, 521, … |
| Vegetation | 1 m class grids and f32 density fields in `render_vegetation`, drawn as discs and rectangles | No | 403, 406, 408, 410, and 409 or 406 for undergrowth |
| Cliffs | One black dot per 0.5 m DEM cell where slope exceeds the threshold (`cliffs.rs`) | No | 201 / 202 (lines with tags) |

Cassini's raster colours already match standard ISOM screens:
- `YELLOW` (255, 221, 154) is 50 % yellow, which is **403 Rough open land**, not 401.
- `GREEN_1`, `GREEN_2` and `GREEN_3` are green 30 %, 60 % and 100 %, which are **406, 408 and 410**.
- Forest (405) is white, so it needs no objects.

The mapping from Cassini's classes to ISOM symbols is therefore direct.

## OCD format findings

### What the OCAD wiki gives, and where it is wrong or silent

The OCAD 12 page documents the file header (60 bytes), the symbol, object and string index blocks (256 entries each, chained), the symbol structures (`TBaseSym`, `TLineSym`, `TAreaSym`, …), `TOcadObject`, and the coordinate encoding. It states that this "is an internal format and may change in future versions".

Gaps and inconsistencies, each resolved by reading OOM's writer (`src/fileformats/ocd_types_v12.h` and `ocd_file_export.cpp`):

| Question | Wiki | OOM (works with OCAD) |
| --- | --- | --- |
| Object size in the index | `32 + 8*Len` | Size in **bytes**. OOM's comment says that following the documentation "would make files unusable (due to "damaged objects")". |
| Object header size | Not stated (fields sum to 56) | 56 bytes, then the coordinates |
| Symbol `Description` width | `array[0..63] of char`, width not stated | UTF-16LE, 128 bytes, so `BaseSymbol` is 796 bytes |
| Parameter string encoding | Not stated | UTF-8, NUL-terminated. The index size includes the NUL. |
| Alignment | Not stated | Every block is padded to an 8-byte boundary |
| End of an index chain, empty slot | Defined only for strings (0) | `next_block = 0`, `entry.pos = 0` |
| Y axis | Not stated | Y points up (OOM negates its own Y) |
| Coordinates | `(value << 8) \| flags`, 0.01 mm | Same. x flags: Bézier control points. y flags: corner, hole start, dash. |
| Required records | Not stated | OOM writes 1039 (scale/georef), 1030 (view), 9/10 (colours), 1061 (notes), plus symbols and objects |

Versions: OOM writes OCD 8, 9, 10, 11 and 12. It reads OCAD 2018 files experimentally, by treating them as v12. It has no writer for the 2018+ format. Which OCAD releases open v12 files is not documented. Current OCAD releases are expected to open older formats, but this needs a manual check in OCAD and is the first acceptance test.

### Coordinates and georeferencing

- Coordinate values have 24 signed bits in 0.01 mm paper units, so ±83.9 m of paper. At 1:15 000 that is ±1 258 km of terrain around the origin, which is ample for any batch. Resolution is 0.15 m of terrain.
- Georeferencing is the 1039 string: `m` (scale), `x`/`y` (integer real-world offset), `a` (grivation), `r=1` (real-world coordinates on), `i` (OCAD grid and zone code), and from v10 also `b`/`c`. The conversion is `paper_0.01mm = (E − x) · 100 000 / m`, rotated by `a`.
- Cassini hardcodes EPSG:2154 (Lambert-93) in the `ogr2ogr` calls. The OCAD `i` code for Lambert-93 is not documented. The simplest fix is to take the 1039 and 1053 records from the template, so the coordinate system is set once in OCAD, and have Cassini rewrite only `x`/`y`. OOM's `ocd_georef_fields.cpp` table may contain the mapping.
- Scale: ISOM's base scale is 1:15 000. Cassini's raster is drawn at 1:10 000 (1 m = 0.1 mm at 600 DPI). The OCD file should take its scale from the template. With a 1:15 000 ISOM template, OCAD enlarges symbols to 150 % when printing at 1:10 000.

### Symbols: use a template, do not synthesize

Writing correct ISOM symbol definitions from code means getting dash patterns, double lines (502), hatch and structure fills (407, 409, 402/404 dots), 22×22 palette icons and many partly documented fields right. That is where most of the format risk lies.

Instead, ship a small **template `.ocd`** that contains the ISOM 2017-2 colours and symbols and no objects:

1. Create it once: open OOM's bundled `ISOM 2017-2_15000.omap` (GPL-3; installed here under `/usr/share/openorienteering-mapper/symbol sets/15000/`) and save it as OCD 12. Check it in OCAD, because OOM reports some conversions as lossy (dash groups, some cap/join combinations). OOM also ships `ISOM 2017-2_10000.omap` (under `10000/`), the 150 % enlargement for 1:10 000, which matches Cassini's render scale directly; either works, since the minimum areas in terrain metres are the same at both scales.
2. At export time, Cassini copies the template's colour strings and symbol blocks unchanged, rewrites the 1039 offset, and appends its own object index blocks and objects.
3. Add an `--ocad-template <file>` option so users can supply their own symbol set, such as a club set saved from OCAD. Cassini checks at start-up that every symbol number it emits (403.000, 406.000, …) exists in the template, and fails with a list of the missing ones.

The writer then only needs to read index chains, append objects and strings, and fix up offsets. Before bundling any symbol set other than OOM's, check its licence.

## Vegetation as vector polygons

### Post-process the fields, not the PNG

Vectorizing as a post-process is the right approach, but it should start from the **classification fields** (1 m grids already in memory in `render_vegetation`), not from `vegetation.png`:

- The PNG contains fixed-size discs and overlapping rectangles in RGB at 600 DPI. Tracing it gives staircase edges and requires decoding colours.
- The green density is a continuous, Gaussian-smoothed field. Tracing its iso-lines at the thresholds gives smooth sub-pixel boundaries. Neighbouring bands share exactly the same iso-line, so there are no gaps or overlaps between 406, 408 and 410.
- Open land is the complement of the discs Cassini draws around high-vegetation cells. As a vector, that is the iso-line at about 2.1 m of a distance-to-high-vegetation field.

To make vectorization a separate stage (for example a `cassini vectorize` command), persist these fields as small 1 m GeoTIFFs next to the existing `*_with_buffer.tif` files.

### Prototype

A throwaway prototype (outside the repository) re-implemented Cassini's classification for the `out-test` tile: E 907 000–908 000, N 6 454 000–6 455 000. That tile was rendered on 2026-09-08 with green thresholds 1/2/3, before `39dfc8d` changed the defaults to 2/4/6. The prototype then:

1. **Generalised the field before tracing.** For each threshold, it labelled connected components. Islands smaller than the ISOM minimum area were lowered just below the threshold, which merges them into the surrounding class. Enclosed holes smaller than the minimum were raised just above it. Doing this on the field keeps the topology consistent, which deleting polygons after tracing would not. Minimum areas used, at 1:15 000: 410 = 64 m², 408 = 110 m², 406 = 225 m², 403 = 225 m² (forest openings in yellow: 110 m²).
2. **Traced isobands** with the [`contour`](https://crates.io/crates/contour) crate (0.13, MIT/Apache, marching squares with linear interpolation, georeferenced origin and step), which returns `geo-types` polygons with holes.
3. **Simplified** with Ramer–Douglas–Peucker at 0.5 m, using the [`geo`](https://crates.io/crates/geo) crate.

Results for the tile, with thresholds 1/2/3 to match the rendered raster. Each cell is polygons / holes / vertices:

| Symbol | Raw isobands | Generalised | Generalised + simplified |
| --- | --- | --- | --- |
| 403 Rough open land | 3 861 / 101 / 68 542 | 36 / 6 / 19 536 | 36 / 6 / 4 679 |
| 406 Vegetation: slow | 4 281 / 9 480 / 427 545 | 88 / 308 / 183 275 | 88 / 308 / 47 078 |
| 408 Vegetation: walk | 5 296 / 8 328 / 383 294 | 208 / 472 / 175 182 | 208 / 472 / 45 318 |
| 410 Vegetation: fight | 6 312 / 383 / 160 611 | 404 / 11 / 68 949 | 404 / 11 / 16 844 |

- **Volume.** About 740 area objects and 114 k vertices per km². This is far below OCAD's limits (16 M objects, 4 M vertices per object, 2 GB per file). A 100 km² batch would hold about 74 k objects and about 90 MB of coordinates. Bézier fitting (OCAD supports Bézier control-point flags) or a larger tolerance would shrink that further and make the polygons easier to edit.
- **Speed.** Classification took 0.15 s, generalisation plus tracing about 1 s, and simplification 0.05 s per km².
- **Generalise before tracing.** On the raw, speckled field, tracing took **48 s** (14 s with thresholds 2/4/6) because `contour` assigns holes to rings in quadratic time.
- **Fidelity.** The spatial pattern matches Cassini's `vegetation.png`, minus the single-cell speckle that ISOM's minimum areas exclude anyway.
- **Current defaults.** With thresholds 2/4/6, the minimum-area rule removes about 80 % of the 408 area (44 777 → 8 976 m²). Those 408 patches are mostly too small to map. Expect the vector map to look less green than today's raster.

Not done in the prototype:
- **Minimum width** (0.25–0.4 mm on paper, 3.75–6 m on the ground) needs a morphological opening per threshold, also applied on the field.
- **Undergrowth** (mode 409) is a separate iso-line of `low_density`, producing 409 objects on top of 403 or 405. ISOM's screen-combination table forbids 409 on top of 406, 408 or 410.
- **Tile-edge artifact.** Tiles without neighbours get a dark-green frame along the outer edge, visible in Cassini's raster too. As a vector it becomes a 410 ring object, so the outer boundary needs masking.

## Tile seams in batch mode

Per-tile vectors clipped at tile edges split every feature that crosses a seam. In OCAD, mappers would see straight cuts in areas and lines every kilometre.

- **Vegetation.** Vectorize in the batch merge step, on a mosaic of the per-tile fields. Each tile's interior field depends only on data within a few metres (blur radius ≤ 4 m, buffer 200 m), so the mosaic matches a single whole-area computation. A 1 m f32 field is 4 MB/km². Process large batches in overlapping strips.
- **Contours and form lines.** Clip each tile's lines at the tile edge, then join their ends across seams within a small tolerance. Both tiles derive them from the same DEM, so their ends should nearly coincide.
- **OSM features.** These are downloaded per tile with overlap. De-duplicate by OSM id, or clip and join.

## Cliffs

Cliffs are the weakest layer for vector output. Today they are dots wherever slope exceeds the threshold. ISOM 201 and 202 are lines whose tags point downhill, and OCAD draws the tags on one side of the line, so the line's direction matters. Options:

1. Polygonize the steep-cell mask, extract centrelines, and orient each one using the DEM gradient so the tags fall downhill. This gives the best result and is the most work.
2. Join short segments laid along the contour direction for each steep cell, as Karttapullautin does. This is simpler but noisier.
3. Leave cliffs out of the first version. Mappers can trace them over Cassini's raster, which OCAD can load as a georeferenced background map from `full-map.png` and `full-map.pgw`.

## Required architecture change

`vectors.rs` classifies each OSM record and calls a `MapRenderer` drawing method immediately. Contours are drawn as they are culled, and vegetation and cliffs go straight to bitmaps. OCD export needs one intermediate model:

```rust
pub struct MapFeature {
    pub symbol: IsomSymbol,      // e.g. Symbol::RoughOpenLand403, mapped to an OCD symbol number
    pub geometry: FeatureGeometry, // Point / Line(LineString) / Area(Polygon with holes), in projected metres
}
```

Each stage produces `Vec<MapFeature>` per tile. The Skia renderer draws them, and the OCD writer serializes them. The vegetation and cliff rasters stay as they are for PNG output. The vectorizer is a second consumer of their fields.

## Work breakdown

| # | Item | Size | Notes |
| --- | --- | --- | --- |
| 1 | OCD v12 writer: header, index chains, objects, strings, coordinates, template copy, 1039 offset | M | Port the relevant parts of OOM's `ocd_types_v12.h` and `ocd_file_export.cpp`, keeping attribution. About 600–900 lines of Rust. |
| 2 | ISOM template file, symbol-number mapping and start-up check | S | Built once with OOM, checked in OCAD |
| 3 | `MapFeature` model; split `vectors.rs` and `map_renderer.rs` into classification and drawing | M | Also covers architecture report §6 |
| 4 | Contours, form lines and OSM features as features (101/102/103, 3xx, 5xx) | S | Geometry already exists |
| 5 | Vegetation vectorization: field generalisation, minimum area and width, isobands, simplification | M | Prototype shows the approach works |
| 6 | Batch: mosaic vectorization, joining contours across seams, one merged `.ocd` | M | |
| 7 | Cliffs as oriented lines | M–L | Can be deferred |
| 8 | Tests: byte-layout unit tests, a round-trip reader, a golden file, and a manual check that OCAD opens the file | S–M | Opening in OCAD cannot be automated in CI |

CLI: add an OCD output flag to `process`, `render` and `batch` (for example `--ocad`), plus `--ocad-template <file>`. Batch mode writes one `merged-map.ocd` next to the merged PNG chunks.

## Milestones

1. **OCD round trip with existing vectors.** Template plus writer, and one tile's contours, form lines and OSM features in an `.ocd` file that OCAD opens with correct symbols and georeferencing. This settles the format risk with the least new code.
2. **Vegetation polygons** for a single tile, with ISOM minimum areas and widths.
3. **Batch output**: a single seamless `.ocd` file for the whole input directory.
4. **Cliffs** as lines.

An optional extra milestone is to emit the same features as shapefiles or GeoPackage with an `isom` attribute. This costs little once item 3 exists, helps debugging in QGIS, and suits OOM and CRT-based import workflows.

## Risks

- **OCAD acceptance.** No documented minimal record set exists. Copying records from a real file (the template) and following OOM's proven writer reduce the risk. A licensed OCAD installation is needed for acceptance testing.
- **Format drift.** OCAD calls the format internal and subject to change, and v12 is no longer the newest version. Check that current OCAD releases open v12 files before investing beyond milestone 1.
- **Template fidelity.** OOM's OCD export of its ISOM set is lossy in a few places. Inspect and fix the template in OCAD once.
- **Editing cost.** Auto-generated vegetation is a base map that mappers edit. Noisy or vertex-heavy polygons cost them time. Tune simplification, minimum width and Bézier fitting with a mapper's feedback.
- **CRS.** Only Lambert-93 is supported today. Template-driven coordinate-system records keep this out of the code.
