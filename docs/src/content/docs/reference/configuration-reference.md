---
title: Configuration Reference
description: A reference page in my new Starlight docs site.
---

The following reference covers all supported configuration options in Cassini. The configuration should be in a `config.json` file in the directory you are running the `cassini` command. To generate a default configuration file, run:

```sh
cassini --default-config
```

## Yellow threshold

<p>

**Type:** `number`<br />
**Default:** `0.5`

</p>

The number of points below witch a one metter by one metter cell will be yellow (ISOM 403 Rough open land). Otherwise it will be white (405 Forest).

```json
{
  "yellow_threshold": 0.5
}
```

## Green threshold 1

<p>

**Type:** `number`<br />
**Default:** `1.0`

</p>

The number of points above witch a one metter by one metter cell will be light green (ISOM 406 Vegetation, slow running).

```json
{
  "green_threshold_1": 1.0
}
```

## Green threshold 2

<p>

**Type:** `number`<br />
**Default:** `2.0`

</p>

The number of points above witch a one metter by one metter cell will be medium green (ISOM 408 Vegetation, walk).

```json
{
  "green_threshold_1": 2.0
}
```

## Green threshold 3

<p>

**Type:** `number`<br />
**Default:** `3.0`

</p>

The number of points above witch a one metter by one metter cell will be dark green (ISOM 410 Vegetation, fight).

```json
{
  "green_threshold_1": 3.0
}
```

## Cliff threshold 1

<p>

**Type:** `number`<br />
**Default:** `45.0`

</p>

An arbitrary incline value above witch a one metter by one metter cell will be drawn as a small cliff (ISOM 202 Cliff).

```json
{
  "cliff_threshold_1": 45.0
}
```

## Cliff threshold 2

<p>

**Type:** `number`<br />
**Default:** `55.0`

</p>

An arbitrary incline value above witch a one metter by one metter cell will be drawn as a large cliff (ISOM 201 Impassable cliff).

```json
{
  "cliff_threshold_2": 55.0
}
```

## DPI resolution

<p>

**Type:** `number`<br />
**Default:** `600.0`

</p>

The DPI (Dots Per Inche) resolution of the generated png map.

```json
{
  "dpi_resolution": 600.0
}
```
