// Position of the vertical axis.
const X_LINE = 50;
// General-use margin size.
const MARGIN = 5;
// Position of the horizontal axis, relative to the bottom.
const Y_LINE = 35;
// Minimum distance between time tick labels.
const MIN_TICK_DIST = 50;
// Radius for rounded rectangle corners.
const RADIUS = 3;
// Height of unit boxes.
const BOX_HEIGHT = 25;
// Distance between Y tick marks on the unit graph.
const Y_TICK_DIST = BOX_HEIGHT + 2;
// Rects used for mouseover detection.
// Objects of {x, y, x2, y2, i} where `i` is the index into UNIT_DATA.
let HIT_BOXES = [];
// Index into UNIT_DATA of the last unit hovered over by mouse.
let LAST_HOVER = null;
// Key is unit index, value is {x, y, width, rmeta_x} of the box.
let UNIT_COORDS = {};
// Map of unit index to the index it was unlocked by.
let REVERSE_UNIT_DEPS = {};
let REVERSE_UNIT_RMETA_DEPS = {};
for (let n=0; n<UNIT_DATA.length; n++) {
  let unit = UNIT_DATA[n];
  for (let unlocked of unit.unlocked_units) {
    REVERSE_UNIT_DEPS[unlocked] = n;
  }
  for (let unlocked of unit.unlocked_rmeta_units) {
    REVERSE_UNIT_RMETA_DEPS[unlocked] = n;
  }
}

function render_pipeline_graph() {
  if (UNIT_DATA.length == 0) {
    return;
  }
  let g = document.getElementById('pipeline-graph');
  HIT_BOXES.length = 0;
  g.onmousemove = pipeline_mousemove;
  const min_time = document.getElementById('min-unit-time').valueAsNumber;

  const units = UNIT_DATA.filter(unit => unit.duration >= min_time);

  const graph_height = Y_TICK_DIST * units.length;
  const {ctx, graph_width, width, height, px_per_sec} = draw_graph_axes('pipeline-graph', graph_height);
  const container = document.getElementById('pipeline-container');
  container.style.width = width;
  container.style.height = height;

  // Canvas for hover highlights. This is a separate layer to improve performance.
  const linectx = setup_canvas('pipeline-graph-lines', width, height);
  linectx.clearRect(0, 0, width, height);

  // Draw Y tick marks.
  for (let n=1; n<units.length; n++) {
    const y = graph_height - (n * Y_TICK_DIST);
    ctx.beginPath();
    ctx.moveTo(X_LINE, y);
    ctx.lineTo(X_LINE-5, y);
    ctx.stroke();
  }

  // Draw Y labels.
  ctx.textAlign = 'end';
  for (let n=0; n<units.length; n++) {
    let y = MARGIN + (Y_TICK_DIST * (n + 1)) - 13;
    ctx.fillText(n+1, X_LINE-4, y);
  }

  // Draw the graph.
  ctx.save();
  ctx.translate(X_LINE, MARGIN);

  // Compute x,y coordinate of each block.
  UNIT_COORDS = {};
  for (i=0; i<units.length; i++) {
    let unit = units[i];
    let y = i * Y_TICK_DIST + 1;
    let x = px_per_sec * unit.start;
    let rmeta_x = null;
    if (unit.rmeta_time != null) {
      rmeta_x = x + px_per_sec * unit.rmeta_time;
    }
    let width = Math.max(px_per_sec * unit.duration, 1.0);
    UNIT_COORDS[unit.i] = {x, y, width, rmeta_x};
  }

  // Draw the blocks.
  for (i=0; i<units.length; i++) {
    let unit = units[i];
    let {x, y, width, rmeta_x} = UNIT_COORDS[unit.i];

    HIT_BOXES.push({x: X_LINE+x, y:MARGIN+y, x2: X_LINE+x+width, y2: MARGIN+y+BOX_HEIGHT, i: unit.i});

    ctx.beginPath();
    ctx.fillStyle = unit.mode == 'run-custom-build' ? '#f0b165' : '#95cce8';
    roundedRect(ctx, x, y, width, BOX_HEIGHT, RADIUS);
    ctx.fill();

    if (unit.rmeta_time != null) {
      ctx.beginPath();
      ctx.fillStyle = '#aa95e8';
      let ctime = unit.duration - unit.rmeta_time;
      roundedRect(ctx, rmeta_x, y, px_per_sec * ctime, BOX_HEIGHT, RADIUS);
      ctx.fill();
    }
    ctx.fillStyle = "#000";
    ctx.textAlign = 'start';
    ctx.textBaseline = 'hanging';
    ctx.font = '14px sans-serif';
    const label = `${unit.name}${unit.target} ${unit.duration}s`;
    const text_info = ctx.measureText(label);
    const label_x = Math.min(x + 5.0, graph_width - text_info.width);
    ctx.fillText(label, label_x, y + BOX_HEIGHT / 2 - 6);
    draw_dep_lines(ctx, unit.i, false);
  }
  ctx.restore();
}

