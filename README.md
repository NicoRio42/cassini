conda activate lidar
pdal pipeline ./pipeline.json
gdal_contour -a elev dem.tif contour.shp -i 10.0
gdaldem slope dem.tif slopes.tif
