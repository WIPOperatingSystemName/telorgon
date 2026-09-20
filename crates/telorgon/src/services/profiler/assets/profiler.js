"use strict";

const PROTOCOL_MAJOR = 3;
const PROTOCOL_MINOR = 0;
const EVENT_BYTES = 80;
const KIND = Object.freeze({
  SPAN: 1,
  INSTANT: 2,
  COUNTER: 3,
  FRAME_BEGIN: 4,
  FRAME_END: 5,
  PRESENTATION_BEGIN: 6,
  PRESENTATION_END: 7,
  DIAGNOSTIC: 8,
  GAP: 9,
  GPU_SPAN: 10,
});
const CATEGORY = Object.freeze({
  OTHER: 0,
  RUNTIME: 1,
  INPUT: 2,
  THEME: 3,
  LAYOUT: 4,
  SCENE: 5,
  RENDERER: 6,
  GPU: 7,
  WAIT: 8,
  DIAGNOSTIC: 9,
  PRESENTATION: 10,
});
const PERFORMANCE_CATEGORY = Object.freeze({ CPU: "cpu", GPU: "gpu", PRESENTATION: "presentation" });
const PERFORMANCE_CATEGORIES = Object.freeze([
  PERFORMANCE_CATEGORY.CPU,
  PERFORMANCE_CATEGORY.GPU,
  PERFORMANCE_CATEGORY.PRESENTATION,
]);
const PERFORMANCE_COLORS = Object.freeze({
  [PERFORMANCE_CATEGORY.CPU]: "#669df6",
  [PERFORMANCE_CATEGORY.GPU]: "#68bd87",
  [PERFORMANCE_CATEGORY.PRESENTATION]: "#e5a14a",
});
const UNIT = Object.freeze({
  NONE: 0,
  DURATION: 1,
  COUNT: 2,
  BYTES: 3,
  NANOSECONDS: 4,
  AREA: 5,
  SCALAR: 6,
  IDENTIFIER: 7,
});
const AGGREGATION = Object.freeze({ EVENT: 0, GAUGE: 1, SUM: 2, CUMULATIVE: 3 });
const LABEL_FLAG_RESOURCE = 1;
const VULKAN_WSI_STALL_DIAGNOSTIC = "presentation.vulkan_wsi.zero_timeout_acquire_stall";
const VULKAN_RAW_ACQUIRE_COUNTER = "presentation.acquire.raw_dispatch_duration_ns";
const POINTER_PIPELINE_PREFIX = "input.libinput.pointer_motion.pipeline.";

const state = {
  metadata: null,
  lanes: new Map(),
  views: new Map(),
  labels: new Map(),
  frames: [],
  framesById: new Map(),
  events: [],
  counters: new Map(),
  diagnostics: [],
  seenSequences: new Set(),
  selectedFrame: null,
  selectedEvent: null,
  selectedHotspot: null,
  followLiveSelection: true,
  workspace: "performance",
  performanceView: "overview",
  viewFilter: "all",
  rawTimelineVisible: false,
  includePointerMoves: false,
  inputRecordingSources: new Set(),
  paused: false,
  zoom: 1,
  rangeStartId: null,
  rangeEndId: null,
  plotFrameLimit: null,
  plotView: null,
  plotViewHistory: [],
  selectedPlotPointKey: null,
  hoveredPlotPointKey: null,
  tableSorts: new Map(),
  inspectorWidth: 288,
  counterBaselines: new Map(),
  dropped: 0n,
  renderQueued: false,
  liveRenderTimer: null,
  lastRenderAt: 0,
};

const LIVE_RENDER_INTERVAL_MS = 100;
const INSPECTOR_MIN_WIDTH = 220;
const INSPECTOR_MAX_WIDTH = 760;
const INSPECTOR_DEFAULT_WIDTH = 288;
const INSPECTOR_MIN_WORKSPACE_WIDTH = 360;
const INSPECTOR_KEYBOARD_STEP = 16;
const TABLE_SORT_COLLATOR = new Intl.Collator(undefined, { numeric: true, sensitivity: "base" });

let activeSocket = null;
let reconnectTimer = null;
let reconnectDelay = 500;
let inspectorDragFrame = null;
let inspectorDragActive = false;

const element = (id) => document.getElementById(id);
const decoder = new TextDecoder();
const plot = element("frame-plot");

function connect() {
  if (activeSocket && activeSocket.readyState < WebSocket.CLOSING) return;
  const protocol = location.protocol === "https:" ? "wss:" : "ws:";
  const socket = new WebSocket(`${protocol}//${location.host}/live`);
  activeSocket = socket;
  socket.binaryType = "arraybuffer";
  socket.addEventListener("open", () => {
    reconnectDelay = 500;
    setConnection("Live", "live");
    sendInputRecordingPreferences();
  });
  socket.addEventListener("message", (message) => {
    if (socket !== activeSocket) return;
    if (typeof message.data === "string") {
      acceptMetadata(message.data);
    } else {
      acceptBinary(message.data);
    }
  });
  socket.addEventListener("close", () => {
    if (socket !== activeSocket) return;
    activeSocket = null;
    disconnectViewer();
    scheduleReconnect();
  });
  socket.addEventListener("error", () => socket.close());
}

function sendInputRecordingPreference(source, enabled) {
  if (!activeSocket || activeSocket.readyState !== WebSocket.OPEN) return;
  activeSocket.send(JSON.stringify({
    type: "set_input_recording",
    source,
    enabled,
  }));
}

function sendInputRecordingPreferences() {
  state.inputRecordingSources.forEach((source) => sendInputRecordingPreference(source, true));
}

function scheduleReconnect() {
  if (reconnectTimer !== null) return;
  reconnectTimer = window.setTimeout(() => {
    reconnectTimer = null;
    renewSessionAndReconnect();
  }, reconnectDelay);
  reconnectDelay = Math.min(3000, reconnectDelay * 1.5);
}

async function renewSessionAndReconnect() {
  try {
    const response = await fetch("/reconnect", {
      method: "POST",
      cache: "no-store",
      credentials: "same-origin",
    });
    if (response.ok) {
      connect();
      return;
    }
  } catch (_) {
    // The app-owned service is expected to be absent while the app is closed.
  }
  scheduleReconnect();
}

function setConnection(label, style) {
  element("connection-state").textContent = label;
  element("status-dot").className = `status-dot ${style}`;
}

function acceptMetadata(text) {
  try {
    const envelope = JSON.parse(text);
    if (envelope.protocol_major !== PROTOCOL_MAJOR) {
      throw new Error(`Unsupported protocol ${envelope.protocol_major}; viewer supports ${PROTOCOL_MAJOR}.`);
    }
    if (envelope.protocol_minor < PROTOCOL_MINOR) {
      throw new Error(`Profiler protocol ${envelope.protocol_major}.${envelope.protocol_minor} does not include metric descriptors required by this viewer.`);
    }
    state.metadata = envelope;
    state.lanes = new Map((envelope.lanes || []).map((lane) => [lane.id, lane.name]));
    state.views = new Map((envelope.views || []).map((view) => [BigInt(view.id).toString(), view.role]));
    element("application-name").textContent = envelope.session.application;
    setApplicationConnected(true);
    renderInputRecordingControls();
    sendInputRecordingPreferences();
    hideError();
    renderSession();
  } catch (error) {
    showError(error instanceof Error ? error.message : "Invalid profiler metadata.");
  }
}

function availableInputSources() {
  return state.metadata?.session?.input_recording_sources || [];
}

function renderInputRecordingControls() {
  const root = element("input-recording-options");
  root.replaceChildren();
  const sources = availableInputSources();
  if (sources.length === 0) {
    setText("input-recording-target", "Connect a target to list its native inputs.");
    return;
  }
  setText("input-recording-target", `${formatEntrypoint(state.metadata.session.entrypoint)} native sources`);
  const supported = new Set(sources.map((source) => source.id));
  [...state.inputRecordingSources]
    .filter((source) => !supported.has(source))
    .forEach((source) => {
      state.inputRecordingSources.delete(source);
      sendInputRecordingPreference(source, false);
    });
  state.includePointerMoves = state.inputRecordingSources.has("pointer_motion");
  sources.forEach((source) => {
    const label = document.createElement("label");
    label.className = "input-recording-option";
    label.title = source.description;
    const checkbox = document.createElement("input");
    checkbox.type = "checkbox";
    checkbox.checked = state.inputRecordingSources.has(source.id);
    checkbox.disabled = state.metadata === null;
    checkbox.addEventListener("change", () => setInputRecordingSource(source.id, checkbox.checked));
    const copy = document.createElement("span");
    copy.textContent = source.label;
    const description = document.createElement("small");
    description.textContent = source.description;
    copy.append(description);
    label.append(checkbox, copy);
    root.append(label);
  });
}

function setInputRecordingSource(source, enabled) {
  if (enabled) state.inputRecordingSources.add(source);
  else state.inputRecordingSources.delete(source);
  state.includePointerMoves = state.inputRecordingSources.has("pointer_motion");
  sendInputRecordingPreference(source, enabled);
  if (!state.includePointerMoves && state.selectedFrame && !isVisibleCompletedFrame(state.selectedFrame)) {
    state.selectedFrame = filteredCompletedFrames().at(-1) || null;
    state.selectedEvent = null;
  }
  renderInputRecordingControls();
  scheduleRender();
}

function inputSourceForEvent(event) {
  const label = labelFor(event.labelId);
  return availableInputSources().find((source) => label.startsWith(source.event_prefix)) || null;
}

function inputSampleForEvent(event) {
  if (event.kind !== KIND.INSTANT) return null;
  const label = labelFor(event.labelId);
  if (label.startsWith(POINTER_PIPELINE_PREFIX)) return null;
  const source = inputSourceForEvent(event);
  if (!source) return null;
  return {
    event,
    source,
    queueAge: label.endsWith(".queue_age_ns") ? event.value : null,
  };
}

function recordedInputSamples() {
  return state.events.map(inputSampleForEvent).filter((sample) => sample !== null);
}

function pointerPipelineSampleForEvent(event) {
  if (event.kind !== KIND.INSTANT) return null;
  const label = labelFor(event.labelId);
  if (!label.startsWith(POINTER_PIPELINE_PREFIX)) return null;
  return {
    event,
    name: label.slice(POINTER_PIPELINE_PREFIX.length),
    value: event.value,
    unit: labelMetadataFor(event.labelId).unit,
  };
}

function recordedPointerPipelineSamples() {
  return state.events.map(pointerPipelineSampleForEvent).filter((sample) => sample !== null);
}

function acceptBinary(buffer) {
  const view = new DataView(buffer);
  if (view.byteLength < 12 || String.fromCharCode(...new Uint8Array(buffer, 0, 4)) !== "LTPR") {
    showError("The profiler sent an invalid binary message.");
    return;
  }
  const major = view.getUint16(4, true);
  if (major !== PROTOCOL_MAJOR) {
    showError(`Unsupported protocol ${major}; viewer supports ${PROTOCOL_MAJOR}.`);
    return;
  }
  const messageKind = view.getUint8(8);
  if (messageKind === 1) decodeLabels(view);
  else if (messageKind === 2) decodeEvents(view);
  else if (messageKind === 3 && !state.paused) {
    state.dropped += view.getBigUint64(12, true);
    scheduleLiveRender();
  }
}

function decodeLabels(view) {
  const count = view.getUint32(12, true);
  let offset = 16;
  for (let index = 0; index < count; index += 1) {
    if (offset + 10 > view.byteLength) return showError("A label batch was truncated.");
    const id = view.getUint32(offset, true);
    const category = view.getUint8(offset + 4);
    const unit = view.getUint8(offset + 5);
    const aggregation = view.getUint8(offset + 6);
    const flags = view.getUint8(offset + 7);
    const length = view.getUint16(offset + 8, true);
    offset += 10;
    if (offset + length > view.byteLength) return showError("A label value was truncated.");
    const name = decoder.decode(new Uint8Array(view.buffer, view.byteOffset + offset, length));
    state.labels.set(id, { id, name, category, unit, aggregation, flags });
    offset += length;
  }
}

function decodeEvents(view) {
  if (state.paused) return;
  const count = view.getUint32(12, true);
  if (16 + count * EVENT_BYTES > view.byteLength) {
    showError("An event batch was truncated.");
    return;
  }
  let offset = 16;
  let renderNeeded = false;
  for (let index = 0; index < count; index += 1, offset += EVENT_BYTES) {
    const event = {
      sequence: view.getBigUint64(offset, true),
      timestamp: view.getBigUint64(offset + 8, true),
      duration: view.getBigUint64(offset + 16, true),
      view: view.getBigUint64(offset + 24, true),
      frame: view.getBigUint64(offset + 32, true),
      presentation: view.getBigUint64(offset + 40, true),
      value: view.getBigUint64(offset + 48, true),
      auxiliary: view.getBigUint64(offset + 56, true),
      lane: view.getUint32(offset + 64, true),
      labelId: view.getUint32(offset + 68, true),
      parentLabelId: view.getUint32(offset + 72, true),
      kind: view.getUint8(offset + 76),
      domain: view.getUint8(offset + 77),
    };
    const sequence = event.sequence.toString();
    if (state.seenSequences.has(sequence)) continue;
    state.seenSequences.add(sequence);
    renderNeeded = acceptEvent(event) || renderNeeded;
  }
  trimClientHistory();
  if (renderNeeded) scheduleLiveRender();
}

function acceptEvent(event) {
  state.events.push(event);
  const eventLabel = labelFor(event.labelId);
  if (eventLabel === "gpu.timestamps.available" && state.metadata) {
    const capabilities = state.metadata.session.capabilities;
    const unavailable = state.metadata.session.unavailable_metrics;
    const capabilityChanged = !capabilities.includes("gpu-relative-timestamps");
    const unavailableChanged = unavailable.includes("gpu-relative-timestamps");
    if (capabilityChanged) capabilities.push("gpu-relative-timestamps");
    if (unavailableChanged) {
      state.metadata.session.unavailable_metrics = unavailable.filter((value) => value !== "gpu-relative-timestamps");
    }
    if (state.workspace === "session" && (capabilityChanged || unavailableChanged)) renderSession();
  }
  const frame = frameForEvent(event);
  if (frame) {
    frame.events.push(event);
    frame.analysis = null;
  }
  if (event.kind === KIND.COUNTER) {
    normalizeCounter(event);
    state.counters.set(event.labelId, event);
  }
  if (event.kind === KIND.DIAGNOSTIC || event.kind === KIND.GAP) {
    state.diagnostics.push(event);
    if (event.kind === KIND.GAP) state.dropped += event.value;
  }
  if (!state.paused && state.followLiveSelection && event.kind === KIND.FRAME_END && frame) {
    if (isVisibleCompletedFrame(frame)) {
      state.selectedFrame = frame;
      state.selectedEvent = null;
    }
  }
  return eventAffectsActiveWorkspace(event);
}

function eventAffectsActiveWorkspace(event) {
  if (state.workspace === "resources") {
    return event.kind === KIND.COUNTER
      && (labelMetadataFor(event.labelId).flags & LABEL_FLAG_RESOURCE) !== 0;
  }
  if (state.workspace === "diagnostics") {
    return event.kind === KIND.DIAGNOSTIC || event.kind === KIND.GAP;
  }
  if (state.workspace !== "performance") return false;
  if (state.performanceView === "inputs") {
    return inputSampleForEvent(event) !== null || pointerPipelineSampleForEvent(event) !== null;
  }
  const frame = event.frame === 0n ? null : state.framesById.get(event.frame.toString());
  const visibleFrameEvent = frame !== null && isVisibleCompletedFrame(frame);
  if (state.performanceView !== "responsiveness") return visibleFrameEvent;
  const label = labelFor(event.labelId);
  return visibleFrameEvent
    || event.presentation !== 0n
    || isInputSignal(event)
    || label.startsWith("presentation.")
    || label.startsWith("responsiveness.");
}

function normalizeCounter(event) {
  const metadata = labelMetadataFor(event.labelId);
  const raw = counterNumber(event);
  event.counterRaw = raw;
  event.counterValue = raw;
  if (metadata.aggregation !== AGGREGATION.CUMULATIVE) return;
  const key = `${event.lane}:${event.labelId}`;
  const previous = state.counterBaselines.get(key);
  state.counterBaselines.set(key, raw);
  event.counterValue = previous === undefined ? null : (raw >= previous ? raw - previous : raw);
}

function frameForEvent(event) {
  if (event.frame === 0n) return null;
  const key = event.frame.toString();
  let frame = state.framesById.get(key);
  if (!frame) {
    frame = { id: event.frame, view: event.view, start: event.timestamp, duration: 0n, events: [], analysis: null };
    state.framesById.set(key, frame);
    state.frames.push(frame);
  }
  if (event.view !== 0n) frame.view = event.view;
  if (event.kind === KIND.FRAME_BEGIN) frame.start = event.timestamp;
  if (event.kind === KIND.FRAME_END) {
    frame.start = event.timestamp;
    frame.duration = event.duration;
  }
  return frame;
}