// Draws lines from the given unit to the units it unlocks.
function draw_dep_lines(ctx, unit_idx, highlighted) {
  const unit = UNIT_DATA[unit_idx];
  const {x, y, rmeta_x} = UNIT_COORDS[unit_idx];
  ctx.save();
  for (const unlocked of unit.unlocked_units) {
    draw_one_dep_line(ctx, x, y, unlocked, highlighted);
  }
  for (const unlocked of unit.unlocked_rmeta_units) {
    draw_one_dep_line(ctx, rmeta_x, y, unlocked, highlighted);
  }
  ctx.restore();
}

function draw_one_dep_line(ctx, from_x, from_y, to_unit, highlighted) {
  if (to_unit in UNIT_COORDS) {
    let {x: u_x, y: u_y} = UNIT_COORDS[to_unit];
    ctx.strokeStyle = highlighted ? '#000' : '#ddd';
    ctx.setLineDash([2]);
    ctx.beginPath();
    ctx.moveTo(from_x, from_y+BOX_HEIGHT/2);
    ctx.lineTo(from_x-5, from_y+BOX_HEIGHT/2);
    ctx.lineTo(from_x-5, u_y+BOX_HEIGHT/2);
    ctx.lineTo(u_x, u_y+BOX_HEIGHT/2);
    ctx.stroke();
  }
}

function render_timing_graph() {
  if (CONCURRENCY_DATA.length == 0) {
    return;
  }
  const HEIGHT = 400;
  const AXIS_HEIGHT = HEIGHT - MARGIN - Y_LINE;
  const TOP_MARGIN = 10;
  const GRAPH_HEIGHT = AXIS_HEIGHT - TOP_MARGIN;

  const {width, graph_width, ctx} = draw_graph_axes('timing-graph', AXIS_HEIGHT);

  // Draw Y tick marks and labels.
  let max_v = 0;
  for (c of CONCURRENCY_DATA) {
    max_v = Math.max(max_v, c.active, c.waiting, c.inactive);
  }
  let [step, top] = split_ticks(max_v, GRAPH_HEIGHT / MIN_TICK_DIST);
  let num_ticks = top / step;
  let tick_dist = GRAPH_HEIGHT / num_ticks;
  ctx.textAlign = 'end';
  for (n=0; n<num_ticks; n++) {
    let y = HEIGHT - Y_LINE - ((n + 1) * tick_dist);
    ctx.beginPath();
    ctx.moveTo(X_LINE, y);
    ctx.lineTo(X_LINE-5, y);
    ctx.stroke();
    ctx.fillText((n+1) * step, X_LINE-10, y+5);
  }

  // Label the Y axis.
  let label_y = (HEIGHT - Y_LINE) / 2;
  ctx.save();
  ctx.translate(15, label_y);
  ctx.rotate(3*Math.PI/2);
  ctx.textAlign = 'center';
  ctx.fillText('# Units', 0, 0);
  ctx.restore();

  // Draw the graph.
  ctx.save();
  ctx.translate(X_LINE, MARGIN);

  function coord(t, v) {
    return {
      x: graph_width * (t/DURATION),
      y: TOP_MARGIN + GRAPH_HEIGHT * (1.0 - (v / max_v))
    };
  }

  const cpuFillStyle = 'rgba(250, 119, 0, 0.2)';
  if (CPU_USAGE.length > 1) {
    ctx.beginPath();
    ctx.fillStyle = cpuFillStyle;
    let bottomLeft = coord(CPU_USAGE[0][0], 0);
    ctx.moveTo(bottomLeft.x, bottomLeft.y);
    for (let i=0; i < CPU_USAGE.length; i++) {
      let [time, usage] = CPU_USAGE[i];
      let {x, y} = coord(time, usage / 100.0 * max_v);
      ctx.lineTo(x, y);
    }
    let bottomRight = coord(CPU_USAGE[CPU_USAGE.length - 1][0], 0);
    ctx.lineTo(bottomRight.x, bottomRight.y);
    ctx.fill();
  }

  function draw_line(style, key) {
    let first = CONCURRENCY_DATA[0];
    let last = coord(first.t, key(first));
    ctx.strokeStyle = style;
    ctx.beginPath();
    ctx.moveTo(last.x, last.y);
    for (let i=1; i<CONCURRENCY_DATA.length; i++) {
      let c = CONCURRENCY_DATA[i];
      let {x, y} = coord(c.t, key(c));
      ctx.lineTo(x, last.y);
      ctx.lineTo(x, y);
      last = {x, y};
    }
    ctx.stroke();
  }

  draw_line('blue', function(c) {return c.inactive;});
  draw_line('red', function(c) {return c.waiting;});
  draw_line('green', function(c) {return c.active;});

  // Draw a legend.
  ctx.restore();
  ctx.save();
  ctx.translate(width-200, MARGIN);
  // background
  ctx.fillStyle = '#fff';
  ctx.strokeStyle = '#000';
  ctx.lineWidth = 1;
  ctx.textBaseline = 'middle'
  ctx.textAlign = 'start';
  ctx.beginPath();
  ctx.rect(0, 0, 150, 82);
  ctx.stroke();
  ctx.fill();

  ctx.fillStyle = '#000'
  ctx.beginPath();
  ctx.lineWidth = 2;
  ctx.strokeStyle = 'red';
  ctx.moveTo(5, 10);
  ctx.lineTo(50, 10);
  ctx.stroke();
  ctx.fillText('Waiting', 54, 11);

  ctx.beginPath();
  ctx.strokeStyle = 'blue';
  ctx.moveTo(5, 30);
  ctx.lineTo(50, 30);
  ctx.stroke();
  ctx.fillText('Inactive', 54, 31);

  ctx.beginPath();
  ctx.strokeStyle = 'green';
  ctx.moveTo(5, 50);
  ctx.lineTo(50, 50);
  ctx.stroke();
  ctx.fillText('Active', 54, 51);

  ctx.beginPath();
  ctx.fillStyle = cpuFillStyle
  ctx.fillRect(15, 60, 30, 15);
  ctx.fill();
  ctx.fillStyle = 'black';
  ctx.fillText('CPU Usage', 54, 71);

  ctx.restore();
}

