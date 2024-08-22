---
title: The what and the why
description: What is Cassini and why it does exist.
---

## What is Cassini

Cassini is a software that generates highly accurate topographic maps from [LiDAR](https://en.wikipedia.org/wiki/Lidar) data and shapefiles vector data. The maps produced by Cassini follow the [International Specification for Orienteering Maps (ISOM)](https://orienteering.sport/wp-admin/admin-ajax.php?action=shareonedrive-download&id=663580750D0C0BCE!50104&dl=1&account_id=663580750d0c0bce&drive_id=663580750d0c0bce&listtoken=b03290e8f4203fe6219ea68270f084bc), witch is the most detailed specification for topographic maps.

This project is heavily inspired by [Karttapullautin](https://github.com/rphlo/karttapullautin/tree/master) and [Terje Mathisen's pipeline](https://tmsw.no/mapping/basemap_generation.html). Unlike them, it uses the [PDAL](https://pdal.io) and the [GDAL](https://gdal.org) libraries to preprocess the LiDAR data.

:::caution
Cassini is very early stage and still an experimental project. Use it at your own risks, expect API changes and bugs!
:::

## Why does Cassini exist

Cassini is developped to be the main rendering engine for the [Mapant.fr](https://mapant.fr) project. It consists in generating the most precise topographic map of France out of freely available LiDAR data and shapefiles data. It is inspired by its predecessors:

- [Mapant.fi](https://www.mapant.fi/) for Finland
- [Mapant.no](https://mapant.no/) for Norway
- [Gokartor](https://kartor.gokartor.se/) for Sweden
- [Mapant.es](https://mapant.es/) for Spain
- [Mapant.ch](https://www.mapant.ch/) for Switzerland
- [Mapant.orienteering.lu](https://mapant.orienteering.lu/) for Luxembourg

All of these projects somehow used [Jarkko Ryyppö](https://x.com/RouteGadget)'s [awsome Karttapullautin original Perl program](https://routegadget.net/karttapullautin/) to generate the map (at the exeption of Mapant.ch that used [OCAD](https://www.ocad.com/)). Now that Karttapullautin has been [rewritten in Rust](https://github.com/rphlo/karttapullautin/tree/master) by [Raphaël Stefanini](https://www.linkedin.com/in/rphlo/), the performances are better than ever.

However, there is some reasons that pushed me to develop my own rendering engine for Mapant.fr.

### The point cloud reading bottleneck

A LiDAR file is basically just a list of millions of 3 dimensions points (with some metadata). To process it, a program should at some point loop other all these points, witch is a very time consuming step. Karttapullautin uses the popular `las` Rust library to do so. For some reason (that I ignore), this library performs worst than the C++ equivalent programs (PDAL or LasTools).

### The edges artifacts problem

### The Cassini approach

## Alternatives to Cassini

### Karttapullautin

### Terje Mathisen's pipeline

### OCAD