function trimClientHistory() {
  while (state.frames.length > 600) {
    const removed = state.frames.shift();
    state.framesById.delete(removed.id.toString());
    if (state.selectedFrame === removed) state.selectedFrame = state.frames[0] || null;
  }
  if (state.events.length > 60000) state.events.splice(0, state.events.length - 60000);
  if (state.diagnostics.length > 2000) state.diagnostics.splice(0, state.diagnostics.length - 2000);
  if (state.seenSequences.size > 100000) {
    state.seenSequences = new Set(state.events.map((event) => event.sequence.toString()));
  }
}

function scheduleRender() {
  if (state.liveRenderTimer !== null) {
    window.clearTimeout(state.liveRenderTimer);
    state.liveRenderTimer = null;
  }
  if (state.renderQueued) return;
  state.renderQueued = true;
  requestAnimationFrame((timestamp) => {
    state.renderQueued = false;
    if (inspectorDragActive) return;
    state.lastRenderAt = timestamp;
    if (state.workspace === "performance") renderFrames();
    else if (state.workspace === "resources") renderResources();
    else if (state.workspace === "diagnostics") renderDiagnostics();
    else if (state.workspace === "session") renderSession();
    applyStoredTableSorts(document);
  });
}

function scheduleLiveRender() {
  if (state.paused || inspectorDragActive || state.renderQueued || state.liveRenderTimer !== null) return;
  const elapsed = performance.now() - state.lastRenderAt;
  const delay = Math.max(0, LIVE_RENDER_INTERVAL_MS - elapsed);
  if (delay === 0) {
    scheduleRender();
    return;
  }
  state.liveRenderTimer = window.setTimeout(() => {
    state.liveRenderTimer = null;
    scheduleRender();
  }, delay);
}

function renderFrames() {
  renderPerformancePanels();
  if (state.performanceView === "inputs") {
    renderInputPerformance();
    renderInspector();
    return;
  }
  const rangeFrames = selectedRangeFrames();
  const rangeResponsiveness = state.performanceView === "responsiveness"
    ? analyzeResponsiveness(rangeFrames)
    : null;
  renderMetrics(rangeFrames, rangeResponsiveness);
  renderPlot();
  renderRangeAnalysis(rangeFrames, rangeResponsiveness);
  renderInspector();
  if (state.performanceView === "work") {
    renderTimeline();
  }
}

function completedFrames() {
  return state.frames.filter(isVisibleCompletedFrame);
}

function isVisibleCompletedFrame(frame) {
  return frame.duration > 0n && (state.includePointerMoves || !isPointerMoveOnlyFrame(frame));
}

function isPointerMoveOnlyFrame(frame) {
  return frame.events.some((event) => labelFor(event.labelId) === "frame.trigger.pointer_move_only"
    && (event.counterValue ?? 0) > 0);
}

function filteredCompletedFrames() {
  const completed = completedFrames();
  if (state.viewFilter === "all") return completed;
  return completed.filter((frame) => frame.view.toString() === state.viewFilter);
}

function plotViewFrames() {
  const retained = filteredCompletedFrames();
  if (state.plotView === null) {
    return state.plotFrameLimit === null
      ? retained
      : retained.slice(-state.plotFrameLimit);
  }
  const visible = retained.filter((frame) => frame.id >= state.plotView.startId && frame.id <= state.plotView.endId);
  if (visible.length > 0) return visible;
  state.plotView = null;
  state.plotViewHistory.length = 0;
  return state.plotFrameLimit === null
    ? retained
    : retained.slice(-state.plotFrameLimit);
}

function resetPlotView() {
  clearManualRangeSelection();
  state.plotView = null;
  state.plotViewHistory.length = 0;
  state.hoveredPlotPointKey = null;
  updatePlotViewControls();
  scheduleRender();
}

function restorePreviousPlotView() {
  if (state.plotViewHistory.length === 0) return;
  clearManualRangeSelection();
  state.plotView = state.plotViewHistory.pop();
  state.hoveredPlotPointKey = null;
  updatePlotViewControls();
  scheduleRender();
}

function updatePlotViewControls() {
  const back = element("plot-view-back");
  const reset = element("plot-view-reset");
  back.disabled = state.plotViewHistory.length === 0;
  reset.disabled = state.plotView === null && state.plotViewHistory.length === 0;
  const status = element("plot-view-status");
  if (state.plotView !== null) {
    status.textContent = `Zoomed to #${state.plotView.startId.toString()}–#${state.plotView.endId.toString()}`;
    return;
  }
  const retainedCount = filteredCompletedFrames().length;
  const visibleCount = state.plotFrameLimit === null
    ? retainedCount
    : Math.min(state.plotFrameLimit, retainedCount);
  status.textContent = state.plotFrameLimit === null
    ? `${visibleCount.toString()} retained frame${visibleCount === 1 ? "" : "s"}`
    : `Latest ${visibleCount.toString()} of ${retainedCount.toString()} frames`;
}

function setPlotFrameLimit(value) {
  clearManualRangeSelection();
  state.plotFrameLimit = value === "all" ? null : Number.parseInt(value, 10);
  state.plotView = null;
  state.plotViewHistory.length = 0;
  state.selectedPlotPointKey = null;
  state.hoveredPlotPointKey = null;
  scheduleRender();
}

function selectedRangeFrames() {
  if (hasManualRangeSelection()) {
    const selected = filteredCompletedFrames()
      .filter((frame) => frame.id >= state.rangeStartId && frame.id <= state.rangeEndId);
    if (selected.length > 0) return selected;
    clearManualRangeSelection();
  }
  return plotViewFrames();
}

function renderMetrics(frames, responsiveness) {
  if (state.performanceView === "responsiveness") {
    setMetric(1, "Input → present p95", formatOptionalNs(responsiveness.p95InputToPresent));
    setMetric(2, "Active frame gap", formatOptionalNs(responsiveness.maxActiveFrameGap));
    setMetric(3, "Present gap", formatOptionalNs(responsiveness.maxPresentGap));
    setMetric(4, "Retry signals", responsiveness.retryEvents.length.toString());
  } else if (state.performanceView === "work") {
    const categories = aggregatePerformanceCategories(frames);
    setMetric(1, "CPU p95/frame", formatNs(categories.get(PERFORMANCE_CATEGORY.CPU).p95));
    setMetric(2, "GPU p95/frame", formatNs(categories.get(PERFORMANCE_CATEGORY.GPU).p95));
    setMetric(3, "Present p95/frame", formatNs(categories.get(PERFORMANCE_CATEGORY.PRESENTATION).p95));
    setMetric(4, "Frames", frames.length.toString());
  } else {
    const durations = frames.map((frame) => nsToMs(frame.duration)).sort((a, b) => a - b);
    setMetric(1, "Host turn p50", percentile(durations, 0.50));
    setMetric(2, "Host turn p95", percentile(durations, 0.95));
    setMetric(3, "Host turn p99", percentile(durations, 0.99));
    setMetric(4, "Over budget", frames.filter((frame) => frame.duration > frameBudgetNs(frame)).length.toString());
  }
  setText("metric-dropped", state.dropped.toString());
  renderAnalysisScope(frames);
}

function setMetric(index, label, value) {
  setText(`metric-label-${index}`, label);
  setText(`metric-${index}`, value);
}

function percentile(values, position) {
  if (values.length === 0) return "—";
  return `${values[Math.min(values.length - 1, Math.floor((values.length - 1) * position))].toFixed(2)} ms`;
}

function renderPlot() {
  const context = plot.getContext("2d");
  const bounds = plot.getBoundingClientRect();
  const ratio = window.devicePixelRatio || 1;
  const width = Math.max(1, Math.floor(bounds.width * ratio));
  const height = Math.max(1, Math.floor(bounds.height * ratio));
  if (plot.width !== width || plot.height !== height) {
    plot.width = width;
    plot.height = height;
  }
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, bounds.width, bounds.height);
  plot._interactivePoints = [];
  const frames = plotViewFrames();
  if (frames.length === 0) {
    context.fillStyle = "#9aa2ad";
    context.fillText("Waiting for a rendered frame; idle Telorgon apps do not redraw continuously", 8, 24);
    plot._visibleFrames = [];
    plot._frameXs = [];
    renderPlotPointDetails(null);
    updatePlotViewControls();
    return;
  }
  const axis = ordinalPlotAxis(frames);
  plot._visibleFrames = frames;
  plot._frameXs = frames.map((_, index) => ordinalFrameX(index, frames.length, bounds.width));
  if (state.performanceView === "work") renderWorkPlot(context, bounds, frames, axis);
  else if (state.performanceView === "responsiveness") renderResponsivenessPlot(context, bounds, frames, axis);
  else renderFrameTrendPlot(context, bounds, frames, axis);
  renderPlotSelection(context, bounds, frames);
  renderPlotPointFeedback(context);
  renderPlotPointDetails(plotPointByKey(state.selectedPlotPointKey));
  updatePlotViewControls();
}

function setPlotLegend(first, second, third, caption, styles = ["cpu", "gpu", "presentation"]) {
  setText("plot-legend-1", first);
  setText("plot-legend-2", second);
  setText("plot-legend-3", third);
  setText("plot-caption", caption);
  plot._defaultCaption = caption;
  plot._captionFrameId = null;
  ["plot-legend-1", "plot-legend-2", "plot-legend-3"].forEach((id, index) => {
    element(id).className = `category-badge ${styles[index]}`;
  });
}

function registerPlotPoints(points) {
  plot._interactivePoints.push(...points);
}

function plotPointByKey(key) {
  if (key === null) return null;
  return (plot._interactivePoints || []).find((point) => point.key === key) || null;
}

function frameAtPlotTimestamp(frames, timestamp) {
  if (frames.length === 0) return null;
  let nearest = frames[0];
  let nearestDistance = nearest.start > timestamp ? nearest.start - timestamp : timestamp - nearest.start;
  for (let index = 1; index < frames.length; index += 1) {
    const frame = frames[index];
    const distance = frame.start > timestamp ? frame.start - timestamp : timestamp - frame.start;
    if (distance >= nearestDistance) continue;
    nearest = frame;
    nearestDistance = distance;
  }
  return nearest;
}

function renderPlotPointFeedback(context) {
  const selected = plotPointByKey(state.selectedPlotPointKey);
  const hovered = plotPointByKey(state.hoveredPlotPointKey);
  const captionPoint = hovered || selected;
  if (captionPoint) {
    const frameLabel = captionPoint.frame ? `Frame #${captionPoint.frame.id.toString()}` : "Sample";
    const label = `${frameLabel} · ${captionPoint.label}: ${formatNs(captionPoint.value)} · ${formatSessionTimestamp(captionPoint.timestamp)}`;
    setText("plot-caption", label);
    plot.title = label;
    plot._captionFrameId = null;
  } else {
    plot.removeAttribute("title");
  }
  if (selected) {
    const area = plotArea({ height: plot.height / (window.devicePixelRatio || 1) });
    context.strokeStyle = "rgba(242, 244, 247, 0.92)";
    context.lineWidth = 2;
    context.beginPath();
    context.moveTo(selected.x, area.top);
    context.lineTo(selected.x, area.bottom);
    context.stroke();
    context.fillStyle = "#101318";
    context.strokeStyle = "#f2f4f7";
    context.lineWidth = 2;
    context.beginPath();
    context.arc(selected.x, selected.y, 6, 0, Math.PI * 2);
    context.fill();
    context.stroke();
  }
  if (hovered && hovered.key !== selected?.key) {
    context.strokeStyle = hovered.color;
    context.lineWidth = 2;
    context.beginPath();
    context.arc(hovered.x, hovered.y, 5, 0, Math.PI * 2);
    context.stroke();
  }
}

function renderPlotPointDetails(point) {
  const heading = element("plot-point-heading");
  const values = element("plot-point-values");
  if (!point) {
    heading.textContent = state.selectedPlotPointKey === null ? "No point selected" : "Selected point is outside this view";
    values.textContent = "Left-click a plotted point to inspect every displayed value at that frame.";
    return;
  }
  const timestamp = point.timestamp;
  heading.textContent = point.groupByFrame
    ? `Selected frame #${point.frame.id.toString()} · ${formatSessionTimestamp(timestamp)}`
    : `Selected ${point.label.toLowerCase()} sample · ${formatSessionTimestamp(timestamp)}`;
  const siblings = (plot._interactivePoints || []).filter((candidate) => {
    if (point.groupByFrame && candidate.groupByFrame) return candidate.frame.id === point.frame.id;
    return candidate.timestamp === point.timestamp;
  });
  const displayed = [
    ...siblings.map((candidate) => ({
      label: candidate.label,
      value: candidate.value,
      category: candidate.category,
    })),
    ...(point.details || []),
  ];
  values.replaceChildren(...displayed.map((candidate, index) => {
    const fragment = document.createDocumentFragment();
    if (index > 0) fragment.append(document.createTextNode(" · "));
    const label = document.createElement("span");
    label.className = `plot-point-value category-${candidate.category}`;
    label.textContent = `${candidate.label}: ${formatNs(candidate.value)}`;
    fragment.append(label);
    return fragment;
  }));
}

function plotArea(bounds) {
  return { top: 18, bottom: bounds.height - 20, height: Math.max(1, bounds.height - 38) };
}

function plotY(value, maximum, area) {
  return area.bottom - Number(value) / Math.max(1, Number(maximum)) * area.height;
}

function drawLineSeries(context, points, color, maximum, area, alpha = 1, lineWidth = 1.5) {
  if (points.length === 0) return;
  context.strokeStyle = color;
  context.globalAlpha = alpha;
  context.lineWidth = lineWidth;
  context.beginPath();
  points.forEach((point, index) => {
    const y = plotY(point.value, maximum, area);
    if (index === 0) context.moveTo(point.x, y);
    else context.lineTo(point.x, y);
  });
  context.stroke();
  context.globalAlpha = 1;
  context.lineWidth = 1;
}

function drawPointSeries(context, points, color, maximum, area, radius = 2.25) {
  context.fillStyle = color;
  points.forEach((point) => {
    context.beginPath();
    context.arc(point.x, plotY(point.value, maximum, area), radius, 0, Math.PI * 2);
    context.fill();
  });
}

function drawBudget(context, bounds, area, budget, maximum) {
  if (budget <= 0n || budget > maximum) return;
  const y = plotY(budget, maximum, area);
  context.strokeStyle = "#59616d";
  context.setLineDash([4, 4]);
  context.beginPath();
  context.moveTo(0, y);
  context.lineTo(bounds.width, y);
  context.stroke();
  context.setLineDash([]);
  context.fillStyle = "#9aa2ad";
  context.textAlign = "left";
  context.fillText(`Budget ${formatNs(budget)}`, 4, Math.max(11, y - 4));
}

function renderFrameTrendPlot(context, bounds, frames, axis) {
  setPlotLegend(
    "Host turn duration",
    "Frame budget",
    "Recorded frame",
    "One point per completed frame in recorded order; idle time does not stretch the axis",
    ["cpu statistic-line", "cpu statistic-heavy", "cpu statistic-point"],
  );
  const area = plotArea(bounds);
  const points = frames.map((frame, index) => ({
    x: ordinalFrameX(index, frames.length, bounds.width),
    value: frame.duration,
  }));
  const maximum = frames.reduce((value, frame) => frame.duration > value ? frame.duration : value, 1n);
  const budgets = frames.map(frameBudgetNs).sort(compareBigInt);
  drawBudget(context, bounds, area, bigintPercentile(budgets, 0.50), maximum);
  drawLineSeries(context, points, PERFORMANCE_COLORS[PERFORMANCE_CATEGORY.CPU], maximum, area, 0.65, 1.25);
  drawPointSeries(context, points, PERFORMANCE_COLORS[PERFORMANCE_CATEGORY.CPU], maximum, area, 2);
  registerPlotPoints(points.map((point, index) => ({
    ...point,
    y: plotY(point.value, maximum, area),
    key: `overview:host:${frames[index].id.toString()}`,
    label: "Host turn",
    category: PERFORMANCE_CATEGORY.CPU,
    color: PERFORMANCE_COLORS[PERFORMANCE_CATEGORY.CPU],
    timestamp: frames[index].start,
    frame: frames[index],
    groupByFrame: true,
    details: [{
      label: "Frame budget",
      value: frameBudgetNs(frames[index]),
      category: PERFORMANCE_CATEGORY.CPU,
    }],
  })));
  context.fillStyle = "#9aa2ad";
  context.textAlign = "right";
  context.fillText(`0–${formatNs(maximum)} · ${plotAxisLabel(axis)}`, bounds.width - 4, 11);
}

function renderWorkPlot(context, bounds, frames, axis) {
  setPlotLegend("CPU work", "GPU work", "Presentation work", "One value per recorded frame; timestamps remain attached to each frame");
  const area = plotArea(bounds);
  const breakdowns = frames.map(framePerformanceBreakdown);
  const maximum = breakdowns.reduce((current, breakdown) => PERFORMANCE_CATEGORIES.reduce((value, category) => {
    const duration = breakdown.get(category).duration;
    return duration > value ? duration : value;
  }, current), 1n);
  PERFORMANCE_CATEGORIES.forEach((category) => {
    const points = breakdowns.map((breakdown, index) => ({
      x: ordinalFrameX(index, frames.length, bounds.width),
      value: breakdown.get(category).duration,
    }));
    drawLineSeries(context, points, PERFORMANCE_COLORS[category], maximum, area, 0.7, 1.35);
    drawPointSeries(context, points, PERFORMANCE_COLORS[category], maximum, area, 1.8);
    registerPlotPoints(points.map((point, index) => ({
      ...point,
      y: plotY(point.value, maximum, area),
      key: `work:${category}:${frames[index].id.toString()}`,
      label: `${performanceCategoryName(category)} work`,
      category,
      color: PERFORMANCE_COLORS[category],
      timestamp: frames[index].start,
      frame: frames[index],
      groupByFrame: true,
    })));
  });
  context.fillStyle = "#9aa2ad";
  context.textAlign = "right";
  context.fillText(`0–${formatNs(maximum)} self/frame · ${plotAxisLabel(axis)}`, bounds.width - 4, 11);
}

