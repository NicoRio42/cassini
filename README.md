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

```sh
gdal_translate -of PNG -scale -ot byte out/dem.tif out/dem.png
```

## Sources

https://tmsw.no/mapping/basemap_generation.html
https://geoservices.ign.fr/sites/default/files/2022-05/DT_LiDAR_HD_1-0.pdf
https://github.com/mapbox/vector-tile-spec/tree/master/2.1
