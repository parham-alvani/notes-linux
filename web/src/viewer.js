// Pan and zoom for one picture: a picture file on its own page, and a
// picture or a diagram opened large from a note. The picture is laid out at
// its natural size times the scale -- its width and height change, not a
// transform's scale -- so WebKit draws an SVG or a diagram again at each size
// and it stays sharp. The math is apart from the page so node can test it.

/** The largest zoom: 1600 %. */
export const MAX_SCALE = 16;

/**
 * The scale that shows all of a [width] × [height] picture in a [boxWidth] ×
 * [boxHeight] box, [pad] from each side, never above 100 %: a small picture
 * is shown at its own size, as Preview does.
 */
export function fitScale(width, height, boxWidth, boxHeight, pad = 0) {
  if (!(width > 0 && height > 0 && boxWidth > 0 && boxHeight > 0)) return 1;
  return Math.min(1, Math.max(boxWidth - 2 * pad, 1) / width, Math.max(boxHeight - 2 * pad, 1) / height);
}

/** [scale] kept between a quarter of the fitted scale and [MAX_SCALE]. */
export function clampScale(scale, fit) {
  return Math.min(Math.max(scale, Math.min(fit, 1) / 4), MAX_SCALE);
}

/**
 * The view at [scale] that keeps the point ([px], [py]) of the box where it
 * is: the picture grows or shrinks around the pointer.
 */
export function zoomAbout(view, scale, px, py) {
  return { scale, x: px - ((px - view.x) * scale) / view.scale, y: py - ((py - view.y) * scale) / view.scale };
}

/**
 * The view moved back into the box: a picture smaller than the box on an
 * axis is in its middle there; a larger one covers the box, with no gap.
 */
export function clampPan(view, width, height, boxWidth, boxHeight) {
  const w = width * view.scale, h = height * view.scale;
  const axis = (at, size, box) => (size <= box ? (box - size) / 2 : Math.min(0, Math.max(box - size, at)));
  return { scale: view.scale, x: axis(view.x, w, boxWidth), y: axis(view.y, h, boxHeight) };
}

/** A double click: from the fitted size to 100 % (200 % for a small picture), from any other size back to fit. */
export function toggleScale(scale, fit) {
  if (Math.abs(scale - fit) > 0.001) return fit;
  return Math.abs(fit - 1) > 0.001 ? 1 : 2;
}

/** "250 %" as the bar shows it. */
export function percent(scale) {
  return `${Math.round(scale * 100)} %`;
}

/** The natural size of an inline SVG: its viewBox, its width and height, or the box it was drawn in. */
export function svgSize(viewBox, width, height, drawn) {
  const box = String(viewBox ?? "").trim().split(/[\s,]+/).map(Number);
  if (box.length === 4 && box[2] > 0 && box[3] > 0) return { width: box[2], height: box[3] };
  const w = parseFloat(width), h = parseFloat(height);
  if (w > 0 && h > 0 && !/%/.test(String(width)) && !/%/.test(String(height))) return { width: w, height: h };
  return { width: drawn?.width || 300, height: drawn?.height || 150 };
}

/**
 * Pans and zooms [content] (an `<img>` or an `<svg>`) inside [stage], a
 * box with `overflow: hidden`. A pinch or ⌘-scroll zooms around the
 * pointer, a scroll or a drag pans, a double click goes between fit and
 * 100 %, and `key` takes + − 0 1 and the arrows. Until the person zooms,
 * the picture follows the box's size.
 */