function renderResponsivenessPlot(context, bounds, frames, axis) {
  const pointerCaption = state.includePointerMoves
    ? "pointer-movement input included"
    : "pointer-movement input excluded";
  setPlotLegend(
    "Active frame gap",
    "Input → present",
    "Successful-present gap",
    `Only correlated latency samples; ${pointerCaption}; raw event occupancy is excluded`,
    ["presentation statistic-line", "presentation statistic-heavy", "presentation statistic-point"],
  );
  const area = plotArea(bounds);
  const analysis = analyzeResponsiveness(frames);
  const series = [
    {
      color: PERFORMANCE_COLORS[PERFORMANCE_CATEGORY.PRESENTATION],
      alpha: 0.38,
      radius: 1.8,
      key: "active-frame-gap",
      label: "Active frame gap",
      samples: analysis.activeFrameGaps.map((gap) => ({ timestamp: gap.end, duration: gap.activeDuration })),
    },
    {
      color: PERFORMANCE_COLORS[PERFORMANCE_CATEGORY.PRESENTATION],
      alpha: 0.68,
      radius: 2.3,
      key: "input-to-present",
      label: "Input → present",
      samples: analysis.inputToPresentSamples,
    },
    {
      color: PERFORMANCE_COLORS[PERFORMANCE_CATEGORY.PRESENTATION],
      alpha: 1,
      radius: 3,
      key: "successful-present-gap",
      label: "Successful-present gap",
      samples: analysis.activePresentGaps.map((gap) => ({ timestamp: gap.end, duration: gap.activeDuration })),
    },
  ];
  const allValues = series.flatMap((entry) => entry.samples.map((sample) => sample.duration));
  const maximum = allValues.reduce((value, sample) => sample > value ? sample : value, 1n);
  const budgets = frames.map(frameBudgetNs).sort(compareBigInt);
  drawBudget(context, bounds, area, bigintPercentile(budgets, 0.50), maximum);
  series.forEach((entry) => {
    const points = latencyEventPoints(entry.samples, axis, bounds.width);
    drawLineSeries(context, points, entry.color, maximum, area, entry.alpha, 1);
    context.globalAlpha = entry.alpha;
    drawPointSeries(context, points, entry.color, maximum, area, entry.radius);
    context.globalAlpha = 1;
    registerPlotPoints(points.map((point, index) => ({
      ...point,
      y: plotY(point.value, maximum, area),
      key: `responsiveness:${entry.key}:${point.timestamp.toString()}:${index}`,
      label: entry.label,
      category: PERFORMANCE_CATEGORY.PRESENTATION,
      color: entry.color,
      frame: frameAtPlotTimestamp(frames, point.timestamp),
    })));
  });
  if (allValues.length === 0) {
    context.fillStyle = "#9aa2ad";
    context.textAlign = "left";
    context.fillText("No correlated over-budget latency incidents in this range", 8, 34);
  }
  context.fillStyle = "#9aa2ad";
  context.textAlign = "right";
  context.fillText(`0–${formatNs(maximum)} latency · ${plotAxisLabel(axis)}`, bounds.width - 4, 11);
}

function latencyEventPoints(samples, axis, width) {
  return samples
    .filter((sample) => sample.timestamp !== null)
    .map((sample) => ({
      x: ordinalTimestampX(sample.timestamp, axis, width),
      value: sample.duration,
      timestamp: sample.timestamp,
    }))
    .sort((left, right) => left.x - right.x);
}

function renderPlotSelection(context, bounds, frames) {
  if (hasManualRangeSelection()) {
    const rangeFrames = selectedRangeFrames();
    const rangeIds = new Set(rangeFrames.map((frame) => frame.id.toString()));
    const selected = frames.filter((frame) => rangeIds.has(frame.id.toString()));
    if (selected.length > 0) {
      const selection = plotFrameSelectionBounds(frames, selected, bounds.width);
      const area = plotArea(bounds);
      context.fillStyle = "rgba(102, 157, 246, 0.12)";
      context.strokeStyle = "rgba(102, 157, 246, 0.82)";
      context.lineWidth = 1;
      context.fillRect(selection.start, area.top, selection.width, area.height);
      context.strokeRect(selection.start + 0.5, area.top + 0.5, Math.max(0, selection.width - 1), area.height - 1);
      drawRangeBoundaryGuides(context, area, selection.start, selection.start + selection.width);
      if (selection.width >= 92) {
        context.fillStyle = "#c7dcff";
        context.textAlign = "left";
        context.fillText(
          `#${rangeFrames[0].id.toString()}–#${rangeFrames.at(-1).id.toString()} · ${rangeFrames.length.toString()} frames`,
          selection.start + 6,
          area.top + 14,
        );
      }
    }
  }
  if (plot._dragStart !== undefined && plot._dragCurrent !== undefined) {
    const start = Math.min(plot._dragStart, plot._dragCurrent);
    const end = Math.max(plot._dragStart, plot._dragCurrent);
    if (plot._dragMode === "zoom") {
      const area = plotArea(bounds);
      context.fillStyle = "rgba(102, 157, 246, 0.16)";
      context.strokeStyle = "rgba(102, 157, 246, 0.95)";
      context.lineWidth = 1;
      context.fillRect(start, area.top, Math.max(1, end - start), area.height);
      context.strokeRect(start + 0.5, area.top + 0.5, Math.max(1, end - start) - 1, area.height - 1);
      context.fillStyle = "#c7dcff";
      context.textAlign = "left";
      context.fillText("Release to zoom", Math.min(bounds.width - 96, start + 6), area.top + 14);
    } else {
      const area = plotArea(bounds);
      const width = Math.max(1, end - start);
      context.fillStyle = "rgba(102, 157, 246, 0.16)";
      context.strokeStyle = "rgba(102, 157, 246, 0.95)";
      context.lineWidth = 1;
      context.fillRect(start, area.top, width, area.height);
      context.strokeRect(start + 0.5, area.top + 0.5, Math.max(0, width - 1), area.height - 1);
      drawRangeBoundaryGuides(context, area, start, end);
      context.fillStyle = "#c7dcff";
      context.textAlign = "left";
      context.fillText("Release to analyze range", Math.max(4, Math.min(bounds.width - 132, start + 6)), area.top + 14);
    }
  }
}

function drawRangeBoundaryGuides(context, area, start, end) {
  context.strokeStyle = "rgba(242, 244, 247, 0.92)";
  context.lineWidth = 2;
  [start, end].forEach((x) => {
    context.beginPath();
    context.moveTo(x, area.top);
    context.lineTo(x, area.bottom);
    context.stroke();
  });
  context.lineWidth = 1;
}

function plotFrameSelectionBounds(frames, selected, width) {
  const firstIndex = frames.indexOf(selected[0]);
  const lastIndex = frames.indexOf(selected.at(-1));
  if (frames.length <= 1) return { start: 0, width };
  const firstX = ordinalFrameX(firstIndex, frames.length, width);
  const lastX = ordinalFrameX(lastIndex, frames.length, width);
  const previousX = firstIndex > 0 ? ordinalFrameX(firstIndex - 1, frames.length, width) : firstX;
  const nextX = lastIndex < frames.length - 1 ? ordinalFrameX(lastIndex + 1, frames.length, width) : lastX;
  const start = firstIndex === 0 ? 0 : (previousX + firstX) / 2;
  const end = lastIndex === frames.length - 1 ? width : (lastX + nextX) / 2;
  return { start, width: Math.max(1, end - start) };
}

function ordinalPlotAxis(frames) {
  return { frames, start: frames[0].start, end: frames.at(-1).start };
}

function ordinalFrameX(index, count, width) {
  if (count <= 1) return width / 2;
  return index / (count - 1) * width;
}

function ordinalTimestampX(timestamp, axis, width) {
  const frames = axis.frames;
  if (frames.length <= 1) return width / 2;
  let low = 0;
  let high = frames.length;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (frames[middle].start < timestamp) low = middle + 1;
    else high = middle;
  }
  if (low === 0) return 0;
  if (low === frames.length) return width;
  if (frames[low].start === timestamp) return ordinalFrameX(low, frames.length, width);
  return ordinalFrameX(low - 0.5, frames.length, width);
}

function plotAxisLabel(axis) {
  const count = axis.frames.length;
  return `${count} frame event${count === 1 ? "" : "s"} · ${formatSessionTimestamp(axis.start)}–${formatSessionTimestamp(axis.end)}`;
}

function wallClockDomain(frames) {
  const start = frames[0].start;
  let end = frames.at(-1).start + frames.at(-1).duration;
  if (end <= start) end = start + 1n;
  return { start, end };
}

function selectedWallClockDomain(frames) {
  if (frames.length === 0) return null;
  const completed = filteredCompletedFrames();
  const lastIndex = completed.indexOf(frames.at(-1));
  const nextFrame = lastIndex >= 0 ? completed[lastIndex + 1] : null;
  const retained = wallClockDomain(completed);
  const end = nextFrame?.start ?? retained.end;
  return { start: frames[0].start, end: end > frames[0].start ? end : frames[0].start + 1n };
}

function eventWallTimestamp(event) {
  if (event.domain !== 2) return event.timestamp;
  const frame = event.frame ? state.framesById.get(event.frame.toString()) : null;
  return frame ? frame.start + event.timestamp : null;
}

function successfulPresentEvents(events) {
  return events.filter((event) => {
    const label = labelFor(event.labelId);
    return label === "presentation.presented" || label === "presentation.presented_suboptimal";
  });
}

function framePerformanceBreakdown(frame) {
  const result = new Map(PERFORMANCE_CATEGORIES.map((category) => [category, { duration: 0n, events: 0 }]));
  const analysis = analyzeFrame(frame);
  analysis.spans.forEach((event) => {
    const category = performanceCategoryFor(event);
    result.get(category).duration += event.selfTime ?? event.duration;
  });
  frame.events.forEach((event) => { result.get(performanceCategoryFor(event)).events += 1; });
  return result;
}

function aggregatePerformanceCategories(frames) {
  const result = new Map(PERFORMANCE_CATEGORIES.map((category) => [category, {
    category,
    values: [],
    p50: 0n,
    p95: 0n,
    p99: 0n,
    max: 0n,
    total: 0n,
    events: 0,
    framesWithWork: 0,
  }]));
  frames.forEach((frame) => {
    const breakdown = framePerformanceBreakdown(frame);
    PERFORMANCE_CATEGORIES.forEach((category) => {
      const aggregate = result.get(category);
      const value = breakdown.get(category);
      aggregate.values.push(value.duration);
      aggregate.total += value.duration;
      aggregate.max = value.duration > aggregate.max ? value.duration : aggregate.max;
      aggregate.events += value.events;
      if (value.duration > 0n || value.events > 0) aggregate.framesWithWork += 1;
    });
  });
  result.forEach((aggregate) => {
    const sorted = [...aggregate.values].sort(compareBigInt);
    aggregate.p50 = bigintPercentile(sorted, 0.50);
    aggregate.p95 = bigintPercentile(sorted, 0.95);
    aggregate.p99 = bigintPercentile(sorted, 0.99);
  });
  return result;
}

function analyzeResponsiveness(frames) {
  const domain = selectedWallClockDomain(frames);
  if (!domain) return emptyResponsivenessAnalysis();
  const selectedViews = new Set(frames.map((frame) => frame.view.toString()));
  const events = state.events.filter((event) => {
    const timestamp = eventWallTimestamp(event);
    return timestamp !== null
      && timestamp >= domain.start
      && timestamp <= domain.end
      && selectedViews.has(eventView(event).toString())
      && (event.frame === 0n
        || state.includePointerMoves
        || !isPointerMoveOnlyFrame(state.framesById.get(event.frame.toString()) || { events: [] }));
  });
  const eventsByPresentation = new Map();
  events.forEach((event) => {
    if (!event.presentation) return;
    const key = event.presentation.toString();
    if (!eventsByPresentation.has(key)) eventsByPresentation.set(key, []);
    eventsByPresentation.get(key).push(event);
  });
  const attempts = events
    .filter((event) => event.kind === KIND.PRESENTATION_END)
    .map((endEvent) => presentationAttempt(endEvent, eventsByPresentation.get(endEvent.presentation.toString()) || []))
    .sort((left, right) => compareBigInt(left.start, right.start));
  const retryEvents = events.filter((event) => labelFor(event.labelId).startsWith("presentation.retry."));
  const framePoints = frames.map((frame) => ({
    timestamp: frame.start,
    label: `Frame #${frame.id.toString()}`,
    frame,
    view: frame.view,
  }));
  const frameGaps = consecutiveGapsByView(framePoints, "Frame start gap", events);
  const presents = successfulPresentEvents(events)
    .map((event) => ({
      timestamp: eventWallTimestamp(event),
      label: `Presentation #${event.presentation.toString()}`,
      event,
      frame: event.frame ? state.framesById.get(event.frame.toString()) : null,
      view: eventView(event),
    }))
    .sort((left, right) => compareBigInt(left.timestamp, right.timestamp));
  const presentGaps = consecutiveGapsByView(presents, "Successful present gap", events);
  const inputSignals = events.filter((event) => {
    const label = labelFor(event.labelId);
    return isInputSignal(event)
      || label === "responsiveness.resize.started"
      || label === "responsiveness.resize.updating"
      || label === "responsiveness.resize.ended"
      || label === "responsiveness.resize.native_ended";
  });
  const inputToPresentSamples = correlateSignalsToNextPoint(inputSignals, presents);
  const inputToPresent = inputToPresentSamples.map((sample) => sample.duration);
  const resizeReleases = inputSignals.filter((event) => labelFor(event.labelId) === "responsiveness.resize.native_ended");
  const releaseToFrame = pointDelays(resizeReleases, framePoints);
  const releaseToPresent = pointDelays(resizeReleases, presents);
  const activeFrameGaps = frameGaps.filter((gap) => gap.activeDuration !== null && gap.activeDuration > gap.budget);
  const activePresentGaps = presentGaps.filter((gap) => gap.activeDuration !== null && gap.activeDuration > gap.budget);
  const attemptDurations = attempts.map((attempt) => attempt.duration).sort(compareBigInt);
  return {
    domain,
    events,
    attempts,
    retryEvents,
    frameGaps,
    presentGaps,
    activeFrameGaps,
    activePresentGaps,
    inputToPresentSamples,
    p95AttemptDuration: attemptDurations.length ? bigintPercentile(attemptDurations, 0.95) : null,
    maxActiveFrameGap: activeFrameGaps.length ? activeFrameGaps.reduce((maximum, gap) => gap.activeDuration > maximum ? gap.activeDuration : maximum, 0n) : null,
    maxPresentGap: activePresentGaps.length ? activePresentGaps.reduce((maximum, gap) => gap.activeDuration > maximum ? gap.activeDuration : maximum, 0n) : null,
    p95InputToPresent: inputToPresent.length ? bigintPercentile(inputToPresent, 0.95) : null,
    maxInputToPresent: inputToPresent.length ? inputToPresent.at(-1) : null,
    p95ResizeReleaseToFrame: releaseToFrame.length ? bigintPercentile(releaseToFrame, 0.95) : null,
    maxResizeReleaseToFrame: releaseToFrame.length ? releaseToFrame.at(-1) : null,
    p95ResizeReleaseToPresent: releaseToPresent.length ? bigintPercentile(releaseToPresent, 0.95) : null,
    maxResizeReleaseToPresent: releaseToPresent.length ? releaseToPresent.at(-1) : null,
  };
}

function emptyResponsivenessAnalysis() {
  return {
    domain: null,
    events: [],
    attempts: [],
    retryEvents: [],
    frameGaps: [],
    presentGaps: [],
    activeFrameGaps: [],
    activePresentGaps: [],
    inputToPresentSamples: [],
    p95AttemptDuration: null,
    maxActiveFrameGap: null,
    maxPresentGap: null,
    p95InputToPresent: null,
    maxInputToPresent: null,
    p95ResizeReleaseToFrame: null,
    maxResizeReleaseToFrame: null,
    p95ResizeReleaseToPresent: null,
    maxResizeReleaseToPresent: null,
  };
}

function pointDelays(signals, points) {
  return correlateSignalsToNextPoint(signals, points).map((sample) => sample.duration);
}

