# Orienteering map generator

```sh
conda activate lidar &&
rm -rf out &&
mkdir out &&
pdal pipeline ./pipeline.json &&
gdal_contour -a elev out/dem.tif out/contours.shp -i 2.5 &&
gdaldem slope out/dem.tif out/slopes.tif
```

Creating contours with gdal_rasterize

```sh
gdal_rasterize -burn 255 -burn 0 -burn 0 -at -ts 1024 1024 -tr 1 1 out/contours.shp out/contours.tif
```

Translate tif to png

```sh
gdal_translate -of PNG -scale -ot byte out/dem.tif out/dem.png
```

Translate osm to shapefile

```sh
ogr2ogr --config OSM_USE_CUSTOM_INDEXING NO -skipfailures -f "ESRI Shapefile" out/map.shp in/map.osm
```

Create tile with buffer with las2las

```sh
las2las64 -lof file_list.txt -merged -o tile-with-buffer.laz -keep_xy 615800 6162800 617200 6164200
```

## Sources

https://tmsw.no/mapping/basemap_generation.html
https://geoservices.ign.fr/sites/default/files/2022-05/DT_LiDAR_HD_1-0.pdf
https://github.com/mapbox/vector-tile-spec/tree/master/2.1

## TODO

- Batch mode
- Vector data
- Smooth contours
- Formline algorithme

## Mapant batch mode with downloading

- Input is an extent
- Tiles are processed one by one, laz files are downloaded when needed (as the downloading speed is the bottleneck, no need to process in parralel)
- When starting processing a tile, we check if all surounded tiles are downloadded
- Downloading should be anticipated so it happens during previous tile processing
- tile is resized to add a buffer
- Outputs are croped in the end
- Output are uploaded to mapant server

## Classic batch mode

- All tiles are present in the "in" folder.
- There is a tiling step at the begining of the PDAL pipeline to add a buffer to all tiles
- Tiles should be processed in parallel
- Outputs should be croped in the end
