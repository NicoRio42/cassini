FROM pdal/pdal
WORKDIR /app
COPY ./target/release/cassini /app
RUN pip install GDAL=="3.9.2"
ENTRYPOINT ["./cassini"]
#docker run -it -v "$(pwd)":/app/out test ./out/tile.laz --skip-vector