function correlateSignalsToNextPoint(signals, points) {
  const signalsByView = new Map();
  const pointsByView = new Map();
  signals.forEach((event) => {
    const timestamp = eventWallTimestamp(event);
    if (timestamp === null) return;
    const key = eventView(event).toString();
    if (!signalsByView.has(key)) signalsByView.set(key, []);
    signalsByView.get(key).push({ event, timestamp });
  });
  points.forEach((point) => {
    if (point.timestamp === null) return;
    const key = point.view.toString();
    if (!pointsByView.has(key)) pointsByView.set(key, []);
    pointsByView.get(key).push(point);
  });

  const samples = [];
  for (const [key, viewSignals] of signalsByView) {
    const viewPoints = pointsByView.get(key);
    if (!viewPoints || viewPoints.length === 0) continue;
    viewSignals.sort((left, right) => compareBigInt(left.timestamp, right.timestamp));
    viewPoints.sort((left, right) => compareBigInt(left.timestamp, right.timestamp));
    let pointIndex = 0;
    viewSignals.forEach((signal) => {
      while (pointIndex < viewPoints.length && viewPoints[pointIndex].timestamp < signal.timestamp) pointIndex += 1;
      const next = viewPoints[pointIndex];
      if (next) samples.push({ timestamp: next.timestamp, duration: next.timestamp - signal.timestamp, event: signal.event });
    });
  }
  return samples.sort((left, right) => compareBigInt(left.duration, right.duration));
}

function eventView(event) {
  if (event.view !== 0n) return event.view;
  if (event.frame !== 0n) return state.framesById.get(event.frame.toString())?.view ?? 0n;
  return 0n;
}

function presentationAttempt(endEvent, events) {
  const labelled = new Map(events.map((event) => [labelFor(event.labelId), event]));
  const outcomes = [
    ["presentation.surface_lost", "Surface lost"],
    ["presentation.needs_reconfigure", "Needs reconfigure"],
    ["presentation.retry.frame_slot", "Frame-slot retry"],
    ["presentation.retry.acquire_not_ready", "Acquire retry"],
    ["presentation.retry.reconfigure", "Reconfigure retry"],
    ["presentation.idle", "Idle"],
    ["presentation.presented_suboptimal", "Presented suboptimal"],
    ["presentation.presented", "Presented"],
    ["presentation.resize.preview_presented", "Resize preview"],
  ];
  const outcome = outcomes.find(([label]) => labelled.has(label));
  const event = outcome ? labelled.get(outcome[0]) : endEvent;
  const frameEvent = events.find((candidate) => candidate.frame !== 0n);
  return {
    id: endEvent.presentation,
    start: endEvent.timestamp,
    duration: endEvent.duration,
    result: outcome?.[1] || "Attempt complete",
    event,
    frame: frameEvent ? state.framesById.get(frameEvent.frame.toString()) || null : null,
  };
}

function consecutiveGapsByView(points, kind, events) {
  const views = new Map();
  const eventsByView = new Map();
  points.forEach((point) => {
    const key = point.view.toString();
    if (!views.has(key)) views.set(key, []);
    views.get(key).push(point);
  });
  events.forEach((event) => {
    const timestamp = eventWallTimestamp(event);
    if (timestamp === null) return;
    const key = eventView(event).toString();
    if (!eventsByView.has(key)) eventsByView.set(key, []);
    eventsByView.get(key).push({ event, timestamp });
  });
  eventsByView.forEach((viewEvents) => viewEvents.sort((left, right) => compareBigInt(left.timestamp, right.timestamp)));
  return [...views.entries()]
    .flatMap(([key, viewPoints]) => consecutiveGaps(
      viewPoints.sort((left, right) => compareBigInt(left.timestamp, right.timestamp)),
      kind,
      eventsByView.get(key) || [],
    ))
    .sort((left, right) => compareBigInt(left.start, right.start));
}

function consecutiveGaps(points, kind, timedEvents) {
  const result = [];
  let eventIndex = 0;
  let latestResize = null;
  for (let index = 1; index < points.length; index += 1) {
    const previous = points[index - 1];
    const current = points[index];
    if (previous.timestamp === null || current.timestamp === null || current.timestamp <= previous.timestamp) continue;
    const duration = current.timestamp - previous.timestamp;
    const budget = previous.frame ? frameBudgetNs(previous.frame) : 16_667_000n;
    while (eventIndex < timedEvents.length && timedEvents[eventIndex].timestamp <= previous.timestamp) {
      const candidate = timedEvents[eventIndex];
      if (isResizeSignal(candidate.event)) latestResize = candidate;
      eventIndex += 1;
    }
    let activity = latestResize !== null
      && labelFor(latestResize.event.labelId) === "responsiveness.resize.native_started"
      ? { event: latestResize.event, timestamp: previous.timestamp }
      : null;
    while (eventIndex < timedEvents.length && timedEvents[eventIndex].timestamp < current.timestamp) {
      const candidate = timedEvents[eventIndex];
      if (isResizeSignal(candidate.event)) latestResize = candidate;
      if (isGapActivity(candidate.event, kind)) activity = candidate;
      eventIndex += 1;
    }
    result.push({
      kind,
      start: previous.timestamp,
      end: current.timestamp,
      timestamp: current.timestamp,
      duration,
      budget,
      active: activity !== null,
      activeDuration: activity === null ? null : current.timestamp - activity.timestamp,
      from: activity === null ? previous.label : activityLabel(activity.event),
      to: current.label,
      frame: current.frame || null,
      event: current.event || null,
      view: current.view,
    });
  }
  return result;
}

function isResizeSignal(event) {
  const label = labelFor(event.labelId);
  return label === "responsiveness.resize.native_started" || label === "responsiveness.resize.native_ended";
}

function isGapActivity(event, kind) {
  const label = labelFor(event.labelId);
  if (label === "responsiveness.resize.native_started" || label === "responsiveness.resize.native_ended") return true;
  if (kind === "Successful present gap") {
    return event.kind === KIND.FRAME_BEGIN
      || isInputSignal(event)
      || label.startsWith("presentation.retry.");
  }
  return false;
}

function isInputSignal(event) {
  const expected = state.includePointerMoves
    ? "input.events.received"
    : "input.non_pointer_events.received";
  return labelFor(event.labelId) === expected && (event.counterValue ?? 0) > 0;
}

function activityLabel(event) {
  const label = labelFor(event.labelId);
  if (label === "responsiveness.resize.native_started") return "Resize started";
  if (label === "responsiveness.resize.native_ended") return "Resize released";
  if (label === "input.events.received" || label === "input.non_pointer_events.received") return "Input received";
  return label;
}

function renderAnalysisScope(frames) {
  renderViewFilter();
  if (frames.length === 0) {
    setText("range-label", "Waiting for frames");
  } else {
    const view = state.viewFilter === "all" ? "all views" : viewName(BigInt(state.viewFilter));
    const scope = hasManualRangeSelection() ? "Selected range" : "Visible graph";
    setText("range-label", `${scope} · #${frames[0].id.toString()}–#${frames.at(-1).id.toString()} · ${frames.length} frame${frames.length === 1 ? "" : "s"} · ${view}`);
  }
  element("clear-analysis-range").hidden = !hasManualRangeSelection();
}

function renderViewFilter() {
  const select = element("view-filter");
  const views = [...new Set(completedFrames().map((frame) => frame.view.toString()))]
    .sort((left, right) => compareBigInt(BigInt(left), BigInt(right)));
  const expected = ["all", ...views];
  const current = [...select.options].map((option) => option.value);
  if (expected.length !== current.length || expected.some((value, index) => value !== current[index])) {
    select.replaceChildren();
    expected.forEach((value) => {
      const option = document.createElement("option");
      option.value = value;
      option.textContent = value === "all" ? "All views" : viewName(BigInt(value));
      select.append(option);
    });
  }
  if (!expected.includes(state.viewFilter)) state.viewFilter = "all";
  select.value = state.viewFilter;
}

function viewName(view) {
  if (view === 0n) return "Unscoped";
  const role = state.views.get(view.toString());
  return role ? `${role} · #${view.toString()}` : `View #${view.toString()}`;
}

function renderRangeAnalysis(frames, responsiveness) {
  if (state.performanceView === "overview") {
    renderFrameDistribution(frames);
    renderCategorySummary(frames);
    renderSlowFrames(frames);
  } else if (state.performanceView === "work") {
    renderHotspots(frames);
    renderRangeWork(frames);
  } else {
    renderResponsivenessSummary(responsiveness);
    renderPresentationAttempts(responsiveness);
    renderResponsivenessGaps(responsiveness);
  }
}

function renderPerformancePanels() {
  const panels = {
    overview: element("overview-panels"),
    work: element("work-panels"),
    responsiveness: element("responsiveness-panels"),
    inputs: element("inputs-panels"),
  };
  Object.entries(panels).forEach(([view, panel]) => { panel.hidden = state.performanceView !== view; });
  const inputsVisible = state.performanceView === "inputs";
  element("frame-plot-region").hidden = inputsVisible;
  element("analysis-range-toolbar").hidden = inputsVisible;
  document.querySelectorAll(".performance-view-button").forEach((button) => {
    const current = button.dataset.performanceView === state.performanceView;
    button.classList.toggle("current", current);
    button.setAttribute("aria-pressed", String(current));
  });
}

function renderInputPerformance() {
  const samples = recordedInputSamples();
  const pipelineSamples = recordedPointerPipelineSamples();
  const enabled = state.inputRecordingSources.size;
  const queueAges = samples
    .map((sample) => sample.queueAge)
    .filter((value) => value !== null)
    .sort(compareBigInt);
  const overTwentyMilliseconds = queueAges.filter((value) => value >= 20_000_000n).length;
  setMetric(1, "Recorded events", samples.length.toString());
  setMetric(2, "Sources enabled", enabled.toString());
  setMetric(3, "Queue age p95", queueAges.length ? formatNs(bigintPercentile(queueAges, 0.95)) : "—");
  setMetric(4, "Queue age ≥ 20 ms", overTwentyMilliseconds.toString());
  setText("metric-dropped", state.dropped.toString());
  renderInputSummary(samples);
  renderPointerPipeline(pipelineSamples);
  renderPointerPaths(pipelineSamples);
  renderInputEventPlot(samples);
  renderInputEventLog(samples);
}

function renderInputSummary(samples) {
  const body = element("input-summary-body");
  body.replaceChildren();
  availableInputSources().forEach((source) => {
    const sourceSamples = samples.filter((sample) => sample.source.id === source.id);
    if (sourceSamples.length === 0 && !state.inputRecordingSources.has(source.id)) return;
    const queueAges = sourceSamples
      .map((sample) => sample.queueAge)
      .filter((value) => value !== null)
      .sort(compareBigInt);
    const row = document.createElement("tr");
    row.append(
      tableCell(source.label),
      tableCell(sourceSamples.length.toString()),
      tableCell(queueAges.length ? formatNs(bigintPercentile(queueAges, 0.95)) : "—"),
      tableCell(queueAges.length ? formatNs(queueAges.at(-1)) : "—"),
    );
    body.append(row);
  });
  element("input-summary-empty").hidden = body.childElementCount !== 0;
}

const POINTER_PIPELINE_ORDER = Object.freeze([
  "batch_interval_ns",
  "fd_callback_to_dispatch_ns",
  "dispatch_duration_ns",
  "batch_handler_duration_ns",
  "cursor_ioctl_duration_ns",
  "freshest_event_to_cursor_submit_ns",
  "oldest_event_to_cursor_submit_ns",
  "event_to_primary_submit_ns",
  "event_to_primary_scanout_ns",
  "events_per_batch",
]);

const POINTER_PATH_ORDER = Object.freeze([
  "hardware_cursor",
  "software_damage",
  "deferred",
  "hidden",
  "hardware_failed",
  "position_unchanged",
]);

function renderPointerPipeline(samples) {
  const body = element("pointer-pipeline-body");
  body.replaceChildren();
  const measurements = new Map();
  samples.filter((sample) => !sample.name.startsWith("path.")).forEach((sample) => {
    if (!measurements.has(sample.name)) measurements.set(sample.name, []);
    measurements.get(sample.name).push(sample.value);
  });
  const names = [...measurements.keys()].sort((left, right) => {
    const leftIndex = POINTER_PIPELINE_ORDER.indexOf(left);
    const rightIndex = POINTER_PIPELINE_ORDER.indexOf(right);
    if (leftIndex < 0 && rightIndex < 0) return left.localeCompare(right);
    if (leftIndex < 0) return 1;
    if (rightIndex < 0) return -1;
    return leftIndex - rightIndex;
  });
  names.forEach((name) => {
    const values = measurements.get(name).sort(compareBigInt);
    const format = (value) => pointerPipelineValue(name, value);
    const row = document.createElement("tr");
    row.append(
      tableCell(pointerPipelineMeasurementName(name)),
      tableCell(format(bigintPercentile(values, 0.50))),
      tableCell(format(bigintPercentile(values, 0.95))),
      tableCell(format(values.at(-1))),
      tableCell(values.length.toString()),
    );
    body.append(row);
  });
  element("pointer-pipeline-empty").hidden = body.childElementCount !== 0;
}

function renderPointerPaths(samples) {
  const body = element("pointer-path-body");
  body.replaceChildren();
  const counts = new Map();
  samples.filter((sample) => sample.name.startsWith("path.")).forEach((sample) => {
    const path = sample.name.slice("path.".length);
    counts.set(path, (counts.get(path) || 0) + 1);
  });
  const total = [...counts.values()].reduce((sum, count) => sum + count, 0);
  const paths = [...counts.keys()].sort((left, right) => {
    const leftIndex = POINTER_PATH_ORDER.indexOf(left);
    const rightIndex = POINTER_PATH_ORDER.indexOf(right);
    if (leftIndex < 0 && rightIndex < 0) return left.localeCompare(right);
    if (leftIndex < 0) return 1;
    if (rightIndex < 0) return -1;
    return leftIndex - rightIndex;
  });
  paths.forEach((path) => {
    const count = counts.get(path);
    const row = document.createElement("tr");
    row.append(
      tableCell(pointerPathName(path)),
      tableCell(count.toString()),
      tableCell(formatPercentage(count, total)),
    );
    body.append(row);
  });
  element("pointer-path-empty").hidden = body.childElementCount !== 0;
}

function pointerPipelineValue(name, value) {
  return name.endsWith("_ns") ? formatNs(value) : value.toString();
}

function pointerPipelineMeasurementName(name) {
  return ({
    batch_interval_ns: "Interval between pointer batches",
    fd_callback_to_dispatch_ns: "Input-fd callback → libinput dispatch",
    dispatch_duration_ns: "libinput dispatch",
    batch_handler_duration_ns: "Batch handling to cursor decision",
    cursor_ioctl_duration_ns: "Hardware cursor ioctl",
    freshest_event_to_cursor_submit_ns: "Freshest event → cursor submit",
    oldest_event_to_cursor_submit_ns: "Oldest event → cursor submit",
    event_to_primary_submit_ns: "Pointer event → primary submit",
    event_to_primary_scanout_ns: "Pointer event → primary scanout",
    events_per_batch: "Motion events per batch",
  })[name] || name.replaceAll("_", " ");
}

function pointerPathName(path) {
  return ({
    hardware_cursor: "Hardware cursor",
    software_damage: "Software cursor damage",
    deferred: "Deferred until modeset/image",
    hidden: "Cursor hidden",
    hardware_failed: "Hardware cursor failed",
    position_unchanged: "Position unchanged/locked",
  })[path] || path.replaceAll("_", " ");
}

function renderInputEventPlot(samples) {
  const canvas = element("input-event-plot");
  const context = canvas.getContext("2d");
  const bounds = canvas.getBoundingClientRect();
  const ratio = window.devicePixelRatio || 1;
  const width = Math.max(1, Math.round(bounds.width * ratio));
  const height = Math.max(1, Math.round(bounds.height * ratio));
  if (canvas.width !== width || canvas.height !== height) {
    canvas.width = width;
    canvas.height = height;
  }
  context.setTransform(ratio, 0, 0, ratio, 0, 0);
  context.clearRect(0, 0, bounds.width, bounds.height);
  const sources = availableInputSources();
  const visible = samples.slice(-300);
  if (sources.length === 0 || visible.length === 0) {
    context.fillStyle = "#9aa2ad";
    context.fillText("Enable a native input source, then interact with the target.", 8, 24);
    return;
  }
  const left = Math.min(150, Math.max(92, bounds.width * 0.28));
  const right = Math.max(left + 1, bounds.width - 8);
  const top = 12;
  const rowHeight = Math.max(18, (bounds.height - top - 8) / sources.length);
  const first = visible[0].event.timestamp;
  const last = visible.at(-1).event.timestamp;
  const duration = last > first ? last - first : 1n;
  context.font = "10px Inter, ui-sans-serif, system-ui, sans-serif";
  context.textBaseline = "middle";
  sources.forEach((source, index) => {
    const y = top + rowHeight * (index + 0.5);
    context.strokeStyle = "#2a2f37";
    context.beginPath();
    context.moveTo(left, y);
    context.lineTo(right, y);
    context.stroke();
    context.fillStyle = "#9aa2ad";
    context.fillText(source.label, 4, y);
  });
  visible.forEach((sample) => {
    const sourceIndex = sources.findIndex((source) => source.id === sample.source.id);
    if (sourceIndex < 0) return;
    const x = left + Number((sample.event.timestamp - first) * 10_000n / duration) / 10_000 * (right - left);
    const y = top + rowHeight * (sourceIndex + 0.5);
    const age = sample.queueAge ?? 0n;
    context.fillStyle = age >= 25_000_000n ? "#e36b6b" : (age >= 8_000_000n ? "#e5a14a" : "#669df6");
    context.beginPath();
    context.arc(x, y, age >= 20_000_000n ? 3.5 : 2.5, 0, Math.PI * 2);
    context.fill();
  });
}

