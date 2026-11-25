"use strict";

const TOKEN = new URLSearchParams(location.search).get("token");
const BASE_TILES = "https://data.geopf.fr/wmts?SERVICE=WMTS&VERSION=1.0.0&REQUEST=GetTile" +
  "&LAYER=GEOGRAPHICALGRIDSYSTEMS.PLANIGNV2&STYLE=normal&FORMAT=image/png" +
  "&TILEMATRIXSET=PM_0_19&TILEMATRIX={z}&TILECOL={x}&TILEROW={y}";
const LIDAR_TILES = "https://data.geopf.fr/tms/1.0.0/IGNF_LIDAR-HD_METADONNEE/{z}/{x}/{y}.pbf";
// Above this many tiles, launching asks for a confirmation.
const CONFIRM_ABOVE_TILES = 20;
// LiDAR HD's COPC files take about 6.5 bytes per point, and a DEM tile 16 MB.
const BYTES_PER_POINT = 6.5;
const DEM_BYTES = 16e6;
const MAX_LOG_LINES = 2000;
// Bound decoded images and GPU textures even when viewing a large area.
const MAX_RESULT_IMAGES = 16;
const OPTIONS_STORAGE_KEY = "cassini-ui-options";

const OPTION_FIELDS = [
  { key: "threads", label: "Threads", type: "number", min: 1, step: 1, help: "Tiles processed in parallel" },
  { key: "vegetation_layers", label: "Vegetation layers", type: "select", help: "How vegetation points are sorted into layers" },
  { key: "undergrowth", label: "Undergrowth", type: "select", help: "Undergrowth rendering mode" },
  { key: "skip_vector", label: "Skip vector processing (no OpenStreetMap data)", type: "checkbox" },
  { key: "skip_520", label: "Don't draw the 520 symbol (area that shall not be entered)", type: "checkbox" },
  { key: "ocad", label: "Export an OCAD 12 file (merged-map.ocd)", type: "checkbox" },
  { key: "ocad_template", label: "OCAD template", type: "text", ocad: true, help: "Path to an OCAD 12 file whose symbols to use, empty for ISOM 2017-2" },
  { key: "ocad_simplify", label: "OCAD simplification tolerance (m)", type: "number", min: 0, step: 0.1, ocad: true },
];

const MODE_HINTS = {
  click: "Click a tile to add it to the selection, or to remove it.",
  rectangle: "Drag a rectangle to add its tiles. Hold Shift to remove them instead.",
  polygon: "Click to add points, double-click or click the first point to finish, Escape to cancel. Hold Shift when finishing to remove the tiles.",
};

const $ = (id) => document.getElementById(id);

/** The tiles of the selection, by id. */
const selected = new Map();
let appState = null;
let mode = "click";
let logIndex = 0;
let pollTimer = null;
let resultTiles = [];
const activeResults = new Map(); // Loaded result names and image URLs.
let mapReady = false;

// ---------------------------------------------------------------------------
// API

