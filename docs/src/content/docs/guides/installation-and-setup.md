---
title: Installation and Setup
description: A guide to generate a map with Cassini.
---

This document will guide you to install Cassini and all its dependencies on your machine.

## Installing PDAL and GDAL

Cassini uses the [PDAL](https://pdal.io) and the [GDAL](https://gdal.org) libraries to process LiDAR and shapefiles data. To use Cassini, you first need to install them on your machine.

The easiest way to install PDAL and GDAL is with Miniconda. Follow [this link](https://docs.anaconda.com/miniconda/#quick-command-line-install) and copy past the commmand line instruction to a terminal to install Miniconda.

After following these instructions, the `conda` command should be available in you terminal. To check that everything worked:

```sh
conda --version
```

This should print the version of the conda program.

Then create a new miniconda environment named `cassini` with pdal and gdal installed:

```sh
conda create --yes --name cassini --channel conda-forge pdal gdal
```

Everytime you will open a new terminal and want to use Cassini, you will have to activate this environment:

```sh
conda activate cassini
```

## Downloading Cassini executable

Go to the [Latest release page on Github](https://github.com/NicoRio42/cassini/releases/latest). Then download the tar file corresponding to your operating system. Decompress it and copy the cassini executable where you want to use it on your machine.

:::caution
Cassini is only available on Linux at the moment.
:::

If you want to make the `cassini` command globally available on you machine, you can follow [this tutorial](https://zwbetz.com/how-to-add-a-binary-to-your-path-on-macos-linux-windows/).