function renderInputEventLog(samples) {
  const body = element("input-event-log-body");
  body.replaceChildren();
  samples.slice(-250).reverse().forEach((sample) => {
    const row = document.createElement("tr");
    const eventCell = document.createElement("td");
    eventCell.append(linkButton(inputEventName(sample), () => selectProfilerEvent(sample.event, null)));
    row.append(
      tableCell(formatSessionTimestamp(sample.event.timestamp)),
      tableCell(sample.source.label),
      eventCell,
      tableCell(sample.queueAge === null ? "—" : formatNs(sample.queueAge)),
      tableCell(sample.event.frame === 0n ? "—" : `#${sample.event.frame.toString()}`),
    );
    body.append(row);
  });
  element("input-event-log-empty").hidden = body.childElementCount !== 0;
}

function inputEventName(sample) {
  const suffix = labelFor(sample.event.labelId)
    .slice(sample.source.event_prefix.length)
    .replace(/^\./, "")
    .replace(/\.queue_age_ns$/, "")
    .replaceAll(".", " ")
    .replaceAll("_", " ");
  return suffix || "event";
}

function renderCategorySummary(frames) {
  const body = element("category-summary-body");
  body.replaceChildren();
  const aggregates = aggregatePerformanceCategories(frames);
  PERFORMANCE_CATEGORIES.forEach((category) => {
    const aggregate = aggregates.get(category);
    if (aggregate.events === 0 && aggregate.total === 0n) return;
    const row = document.createElement("tr");
    row.append(
      categoryTableCell(performanceCategoryName(category), category),
      tableCell(formatNs(aggregate.p50)),
      tableCell(formatNs(aggregate.p95)),
      tableCell(formatNs(aggregate.max)),
      tableCell(formatPercentage(aggregate.framesWithWork, frames.length)),
    );
    body.append(row);
  });
  element("category-summary-empty").hidden = body.childElementCount !== 0;
}

function renderFrameDistribution(frames) {
  const body = element("frame-distribution-body");
  body.replaceChildren();
  const sorted = frames.map((frame) => frame.duration).sort(compareBigInt);
  const p50 = bigintPercentile(sorted, 0.50);
  const p95 = bigintPercentile(sorted, 0.95);
  const misses = frames.filter((frame) => frame.duration > frameBudgetNs(frame)).length;
  const views = new Set(frames.map((frame) => frame.view.toString()));
  const entries = [
    ["Frames sampled", frames.length.toString()],
    ["Views represented", views.size.toString()],
    ["Median host turn", frames.length ? formatNs(p50) : "—"],
    ["p95 host turn", frames.length ? formatNs(p95) : "—"],
    ["p99 host turn", frames.length ? formatNs(bigintPercentile(sorted, 0.99)) : "—"],
    ["Maximum host turn", frames.length ? formatNs(sorted.at(-1)) : "—"],
    ["Budget misses", `${misses} (${formatPercentage(misses, frames.length)})`],
    ["Tail spread (p95 − median)", frames.length ? formatNs(p95 - p50) : "—"],
  ];
  entries.forEach(([name, value]) => {
    const row = document.createElement("tr");
    row.append(tableCell(name), tableCell(value));
    body.append(row);
  });
}

function renderSlowFrames(frames) {
  const body = element("slow-frames-body");
  body.replaceChildren();
  const important = [...frames]
    .sort((left, right) => {
      const leftScore = Number(left.duration) / Math.max(1, Number(frameBudgetNs(left)));
      const rightScore = Number(right.duration) / Math.max(1, Number(frameBudgetNs(right)));
      return rightScore - leftScore;
    })
    .slice(0, 12);
  important.forEach((frame) => {
    const breakdown = framePerformanceBreakdown(frame);
    const dominant = PERFORMANCE_CATEGORIES.reduce((current, category) => (
      breakdown.get(category).duration > breakdown.get(current).duration ? category : current
    ), PERFORMANCE_CATEGORY.CPU);
    const sameView = completedFrames().filter((candidate) => candidate.view === frame.view);
    const index = sameView.indexOf(frame);
    const previous = index > 0 ? sameView[index - 1] : null;
    const previousEnd = previous ? previous.start + previous.duration : frame.start;
    const priorGap = frame.start > previousEnd ? frame.start - previousEnd : 0n;
    const budget = frameBudgetNs(frame);
    const frameCell = document.createElement("td");
    const link = linkButton(`#${frame.id.toString()}`, () => selectFrame(frame));
    link.title = viewName(frame.view);
    frameCell.append(link);
    const dominantCell = categoryTableCell(performanceCategoryName(dominant), dominant);
    const row = document.createElement("tr");
    if (frame.duration > budget) row.className = "incident-row";
    row.append(
      frameCell,
      tableCell(formatNs(frame.duration)),
      tableCell(`${formatNs(budget)} · ${formatRatio(frame.duration, budget)}`),
      tableCell(formatNs(breakdown.get(PERFORMANCE_CATEGORY.CPU).duration)),
      tableCell(formatNs(breakdown.get(PERFORMANCE_CATEGORY.GPU).duration)),
      tableCell(formatNs(breakdown.get(PERFORMANCE_CATEGORY.PRESENTATION).duration)),
      dominantCell,
      tableCell(formatNs(priorGap)),
    );
    body.append(row);
  });
  element("slow-frames-empty").hidden = important.length !== 0;
}

function renderResponsivenessSummary(analysis) {
  const body = element("responsiveness-summary-body");
  body.replaceChildren();
  const unframed = analysis.attempts.filter((attempt) => !attempt.frame).length;
  const successful = analysis.attempts.filter((attempt) => attempt.result.startsWith("Presented") || attempt.result === "Resize preview").length;
  [
    ["Presentation attempts", analysis.attempts.length.toString()],
    ["Successful presents", successful.toString()],
    ["Retry signals", analysis.retryEvents.length.toString()],
    ["Unframed attempts", unframed.toString()],
    ["p95 attempt duration", formatOptionalNs(analysis.p95AttemptDuration)],
    ["p95 input/resize → present", formatOptionalNs(analysis.p95InputToPresent)],
    ["Max input/resize → present", formatOptionalNs(analysis.maxInputToPresent)],
    ["p95 resize release → frame", formatOptionalNs(analysis.p95ResizeReleaseToFrame)],
    ["Max resize release → frame", formatOptionalNs(analysis.maxResizeReleaseToFrame)],
    ["p95 resize release → present", formatOptionalNs(analysis.p95ResizeReleaseToPresent)],
    ["Max resize release → present", formatOptionalNs(analysis.maxResizeReleaseToPresent)],
    ["Longest active frame gap", formatOptionalNs(analysis.maxActiveFrameGap)],
    ["Longest successful-present gap", formatOptionalNs(analysis.maxPresentGap)],
  ].forEach(([name, value]) => {
    const row = document.createElement("tr");
    row.append(tableCell(name), tableCell(value));
    body.append(row);
  });
}

function renderPresentationAttempts(analysis) {
  const body = element("presentation-attempts-body");
  body.replaceChildren();
  const outcomes = new Map();
  analysis.attempts.forEach((attempt) => {
    if (!outcomes.has(attempt.result)) outcomes.set(attempt.result, []);
    outcomes.get(attempt.result).push(attempt.duration);
  });
  [...outcomes.entries()]
    .map(([result, durations]) => ({ result, durations: durations.sort(compareBigInt) }))
    .sort((left, right) => right.durations.length - left.durations.length)
    .forEach((outcome) => {
    const row = document.createElement("tr");
    row.append(
      tableCell(outcome.result),
      categoryTableCell(performanceCategoryName(PERFORMANCE_CATEGORY.PRESENTATION), PERFORMANCE_CATEGORY.PRESENTATION),
      tableCell(outcome.durations.length.toString()),
      tableCell(formatPercentage(outcome.durations.length, analysis.attempts.length)),
      tableCell(formatNs(bigintPercentile(outcome.durations, 0.95))),
      tableCell(formatNs(outcome.durations.at(-1))),
    );
    body.append(row);
  });
  element("presentation-attempts-empty").hidden = analysis.attempts.length !== 0;
}

function renderResponsivenessGaps(analysis) {
  const body = element("responsiveness-gaps-body");
  body.replaceChildren();
  const gaps = [...analysis.activeFrameGaps, ...analysis.activePresentGaps]
    .sort((left, right) => compareBigInt(right.activeDuration, left.activeDuration))
    .slice(0, 30);
  gaps.forEach((gap) => {
    const row = document.createElement("tr");
    const name = gap.kind === "Frame start gap" && gap.active ? "Active frame gap" : gap.kind;
    const nameCell = document.createElement("td");
    if (gap.frame) {
      nameCell.append(linkButton(name, () => selectFrame(gap.frame)));
    } else if (gap.event) {
      nameCell.append(linkButton(name, () => selectProfilerEvent(gap.event, null)));
    } else {
      nameCell.textContent = name;
    }
    row.append(
      nameCell,
      categoryTableCell(performanceCategoryName(PERFORMANCE_CATEGORY.PRESENTATION), PERFORMANCE_CATEGORY.PRESENTATION),
      tableCell(formatNs(gap.activeDuration)),
      tableCell(gap.from),
      tableCell(gap.to),
    );
    body.append(row);
  });
  element("responsiveness-gaps-empty").hidden = gaps.length !== 0;
}

function renderHotspots(frames) {
  const body = element("hotspots-body");
  body.replaceChildren();
  const groups = aggregateHotspots(frames);
  const totalWork = groups.reduce((sum, group) => sum + group.totalSelf, 0n);
  const threshold = Math.max(50_000, Number(totalWork) * 0.005);
  const visible = [];
  const hidden = [];
  groups.forEach((group, index) => {
    if (visible.length < 20 && (index < 8 || Number(group.totalSelf) >= threshold)) visible.push(group);
    else hidden.push(group);
  });
  if (hidden.length > 0) visible.push(combineOtherHotspots(hidden, frames));
  visible.forEach((group) => {
    const row = document.createElement("tr");
    const selectable = group.labelId !== null;
    row.dataset.selectable = String(selectable);
    if (selectable && state.selectedHotspot?.key === group.key) row.className = "current";
    const labelCell = document.createElement("td");
    const performanceCategory = performanceCategoryForMetadata(group.category, group.domain);
    if (selectable) {
      const button = linkButton(group.name, () => selectHotspot(group));
      button.title = `${group.name} · ${categoryName(group.category)}`;
      labelCell.append(button);
    } else {
      labelCell.append(document.createTextNode(group.name));
    }
    const worstCell = document.createElement("td");
    if (group.worstFrame) worstCell.append(linkButton(`#${group.worstFrame.id.toString()}`, () => selectFrame(group.worstFrame)));
    else worstCell.textContent = "—";
    row.append(
      labelCell,
      categoryTableCell(performanceCategoryName(performanceCategory), performanceCategory),
      tableCell(formatNs(group.p50)),
      tableCell(formatNs(group.p95)),
      tableCell(formatNs(group.max)),
      tableCell(formatNs(group.totalSelf)),
      tableCell(formatPercentage(group.framesWithWork, frames.length)),
      worstCell,
    );
    body.append(row);
  });
  element("hotspots-empty").hidden = visible.length !== 0;
  element("clear-hotspot").hidden = state.selectedHotspot === null;
}

function aggregateHotspots(frames) {
  const groups = new Map();
  frames.forEach((frame, frameIndex) => {
    for (const stage of analyzeFrame(frame).stages.values()) {
      let group = groups.get(stage.key);
      if (!group) {
        group = {
          key: stage.key,
          labelId: stage.labelId,
          name: stage.domain === 2 ? `GPU · ${labelFor(stage.labelId)}` : labelFor(stage.labelId),
          category: labelMetadataFor(stage.labelId).category,
          domain: stage.domain,
          values: Array(frames.length).fill(0n),
          calls: Array(frames.length).fill(0),
        };
        groups.set(stage.key, group);
      }
      group.values[frameIndex] = stage.self;
      group.calls[frameIndex] = stage.count;
    }
  });
  return [...groups.values()]
    .map((group) => finalizeHotspot(group, frames))
    .sort((left, right) => compareBigInt(right.totalSelf, left.totalSelf));
}

function finalizeHotspot(group, frames) {
  const sorted = [...group.values].sort(compareBigInt);
  let max = 0n;
  let maxIndex = 0;
  let totalSelf = 0n;
  group.values.forEach((value, index) => {
    totalSelf += value;
    if (value > max) { max = value; maxIndex = index; }
  });
  return {
    ...group,
    totalSelf,
    totalCalls: group.calls.reduce((sum, value) => sum + value, 0),
    framesWithWork: group.values.filter((value) => value > 0n).length,
    p50: bigintPercentile(sorted, 0.50),
    p95: bigintPercentile(sorted, 0.95),
    max,
    worstFrame: frames[maxIndex] || null,
  };
}

function combineOtherHotspots(groups, frames) {
  const values = Array(frames.length).fill(0n);
  const calls = Array(frames.length).fill(0);
  groups.forEach((group) => group.values.forEach((value, index) => {
    values[index] += value;
    calls[index] += group.calls[index];
  }));
  return finalizeHotspot({
    key: "other",
    labelId: null,
    name: `Other (${groups.length} stages)`,
    category: CATEGORY.OTHER,
    domain: 0,
    values,
    calls,
  }, frames);
}

function renderRangeWork(frames) {
  const body = element("range-work-body");
  body.replaceChildren();
  const groups = new Map();
  frames.forEach((frame, frameIndex) => {
    for (const counter of analyzeFrame(frame).counters.values()) {
      let group = groups.get(counter.labelId);
      if (!group) {
        group = { labelId: counter.labelId, values: Array(frames.length).fill(null) };
        groups.set(counter.labelId, group);
      }
      group.values[frameIndex] = counter.value;
    }
  });
  const entries = [...groups.values()]
    .map((group) => {
      const metadata = labelMetadataFor(group.labelId);
      const values = metadata.aggregation === AGGREGATION.GAUGE
        ? group.values.filter((value) => value !== null)
        : group.values.map((value) => value ?? 0);
      return { ...group, values, metadata };
    })
    .filter((entry) => entry.values.some((value) => value !== 0))
    .sort((left, right) => labelFor(left.labelId).localeCompare(labelFor(right.labelId)));
  entries.forEach((entry) => {
    const sorted = [...entry.values].sort((a, b) => a - b);
    const row = document.createElement("tr");
    const performanceCategory = performanceCategoryForMetadata(entry.metadata.category, 1);
    row.append(
      tableCell(labelFor(entry.labelId)),
      categoryTableCell(performanceCategoryName(performanceCategory), performanceCategory),
      tableCell(formatMetricValue(numberPercentile(sorted, 0.50), entry.metadata)),
      tableCell(formatMetricValue(numberPercentile(sorted, 0.95), entry.metadata)),
      tableCell(formatMetricValue(sorted.at(-1), entry.metadata)),
    );
    body.append(row);
  });
  element("range-work-empty").hidden = entries.length !== 0;
}

function renderTimeline() {
  const root = element("timeline");
  root.replaceChildren();
  const frame = state.selectedFrame;
  const toggle = element("toggle-timeline");
  toggle.textContent = state.rawTimelineVisible ? "Hide raw" : "Show raw";
  toggle.setAttribute("aria-expanded", String(state.rawTimelineVisible));
  root.hidden = !state.rawTimelineVisible;
  if (!frame) {
    if (state.rawTimelineVisible) root.append(emptyMessage("Waiting for a rendered frame. Interact with the app to request one."));
    setText("selected-frame-label", "Waiting for frames");
    return;
  }
  setText("selected-frame-label", `#${frame.id.toString()} · ${formatSessionTimestamp(frame.start)} · ${formatNs(frame.duration)}`);
  if (!state.rawTimelineVisible) return;
  const analysis = analyzeFrame(frame);
  const spans = analysis.spans;
  if (spans.length === 0) {
    root.append(emptyMessage("This frame contains counters but no completed spans."));
    return;
  }
  const frameStart = frame.start;
  const visibleDuration = BigInt(Math.max(1, Math.round(Number(frame.duration) / state.zoom)));
  for (const [laneKey, lane] of analysis.flameLanes) {
    const [laneText, domainText] = laneKey.split(":");
    const laneId = Number(laneText);
    const gpuRelative = Number(domainText) === 2;
    const heading = document.createElement("div");
    heading.className = "lane-heading";
    const laneName = state.lanes.get(laneId) || `Lane ${laneId}`;
    heading.append(textCell(gpuRelative ? `GPU relative · ${laneName}` : laneName), textCell("Start / duration"));
    root.append(heading);
    for (let depth = 0; depth <= lane.maxDepth; depth += 1) {
      const row = document.createElement("div");
      row.className = "flame-row";
      const label = document.createElement("div");
      label.className = "event-label";
      label.textContent = `Depth ${depth}`;
      const track = document.createElement("div");
      track.className = "event-track";
      lane.events.filter((event) => event.depth === depth).forEach((event) => {
        const offset = gpuRelative ? event.timestamp : (event.timestamp > frameStart ? event.timestamp - frameStart : 0n);
        const left = Math.min(100, Number(offset * 10000n / visibleDuration) / 100);
        if (left >= 100) return;
        const width = Math.max(0.3, Math.min(100 - left, Number(event.duration * 10000n / visibleDuration) / 100));
        const hotspotMatch = matchesHotspot(event);
        const bar = document.createElement("button");
        bar.type = "button";
        bar.className = `event-bar ${eventClass(event)}${state.selectedEvent === event ? " selected" : ""}${state.selectedHotspot && !hotspotMatch ? " muted" : ""}${hotspotMatch ? " hotspot" : ""}`;
        bar.style.left = `${left}%`;
        bar.style.width = `${width}%`;
        bar.textContent = width > 12 ? labelFor(event.labelId) : (width > 7 ? formatNs(event.duration) : "");
        bar.title = `${labelFor(event.labelId)} · ${formatNs(event.duration)} (${formatNs(event.selfTime)} self)`;
        bar.addEventListener("click", () => {
          state.selectedEvent = event;
          renderTimeline();
          renderInspector();
        });
        track.append(bar);
      });
      row.append(label, track);
      root.append(row);
    }
  }
}

