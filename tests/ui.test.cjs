// Run with: node tests/ui.test.cjs
const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const { test } = require("node:test");
const vm = require("node:vm");

const script = readFileSync(new URL("../src/ui/assets/app.js", `file://${__filename}`), "utf8");
const html = readFileSync(new URL("../src/ui/assets/index.html", `file://${__filename}`), "utf8");
const flush = () => new Promise(setImmediate);

class Element {
  constructor() {
    this.children = [];
    this.dataset = {};
    this.listeners = {};
    this.classList = { toggle() {} };
    this.textContent = "";
    this.hidden = true;
    this.disabled = true;
  }
  append(...children) { this.children.push(...children); }
  replaceChildren(...children) { this.children = children; }
  addEventListener(name, callback) { this.listeners[name] = callback; }
  querySelectorAll() { return this.children.filter((child) => child.dataset?.ocad !== undefined); }
  get elements() {
    const inputs = this.children.flatMap((child) => child.children || []).filter((child) => child.name);
    return Object.assign(inputs, Object.fromEntries(inputs.map((input) => [input.name, input])));
  }
}

function start({ stateError = false, token = "test", results = [] } = {}) {
  const elements = Object.fromEntries([...html.matchAll(/id="([^"]+)"/g)].map(([, id]) => [id, new Element()]));
  const modes = ["click", "rectangle", "polygon"].map((mode) => {
    const button = new Element();
    button.dataset.mode = mode;
    return button;
  });
  const calls = [];
  elements["show-results"].checked = true;
  elements["results-opacity"].value = "1";
  const state = {
    config: { dpi_resolution: 600 },
    default_config: { dpi_resolution: 600 },
    options: {},
    choices: { vegetation_layers: ["height"], undergrowth: ["merge"] },
    input_files: [], input_dir: "in", output_dir: "out",
  };
  let map;
  class FakeMap {
    constructor(options) {
      map = this;
      this.options = options;
      this.sources = new Map();
      this.handlers = {};
      this.layers = [];
      this.imageLoads = [];
      this.cameraOffset = 0;
      this.viewport = [-10, -10, 30, 30];
      this.dragPan = this.boxZoom = this.doubleClickZoom = { enable() {}, disable() {} };
    }
    addControl() {}
    on(name, handler) { (this.handlers[name] ||= []).push(handler); }
    once(name, handler) { this.on(name, handler); }
    fire(name, event) { for (const handler of this.handlers[name] || []) handler(event); }
    addSource(id, spec) {
      if (spec.type === "image") this.imageLoads.push(spec.url);
      const map = this;
      this.sources.set(id, { ...spec, setData(data) { this.data = data; }, updateImage(image) {
        map.imageLoads.push(image.url);
        Object.assign(this, image);
      } });
    }
    getSource(id) { return this.sources.get(id); }
    addLayer(layer) { this.layers.push(layer); }
    removeLayer(id) { this.layers = this.layers.filter((layer) => layer.id !== id); }
    removeSource(id) { assert.ok(!this.layers.some((layer) => layer.source === id)); this.sources.delete(id); }
    setPaintProperty(id, key, value) { this.layers.find((layer) => layer.id === id).paint[key] = value; }
    unproject(point) { return { toArray: () => [point.x + this.cameraOffset, point.y + this.cameraOffset] }; }
    project([lng, lat]) { return { x: lng - this.cameraOffset, y: lat - this.cameraOffset }; }
    getBounds() {
      const [west, south, east, north] = this.viewport;
      return { getWest: () => west, getSouth: () => south, getEast: () => east, getNorth: () => north };
    }
    getCenter() {
      const [west, south, east, north] = this.viewport;
      return { lng: (west + east) / 2, lat: (south + north) / 2 };
    }
  }
  const context = vm.createContext({
    URLSearchParams,
    location: { search: token ? `?token=${token}` : "" },
    document: {
      getElementById: (id) => elements[id],
      querySelectorAll: () => modes,
      createElement: () => new Element(),
      addEventListener() {},
    },
    // Point is deliberately absent: MapLibre does not export that constructor.
    maplibregl: { Map: FakeMap, NavigationControl: class {}, ScaleControl: class {} },
    localStorage: { getItem() { return null; } },
    setTimeout, clearTimeout,
    fetch: async (path, options) => {
      calls.push({ path, body: options.body && JSON.parse(options.body) });
      if (stateError && path === "/api/state") throw new Error("unreachable");
      const data = path === "/api/state" ? state : path.startsWith("/api/job?") ? null : path === "/api/results" ? results : [];
      return { ok: true, json: async () => data };
    },
  });
  vm.runInContext(script, context);
  return { map, elements, modes, calls, run: (code) => vm.runInContext(code, context) };
}

test("configuration initializes before map tiles, and tile failures do not block drawing", async () => {
  const app = start();
  await flush();
  assert.equal(app.elements["reset-config"].disabled, false);
  assert.equal(app.elements["config-form"].elements.dpi_resolution.value, 600);
  assert.equal(app.modes[0].disabled, true);
  assert.ok(app.calls.some((call) => call.path.startsWith("/api/job?")));
  assert.ok(!app.calls.some((call) => call.path === "/api/results"));
  assert.doesNotThrow(() => app.run("drawShape([[0, 0]])"));

  // No map 'load' event is ever fired, simulating tiles that never finish.
  app.map.fire("style.load");
  app.map.fire("error", { error: new Error("tile request blocked") });
  await flush();
  assert.equal(app.modes[0].disabled, false);
  assert.equal(app.elements["map-error"].hidden, false);
  assert.match(app.elements["map-error"].textContent, /tile request blocked/);
  assert.ok(app.calls.some((call) => call.path === "/api/results"));
  assert.equal(app.map.getSource("basemap").type, "raster");

  app.run('setMode("rectangle")');
  app.map.fire("mousedown", { point: { x: 0, y: 0 }, originalEvent: { button: 0 } });
  app.map.fire("mousemove", { point: { x: 10, y: 20 } });
  assert.equal(app.map.getSource("drawing").data.features.at(-1).geometry.type, "Polygon");
  app.map.fire("mouseup", { point: { x: 10, y: 20 }, originalEvent: { shiftKey: false } });
  await flush();
  assert.deepEqual(app.calls.find((call) => call.path === "/api/tiles").body.geometry.coordinates,
    [[[0, 0], [10, 0], [10, 20], [0, 20], [0, 0]]]);
  assert.equal(app.map.getSource("drawing").data.features.length, 0);

  app.run('setMode("polygon")');
  const point = (x, y) => ({ x, y, dist(other) { return Math.hypot(x - other.x, y - other.y); } });
  for (const p of [point(0, 0), point(20, 0), point(20, 20), point(0, 0)]) {
    app.map.fire("click", { point: p, originalEvent: { shiftKey: false } });
  }
  await flush();
  assert.equal(app.calls.filter((call) => call.path === "/api/tiles").length, 2);
});

test("failed API startup leaves dependent controls disabled without null errors", async () => {
  const app = start({ stateError: true });
  app.map.fire("style.load");
  await flush();
  assert.equal(app.elements["reset-config"].disabled, true);
  assert.equal(app.elements.launch.disabled, true);
  assert.ok(app.modes.every((button) => button.disabled));
  assert.match(app.elements.message.textContent, /unreachable/);
  assert.doesNotThrow(() => app.elements["reset-config"].listeners.click());
  app.map.fire("click", { lngLat: { lng: 2, lat: 46 } });
  assert.equal(app.calls.length, 1);
});

test("missing token is reported independently of the map", async () => {
  const app = start({ token: null });
  await flush();
  assert.match(app.elements.message.textContent, /with its token/);
  assert.equal(app.calls.length, 0);
  assert.equal(app.elements["reset-config"].disabled, true);
});

test("polygon vertices and closing hit target follow their geographic locations after moving the map", async () => {
  const app = start();
  app.map.fire("style.load");
  await flush();
  app.run('setMode("polygon")');
  const point = (x, y) => ({ x, y, dist(other) { return Math.hypot(x - other.x, y - other.y); } });
  const click = (x, y) => app.map.fire("click", { point: point(x, y), originalEvent: { shiftKey: false } });
  click(0, 0);
  click(20, 0);
  app.map.cameraOffset = 100;
  click(-80, -80); // Geographic location [20, 20] after moving the camera.
  app.map.fire("mousemove", { point: point(-70, -70) });
  assert.deepEqual(JSON.parse(JSON.stringify(app.map.getSource("drawing").data.features.at(-1).geometry.coordinates)),
    [[[0, 0], [20, 0], [20, 20], [30, 30], [0, 0]]]);
  click(-100, -100); // Close at the first vertex's new screen location.
  await flush();
  assert.deepEqual(app.calls.find((call) => call.path === "/api/tiles").body.geometry.coordinates,
    [[[0, 0], [20, 0], [20, 20], [0, 0]]]);
});

function result(name, x, y = 0) {
  return { name, url: `/results/${name}.png?v=1`, corners: [[x, y + 1], [x + 1, y + 1], [x + 1, y], [x, y]] };
}

test("results load only while visible and release images when hidden, moved away, or deleted", async () => {
  const results = [result("near", 0), result("far", 100)];
  const app = start({ results });
  app.elements["show-results"].checked = false;
  app.map.fire("style.load");
  await flush();
  assert.equal(app.map.imageLoads.length, 0);

  app.elements["show-results"].checked = true;
  app.elements["show-results"].listeners.change();
  assert.ok(app.map.getSource("result-near"));
  assert.equal(app.map.getSource("result-far"), undefined);
  app.elements["results-opacity"].value = "0.4";
  app.elements["results-opacity"].listeners.input();
  assert.equal(app.map.layers.find((layer) => layer.id === "result-near").paint["raster-opacity"], 0.4);

  app.map.viewport = [99, -1, 102, 2];
  app.map.fire("moveend");
  assert.equal(app.map.getSource("result-near"), undefined);
  assert.ok(app.map.getSource("result-far"));
  assert.equal(app.map.layers.find((layer) => layer.id === "result-far").paint["raster-opacity"], 0.4);
  const loads = app.map.imageLoads.length;
  await app.run("loadResults()");
  assert.equal(app.map.imageLoads.length, loads, "unchanged images must not reload");
  results[1].url += "2";
  await app.run("loadResults()");
  assert.equal(app.map.imageLoads.at(-1), results[1].url, "regenerated images must reload");

  app.elements["show-results"].checked = false;
  app.elements["show-results"].listeners.change();
  assert.equal(app.map.getSource("result-far"), undefined);
  app.elements["show-results"].checked = true;
  app.elements["show-results"].listeners.change();
  assert.ok(app.map.getSource("result-far"));
  results.pop();
  await app.run("loadResults()");
  assert.equal(app.map.getSource("result-far"), undefined);
});

test("overview bounds result images and loads other tiles when zooming in", async () => {
  const app = start({ results: Array.from({ length: 100 }, (_, index) => result(String(index), index / 10)) });
  app.map.fire("style.load");
  await flush();
  const images = () => [...app.map.sources.values()].filter((source) => source.type === "image");
  assert.equal(images().length, 16);
  assert.equal(app.map.imageLoads.length, 16);
  assert.match(app.elements["results-summary"].textContent, /closest 16 of 100 visible tiles/);
  assert.equal(app.map.getSource("result-0"), undefined);

  app.map.viewport = [-0.1, -0.1, 0.05, 1.1];
  app.map.fire("moveend");
  assert.equal(images().length, 1);
  assert.ok(app.map.getSource("result-0"));
  assert.doesNotMatch(app.elements["results-summary"].textContent, /Zoom in/);
});