async function api(method, path, body) {
  const headers = { "X-Cassini-Token": TOKEN };
  if (body !== undefined) headers["Content-Type"] = "application/json";
  const response = await fetch(path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const data = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(data.error || `HTTP ${response.status}`);
  return data;
}

function showMessage(text) {
  $("message").textContent = text || "";
  $("message").hidden = !text;
}

// ---------------------------------------------------------------------------
// Map

const map = new maplibregl.Map({
  container: "map",
  // Load the style locally so a failed tile request cannot prevent the UI
  // sources from being created. IGN serves both the basemap and LiDAR index.
  style: {
    version: 8,
    sources: {},
    layers: [{ id: "background", type: "background", paint: { "background-color": "#f2f1ed" } }],
  },
  center: [2.6, 46.6],
  zoom: 5.5,
});
map.addControl(new maplibregl.NavigationControl({ showCompass: false }), "top-right");
map.addControl(new maplibregl.ScaleControl(), "bottom-left");

const emptyCollection = () => ({ type: "FeatureCollection", features: [] });

const whenMapReady = new Promise((resolve) => map.once("style.load", () => {
  map.addSource("basemap", {
    type: "raster",
    tiles: [BASE_TILES],
    tileSize: 256,
    maxzoom: 19,
    attribution: '<a href="https://www.ign.fr/">Plan IGN © IGN</a>',
  });
  map.addLayer({ id: "basemap", type: "raster", source: "basemap" });

  map.addSource("lidar", {
    type: "vector",
    tiles: [LIDAR_TILES],
    minzoom: 6,
    maxzoom: 17,
    attribution: "LiDAR HD © IGN",
  });
  map.addLayer({
    id: "lidar-fill",
    type: "fill",
    source: "lidar",
    "source-layer": "metadata",
    paint: { "fill-color": "#6a6af4", "fill-opacity": 0.12 },
  });
  map.addLayer({
    id: "lidar-line",
    type: "line",
    source: "lidar",
    "source-layer": "metadata",
    minzoom: 8,
    paint: { "line-color": "#4b4bb3", "line-width": 0.5, "line-opacity": 0.6 },
  });

  map.addSource("selection", { type: "geojson", data: emptyCollection() });
  map.addLayer({
    id: "selection-fill",
    type: "fill",
    source: "selection",
    paint: { "fill-color": "#000091", "fill-opacity": 0.3 },
  });
  map.addLayer({
    id: "selection-line",
    type: "line",
    source: "selection",
    paint: { "line-color": "#000091", "line-width": 1.5 },
  });

  map.addSource("drawing", { type: "geojson", data: emptyCollection() });
  map.addLayer({
    id: "drawing-fill",
    type: "fill",
    source: "drawing",
    filter: ["==", ["geometry-type"], "Polygon"],
    paint: { "fill-color": "#e1000f", "fill-opacity": 0.12 },
  });
  map.addLayer({
    id: "drawing-line",
    type: "line",
    source: "drawing",
    filter: ["!=", ["geometry-type"], "Point"],
    paint: { "line-color": "#e1000f", "line-width": 2, "line-dasharray": [2, 1] },
  });
  map.addLayer({
    id: "drawing-points",
    type: "circle",
    source: "drawing",
    filter: ["==", ["geometry-type"], "Point"],
    paint: { "circle-radius": 4, "circle-color": "#fff", "circle-stroke-color": "#e1000f", "circle-stroke-width": 2 },
  });

  mapReady = true;
  updateControls();
  renderSelection();
  resolve();
}));

map.on("error", (event) => {
  $("map-error").hidden = false;
  $("map-error").textContent = `Could not load map data. Check your connection or content blocker. ${event.error?.message || ""}`;
});

function tilePolygon(corners) {
  return { type: "Polygon", coordinates: [[...corners, corners[0]]] };
}

function renderSelection() {
  const tiles = [...selected.values()].sort((a, b) => a.id.localeCompare(b.id));
  if (map.getSource("selection")) {
    map.getSource("selection").setData({
      type: "FeatureCollection",
      features: tiles.map((tile) => ({ type: "Feature", properties: { id: tile.id }, geometry: tilePolygon(tile.corners) })),
    });
  }

  const list = $("selection-list");
  list.replaceChildren(
    ...tiles.map((tile) => {
      const item = document.createElement("li");
      const label = document.createElement("div");
      label.textContent = tile.id.replace("_", " – ") + " ";
      const detail = document.createElement("span");
      detail.textContent = tile.downloaded ? "downloaded" : formatBytes(tileBytes(tile));
      label.append(detail);
      const remove = document.createElement("button");
      remove.type = "button";
      remove.textContent = "×";
      remove.title = `Remove tile ${tile.id}`;
      remove.addEventListener("click", () => {
        selected.delete(tile.id);
        renderSelection();
      });
      item.append(label, remove);
      return item;
    }),
  );

  const toDownload = tiles.filter((tile) => !tile.downloaded);
  const bytes = toDownload.reduce((sum, tile) => sum + tileBytes(tile), 0);
  $("selection-summary").textContent =
    tiles.length === 0
      ? "No tile selected."
      : `${plural(tiles.length, "tile")} selected` +
        (toDownload.length < tiles.length ? `, ${tiles.length - toDownload.length} already downloaded` : "") +
        (toDownload.length > 0 ? `. About ${formatBytes(bytes)} to download.` : ".");
  $("clear-selection").hidden = tiles.length === 0;
  updateLaunchButton();
}

function tileBytes(tile) {
  return tile.points * BYTES_PER_POINT + DEM_BYTES;
}

function formatBytes(bytes) {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  return `${Math.round(bytes / 1e6)} MB`;
}

function plural(count, word) {
  return `${count} ${word}${count === 1 ? "" : "s"}`;
}

/** Adds the tiles in the geometry to the selection, or removes them. */
async function selectIn(geometry, remove) {
  showMessage("");
  try {
    const tiles = await api("POST", "/api/tiles", { geometry });
    for (const tile of tiles) {
      if (remove) selected.delete(tile.id);
      else selected.set(tile.id, tile);
    }
    if (tiles.length === 0) showMessage("No LiDAR HD tile there.");
    renderSelection();
  } catch (error) {
    showMessage(`Could not get the tiles: ${error.message}`);
  }
}

async function toggleTileAt(lngLat) {
  showMessage("");
  try {
    const [tile] = await api("POST", "/api/tiles", { geometry: { type: "Point", coordinates: [lngLat.lng, lngLat.lat] } });
    if (!tile) return showMessage("No LiDAR HD tile there.");
    if (selected.has(tile.id)) selected.delete(tile.id);
    else selected.set(tile.id, tile);
    renderSelection();
  } catch (error) {
    showMessage(`Could not get the tile: ${error.message}`);
  }
}

// ---------------------------------------------------------------------------
// Drawing

let drag = null; // The screen point where a rectangle started.
let vertices = []; // The polygon vertices, in longitudes and latitudes.

function setMode(next) {
  mode = next;
  cancelDrawing();
  for (const button of document.querySelectorAll(".modes button")) {
    button.classList.toggle("active", button.dataset.mode === mode);
  }
  $("mode-hint").textContent = MODE_HINTS[mode];
  $("map").classList.toggle("drawing", mode !== "click");
  if (mode === "rectangle") map.dragPan.disable();
  else map.dragPan.enable();
  if (mode === "click") {
    map.boxZoom.enable();
    map.doubleClickZoom.enable();
  } else {
    map.boxZoom.disable();
    map.doubleClickZoom.disable();
  }
}

function cancelDrawing() {
  drag = null;
  vertices = [];
  map.getSource("drawing")?.setData(emptyCollection());
}

function coordinatesOf(points) {
  return points.map((point) => map.unproject(point).toArray());
}

/** Close a ring of longitude/latitude coordinates. */
function ringOf(points) {
  return [...points, points[0]];
}

function drawShape(points, cursor) {
  const all = cursor ? [...points, cursor] : points;
  const features = points.map((point) => ({
    type: "Feature",
    properties: {},
    geometry: { type: "Point", coordinates: point },
  }));
  if (all.length >= 3) {
    features.push({ type: "Feature", properties: {}, geometry: { type: "Polygon", coordinates: [ringOf(all)] } });
  } else if (all.length === 2) {
    features.push({
      type: "Feature",
      properties: {},
      geometry: { type: "LineString", coordinates: all },
    });
  }
  map.getSource("drawing")?.setData({ type: "FeatureCollection", features });
}

function rectanglePoints(a, b) {
  return [a, { x: b.x, y: a.y }, b, { x: a.x, y: b.y }];
}

map.on("mousedown", (event) => {
  if (!mapReady || !appState || mode !== "rectangle" || event.originalEvent.button !== 0) return;
  drag = event.point;
});

map.on("mousemove", (event) => {
  if (mode === "rectangle" && drag) drawShape(coordinatesOf(rectanglePoints(drag, event.point)));
  if (mode === "polygon" && vertices.length > 0) drawShape(vertices, map.unproject(event.point).toArray());
});

map.on("mouseup", (event) => {
  if (mode !== "rectangle" || !drag) return;
  const start = drag;
  drag = null;
  map.getSource("drawing")?.setData(emptyCollection());
  if (Math.abs(start.x - event.point.x) < 3 || Math.abs(start.y - event.point.y) < 3) return;
  selectIn({ type: "Polygon", coordinates: [ringOf(coordinatesOf(rectanglePoints(start, event.point)))] }, event.originalEvent.shiftKey);
});

map.on("click", (event) => {
  if (!mapReady || !appState) return;
  if (mode === "click") return toggleTileAt(event.lngLat);
  if (mode !== "polygon") return;
  const point = event.point;
  if (vertices.length >= 3 && point.dist(map.project(vertices[0])) < 10) {
    return finishPolygon(event.originalEvent.shiftKey);
  }
  // A double click sends two clicks on the same point first.
  if (vertices.length > 0 && point.dist(map.project(vertices[vertices.length - 1])) < 3) return;
  vertices.push(map.unproject(point).toArray());
  drawShape(vertices);
});

map.on("dblclick", (event) => {
  if (mode !== "polygon") return;
  event.preventDefault();
  finishPolygon(event.originalEvent.shiftKey);
});

function finishPolygon(remove) {
  const points = vertices;
  cancelDrawing();
  if (points.length < 3) return;
  selectIn({ type: "Polygon", coordinates: [ringOf(points)] }, remove);
}

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") cancelDrawing();
});