function renderInspector() {
  const details = element("inspector-details");
  const analysisRoot = element("inspector-analysis");
  details.replaceChildren();
  analysisRoot.replaceChildren();
  if (state.selectedEvent) {
    element("inspector-heading").textContent = "Event details";
    const event = state.selectedEvent;
    const performanceCategory = performanceCategoryFor(event);
    const frameStart = event.frame === 0n ? 0n : (state.selectedFrame?.start || 0n);
    const relativeStart = event.domain === 2
      ? event.timestamp
      : (event.timestamp > frameStart ? event.timestamp - frameStart : 0n);
    const inputSample = inputSampleForEvent(event);
    appendDetails(details, [
      ["Label", labelFor(event.labelId)],
      ["Performance category", performanceCategoryName(performanceCategory), `category-${performanceCategory}`],
      ["Metric category", categoryName(labelMetadataFor(event.labelId).category)],
      ["Lane", state.lanes.get(event.lane) || `Lane ${event.lane}`],
      ["Start", formatNs(relativeStart)],
      ["Duration", formatNs(event.duration)],
      ...(inputSample ? [["Input source", inputSample.source.label]] : []),
      ...(inputSample?.queueAge !== null && inputSample?.queueAge !== undefined
        ? [["Queue age", formatNs(inputSample.queueAge)]]
        : []),
      ["Self time", formatNs(event.selfTime ?? eventSelfTime(event))],
      ["Parent", event.parentLabelId ? labelFor(event.parentLabelId) : "None"],
      ["Domain", event.domain === 2 ? "GPU relative" : "CPU monotonic"],
      ["View", viewName(eventView(event))],
      ["Frame", event.frame ? event.frame.toString() : "None"],
      ["Presentation", event.presentation ? event.presentation.toString() : "None"],
      ["Sequence", event.sequence.toString()],
    ]);
    return;
  }
  const rangeFrames = hasManualRangeSelection() ? selectedRangeFrames() : [];
  if (rangeFrames.length > 1) {
    element("inspector-heading").textContent = "Range summary";
    renderRangeInspector(details, analysisRoot, rangeFrames);
    return;
  }
  element("inspector-heading").textContent = "Frame summary";
  const frame = state.selectedFrame;
  if (!frame) {
    appendDetails(details, [["Status", "Waiting for a completed frame"]]);
    return;
  }
  const spans = frame.events.filter((event) => event.kind === KIND.SPAN || event.kind === KIND.GPU_SPAN);
  const counters = frame.events.filter((event) => event.kind === KIND.COUNTER);
  const breakdown = framePerformanceBreakdown(frame);
  appendDetails(details, [
    ["Frame", frame.id.toString()],
    ["Timestamp", formatSessionTimestamp(frame.start)],
    ["View", viewName(frame.view)],
    ["Host turn duration", formatNs(frame.duration)],
    ["CPU work", formatNs(breakdown.get(PERFORMANCE_CATEGORY.CPU).duration), "category-cpu"],
    ["GPU work", formatNs(breakdown.get(PERFORMANCE_CATEGORY.GPU).duration), "category-gpu"],
    ["Presentation", formatNs(breakdown.get(PERFORMANCE_CATEGORY.PRESENTATION).duration), "category-presentation"],
    ["Completed spans", spans.length.toString()],
    ["Counters", counters.length.toString()],
    ["First sequence", frame.events[0]?.sequence.toString() || "—"],
    ["Last sequence", frame.events.at(-1)?.sequence.toString() || "—"],
  ]);
  renderFrameAnalysis(analysisRoot, analyzeFrame(frame));
}

function renderRangeInspector(details, root, frames) {
  const first = frames[0];
  const last = frames.at(-1);
  const durations = frames.map((frame) => frame.duration).sort(compareBigInt);
  const totalHostTime = durations.reduce((total, duration) => total + duration, 0n);
  const intervalEnd = last.start + last.duration;
  const interval = intervalEnd > first.start ? intervalEnd - first.start : 0n;
  const misses = frames.filter((frame) => frame.duration > frameBudgetNs(frame)).length;
  const viewIds = [...new Set(frames.map((frame) => frame.view.toString()))];
  const focused = state.selectedFrame && frames.includes(state.selectedFrame)
    ? `#${state.selectedFrame.id.toString()}`
    : "None";
  appendDetails(details, [
    ["Frame range", `#${first.id.toString()}–#${last.id.toString()}`],
    ["Recorded frames", frames.length.toString()],
    ["Session timestamps", `${formatSessionTimestamp(first.start)}–${formatSessionTimestamp(last.start)}`],
    ["Elapsed interval", formatNs(interval)],
    ["Views", viewIds.length === 1 ? viewName(BigInt(viewIds[0])) : `${viewIds.length.toString()} views`],
    ["Focused frame", focused],
    ["Host turn p50", formatNs(bigintPercentile(durations, 0.50))],
    ["Host turn p95", formatNs(bigintPercentile(durations, 0.95))],
    ["Longest host turn", formatNs(durations.at(-1) ?? 0n)],
    ["Total host work", formatNs(totalHostTime)],
    ["Over budget", `${misses.toString()} of ${frames.length.toString()}`],
  ]);

  const categoryHeading = document.createElement("h3");
  categoryHeading.textContent = "Grouped work by category";
  root.append(categoryHeading);
  const categoryTable = compactTable(["Category", "Total", "p95/frame", "Max/frame", "Active"]);
  const categories = aggregatePerformanceCategories(frames);
  PERFORMANCE_CATEGORIES.forEach((category) => {
    const aggregate = categories.get(category);
    const row = document.createElement("tr");
    row.append(
      categoryTableCell(performanceCategoryName(category), category),
      tableCell(formatNs(aggregate.total)),
      tableCell(formatNs(aggregate.p95)),
      tableCell(formatNs(aggregate.max)),
      tableCell(`${aggregate.framesWithWork.toString()}/${frames.length.toString()}`),
    );
    categoryTable.tBodies[0].append(row);
  });
  root.append(categoryTable);

  const hotspots = aggregateHotspots(frames).filter((group) => group.totalSelf > 0n).slice(0, 10);
  if (hotspots.length === 0) return;
  const stageHeading = document.createElement("h3");
  stageHeading.textContent = "Dominant stages in range";
  root.append(stageHeading);
  const stageTable = compactTable(["Stage", "Category", "Total self", "p95/frame", "Max/frame", "Frames"]);
  hotspots.forEach((group) => {
    const row = document.createElement("tr");
    const performanceCategory = performanceCategoryForMetadata(group.category, group.domain);
    row.append(
      tableCell(group.name),
      categoryTableCell(performanceCategoryName(performanceCategory), performanceCategory),
      tableCell(formatNs(group.totalSelf)),
      tableCell(formatNs(group.p95)),
      tableCell(formatNs(group.max)),
      tableCell(`${group.framesWithWork.toString()}/${frames.length.toString()}`),
    );
    stageTable.tBodies[0].append(row);
  });
  root.append(stageTable);
}

function renderFrameAnalysis(root, analysis) {
  if (analysis.diagnostics.some((event) => labelFor(event.labelId) === VULKAN_WSI_STALL_DIAGNOSTIC)) {
    root.append(vulkanWsiStallGuidance(analysis));
  }

  const stageHeading = document.createElement("h3");
  stageHeading.textContent = "Grouped stage cost";
  root.append(stageHeading);
  const stageTable = compactTable(["Stage", "Category", "Self", "Incl.", "Max", "Count"]);
  [...analysis.groups.values()]
    .sort((left, right) => compareBigInt(right.self, left.self))
    .slice(0, 14)
    .forEach((group) => {
      const row = document.createElement("tr");
      const name = document.createElement("td");
      const sampleEvent = analysis.spans.find((event) => event.labelId === group.labelId && event.domain === group.domain);
      const performanceCategory = sampleEvent
        ? performanceCategoryFor(sampleEvent)
        : performanceCategoryForMetadata(labelMetadataFor(group.labelId).category, group.domain);
      const button = linkButton(labelFor(group.labelId), () => {
        state.selectedHotspot = { key: `${group.domain}:${group.labelId}`, labelId: group.labelId, domain: group.domain };
        state.rawTimelineVisible = true;
        scheduleRender();
      });
      const parent = group.parentLabelId ? labelFor(group.parentLabelId) : "root";
      const lane = state.lanes.get(group.lane) || `Lane ${group.lane}`;
      button.title = `${lane} · ${parent} → ${labelFor(group.labelId)}`;
      name.append(button);
      row.append(
        name,
        categoryTableCell(performanceCategoryName(performanceCategory), performanceCategory),
        tableCell(formatNs(group.self)),
        tableCell(formatNs(group.inclusive)),
        tableCell(formatNs(group.max)),
        tableCell(group.count.toString()),
      );
      stageTable.tBodies[0].append(row);
    });
  root.append(stageTable);

  const visibleCounters = [...analysis.counters.values()].filter((counter) => counter.value !== 0);
  if (visibleCounters.length === 0) return;
  const workHeading = document.createElement("h3");
  workHeading.textContent = "Frame work";
  root.append(workHeading);
  const workTable = compactTable(["Measurement", "Category", "Value"]);
  visibleCounters
    .sort((left, right) => labelFor(left.labelId).localeCompare(labelFor(right.labelId)))
    .forEach((counter) => {
      const row = document.createElement("tr");
      const performanceCategory = performanceCategoryFor(counter.event);
      row.append(
        tableCell(labelFor(counter.labelId)),
        categoryTableCell(performanceCategoryName(performanceCategory), performanceCategory),
        tableCell(formatMetricValue(counter.value, labelMetadataFor(counter.labelId))),
      );
      workTable.tBodies[0].append(row);
    });
  root.append(workTable);
}

function analyzeFrame(frame) {
  if (frame.analysis) return frame.analysis;
  const spans = frame.events.filter((event) => event.kind === KIND.SPAN || event.kind === KIND.GPU_SPAN);
  const flameLanes = new Map();
  spans.forEach((event) => {
    const key = `${event.lane}:${event.domain}`;
    if (!flameLanes.has(key)) flameLanes.set(key, { events: [], maxDepth: 0 });
    flameLanes.get(key).events.push(event);
  });
  for (const lane of flameLanes.values()) {
    lane.events.sort((left, right) => {
      if (left.timestamp !== right.timestamp) return left.timestamp < right.timestamp ? -1 : 1;
      if (left.duration !== right.duration) return left.duration > right.duration ? -1 : 1;
      return left.sequence > right.sequence ? -1 : 1;
    });
    const stack = [];
    lane.events.forEach((event) => {
      const end = event.timestamp + event.duration;
      while (stack.length > 0 && (event.timestamp >= stack.at(-1).end || end > stack.at(-1).end)) stack.pop();
      event.depth = stack.length;
      event.parentEvent = stack.at(-1)?.event || null;
      event.directChildTime = 0n;
      lane.maxDepth = Math.max(lane.maxDepth, event.depth);
      stack.push({ event, end });
    });
    lane.events.forEach((event) => {
      if (event.parentEvent) event.parentEvent.directChildTime += event.duration;
    });
    lane.events.forEach((event) => {
      event.selfTime = event.directChildTime >= event.duration ? 0n : event.duration - event.directChildTime;
    });
  }

  const groups = new Map();
  const stages = new Map();
  spans.forEach((event) => {
    const groupKey = `${event.lane}:${event.domain}:${event.parentLabelId}:${event.labelId}`;
    let group = groups.get(groupKey);
    if (!group) {
      group = {
        key: groupKey,
        lane: event.lane,
        domain: event.domain,
        parentLabelId: event.parentLabelId,
        labelId: event.labelId,
        inclusive: 0n,
        self: 0n,
        max: 0n,
        count: 0,
      };
      groups.set(groupKey, group);
    }
    group.inclusive += event.duration;
    group.self += event.selfTime;
    group.max = event.duration > group.max ? event.duration : group.max;
    group.count += 1;

    const stageKey = `${event.domain}:${event.labelId}`;
    let stage = stages.get(stageKey);
    if (!stage) {
      stage = { key: stageKey, domain: event.domain, labelId: event.labelId, inclusive: 0n, self: 0n, max: 0n, count: 0 };
      stages.set(stageKey, stage);
    }
    stage.inclusive += event.duration;
    stage.self += event.selfTime;
    stage.max = event.duration > stage.max ? event.duration : stage.max;
    stage.count += 1;
  });

  const counters = new Map();
  frame.events.filter((event) => event.kind === KIND.COUNTER && event.counterValue !== null).forEach((event) => {
    const metadata = labelMetadataFor(event.labelId);
    const existing = counters.get(event.labelId);
    if (!existing) {
      counters.set(event.labelId, { labelId: event.labelId, value: event.counterValue, event });
    } else if (metadata.aggregation === AGGREGATION.GAUGE) {
      existing.value = event.counterValue;
      existing.event = event;
    } else {
      existing.value += event.counterValue;
      existing.event = event;
    }
  });

  const diagnostics = frame.events.filter((event) => event.kind === KIND.DIAGNOSTIC);
  frame.analysis = { spans, groups, stages, counters, diagnostics, flameLanes };
  return frame.analysis;
}

function frameBudgetNs(frame) {
  const interval = [...analyzeFrame(frame).counters.values()]
    .find((counter) => labelFor(counter.labelId) === "frame.refresh_interval_ns");
  return interval && Number.isFinite(interval.value)
    ? BigInt(Math.max(1, Math.round(interval.value)))
    : 16_667_000n;
}

function matchesHotspot(event) {
  return state.selectedHotspot !== null
    && event.labelId === state.selectedHotspot.labelId
    && event.domain === state.selectedHotspot.domain;
}

function selectHotspot(group) {
  state.selectedHotspot = { key: group.key, labelId: group.labelId, domain: group.domain };
  state.rawTimelineVisible = true;
  if (group.worstFrame) selectFrame(group.worstFrame);
  else scheduleRender();
}

function hasManualRangeSelection() {
  return state.rangeStartId !== null && state.rangeEndId !== null;
}

function clearManualRangeSelection() {
  state.rangeStartId = null;
  state.rangeEndId = null;
}

function selectFrame(frame) {
  clearManualRangeSelection();
  state.selectedFrame = frame;
  state.selectedEvent = null;
  state.selectedPlotPointKey = null;
  state.followLiveSelection = false;
  scheduleRender();
}

function selectPlotPoint(point) {
  clearManualRangeSelection();
  state.selectedPlotPointKey = point.key;
  state.selectedEvent = null;
  state.followLiveSelection = false;
  if (point.frame) state.selectedFrame = point.frame;
  scheduleRender();
}

function selectManualRange(startFrame, endFrame) {
  state.rangeStartId = startFrame.id < endFrame.id ? startFrame.id : endFrame.id;
  state.rangeEndId = startFrame.id > endFrame.id ? startFrame.id : endFrame.id;
  state.selectedFrame = endFrame;
  state.selectedEvent = null;
  state.selectedPlotPointKey = null;
  state.followLiveSelection = false;
  scheduleRender();
}

function selectProfilerEvent(event, frame) {
  if (frame) state.selectedFrame = frame;
  else if (event.frame) state.selectedFrame = state.framesById.get(event.frame.toString()) || state.selectedFrame;
  else state.selectedFrame = null;
  state.selectedEvent = event;
  state.selectedPlotPointKey = null;
  state.followLiveSelection = false;
  scheduleRender();
}

function renderResources() {
  const body = element("resources-body");
  body.replaceChildren();
  const entries = [...state.counters.entries()]
    .filter(([labelId]) => (labelMetadataFor(labelId).flags & LABEL_FLAG_RESOURCE) !== 0)
    .sort(([left], [right]) => labelFor(left).localeCompare(labelFor(right)));
  entries.forEach(([labelId, event]) => {
    const row = document.createElement("tr");
    const performanceCategory = performanceCategoryFor(event);
    row.append(
      tableCell(labelFor(labelId)),
      categoryTableCell(performanceCategoryName(performanceCategory), performanceCategory),
      tableCell(formatMetricValue(event.counterRaw, labelMetadataFor(labelId))),
      tableCell(event.frame ? event.frame.toString() : "—"),
    );
    body.append(row);
  });
  element("resources-empty").hidden = entries.length !== 0;
}

