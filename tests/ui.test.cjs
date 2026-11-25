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

function start({ stateError = false, token = "test" } = {}) {
  const elements = Object.fromEntries([...html.matchAll(/id="([^"]+)"/g)].map(([, id]) => [id, new Element()]));
  const modes = ["click", "rectangle", "polygon"].map((mode) => {
    const button = new Element();
    button.dataset.mode = mode;
    return button;
  });
  const calls = [];
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
      this.dragPan = this.boxZoom = this.doubleClickZoom = { enable() {}, disable() {} };
    }
    addControl() {}
    on(name, handler) { (this.handlers[name] ||= []).push(handler); }
    once(name, handler) { this.on(name, handler); }
    fire(name, event) { for (const handler of this.handlers[name] || []) handler(event); }
    addSource(id, spec) { this.sources.set(id, { ...spec, setData(data) { this.data = data; } }); }
    getSource(id) { return this.sources.get(id); }
    addLayer(layer) { this.layers.push(layer); }
    unproject(point) { return { toArray: () => [point.x, point.y] }; }
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
      const data = path === "/api/state" ? state : path.startsWith("/api/job?") ? null : [];
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
  assert.doesNotThrow(() => app.run("drawShape([{x: 0, y: 0}])"));

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
