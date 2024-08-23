---
title: Process a single LiDAR file
description: A guide to generate a map with Cassini.
---

The simplest way to use cassini is to pass a single LiDAR file path to the `cassini` command:

```sh
cassini ./path/to/my/tile.laz
```

It will generate a png map in the `out/tile` directory.

If you are not happy with the result (too few cliffs, too much green...), you can modify the configuration and re-generate the map while skipping the LiDAR preprocessing part (which is the most time consuming part) with the `--skip-lidar` flag:

```sh
cassini ./path/to/my/tile.laz --skip-lidar
```