export class PanZoom {
  constructor(stage, content, { width, height, pad = 0, onChange = () => {} }) {
    Object.assign(this, { stage, content, width, height, pad, onChange });
    this.view = { scale: 1, x: 0, y: 0 };
    this.fitted = true;
    this.drag = null;
    this.moved = false;
    this.pinch = null;
    this.raster = content.tagName === "IMG" && !/\.svg(?:[?#]|$)/i.test(content.getAttribute("src") ?? "");
    Object.assign(content.style, { position: "absolute", left: "0", top: "0", transformOrigin: "0 0", maxWidth: "none", maxHeight: "none" });
    this.listen();
    this.observer = new ResizeObserver(() => this.resized());
    this.observer.observe(stage);
    this.fit();
  }

  get box() {
    return { width: this.stage.clientWidth, height: this.stage.clientHeight };
  }

  get fitScale() {
    const { width, height } = this.box;
    return fitScale(this.width, this.height, width, height, this.pad);
  }

  /** The picture's natural size, once it is known (an `<img>` that loaded). */
  setSize(width, height) {
    if (!(width > 0 && height > 0)) return;
    Object.assign(this, { width, height });
    if (this.fitted) this.fit();
    else this.apply(this.view);
  }

  apply(view) {
    const { width, height } = this.box;
    this.view = clampPan(view, this.width, this.height, width, height);
    const { scale, x, y } = this.view;
    const style = this.content.style;
    style.width = `${this.width * scale}px`;
    style.height = `${this.height * scale}px`;
    style.transform = `translate(${x}px, ${y}px)`;
    // Pixels are pixels when the picture is looked at closely.
    if (this.raster) style.imageRendering = scale >= 3 ? "pixelated" : "";
    this.stage.classList.toggle("is-pannable", this.width * scale > width + 0.5 || this.height * scale > height + 0.5);
    this.onChange(this.view);
  }

  fit() {
    this.fitted = true;
    this.apply({ scale: this.fitScale, x: 0, y: 0 });
  }

  /** [scale] around the point ([cx], [cy]) of the window, or the box's middle. */
  zoomTo(scale, cx, cy) {
    const rect = this.stage.getBoundingClientRect();
    const px = (cx ?? rect.left + rect.width / 2) - rect.left, py = (cy ?? rect.top + rect.height / 2) - rect.top;
    this.fitted = false;
    this.apply(zoomAbout(this.view, clampScale(scale, this.fitScale), px, py));
  }

  zoomBy(factor, cx, cy) {
    this.zoomTo(this.view.scale * factor, cx, cy);
  }

  actualSize(cx, cy) {
    this.zoomTo(1, cx, cy);
  }

  panBy(dx, dy) {
    this.apply({ ...this.view, x: this.view.x - dx, y: this.view.y - dy });
  }

  resized() {
    if (!this.stage.isConnected) return this.observer.disconnect();
    if (this.fitted) this.fit();
    else this.apply(this.view);
  }

  /** + − 0 1 and the arrows. True when the key was taken. */
  key(event) {
    if (event.metaKey || event.ctrlKey || event.altKey) return false;
    const step = 60;
    const keys = {
      "+": () => this.zoomBy(1.25), "=": () => this.zoomBy(1.25), "-": () => this.zoomBy(0.8), "−": () => this.zoomBy(0.8),
      "0": () => this.fit(), "1": () => this.actualSize(),
      ArrowLeft: () => this.panBy(-step, 0), ArrowRight: () => this.panBy(step, 0),
      ArrowUp: () => this.panBy(0, -step), ArrowDown: () => this.panBy(0, step),
    };
    if (!keys[event.key]) return false;
    keys[event.key]();
    event.preventDefault();
    return true;
  }

  /** A command from the bar or the app's menu: "in", "out", "fit" or "actual". */
  command(name) {
    ({ in: () => this.zoomBy(1.25), out: () => this.zoomBy(0.8), fit: () => this.fit(), actual: () => this.actualSize() })[name]?.();
  }

  listen() {
    const stage = this.stage;
    stage.addEventListener(
      "wheel",
      (event) => {
        event.preventDefault();
        // A trackpad pinch comes as a wheel with Control, unless WebKit
        // sends it as a gesture; ⌘ and a mouse wheel zoom too.
        if (event.ctrlKey || event.metaKey) {
          if (this.pinch) return;
          this.zoomBy(Math.exp(-event.deltaY * (event.ctrlKey ? 0.01 : 0.002)), event.clientX, event.clientY);
          return;
        }
        this.panBy(event.deltaX, event.deltaY);
      },
      { passive: false },
    );
    // Safari's own pinch.
    stage.addEventListener("gesturestart", (event) => {
      event.preventDefault();
      this.pinch = 1;
    });
    stage.addEventListener("gesturechange", (event) => {
      event.preventDefault();
      this.zoomBy(event.scale / (this.pinch ?? 1), event.clientX, event.clientY);
      this.pinch = event.scale;
    });
    stage.addEventListener("gestureend", (event) => {
      event.preventDefault();
      this.pinch = null;
    });
    stage.addEventListener("pointerdown", (event) => {
      if (event.button !== 0 && event.button !== 1) return;
      if (event.target.closest?.("button, a, [role=toolbar]")) return;
      this.drag = { x: event.clientX, y: event.clientY, view: this.view, id: event.pointerId };
      this.moved = false;
      event.preventDefault();
    });
    stage.addEventListener("pointermove", (event) => {
      const drag = this.drag;
      if (!drag || event.pointerId !== drag.id) return;
      const dx = event.clientX - drag.x, dy = event.clientY - drag.y;
      if (!this.moved && Math.abs(dx) + Math.abs(dy) <= 3) return;
      // Captured only once it is a drag, so a plain click keeps its target.
      if (!this.moved) stage.setPointerCapture?.(event.pointerId);
      this.moved = true;
      stage.classList.add("is-panning");
      this.apply({ ...drag.view, x: drag.view.x + dx, y: drag.view.y + dy });
    });
    const end = () => {
      this.drag = null;
      stage.classList.remove("is-panning");
    };
    stage.addEventListener("pointerup", end);
    stage.addEventListener("pointercancel", end);
    stage.addEventListener("dblclick", (event) => {
      if (event.target.closest?.("button, a, [role=toolbar]")) return;
      event.preventDefault();
      const target = toggleScale(this.view.scale, this.fitScale);
      if (target === this.fitScale) this.fit();
      else this.zoomTo(target, event.clientX, event.clientY);
    });
  }
}
