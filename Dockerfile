FROM pdal/pdal
WORKDIR /app
COPY ./target/x86_64-unknown-linux-gnu/release/cassini /bin
RUN pip install GDAL==$(gdal-config --version)
ENTRYPOINT ["/bin/cassini"]
#docker run -it -v "$(pwd)":/app test ./tile.laz --skip-vector