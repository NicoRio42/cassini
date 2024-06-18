# Orienteering map generator

```sh
conda activate lidar &&
rm -rf out &&
mkdir out &&
pdal pipeline ./pipeline.json &&
gdal_contour -a elev out/dem.tif out/contours.shp -i 2.5 &&
gdaldem slope out/dem.tif out/slopes.tif
```

## Sources

https://tmsw.no/mapping/basemap_generation.html
https://geoservices.ign.fr/sites/default/files/2022-05/DT_LiDAR_HD_1-0.pdf