for (const button of document.querySelectorAll(".modes button")) {
  button.addEventListener("click", () => setMode(button.dataset.mode));
}

$("clear-selection").addEventListener("click", () => {
  selected.clear();
  renderSelection();
});

// ---------------------------------------------------------------------------
// Options and configuration

function field(label, input, help) {
  const wrapper = document.createElement("label");
  if (input.type === "checkbox") {
    wrapper.className = "inline";
    wrapper.append(input, label);
  } else {
    wrapper.append(label, input);
  }
  if (help) {
    const small = document.createElement("small");
    small.textContent = help;
    wrapper.append(small);
  }
  return wrapper;
}

function buildOptionsForm(options, choices) {
  const form = $("options-form");
  form.replaceChildren();
  for (const spec of OPTION_FIELDS) {
    let input;
    if (spec.type === "select") {
      input = document.createElement("select");
      for (const choice of choices[spec.key]) {
        const option = document.createElement("option");
        option.value = option.textContent = choice;
        input.append(option);
      }
      input.value = options[spec.key];
    } else {
      input = document.createElement("input");
      input.type = spec.type;
      if (spec.min !== undefined) input.min = spec.min;
      if (spec.step !== undefined) input.step = spec.step;
      if (spec.type === "checkbox") input.checked = Boolean(options[spec.key]);
      else input.value = options[spec.key] ?? "";
    }
    input.name = spec.key;
    const wrapper = field(spec.label, input, spec.help);
    if (spec.ocad) wrapper.dataset.ocad = "";
    form.append(wrapper);
  }
  form.elements.ocad.addEventListener("change", updateOcadFields);
  updateOcadFields();
}