function setup_canvas(id, width, height) {
  let g = document.getElementById(id);
  let dpr = window.devicePixelRatio || 1;
  g.width = width * dpr;
  g.height = height * dpr;
  g.style.width = width;
  g.style.height = height;
  let ctx = g.getContext('2d');
  ctx.scale(dpr, dpr);
  return ctx;
}

function draw_graph_axes(id, graph_height) {
  const scale = document.getElementById('scale').valueAsNumber;
  // Cap the size of the graph. It is hard to view if it is too large, and
  // browsers may not render a large graph because it takes too much memory.
  // 4096 is still ridiculously large, and probably won't render on mobile
  // browsers, but should be ok for many desktop environments.
  const graph_width = Math.min(scale * DURATION, 4096);
  const px_per_sec = Math.floor(graph_width / DURATION);
  const width = Math.max(graph_width + X_LINE + 30, X_LINE + 250);
  const height = graph_height + MARGIN + Y_LINE;
  let ctx = setup_canvas(id, width, height);
  ctx.fillStyle = '#f7f7f7';
  ctx.fillRect(0, 0, width, height);

  ctx.lineWidth = 2;
  ctx.font = '16px sans-serif';
  ctx.textAlign = 'center';

  // Draw main axes.
  ctx.beginPath();
  ctx.moveTo(X_LINE, MARGIN);
  ctx.lineTo(X_LINE, graph_height + MARGIN);
  ctx.lineTo(X_LINE+graph_width+20, graph_height + MARGIN);
  ctx.stroke();

  // Draw X tick marks.
  const [step, top] = split_ticks(DURATION, graph_width / MIN_TICK_DIST);
  const num_ticks = top / step;
  const tick_dist = graph_width / num_ticks;
  ctx.fillStyle = '#303030';
  for (let n=0; n<num_ticks; n++) {
    const x = X_LINE + ((n + 1) * tick_dist);
    ctx.beginPath();
    ctx.moveTo(x, height-Y_LINE);
    ctx.lineTo(x, height-Y_LINE+5);
    ctx.stroke();

    ctx.fillText(`${(n+1) * step}s`, x, height - Y_LINE + 20);
  }

  // Draw vertical lines.
  ctx.strokeStyle = '#e6e6e6';
  ctx.setLineDash([2, 4]);
  for (n=0; n<num_ticks; n++) {
    const x = X_LINE + ((n + 1) * tick_dist);
    ctx.beginPath();
    ctx.moveTo(x, MARGIN);
    ctx.lineTo(x, MARGIN+graph_height);
    ctx.stroke();
  }
  ctx.strokeStyle = '#000';
  ctx.setLineDash([]);
  return {width, height, graph_width, graph_height, ctx, px_per_sec};
}