function renderDiagnostics() {
  const guidance = element("diagnostics-guidance");
  const incident = state.diagnostics.findLast(
    (event) => labelFor(event.labelId) === VULKAN_WSI_STALL_DIAGNOSTIC,
  );
  if (incident) {
    const frame = incident.frame === 0n ? null : state.framesById.get(incident.frame.toString());
    guidance.replaceChildren(vulkanWsiStallGuidance(frame ? analyzeFrame(frame) : null));
    guidance.hidden = false;
  } else {
    guidance.replaceChildren();
    guidance.hidden = true;
  }

  const body = element("diagnostics-body");
  body.replaceChildren();
  state.diagnostics.slice(-500).reverse().forEach((event) => {
    const row = document.createElement("tr");
    const severity = event.kind === KIND.GAP ? "Warning" : diagnosticSeverity(event.auxiliary);
    const performanceCategory = performanceCategoryFor(event);
    row.append(
      tableCell(labelFor(event.labelId)),
      categoryTableCell(performanceCategoryName(performanceCategory), performanceCategory),
      tableCell(severity),
      tableCell(event.frame ? event.frame.toString() : "—"),
      tableCell(event.value.toString()),
    );
    body.append(row);
  });
  element("diagnostics-empty").hidden = state.diagnostics.length !== 0;
}

function vulkanWsiStallGuidance(analysis) {
  const panel = document.createElement("section");
  panel.className = "diagnostic-callout diagnostic-callout-warning";
  panel.setAttribute("role", "status");

  const heading = document.createElement("h3");
  heading.textContent = "Likely GPU driver / Vulkan WSI stall";
  panel.append(heading);

  const durationCounter = analysis
    ? [...analysis.counters.values()].find(
      (counter) => labelFor(counter.labelId) === VULKAN_RAW_ACQUIRE_COUNTER,
    )
    : null;
  const measured = durationCounter && Number.isFinite(durationCounter.value)
    ? ` for ${formatNs(BigInt(Math.max(0, Math.round(durationCounter.value))))}`
    : "";
  const targetOs = state.metadata?.session?.target_os;
  const platform = targetOs === "windows" ? "Windows Vulkan WSI" : "Vulkan WSI";

  const finding = document.createElement("p");
  finding.textContent = `The raw vkAcquireNextImageKHR dispatch used timeout 0 but did not return promptly${measured}. That is a strong ${platform}/display-driver signature because a zero-timeout acquire should not wait for an image.`;
  panel.append(finding);

  const action = document.createElement("p");
  action.textContent = "Update the GPU driver first and retest. If the problem began with a recent driver, try the vendor's previous stable release. This exact symptom disappeared after an AMD driver update in the system where it was first isolated.";
  panel.append(action);

  const caveat = document.createElement("p");
  caveat.className = "diagnostic-callout-caveat";
  caveat.textContent = "This is a likely cause, not proof by itself. If it persists on a current driver, save this capture and enable Vulkan validation to rule out application synchronization misuse.";
  panel.append(caveat);

  const adapter = gpuAdapterIdentifiers();
  if (adapter) {
    const context = document.createElement("p");
    context.className = "diagnostic-callout-context";
    context.textContent = adapter;
    panel.append(context);
  }
  return panel;
}

function gpuAdapterIdentifiers() {
  const identifiers = new Map();
  state.counters.forEach((event, labelId) => {
    const label = labelFor(labelId);
    if (label.startsWith("gpu.adapter.") && Number.isFinite(event.counterRaw)) {
      identifiers.set(label, Math.max(0, Math.round(event.counterRaw)));
    }
  });
  const vendor = identifiers.get("gpu.adapter.vendor_id");
  const device = identifiers.get("gpu.adapter.device_id");
  const driver = identifiers.get("gpu.adapter.driver_version");
  if (vendor === undefined && device === undefined && driver === undefined) return null;
  const vendorName = new Map([
    [0x1002, "AMD"],
    [0x10de, "NVIDIA"],
    [0x8086, "Intel"],
  ]).get(vendor);
  const fields = [];
  if (vendor !== undefined) fields.push(`vendor ${vendorName || "unknown"} (${formatHexIdentifier(vendor, 4)})`);
  if (device !== undefined) fields.push(`device ${formatHexIdentifier(device, 4)}`);
  if (driver !== undefined) fields.push(`raw Vulkan driver ${formatHexIdentifier(driver, 8)}`);
  return `Captured adapter: ${fields.join(" · ")}`;
}

function formatHexIdentifier(value, width) {
  return `0x${value.toString(16).padStart(width, "0")}`;
}

function renderSession() {
  const details = element("session-details");
  details.replaceChildren();
  if (!state.metadata) return;
  const session = state.metadata.session;
  appendDetails(details, [
    ["Application", session.application],
    ["Executable", session.executable],
    ["Entrypoint", formatEntrypoint(session.entrypoint)],
    ["Build", session.build_profile],
    ["Operating system", session.target_os],
    ["Architecture", session.target_arch],
    ["Renderer", session.renderer],
    ["Git revision", session.git_revision || "Not supplied"],
    ["Protocol", `${state.metadata.protocol_major}.${state.metadata.protocol_minor}`],
    ["Viewer events", String(state.events.length)],
    ["Server retained at snapshot", String(state.metadata.retained_events)],
    ["Event limit", formatBytes(state.metadata.event_byte_limit)],
    ["Frame limit", String(state.metadata.retained_frame_limit)],
    ["Registered views", state.views.size.toString()],
    ["Capabilities", session.capabilities.join(", ") || "None"],
    ["Unavailable", session.unavailable_metrics.join(", ") || "None"],
  ]);
}

function switchWorkspace(name) {
  if (!["performance", "resources", "diagnostics", "session"].includes(name)) return;
  state.workspace = name;
  document.querySelectorAll(".nav-button").forEach((button) => {
    const current = button.dataset.workspace === name;
    button.classList.toggle("current", current);
    if (current) button.setAttribute("aria-current", "page");
    else button.removeAttribute("aria-current");
  });
  document.querySelectorAll(".workspace-panel").forEach((panel) => {
    panel.hidden = panel.id !== `workspace-${name}`;
  });
  scheduleRender();
}

function switchPerformanceView(view) {
  if (!["overview", "work", "responsiveness", "inputs"].includes(view)) return;
  state.performanceView = view;
  state.selectedEvent = null;
  state.selectedPlotPointKey = null;
  state.hoveredPlotPointKey = null;
  scheduleRender();
}

function selectFrameOffset(offset) {
  clearManualRangeSelection();
  const frames = plotViewFrames();
  if (frames.length === 0) return;
  const current = Math.max(0, frames.indexOf(state.selectedFrame));
  const index = Math.max(0, Math.min(frames.length - 1, current + offset));
  state.selectedFrame = frames[index];
  state.selectedEvent = null;
  state.selectedPlotPointKey = null;
  state.followLiveSelection = false;
  scheduleRender();
}

function clearClient() {
  state.frames.length = 0;
  state.framesById.clear();
  state.events.length = 0;
  state.counters.clear();
  state.diagnostics.length = 0;
  state.counterBaselines.clear();
  state.selectedFrame = null;
  state.selectedEvent = null;
  state.selectedHotspot = null;
  state.followLiveSelection = true;
  state.selectedPlotPointKey = null;
  state.hoveredPlotPointKey = null;
  state.plotView = null;
  state.plotViewHistory.length = 0;
  state.rawTimelineVisible = false;
  state.viewFilter = "all";
  state.rangeStartId = null;
  state.rangeEndId = null;
  state.dropped = 0n;
  scheduleRender();
}

function disconnectViewer() {
  if (state.liveRenderTimer !== null) {
    window.clearTimeout(state.liveRenderTimer);
    state.liveRenderTimer = null;
  }
  state.metadata = null;
  renderInputRecordingControls();
  state.lanes.clear();
  state.views.clear();
  state.labels.clear();
  state.frames.length = 0;
  state.framesById.clear();
  state.events.length = 0;
  state.counters.clear();
  state.diagnostics.length = 0;
  state.counterBaselines.clear();
  state.seenSequences.clear();
  state.selectedFrame = null;
  state.selectedEvent = null;
  state.selectedHotspot = null;
  state.followLiveSelection = true;
  state.selectedPlotPointKey = null;
  state.hoveredPlotPointKey = null;
  state.plotView = null;
  state.plotViewHistory.length = 0;
  state.rawTimelineVisible = false;
  state.viewFilter = "all";
  state.rangeStartId = null;
  state.rangeEndId = null;
  state.paused = false;
  state.dropped = 0n;
  element("application-name").textContent = "No target connected";
  updatePauseButton();
  setConnection("Disconnected", "closed");
  setApplicationConnected(false);
  scheduleRender();
}

function setApplicationConnected(connected) {
  element("disconnected-state").hidden = connected;
  element("pause-button").disabled = !connected;
  element("clear-button").disabled = !connected;
  const save = element("save-trace");
  if (connected) {
    save.href = "/capture";
    save.removeAttribute("aria-disabled");
    save.removeAttribute("tabindex");
  } else {
    save.removeAttribute("href");
    save.setAttribute("aria-disabled", "true");
    save.setAttribute("tabindex", "-1");
  }
}

function togglePause() {
  state.paused = !state.paused;
  if (state.paused) {
    if (state.liveRenderTimer !== null) {
      window.clearTimeout(state.liveRenderTimer);
      state.liveRenderTimer = null;
    }
  } else {
    // The first cumulative sample after a pause becomes a new baseline instead of folding
    // activity from the ignored interval into the resumed graph.
    state.counterBaselines.clear();
    state.selectedFrame = filteredCompletedFrames().at(-1) || null;
    state.selectedEvent = null;
    state.followLiveSelection = true;
    state.selectedPlotPointKey = null;
  }
  updatePauseButton();
  scheduleRender();
}

function updatePauseButton() {
  const button = element("pause-button");
  const label = state.paused ? "Resume profiling" : "Pause profiling";
  button.setAttribute("aria-label", label);
  button.title = label;
  button.classList.toggle("is-paused", state.paused);
  if (state.metadata !== null) {
    setConnection(state.paused ? "Paused" : "Live", state.paused ? "paused" : "live");
  }
}

function counterNumber(event) {
  const buffer = new ArrayBuffer(8);
  const view = new DataView(buffer);
  view.setBigUint64(0, event.value, true);
  return view.getFloat64(0, true);
}

function eventSelfTime(event) {
  const frame = state.selectedFrame;
  if (!frame || (event.kind !== KIND.SPAN && event.kind !== KIND.GPU_SPAN)) return event.duration;
  analyzeFrame(frame);
  return event.selfTime ?? event.duration;
}

function formatNs(value) {
  if (value >= 1_000_000n) return `${nsToMs(value).toFixed(2)} ms`;
  if (value >= 1_000n) return `${(Number(value) / 1_000).toFixed(1)} µs`;
  return `${value.toString()} ns`;
}

function formatSessionTimestamp(value) {
  if (value >= 1_000_000_000n) {
    return `+${(Number(value / 1_000_000n) / 1_000).toFixed(3)} s`;
  }
  return `+${formatNs(value)}`;
}

function formatOptionalNs(value) {
  return value === null ? "—" : formatNs(value);
}

function nsToMs(value) {
  const maximumSafe = BigInt(Number.MAX_SAFE_INTEGER);
  const bounded = value > maximumSafe ? maximumSafe : value;
  return Number(bounded) / 1_000_000;
}

function formatAxisDuration(milliseconds) {
  if (milliseconds < 0.001) return `${(milliseconds * 1_000_000).toFixed(0)} ns`;
  if (milliseconds < 1) return `${(milliseconds * 1000).toFixed(milliseconds < 0.1 ? 1 : 0)} µs`;
  return `${milliseconds.toFixed(milliseconds < 10 ? 2 : 1)} ms`;
}