function updateOcadFields() {
  const ocad = $("options-form").elements.ocad.checked;
  for (const wrapper of $("options-form").querySelectorAll("[data-ocad]")) wrapper.hidden = !ocad;
}

function readOptions() {
  const elements = $("options-form").elements;
  const options = {};
  for (const spec of OPTION_FIELDS) {
    const input = elements[spec.key];
    if (spec.type === "checkbox") options[spec.key] = input.checked;
    else if (spec.type === "number") options[spec.key] = Number(input.value);
    else options[spec.key] = input.value;
  }
  options.ocad_template = options.ocad_template.trim() || null;
  return options;
}

function prettify(key) {
  const words = key.replaceAll("_", " ");
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function buildConfigForm(config) {
  const form = $("config-form");
  form.replaceChildren(
    ...Object.entries(config).map(([key, value]) => {
      const input = document.createElement("input");
      input.type = "number";
      input.step = "any";
      input.name = key;
      input.value = value;
      return field(prettify(key), input);
    }),
  );
}

function readConfig() {
  const config = {};
  for (const input of $("config-form").elements) config[input.name] = Number(input.value);
  return config;
}

function loadStoredOptions(defaults) {
  try {
    const stored = JSON.parse(localStorage.getItem(OPTIONS_STORAGE_KEY) || "{}");
    return { ...defaults, ...stored };
  } catch {
    return defaults;
  }
}

function storeOptions(options) {
  try {
    localStorage.setItem(OPTIONS_STORAGE_KEY, JSON.stringify(options));
  } catch {
    // The options are only remembered for convenience.
  }
}

$("reset-config").addEventListener("click", () => {
  if (appState) buildConfigForm(appState.default_config);
});

function updateControls() {
  $("reset-config").disabled = !appState;
  for (const button of document.querySelectorAll(".modes button")) {
    button.disabled = !appState || !mapReady;
  }
  updateLaunchButton();
}

// ---------------------------------------------------------------------------
// Job

function updateLaunchButton(running = isRunning()) {
  $("launch").disabled = !appState || running || selected.size === 0;
}

let lastStatus = null;
const isRunning = () => lastStatus === "downloading" || lastStatus === "processing";

$("launch").addEventListener("click", async () => {
  if (!appState || isRunning() || selected.size === 0) return;
  showMessage("");
  const tiles = [...selected.values()];
  const selectedFiles = new Set(tiles.map((tile) => tile.laz_file_name));
  const others = appState.input_files.filter((name) => !selectedFiles.has(name));
  const toDownload = tiles.filter((tile) => !tile.downloaded);
  const bytes = toDownload.reduce((sum, tile) => sum + tileBytes(tile), 0);

  if (tiles.length > CONFIRM_ABOVE_TILES || others.length > 0) {
    let question = `Process ${plural(tiles.length, "tile")}`;
    if (toDownload.length > 0) {
      question += ` and download ${plural(toDownload.length, "tile")} (about ${formatBytes(bytes)}) into "${appState.input_dir}"`;
    }
    question += "?";
    if (others.length > 0) {
      question += `\n\n"${appState.input_dir}" also holds ${plural(others.length, "other LiDAR file")}, which will be processed too.`;
    }
    if (!confirm(question)) return;
  }

  const options = readOptions();
  try {
    await api("POST", "/api/jobs", { tiles: tiles.map((tile) => tile.id), config: readConfig(), options });
  } catch (error) {
    return showMessage(error.message);
  }
  storeOptions(options);
  logIndex = 0;
  $("logs").textContent = "";
  lastStatus = "downloading";
  updateLaunchButton();
  poll();
});

$("cancel").addEventListener("click", async () => {
  $("cancel").disabled = true;
  try {
    await api("POST", "/api/job/cancel");
  } catch (error) {
    showMessage(error.message);
  }
});

async function poll() {
  clearTimeout(pollTimer);
  let job;
  try {
    job = await api("GET", `/api/job?since=${logIndex}`);
  } catch (error) {
    showMessage(`Lost contact with cassini ui: ${error.message}`);
    pollTimer = setTimeout(poll, 3000);
    return;
  }
  if (!job) return;

  const wasRunning = isRunning();
  renderJob(job);
  if (isRunning()) {
    pollTimer = setTimeout(poll, 1000);
  } else if (wasRunning) {
    // Tiles just downloaded aren't to download any more.
    await refreshState();
    for (const tile of selected.values()) {
      if (appState.input_files.includes(tile.laz_file_name)) tile.downloaded = true;
    }
    renderSelection();
    if (job.status === "done") await loadResults(job.tiles);
  }
}

function renderJob(job) {
  lastStatus = job.status;
  $("job").hidden = false;
  const badge = $("job-status");
  badge.textContent = job.status.replace("_", " ");
  badge.className = `badge ${job.status}`;

  const running = isRunning();
  $("cancel").hidden = !running;
  $("cancel").disabled = false;
  updateLaunchButton(running);

  const download = job.download;
  $("download").hidden = !download;
  if (download) {
    const fraction = download.total ? download.bytes / download.total : 0;
    $("download").querySelector(".bar div").style.width = `${(fraction * 100).toFixed(1)}%`;
    $("download").querySelector("p").textContent =
      `File ${download.index} of ${download.count}: ${download.file_name}, ` +
      `${formatBytes(download.bytes)}${download.total ? ` of ${formatBytes(download.total)}` : ""}`;
  }

  if (job.logs.length > 0) {
    const logs = $("logs");
    const atBottom = logs.scrollTop + logs.clientHeight >= logs.scrollHeight - 4;
    const lines = (logs.textContent ? logs.textContent.split("\n") : []).concat(job.logs);
    logs.textContent = lines.slice(-MAX_LOG_LINES).join("\n");
    if (atBottom) logs.scrollTop = logs.scrollHeight;
  }
  logIndex = job.next_log_index;
  if (job.error) showMessage(job.error);
}

// ---------------------------------------------------------------------------
// Results: one image per 1 km tile, as a whole map could exceed the largest
// texture WebGL allows.

async function loadResults(fitTileIds) {
  await whenMapReady;
  let results;
  try {
    results = await api("GET", "/api/results");
  } catch (error) {
    return showMessage(`Could not list the generated maps: ${error.message}`);
  }

  resultTiles = results;
  $("results").hidden = results.length === 0;
  if (fitTileIds) fitTo(fitTileIds);
  syncResults();
}

/** Keep only nearby, visible images in memory; metadata remains available. */
function syncResults() {
  if (!mapReady) return;
  const bounds = map.getBounds();
  const center = map.getCenter();
  const longitudeScale = Math.cos(center.lat * Math.PI / 180);
  const distance = (result) => {
    const lng = result.corners.reduce((sum, corner) => sum + corner[0], 0) / 4;
    const lat = result.corners.reduce((sum, corner) => sum + corner[1], 0) / 4;
    return ((lng - center.lng) * longitudeScale) ** 2 + (lat - center.lat) ** 2;
  };
  const visible = $("show-results").checked ? resultTiles.filter((result) => {
    const lngs = result.corners.map((corner) => corner[0]);
    const lats = result.corners.map((corner) => corner[1]);
    return Math.max(...lngs) >= bounds.getWest() && Math.min(...lngs) <= bounds.getEast() &&
      Math.max(...lats) >= bounds.getSouth() && Math.min(...lats) <= bounds.getNorth();
  }).sort((a, b) => distance(a) - distance(b)) : [];
  const wanted = new Set(visible.slice(0, MAX_RESULT_IMAGES).map((result) => result.name));
  for (const name of activeResults.keys()) {
    if (!wanted.has(name)) {
      map.removeLayer(`result-${name}`);
      map.removeSource(`result-${name}`);
      activeResults.delete(name);
    }
  }
  const opacity = Number($("results-opacity").value);
  for (const result of visible.slice(0, MAX_RESULT_IMAGES)) {
    const id = `result-${result.name}`;
    const source = map.getSource(id);
    if (source) {
      if (activeResults.get(result.name) !== result.url) {
        source.updateImage({ url: result.url, coordinates: result.corners });
      }
    } else {
      map.addSource(id, { type: "image", url: result.url, coordinates: result.corners });
      map.addLayer(
        { id, type: "raster", source: id, paint: { "raster-opacity": opacity, "raster-fade-duration": 0 } },
        "selection-fill",
      );
    }
    activeResults.set(result.name, result.url);
  }
  $("results-summary").textContent = `${plural(resultTiles.length, "tile")} in "${appState.output_dir}".` +
    (visible.length > MAX_RESULT_IMAGES ? ` Showing the closest ${MAX_RESULT_IMAGES} of ${visible.length} visible tiles. Zoom in to see the others.` : "");
}

function fitTo(tileIds) {
  const corners = tileIds.flatMap((id) => selected.get(id)?.corners ?? []);
  if (corners.length === 0) return;
  const bounds = corners.reduce((bounds, corner) => bounds.extend(corner), new maplibregl.LngLatBounds(corners[0], corners[0]));
  map.fitBounds(bounds, { padding: 40, maxZoom: 15 });
}

map.on("moveend", syncResults);
$("show-results").addEventListener("change", syncResults);

$("results-opacity").addEventListener("input", () => {
  for (const name of activeResults.keys()) map.setPaintProperty(`result-${name}`, "raster-opacity", Number($("results-opacity").value));
});

// ---------------------------------------------------------------------------
// Start

async function refreshState() {
  appState = await api("GET", "/api/state");
}

async function init() {
  setMode("click");
  renderSelection();
  if (!TOKEN) return showMessage("This page needs the URL cassini ui printed, with its token.");
  try {
    await refreshState();
  } catch (error) {
    return showMessage(`Could not reach cassini ui: ${error.message}`);
  }
  buildOptionsForm(loadStoredOptions(appState.options), appState.choices);
  buildConfigForm(appState.config);
  $("config-error").hidden = !appState.config_error;
  $("config-error").textContent = appState.config_error
    ? `config.json could not be read, so the defaults are shown: ${appState.config_error}`
    : "";
  updateControls();
  await Promise.all([poll(), whenMapReady.then(() => loadResults())]);
}

init().catch((error) => showMessage(`Could not initialize cassini ui: ${error.message}`));