function round_up(n, step) {
  if (n % step == 0) {
    return n;
  } else {
    return (step - n % step) + n;
  }
}

// Determine the `(step, max_value)` of the number of ticks along an axis.
function split_ticks(n, max_ticks) {
  max_ticks = Math.ceil(max_ticks);
  if (n <= max_ticks) {
    return [1, n];
  } else if (n <= max_ticks * 2) {
    return [2, round_up(n, 2)];
  } else if (n <= max_ticks * 4) {
    return [4, round_up(n, 4)];
  } else if (n <= max_ticks * 5) {
    return [5, round_up(n, 5)];
  } else {
    let step = 10;
    let count = 0;
    while (true) {
      if (count > 100) {
        throw Error("tick loop too long");
      }
      count += 1;
      let top = round_up(n, step);
      if (top <= max_ticks * step) {
        return [step, top];
      }
      step += 10;
    }
  }
}

function codegen_time(unit) {
  if (unit.rmeta_time == null) {
    return null;
  }
  let ctime = unit.duration - unit.rmeta_time;
  return [unit.rmeta_time, ctime];
}

function roundedRect(ctx, x, y, width, height, r) {
  r = Math.min(r, width, height);
  ctx.beginPath();
  ctx.moveTo(x+r, y);
  ctx.lineTo(x+width-r, y);
  ctx.arc(x+width-r, y+r, r, 3*Math.PI/2, 0);
  ctx.lineTo(x+width, y+height-r);
  ctx.arc(x+width-r, y+height-r, r, 0, Math.PI/2);
  ctx.lineTo(x+r, y+height);
  ctx.arc(x+r, y+height-r, r, Math.PI/2, Math.PI);
  ctx.lineTo(x, y-r);
  ctx.arc(x+r, y+r, r, Math.PI, 3*Math.PI/2);
  ctx.closePath();
}

function pipeline_mouse_hit(event) {
  // This brute-force method can be optimized if needed.
  for (let box of HIT_BOXES) {
    if (event.offsetX >= box.x && event.offsetX <= box.x2 &&
        event.offsetY >= box.y && event.offsetY <= box.y2) {
      return box;
    }
  }
}

function pipeline_mousemove(event) {
  // Highlight dependency lines on mouse hover.
  let box = pipeline_mouse_hit(event);
  if (box) {
    if (box.i != LAST_HOVER) {
      LAST_HOVER = box.i;
      let g = document.getElementById('pipeline-graph-lines');
      let ctx = g.getContext('2d');
      ctx.clearRect(0, 0, g.width, g.height);
      ctx.save();
      ctx.translate(X_LINE, MARGIN);
      ctx.lineWidth = 2;
      draw_dep_lines(ctx, box.i, true);

      if (box.i in REVERSE_UNIT_DEPS) {
        const dep_unit = REVERSE_UNIT_DEPS[box.i];
        if (dep_unit in UNIT_COORDS) {
          const {x, y, rmeta_x} = UNIT_COORDS[dep_unit];
          draw_one_dep_line(ctx, x, y, box.i, true);
        }
      }
      if (box.i in REVERSE_UNIT_RMETA_DEPS) {
        const dep_unit = REVERSE_UNIT_RMETA_DEPS[box.i];
        if (dep_unit in UNIT_COORDS) {
          const {x, y, rmeta_x} = UNIT_COORDS[dep_unit];
          draw_one_dep_line(ctx, rmeta_x, y, box.i, true);
        }
      }
      ctx.restore();
    }
  }
}

render_pipeline_graph();
render_timing_graph();

// Set up and handle controls.
{
  const range = document.getElementById('min-unit-time');
  const time_output = document.getElementById('min-unit-time-output');
  time_output.innerHTML = `${range.value}s`;
  range.oninput = event => {
    time_output.innerHTML = `${range.value}s`;
    render_pipeline_graph();
  };

  const scale = document.getElementById('scale');
  const scale_output = document.getElementById('scale-output');
  scale_output.innerHTML = `${scale.value}`;
  scale.oninput = event => {
    scale_output.innerHTML = `${scale.value}`;
    render_pipeline_graph();
    render_timing_graph();
  };
}