function formatBytes(value) {
  if (!Number.isFinite(Number(value))) return "N/A";
  const bytes = Number(value);
  if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(2)} GiB`;
  if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(2)} MiB`;
  if (bytes >= 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${bytes.toFixed(0)} B`;
}

function formatMetricValue(value, metadata) {
  if (!Number.isFinite(value)) return "N/A";
  if (metadata.unit === UNIT.BYTES) return formatBytes(value);
  if (metadata.unit === UNIT.NANOSECONDS) return formatNs(BigInt(Math.max(0, Math.round(value))));
  if (metadata.unit === UNIT.AREA) return `${formatNumber(value)} area`;
  return formatNumber(value);
}

function formatNumber(value) {
  return Math.abs(value) >= 1000
    ? value.toLocaleString(undefined, { maximumFractionDigits: 1 })
    : value.toFixed(value % 1 === 0 ? 0 : 2);
}

function formatPercentage(part, total) {
  if (total <= 0) return "0%";
  return `${(part / total * 100).toFixed(part === total ? 0 : 1)}%`;
}

function formatRatio(value, baseline) {
  if (baseline <= 0n) return "—";
  return `${(Number(value) / Number(baseline)).toFixed(2)}×`;
}

function formatEntrypoint(value) {
  if (!value) return "Application";
  return value.split("-").map((part) => part.charAt(0).toUpperCase() + part.slice(1)).join(" ");
}

function labelMetadataFor(id) {
  return state.labels.get(id) || {
    id,
    name: id === 0 ? "Unavailable label" : `Label ${id}`,
    category: CATEGORY.OTHER,
    unit: UNIT.NONE,
    aggregation: AGGREGATION.EVENT,
    flags: 0,
  };
}
function labelFor(id) { return labelMetadataFor(id).name; }
function eventClass(event) {
  return performanceCategoryFor(event);
}
function performanceCategoryFor(event) {
  const metadata = labelMetadataFor(event.labelId);
  if (event.domain === 2 || metadata.category === CATEGORY.GPU) return PERFORMANCE_CATEGORY.GPU;
  const label = metadata.name;
  if (event.presentation !== 0n
      || event.kind === KIND.PRESENTATION_BEGIN
      || event.kind === KIND.PRESENTATION_END
      || metadata.category === CATEGORY.PRESENTATION
      || label.startsWith("presentation.")
      || label.startsWith("responsiveness.")) {
    return PERFORMANCE_CATEGORY.PRESENTATION;
  }
  return performanceCategoryForMetadata(metadata.category, event.domain);
}
function performanceCategoryForMetadata(category, domain) {
  if (domain === 2 || category === CATEGORY.GPU) return PERFORMANCE_CATEGORY.GPU;
  if (category === CATEGORY.PRESENTATION || category === CATEGORY.RENDERER || category === CATEGORY.WAIT) {
    return PERFORMANCE_CATEGORY.PRESENTATION;
  }
  return PERFORMANCE_CATEGORY.CPU;
}
function performanceCategoryName(category) {
  if (category === PERFORMANCE_CATEGORY.GPU) return "GPU work";
  if (category === PERFORMANCE_CATEGORY.PRESENTATION) return "Presentation & responsiveness";
  return "CPU work";
}
function categoryName(category) {
  return ["Other", "Runtime", "Input", "Theme", "Layout", "Scene", "Renderer", "GPU", "Wait", "Diagnostic", "Presentation"][category] || "Other";
}
function compareBigInt(left, right) { return left === right ? 0 : (left < right ? -1 : 1); }
function bigintPercentile(values, position) {
  if (values.length === 0) return 0n;
  return values[Math.min(values.length - 1, Math.floor((values.length - 1) * position))];
}
function numberPercentile(values, position) {
  if (values.length === 0) return Number.NaN;
  return values[Math.min(values.length - 1, Math.floor((values.length - 1) * position))];
}
function diagnosticSeverity(value) { return value === 3n ? "Error" : value === 2n ? "Warning" : "Information"; }
function setText(id, value) { element(id).textContent = value; }
function textCell(value) { const cell = document.createElement("span"); cell.textContent = value; return cell; }
function tableCell(value) { const cell = document.createElement("td"); cell.textContent = value; return cell; }
function categorySwatch(category) {
  const swatch = document.createElement("span");
  swatch.className = `category-swatch ${category}`;
  swatch.setAttribute("aria-hidden", "true");
  return swatch;
}
function categoryTableCell(value, category) {
  const cell = document.createElement("td");
  cell.className = "category-cell";
  cell.append(categorySwatch(category), document.createTextNode(value));
  return cell;
}
function linkButton(value, action) {
  const button = document.createElement("button");
  button.type = "button";
  button.className = "link-button";
  button.textContent = value;
  button.addEventListener("click", action);
  return button;
}
function compactTable(headings) {
  const table = document.createElement("table");
  table.dataset.sortKey = `compact:${headings.join(":")}`;
  const head = document.createElement("thead");
  const headRow = document.createElement("tr");
  headings.forEach((heading) => { const cell = document.createElement("th"); cell.scope = "col"; cell.textContent = heading; headRow.append(cell); });
  head.append(headRow);
  table.append(head, document.createElement("tbody"));
  enhanceSortableTable(table);
  return table;
}

function tableSortKey(table) {
  if (table.dataset.sortKey) return table.dataset.sortKey;
  const bodyId = table.tBodies[0]?.id;
  if (bodyId) {
    table.dataset.sortKey = `body:${bodyId}`;
    return table.dataset.sortKey;
  }
  const headings = [...table.querySelectorAll("thead th")].map((heading) => heading.textContent.trim());
  table.dataset.sortKey = `table:${headings.join(":")}`;
  return table.dataset.sortKey;
}

function enhanceSortableTable(table) {
  if (table.dataset.sortable === "true") return;
  table.dataset.sortable = "true";
  const key = tableSortKey(table);
  [...table.querySelectorAll("thead th")].forEach((heading, column) => {
    const label = heading.textContent.trim();
    const button = document.createElement("button");
    button.type = "button";
    button.className = "table-sort-button";
    button.textContent = label;
    button.addEventListener("click", () => {
      const previous = state.tableSorts.get(key);
      const direction = previous?.column === column && previous.direction === "ascending"
        ? "descending"
        : "ascending";
      state.tableSorts.set(key, { column, direction });
      applyTableSort(table);
    });
    heading.replaceChildren(button);
    heading.setAttribute("aria-sort", "none");
  });
  updateTableSortHeaders(table, state.tableSorts.get(key));
}

function applyStoredTableSorts(root) {
  root.querySelectorAll("table").forEach((table) => {
    enhanceSortableTable(table);
    applyTableSort(table);
  });
}

function applyTableSort(table) {
  const sort = state.tableSorts.get(tableSortKey(table));
  updateTableSortHeaders(table, sort);
  if (!sort || !table.tBodies[0]) return;
  const body = table.tBodies[0];
  const rows = [...body.rows];
  const sorted = rows.map((row, index) => ({ row, index, value: tableCellSortValue(row.cells[sort.column]) }))
    .sort((left, right) => {
      if (left.value.missing !== right.value.missing) return left.value.missing ? 1 : -1;
      const comparison = compareTableSortValues(left.value, right.value);
      if (comparison === 0) return left.index - right.index;
      return sort.direction === "ascending" ? comparison : -comparison;
    })
    .map((entry) => entry.row);
  if (sorted.every((row, index) => row === rows[index])) return;
  body.append(...sorted);
}

function updateTableSortHeaders(table, sort) {
  [...table.querySelectorAll("thead th")].forEach((heading, column) => {
    const direction = sort?.column === column ? sort.direction : "none";
    heading.setAttribute("aria-sort", direction);
    const button = heading.querySelector(".table-sort-button");
    if (!button) return;
    const next = direction === "ascending" ? "descending" : "ascending";
    button.title = direction === "none"
      ? `Sort ${button.textContent} ascending`
      : `Sorted ${direction}; activate to sort ${next}`;
  });
}

function tableCellSortValue(cell) {
  if (!cell) return { missing: true, numeric: null, text: "" };
  const text = cell.textContent.trim();
  if (text === "" || text === "—" || text === "N/A" || text === "None") {
    return { missing: true, numeric: null, text };
  }
  const normalized = text.replaceAll(",", "");
  const severity = { Information: 1, Warning: 2, Error: 3 }[normalized];
  if (severity !== undefined) return { missing: false, numeric: severity, text };
  const duration = normalized.match(/^([+-]?\d+(?:\.\d+)?)\s*(ns|µs|us|ms|s)$/i);
  if (duration) {
    const factors = { ns: 1, "µs": 1_000, us: 1_000, ms: 1_000_000, s: 1_000_000_000 };
    return { missing: false, numeric: Number(duration[1]) * factors[duration[2].toLowerCase()], text };
  }
  const bytes = normalized.match(/^([+-]?\d+(?:\.\d+)?)\s*(B|KiB|MiB|GiB)$/i);
  if (bytes) {
    const factors = { b: 1, kib: 1024, mib: 1024 ** 2, gib: 1024 ** 3 };
    return { missing: false, numeric: Number(bytes[1]) * factors[bytes[2].toLowerCase()], text };
  }
  const percentage = normalized.match(/^([+-]?\d+(?:\.\d+)?)%$/);
  if (percentage) return { missing: false, numeric: Number(percentage[1]), text };
  const fraction = normalized.match(/^([+-]?\d+(?:\.\d+)?)\/([+-]?\d+(?:\.\d+)?)$/);
  if (fraction) {
    const denominator = Number(fraction[2]);
    return { missing: false, numeric: denominator === 0 ? 0 : Number(fraction[1]) / denominator, text };
  }
  const frame = normalized.match(/^#(\d+)$/);
  if (frame) return { missing: false, numeric: Number(frame[1]), text };
  const scalar = normalized.match(/^([+-]?\d+(?:\.\d+)?)\s*(?:area|×)?$/);
  if (scalar) return { missing: false, numeric: Number(scalar[1]), text };
  return { missing: false, numeric: null, text };
}

function compareTableSortValues(left, right) {
  if (left.missing) return 0;
  if (left.numeric !== null && right.numeric !== null) return left.numeric - right.numeric;
  return TABLE_SORT_COLLATOR.compare(left.text, right.text);
}
function emptyMessage(value) { const message = document.createElement("p"); message.className = "empty-state"; message.textContent = value; return message; }
function appendDetails(root, entries) {
  entries.forEach(([term, description, className]) => {
    const row = document.createElement("div");
    if (className) row.className = className;
    const dt = document.createElement("dt");
    const dd = document.createElement("dd");
    dt.textContent = term;
    dd.textContent = description;
    row.append(dt, dd);
    root.append(row);
  });
}
function showError(message) { const banner = element("error-banner"); banner.textContent = message; banner.hidden = false; }
function hideError() { element("error-banner").hidden = true; }

function inspectorMaximumWidth() {
  const shell = element("application-shell");
  const navigation = shell.querySelector(".navigation");
  const handle = element("inspector-resize-handle");
  const available = shell.getBoundingClientRect().width
    - (navigation?.getBoundingClientRect().width || 0)
    - handle.getBoundingClientRect().width
    - INSPECTOR_MIN_WORKSPACE_WIDTH;
  if (available <= 0) return INSPECTOR_MAX_WIDTH;
  return Math.max(INSPECTOR_MIN_WIDTH, Math.min(INSPECTOR_MAX_WIDTH, Math.floor(available)));
}

function applyInspectorWidth(maximum = inspectorMaximumWidth()) {
  const next = Math.max(INSPECTOR_MIN_WIDTH, Math.min(maximum, Math.round(state.inspectorWidth)));
  element("application-shell").style.setProperty("--inspector-width", `${next.toString()}px`);
  const handle = element("inspector-resize-handle");
  handle.setAttribute("aria-valuemax", maximum.toString());
  handle.setAttribute("aria-valuenow", next.toString());
}

function queueInspectorDragWidth(handle, width) {
  const maximum = handle._dragMaximumWidth ?? INSPECTOR_MAX_WIDTH;
  state.inspectorWidth = Math.max(INSPECTOR_MIN_WIDTH, Math.min(maximum, Math.round(width)));
  if (inspectorDragFrame !== null) return;
  inspectorDragFrame = requestAnimationFrame(() => {
    inspectorDragFrame = null;
    applyInspectorWidth(maximum);
  });
}

function setInspectorWidth(width, fullRender = false) {
  state.inspectorWidth = Math.max(INSPECTOR_MIN_WIDTH, Math.min(INSPECTOR_MAX_WIDTH, Math.round(width)));
  applyInspectorWidth();
  if (fullRender) scheduleRender();
}

function finishInspectorResize(handle) {
  if (handle._dragStartX === undefined) return;
  if (inspectorDragFrame !== null) {
    cancelAnimationFrame(inspectorDragFrame);
    inspectorDragFrame = null;
  }
  applyInspectorWidth(handle._dragMaximumWidth);
  delete handle._dragStartX;
  delete handle._dragStartWidth;
  delete handle._dragMaximumWidth;
  inspectorDragActive = false;
  handle.classList.remove("dragging");
  document.body.classList.remove("inspector-resizing");
  scheduleRender();
}

document.querySelectorAll(".nav-button").forEach((button) => button.addEventListener("click", () => switchWorkspace(button.dataset.workspace)));
document.querySelectorAll(".performance-view-button").forEach((button) => button.addEventListener("click", () => switchPerformanceView(button.dataset.performanceView)));
element("pause-button").addEventListener("click", togglePause);
element("clear-button").addEventListener("click", clearClient);
element("clear-analysis-range").addEventListener("click", () => {
  clearManualRangeSelection();
  scheduleRender();
});
element("clear-hotspot").addEventListener("click", () => { state.selectedHotspot = null; scheduleRender(); });
element("toggle-timeline").addEventListener("click", () => { state.rawTimelineVisible = !state.rawTimelineVisible; scheduleRender(); });
element("view-filter").addEventListener("change", (event) => {
  state.viewFilter = event.currentTarget.value;
  clearManualRangeSelection();
  state.plotView = null;
  state.plotViewHistory.length = 0;
  state.selectedPlotPointKey = null;
  state.hoveredPlotPointKey = null;
  state.selectedFrame = filteredCompletedFrames().at(-1) || null;
  state.selectedEvent = null;
  scheduleRender();
});
element("zoom-in").addEventListener("click", () => { state.zoom = Math.min(8, state.zoom * 2); setText("zoom-label", `${state.zoom * 100}%`); renderTimeline(); });
element("zoom-out").addEventListener("click", () => { state.zoom = Math.max(1, state.zoom / 2); setText("zoom-label", `${state.zoom * 100}%`); renderTimeline(); });
element("plot-view-back").addEventListener("click", restorePreviousPlotView);
element("plot-view-reset").addEventListener("click", resetPlotView);
element("plot-frame-limit").addEventListener("change", (event) => setPlotFrameLimit(event.currentTarget.value));

const inspectorResizeHandle = element("inspector-resize-handle");
inspectorResizeHandle.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return;
  event.preventDefault();
  inspectorDragActive = true;
  if (state.liveRenderTimer !== null) {
    window.clearTimeout(state.liveRenderTimer);
    state.liveRenderTimer = null;
  }
  inspectorResizeHandle._dragStartX = event.clientX;
  inspectorResizeHandle._dragStartWidth = element("inspector").getBoundingClientRect().width;
  inspectorResizeHandle._dragMaximumWidth = inspectorMaximumWidth();
  inspectorResizeHandle.classList.add("dragging");
  document.body.classList.add("inspector-resizing");
  inspectorResizeHandle.setPointerCapture(event.pointerId);
});
inspectorResizeHandle.addEventListener("pointermove", (event) => {
  if (inspectorResizeHandle._dragStartX === undefined) return;
  event.preventDefault();
  const delta = inspectorResizeHandle._dragStartX - event.clientX;
  queueInspectorDragWidth(inspectorResizeHandle, inspectorResizeHandle._dragStartWidth + delta);
});
inspectorResizeHandle.addEventListener("pointerup", () => finishInspectorResize(inspectorResizeHandle));
inspectorResizeHandle.addEventListener("pointercancel", () => finishInspectorResize(inspectorResizeHandle));
inspectorResizeHandle.addEventListener("keydown", (event) => {
  const current = element("inspector").getBoundingClientRect().width || INSPECTOR_DEFAULT_WIDTH;
  let next = null;
  if (event.key === "ArrowLeft") next = current + INSPECTOR_KEYBOARD_STEP;
  else if (event.key === "ArrowRight") next = current - INSPECTOR_KEYBOARD_STEP;
  else if (event.key === "Home") next = INSPECTOR_MIN_WIDTH;
  else if (event.key === "End") next = inspectorMaximumWidth();
  if (next === null) return;
  event.preventDefault();
  setInspectorWidth(next, true);
});

function plotFrameAt(clientX) {
  const frames = plot._visibleFrames || [];
  const frameXs = plot._frameXs || [];
  if (frames.length === 0) return null;
  const bounds = plot.getBoundingClientRect();
  const x = Math.max(0, Math.min(bounds.width - 0.001, clientX - bounds.left));
  let nearestIndex = 0;
  let nearestDistance = Number.POSITIVE_INFINITY;
  frameXs.forEach((frameX, index) => {
    const distance = Math.abs(frameX - x);
    if (distance < nearestDistance) {
      nearestDistance = distance;
      nearestIndex = index;
    }
  });
  return frames[nearestIndex];
}

function plotPointAt(clientX, clientY, maximumDistance = 13) {
  const bounds = plot.getBoundingClientRect();
  const x = clientX - bounds.left;
  const y = clientY - bounds.top;
  let nearest = null;
  let nearestDistance = Number.POSITIVE_INFINITY;
  (plot._interactivePoints || []).forEach((point) => {
    const distance = Math.hypot(point.x - x, point.y - y);
    if (distance >= nearestDistance) return;
    nearest = point;
    nearestDistance = distance;
  });
  return nearestDistance <= maximumDistance ? nearest : null;
}

function showPlotFrameTimestamp(clientX) {
  const frame = plotFrameAt(clientX);
  if (!frame) return;
  if (plot._captionFrameId === frame.id) return;
  const label = `Frame #${frame.id.toString()} · ${formatSessionTimestamp(frame.start)} · ${formatNs(frame.duration)}`;
  setText("plot-caption", label);
  plot.title = label;
  plot._captionFrameId = frame.id;
}

plot.addEventListener("pointerdown", (event) => {
  if (event.button !== 0 && event.button !== 2) return;
  if (event.button === 2) event.preventDefault();
  const bounds = plot.getBoundingClientRect();
  plot._dragStart = Math.max(0, Math.min(bounds.width, event.clientX - bounds.left));
  plot._dragCurrent = plot._dragStart;
  plot._dragMode = event.button === 2 ? "zoom" : "range";
  if (event.button === 0) state.selectedPlotPointKey = null;
  state.hoveredPlotPointKey = null;
  plot.setPointerCapture(event.pointerId);
  renderPlot();
});
plot.addEventListener("pointermove", (event) => {
  if (plot._dragStart === undefined) {
    const point = plotPointAt(event.clientX, event.clientY);
    const key = point?.key ?? null;
    plot.style.cursor = point ? "pointer" : "crosshair";
    if (key !== state.hoveredPlotPointKey) {
      state.hoveredPlotPointKey = key;
      renderPlot();
    }
    if (point) return;
    showPlotFrameTimestamp(event.clientX);
    return;
  }
  const bounds = plot.getBoundingClientRect();
  plot._dragCurrent = Math.max(0, Math.min(bounds.width, event.clientX - bounds.left));
  renderPlot();
});
plot.addEventListener("pointerup", (event) => {
  if (plot._dragStart === undefined) return;
  const startX = plot._dragStart;
  const dragMode = plot._dragMode;
  const bounds = plot.getBoundingClientRect();
  const endX = Math.max(0, Math.min(bounds.width, event.clientX - bounds.left));
  const startFrame = plotFrameAt(bounds.left + startX);
  const endFrame = plotFrameAt(bounds.left + endX);
  delete plot._dragStart;
  delete plot._dragCurrent;
  delete plot._dragMode;
  if (!startFrame || !endFrame) return scheduleRender();
  if (dragMode === "zoom") {
    if (Math.abs(endX - startX) < 8 || startFrame.id === endFrame.id) {
      renderPlot();
      return;
    }
    clearManualRangeSelection();
    state.plotViewHistory.push(state.plotView === null ? null : { ...state.plotView });
    state.plotView = {
      startId: startFrame.id < endFrame.id ? startFrame.id : endFrame.id,
      endId: startFrame.id > endFrame.id ? startFrame.id : endFrame.id,
    };
    state.hoveredPlotPointKey = null;
    scheduleRender();
    return;
  }
  if (Math.abs(endX - startX) < 4) {
    const point = plotPointAt(event.clientX, event.clientY);
    if (point) {
      selectPlotPoint(point);
      return;
    }
    selectFrame(endFrame);
    return;
  }
  selectManualRange(startFrame, endFrame);
});
plot.addEventListener("pointercancel", () => {
  delete plot._dragStart;
  delete plot._dragCurrent;
  delete plot._dragMode;
  scheduleRender();
});
plot.addEventListener("pointerleave", () => {
  if (plot._dragStart !== undefined) return;
  plot.style.cursor = "crosshair";
  state.hoveredPlotPointKey = null;
  renderPlot();
});
plot.addEventListener("contextmenu", (event) => event.preventDefault());
plot.addEventListener("keydown", (event) => {
  if (event.key === "ArrowLeft" || event.key === "ArrowRight") {
    event.preventDefault();
    selectFrameOffset(event.key === "ArrowLeft" ? -1 : 1);
  }
});
document.addEventListener("keydown", (event) => {
  if (event.target instanceof HTMLInputElement || event.target instanceof HTMLButtonElement || event.target instanceof HTMLAnchorElement || event.target instanceof HTMLSelectElement) return;
  if (event.key === "Escape") {
    state.paused = false;
    state.counterBaselines.clear();
    clearManualRangeSelection();
    state.selectedFrame = filteredCompletedFrames().at(-1) || null;
    state.selectedEvent = null;
    state.selectedHotspot = null;
    state.followLiveSelection = true;
    state.selectedPlotPointKey = null;
    state.hoveredPlotPointKey = null;
    updatePauseButton();
    scheduleRender();
  }
});
window.addEventListener("resize", () => {
  applyInspectorWidth();
  scheduleRender();
});

renderInputRecordingControls();
applyInspectorWidth();
applyStoredTableSorts(document);
connect();
