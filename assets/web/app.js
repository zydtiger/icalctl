"use strict";

const MAX_RANGE_DAYS = 366;
const ROUTE_SOURCE_ID = "icalctl-travel-routes";
const ROUTE_LAYER_ID = "icalctl-travel-route-lines";
const MAPLIBRE_WORKER_URL = "/assets/vendor/maplibre-gl/maplibre-gl-csp-worker.js";
const DEFAULT_MAP_STYLE_URL = "https://tiles.openfreemap.org/styles/bright";
const BLANK_STYLE = {
  version: 8,
  sources: {},
  layers: [
    {
      id: "background",
      type: "background",
      paint: {"background-color": "#0b1317"},
    },
  ],
};

const state = {
  data: null,
  map: null,
  mapReady: false,
  mapFallbackUsed: false,
  routeClickBound: false,
  markers: [],
  selectedIndex: null,
  projection: "globe",
  requestController: null,
  requestSerial: 0,
  requestCount: 0,
  pendingFit: false,
};

const elements = {};

document.addEventListener("DOMContentLoaded", initialize);

function initialize() {
  for (const id of [
    "range-form",
    "start-date",
    "end-date",
    "refresh-button",
    "form-error",
    "map-message",
    "request-status",
    "trip-selector",
    "warning-panel",
    "warning-list",
    "detail-panel",
    "detail-heading",
    "detail-content",
    "empty-state",
    "trip-list",
    "trip-count",
    "generated-at",
  ]) {
    elements[id] = document.getElementById(id);
  }

  elements["range-form"].addEventListener("submit", handleRangeSubmit);
  for (const button of document.querySelectorAll("[data-projection]")) {
    button.addEventListener("click", () => setProjection(button.dataset.projection));
  }

  if (typeof window.maplibregl === "undefined") {
    showMapMessage("MapLibre could not be loaded. The itinerary is still available.", true);
  } else {
    window.maplibregl.setWorkerUrl(MAPLIBRE_WORKER_URL);
    document.body.dataset.maplibreVersion = window.maplibregl.getVersion();
  }

  loadTravel(null);
}

function handleRangeSubmit(event) {
  event.preventDefault();
  const start = elements["start-date"].value;
  const end = elements["end-date"].value;
  const error = validateRange(start, end);
  if (error) {
    showFormError(error);
    return;
  }
  hideFormError();
  loadTravel({start, end});
}

function validateRange(start, end) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(start) || !/^\d{4}-\d{2}-\d{2}$/.test(end)) {
    return "Choose both a start and end date.";
  }
  if (start > end) {
    return "Start date must be on or before end date.";
  }
  const startTime = Date.parse(`${start}T00:00:00Z`);
  const endTime = Date.parse(`${end}T00:00:00Z`);
  if (!Number.isFinite(startTime) || !Number.isFinite(endTime)) {
    return "Choose valid calendar dates.";
  }
  const inclusiveDays = Math.round((endTime - startTime) / 86400000) + 1;
  if (inclusiveDays > MAX_RANGE_DAYS) {
    return `Date range cannot exceed ${MAX_RANGE_DAYS} inclusive days.`;
  }
  return null;
}

async function loadTravel(range) {
  if (state.requestController) {
    state.requestController.abort();
  }
  const controller = new AbortController();
  state.requestController = controller;
  const serial = ++state.requestSerial;
  state.requestCount += 1;
  document.body.dataset.apiRequests = String(state.requestCount);
  setLoading(true);
  hideFormError();
  setRequestStatus("Reading local Calendar itinerary…", false);

  const url = new URL("/api/travel", window.location.origin);
  if (range) {
    url.searchParams.set("start", range.start);
    url.searchParams.set("end", range.end);
  }

  try {
    const response = await fetch(url, {
      method: "GET",
      headers: {Accept: "application/json"},
      credentials: "same-origin",
      signal: controller.signal,
    });
    let payload;
    try {
      payload = await response.json();
    } catch (_error) {
      throw new Error(`The local server returned HTTP ${response.status} without valid JSON.`);
    }
    if (!response.ok) {
      throw new Error(payload?.error?.message || `The local server returned HTTP ${response.status}.`);
    }
    if (!payload || !Array.isArray(payload.legs) || !Array.isArray(payload.warnings)) {
      throw new Error("The local server returned an unexpected travel response.");
    }
    if (serial !== state.requestSerial) {
      return;
    }

    const previousSelection = selectedIdentity();
    const firstLoad = state.data === null;
    state.data = payload;
    state.pendingFit = true;
    elements["start-date"].value = payload.range.start;
    elements["end-date"].value = payload.range.end;
    if (firstLoad) {
      const configuredProjection = payload.map?.projection === "map" ? "mercator" : "globe";
      setProjection(configuredProjection);
    }
    renderAll(previousSelection);
    ensureMap(payload.map);
    const count = payload.legs.length;
    setRequestStatus(
      `${count} ${count === 1 ? "flight" : "flights"} · ${payload.range.start} to ${payload.range.end}`,
      false,
    );
    elements["generated-at"].textContent = `Updated ${formatGeneratedAt(payload.generated_at)}`;
  } catch (error) {
    if (error.name === "AbortError") {
      return;
    }
    if (serial !== state.requestSerial) {
      return;
    }
    const message = error instanceof Error ? error.message : "Travel data could not be loaded.";
    showFormError(message);
    setRequestStatus("Travel data unavailable", true);
  } finally {
    if (serial === state.requestSerial) {
      setLoading(false);
    }
  }
}

function renderAll(previousSelection) {
  if (!state.data) {
    return;
  }
  const legs = state.data.legs;
  renderWarnings(state.data.warnings);
  renderTripList(legs);
  elements["trip-count"].textContent = String(legs.length);
  elements["empty-state"].hidden = legs.length !== 0;
  elements["trip-selector"].hidden = legs.length === 0;

  if (legs.length === 0) {
    state.selectedIndex = null;
    elements["detail-panel"].hidden = true;
  } else {
    const retainedIndex = previousSelection
      ? legs.findIndex((leg) => legIdentity(leg) === previousSelection)
      : -1;
    state.selectedIndex = retainedIndex >= 0 ? retainedIndex : 0;
    updateSelection(false, false);
  }
  renderMapData();
}

function selectedIdentity() {
  if (!state.data || state.selectedIndex === null) {
    return null;
  }
  const leg = state.data.legs[state.selectedIndex];
  return leg ? legIdentity(leg) : null;
}

function legIdentity(leg) {
  return `${leg.source?.event_id || ""}\u0000${leg.source?.occurrence_date || ""}\u0000${leg.departure?.utc || ""}`;
}

function renderWarnings(warnings) {
  elements["warning-list"].replaceChildren();
  for (const warning of warnings) {
    const item = document.createElement("li");
    item.textContent = stringValue(warning.message, "Unspecified travel warning");
    elements["warning-list"].append(item);
  }
  elements["warning-panel"].hidden = warnings.length === 0;
}

function renderTripList(legs) {
  elements["trip-list"].replaceChildren();
  legs.forEach((leg, index) => {
    const item = document.createElement("li");
    const card = document.createElement("button");
    card.type = "button";
    card.className = "trip-card";
    card.dataset.legIndex = String(index);
    card.setAttribute("aria-current", "false");
    card.addEventListener("click", () => selectLeg(index, true));

    const badge = document.createElement("span");
    badge.className = "flight-badge";
    const flightNumber = splitFlightNumber(leg.flight_number);
    badge.append(
      classSpan("flight-carrier", flightNumber.carrier),
      classSpan("flight-number", flightNumber.number),
    );

    const main = document.createElement("span");
    main.className = "trip-main";
    const route = document.createElement("span");
    route.className = "trip-route";
    route.append(
      textSpan(stringValue(leg.departure_airport?.code, "???")),
      classSpan("route-line", ""),
      textSpan(stringValue(leg.arrival_airport?.code, "???")),
    );
    const meta = document.createElement("span");
    meta.className = "trip-meta";
    meta.append(
      textSpan(formatLocalTimestamp(leg.departure?.scheduled)),
      textSpan(`→ ${formatLocalTimestamp(leg.arrival?.scheduled)}`),
    );
    if (!hasRouteCoordinates(leg)) {
      const missing = textSpan("Map coordinates unavailable");
      missing.className = "map-missing";
      meta.append(missing);
    }
    main.append(route, meta);

    const status = document.createElement("span");
    const freshness = leg.live_status?.freshness?.state;
    status.className = `status-chip${freshness ? freshness === "stale" ? " stale" : " live" : ""}`;
    status.textContent = leg.live_status
      ? formatStatus(leg.live_status.status)
      : "Calendar";

    card.append(badge, main, status);
    item.append(card);
    elements["trip-list"].append(item);
  });
}

function selectLeg(index, moveMap) {
  if (!state.data || index < 0 || index >= state.data.legs.length) {
    return;
  }
  state.selectedIndex = index;
  updateSelection(moveMap, true);
}

function updateSelection(moveMap, revealCard) {
  let selectedCard = null;
  for (const card of document.querySelectorAll(".trip-card")) {
    const selected = Number(card.dataset.legIndex) === state.selectedIndex;
    card.setAttribute("aria-current", String(selected));
    if (selected) {
      selectedCard = card;
    }
  }
  if (revealCard) {
    revealSelectedCard(selectedCard);
  }
  document.body.dataset.selectedLeg = state.selectedIndex === null ? "" : String(state.selectedIndex);
  renderDetails();
  renderMapData();

  if (moveMap && state.map && state.selectedIndex !== null) {
    const leg = state.data.legs[state.selectedIndex];
    const coordinates = routeEndpoints(leg);
    if (coordinates) {
      const midpoint = greatCircle(coordinates[0], coordinates[1], 24)[12];
      state.map.easeTo({center: midpoint, duration: 550});
    }
  }
}

function revealSelectedCard(card) {
  const list = elements["trip-list"];
  if (!card) {
    return;
  }
  const listBounds = list.getBoundingClientRect();
  const cardBounds = card.getBoundingClientRect();
  if (list.scrollHeight > list.clientHeight + 1) {
    if (cardBounds.top < listBounds.top) {
      list.scrollTop -= listBounds.top - cardBounds.top;
    } else if (cardBounds.bottom > listBounds.bottom) {
      list.scrollTop += cardBounds.bottom - listBounds.bottom;
    }
  } else if (cardBounds.top < 0 || cardBounds.bottom > window.innerHeight) {
    card.scrollIntoView({block: "nearest", inline: "nearest"});
  }
}

function renderDetails() {
  if (!state.data || state.selectedIndex === null) {
    elements["detail-panel"].hidden = true;
    return;
  }
  const leg = state.data.legs[state.selectedIndex];
  elements["detail-heading"].textContent = `${stringValue(leg.flight_number, "Flight")} · ${stringValue(leg.route, "Unknown route")}`;
  const content = document.createDocumentFragment();
  const timing = document.createElement("div");
  timing.className = "timing-groups";
  timing.append(
    buildTimingGroup("Departure", "departure", leg.departure, leg.live_status),
    buildTimingGroup("Arrival", "arrival", leg.arrival, leg.live_status),
  );
  content.append(timing);

  const list = document.createElement("dl");
  list.className = "detail-grid";
  addDetail(list, "From", airportDescription(leg.departure_airport));
  addDetail(list, "To", airportDescription(leg.arrival_airport));
  addDetail(list, "Calendar", stringValue(leg.source?.calendar, "Unknown calendar"));

  if (leg.live_status) {
    const live = leg.live_status;
    addDetail(list, "Departure gate", gateDescription(live.departure_terminal, live.departure_gate));
    addDetail(list, "Arrival gate", gateDescription(live.arrival_terminal, live.arrival_gate));
    addDetail(list, "Freshness", freshnessDescription(live.freshness), true);
    const operationalDescription = distinctOperationalDescription(live);
    if (live.diverted) {
      addDetail(
        list,
        "Diversion",
        operationalDescription || "FlightAware reports this flight as diverted",
        true,
      );
    } else if (live.tracking_ended) {
      addDetail(
        list,
        "Tracking",
        operationalDescription || "FlightAware is no longer tracking this flight",
        true,
      );
    } else if (operationalDescription) {
      addDetail(list, "Operational update", operationalDescription, true);
    }
    if (live.current_position) {
      addDetail(list, "Current position", positionDescription(live.current_position), true);
    }
  }
  if (!hasRouteCoordinates(leg)) {
    addDetail(list, "Map", missingCoordinateDescription(leg), true);
  }

  content.append(list);
  elements["detail-content"].replaceChildren(content);
  elements["detail-content"].scrollTop = 0;
  elements["detail-panel"].hidden = false;
}

function buildTimingGroup(label, endpoint, schedule, live) {
  const booked = parseLocalTimestamp(schedule?.scheduled);
  const update = preferredLiveTime(live, endpoint, booked);
  const delaySeconds = timingDelaySeconds(live, endpoint, booked, update);
  const group = document.createElement("section");
  group.className = "timing-group";
  group.dataset.endpoint = endpoint;

  const heading = document.createElement("h3");
  heading.className = "timing-heading";
  const headingLabel = document.createElement("span");
  headingLabel.textContent = label;
  const headingDate = document.createElement("span");
  headingDate.className = "timing-date";
  headingDate.textContent = booked?.dateLabel || "Date unavailable";
  heading.append(headingLabel, document.createTextNode(" · "), headingDate);
  group.append(heading);

  if (update) {
    group.append(
      buildTimingLine("Booked", booked?.timeLabel || "Time unavailable", null),
      buildTimingLine(
        "Updated",
        update.dateKey !== booked?.dateKey
          ? `${update.shortDateLabel} · ${update.timeLabel}`
          : update.timeLabel,
        delaySeconds,
      ),
    );
  } else if (delaySeconds !== null && delaySeconds === 0) {
    group.append(buildTimingLine(null, booked?.timeLabel || "Time unavailable", delaySeconds));
  } else {
    group.append(
      buildTimingLine("Booked", booked?.timeLabel || "Time unavailable", delaySeconds),
    );
  }
  return group;
}

function buildTimingLine(kind, value, delaySeconds) {
  const line = document.createElement("p");
  line.className = "timing-line";
  if (kind) {
    const label = document.createElement("span");
    label.className = "timing-kind";
    label.textContent = kind;
    line.append(label, document.createTextNode(" "));
  }
  const time = document.createElement("span");
  time.className = "timing-value";
  time.textContent = value;
  line.append(time);
  if (delaySeconds !== null) {
    const delay = document.createElement("span");
    const delayState = delaySeconds > 0 ? "late" : delaySeconds < 0 ? "early" : "on-time";
    delay.className = `timing-delay ${delayState}`;
    delay.textContent = delayDescription(delaySeconds);
    line.append(document.createTextNode(" · "), delay);
  }
  return line;
}

function addDetail(list, label, value, wide = false) {
  const wrapper = document.createElement("div");
  wrapper.className = `detail-item${wide ? " wide" : ""}`;
  const term = document.createElement("dt");
  term.textContent = label;
  const description = document.createElement("dd");
  description.textContent = stringValue(value, "Unavailable");
  wrapper.append(term, description);
  list.append(wrapper);
}

function ensureMap(mapConfig) {
  if (state.map || typeof window.maplibregl === "undefined") {
    return;
  }
  const styleUrl = stringValue(mapConfig?.style_url, DEFAULT_MAP_STYLE_URL);
  try {
    state.map = new window.maplibregl.Map({
      container: "map",
      style: styleUrl,
      center: [12, 24],
      zoom: 1.25,
      attributionControl: true,
      renderWorldCopies: true,
    });
    state.map.addControl(new window.maplibregl.NavigationControl({showCompass: true}), "top-left");
    state.map.on("style.load", () => {
      state.mapReady = true;
      document.body.dataset.mapReady = "true";
      if (state.mapFallbackUsed) {
        showMapMessage("Base map unavailable; routes are shown without a basemap.", true);
      } else {
        hideMapMessage();
      }
      installRouteLayer();
      applyProjection();
      renderMapData();
    });
    state.map.on("error", () => {
      if (!state.mapReady && !state.mapFallbackUsed) {
        state.mapFallbackUsed = true;
        showMapMessage("Base map unavailable; showing routes without a basemap.", true);
        state.map.setStyle(BLANK_STYLE);
      }
    });
  } catch (_error) {
    showMapMessage("Interactive map unavailable. The itinerary remains usable.", true);
  }
}

function installRouteLayer() {
  if (!state.map.getSource(ROUTE_SOURCE_ID)) {
    state.map.addSource(ROUTE_SOURCE_ID, {
      type: "geojson",
      data: emptyFeatureCollection(),
    });
  }
  if (!state.map.getLayer(ROUTE_LAYER_ID)) {
    state.map.addLayer({
      id: ROUTE_LAYER_ID,
      type: "line",
      source: ROUTE_SOURCE_ID,
      layout: {"line-cap": "round", "line-join": "round"},
      paint: {
        "line-color": [
          "case",
          ["boolean", ["get", "selected"], false],
          "#f8d39f",
          "#62c3ce",
        ],
        "line-width": ["case", ["boolean", ["get", "selected"], false], 4.5, 2.5],
        "line-opacity": ["case", ["boolean", ["get", "selected"], false], 1, 0.68],
      },
    });
  }
  if (!state.routeClickBound) {
    state.routeClickBound = true;
    state.map.on("click", ROUTE_LAYER_ID, (event) => {
      const index = Number(event.features?.[0]?.properties?.legIndex);
      if (Number.isInteger(index)) {
        selectLeg(index, false);
      }
    });
    state.map.on("mouseenter", ROUTE_LAYER_ID, () => {
      state.map.getCanvas().style.cursor = "pointer";
    });
    state.map.on("mouseleave", ROUTE_LAYER_ID, () => {
      state.map.getCanvas().style.cursor = "";
    });
  }
}

function renderMapData() {
  if (!state.map || !state.mapReady || !state.data) {
    return;
  }
  installRouteLayer();
  const features = [];
  const fitCoordinates = [];
  state.data.legs.forEach((leg, index) => {
    const endpoints = routeEndpoints(leg);
    if (!endpoints) {
      return;
    }
    const coordinates = greatCircle(endpoints[0], endpoints[1], 72);
    features.push({
      type: "Feature",
      properties: {legIndex: index, selected: index === state.selectedIndex},
      geometry: {type: "LineString", coordinates},
    });
    fitCoordinates.push(...coordinates);
  });
  const source = state.map.getSource(ROUTE_SOURCE_ID);
  if (source) {
    source.setData({type: "FeatureCollection", features});
  }
  document.body.dataset.routeCount = String(features.length);
  document.body.dataset.routeGeometry = JSON.stringify(
    features.map((feature) => ({
      legIndex: feature.properties.legIndex,
      start: feature.geometry.coordinates[0],
      end: feature.geometry.coordinates[feature.geometry.coordinates.length - 1],
    })),
  );
  renderMarkers();

  if (state.pendingFit && fitCoordinates.length > 0) {
    state.pendingFit = false;
    fitMap(fitCoordinates);
  } else if (state.pendingFit) {
    state.pendingFit = false;
  }
}

function renderMarkers() {
  for (const marker of state.markers) {
    marker.remove();
  }
  state.markers = [];
  if (!state.data || !state.map) {
    return;
  }
  state.data.legs.forEach((leg, index) => {
    addMarker(leg.departure_airport, "departure", index, leg);
    addMarker(leg.arrival_airport, "arrival", index, leg);
  });
}

function addMarker(airport, kind, index, leg) {
  const coordinate = airportCoordinate(airport);
  if (!coordinate) {
    return;
  }
  const button = document.createElement("button");
  button.type = "button";
  button.className = `travel-marker ${kind}${index === state.selectedIndex ? " selected" : ""}`;
  button.dataset.legIndex = String(index);
  button.setAttribute(
    "aria-label",
    `${kind === "departure" ? "Departure" : "Arrival"} ${stringValue(airport.code, "airport")} for ${stringValue(leg.flight_number, "flight")}`,
  );
  button.addEventListener("click", () => selectLeg(index, false));
  const marker = new window.maplibregl.Marker({element: button, anchor: "center"})
    .setLngLat(coordinate)
    .addTo(state.map);
  state.markers.push(marker);
}

function fitMap(coordinates) {
  if (!state.map || coordinates.length === 0) {
    return;
  }
  const continuous = unwrapCoordinates(coordinates);
  const bounds = new window.maplibregl.LngLatBounds();
  continuous.forEach((coordinate) => bounds.extend(coordinate));
  state.map.fitBounds(bounds, {
    padding: {top: 90, right: 90, bottom: 90, left: 90},
    maxZoom: 5.2,
    duration: 650,
  });
}

function setProjection(projection) {
  const requested = projection === "mercator" ? "mercator" : "globe";
  if (state.map && state.mapReady) {
    try {
      state.map.setProjection({type: requested});
      state.projection = requested;
      updateProjectionUi();
      recordActualProjection();
    } catch (_error) {
      showMapMessage("This map style cannot use the selected projection.", true);
      recordActualProjection();
      updateProjectionUi();
    }
    return;
  }
  state.projection = requested;
  updateProjectionUi();
}

function updateProjectionUi() {
  document.body.dataset.projection = state.projection;
  for (const button of document.querySelectorAll("[data-projection]")) {
    button.setAttribute("aria-pressed", String(button.dataset.projection === state.projection));
  }
}

function applyProjection() {
  if (!state.map || !state.mapReady) {
    return;
  }
  try {
    state.map.setProjection({type: state.projection});
    recordActualProjection();
  } catch (_error) {
    const actual = actualProjection();
    if (actual) {
      state.projection = actual;
      updateProjectionUi();
    }
    showMapMessage("This map style cannot use the selected projection.", true);
    recordActualProjection();
  }
}

function actualProjection() {
  const type = state.map?.getProjection?.()?.type;
  if (type === "mercator") {
    return "mercator";
  }
  if (type === "globe" || type === "vertical-perspective") {
    return "globe";
  }
  return null;
}

function recordActualProjection() {
  document.body.dataset.mapProjection = actualProjection() || "unknown";
}

function greatCircle(start, end, segments) {
  const startVector = sphericalVector(start);
  const endVector = sphericalVector(end);
  const dot = clamp(
    startVector[0] * endVector[0] +
      startVector[1] * endVector[1] +
      startVector[2] * endVector[2],
    -1,
    1,
  );
  const omega = Math.acos(dot);
  const sinOmega = Math.sin(omega);
  const coordinates = [];
  for (let index = 0; index <= segments; index += 1) {
    const fraction = index / segments;
    let vector;
    if (Math.abs(sinOmega) < 1e-7) {
      vector = [
        startVector[0] + (endVector[0] - startVector[0]) * fraction,
        startVector[1] + (endVector[1] - startVector[1]) * fraction,
        startVector[2] + (endVector[2] - startVector[2]) * fraction,
      ];
    } else {
      const startWeight = Math.sin((1 - fraction) * omega) / sinOmega;
      const endWeight = Math.sin(fraction * omega) / sinOmega;
      vector = [
        startVector[0] * startWeight + endVector[0] * endWeight,
        startVector[1] * startWeight + endVector[1] * endWeight,
        startVector[2] * startWeight + endVector[2] * endWeight,
      ];
    }
    const longitude = (Math.atan2(vector[1], vector[0]) * 180) / Math.PI;
    const latitude =
      (Math.atan2(vector[2], Math.hypot(vector[0], vector[1])) * 180) / Math.PI;
    coordinates.push([longitude, latitude]);
  }
  return unwrapCoordinates(coordinates);
}

function sphericalVector(coordinate) {
  const longitude = (coordinate[0] * Math.PI) / 180;
  const latitude = (coordinate[1] * Math.PI) / 180;
  const latitudeCosine = Math.cos(latitude);
  return [
    latitudeCosine * Math.cos(longitude),
    latitudeCosine * Math.sin(longitude),
    Math.sin(latitude),
  ];
}

function unwrapCoordinates(coordinates) {
  if (coordinates.length === 0) {
    return [];
  }
  const output = [[coordinates[0][0], coordinates[0][1]]];
  for (let index = 1; index < coordinates.length; index += 1) {
    let longitude = coordinates[index][0];
    const previous = output[index - 1][0];
    while (longitude - previous >= 180) {
      longitude -= 360;
    }
    while (longitude - previous < -180) {
      longitude += 360;
    }
    output.push([longitude, coordinates[index][1]]);
  }
  return output;
}

function routeEndpoints(leg) {
  const departure = airportCoordinate(leg.departure_airport);
  const arrival = airportCoordinate(leg.arrival_airport);
  return departure && arrival ? [departure, arrival] : null;
}

function hasRouteCoordinates(leg) {
  return routeEndpoints(leg) !== null;
}

function airportCoordinate(airport) {
  const latitude = Number(airport?.metadata?.latitude);
  const longitude = Number(airport?.metadata?.longitude);
  if (!Number.isFinite(latitude) || !Number.isFinite(longitude)) {
    return null;
  }
  if (latitude < -90 || latitude > 90 || longitude < -180 || longitude > 180) {
    return null;
  }
  return [longitude, latitude];
}

function airportDescription(airport) {
  const code = stringValue(airport?.code, "Unknown");
  const name = airport?.metadata?.name;
  const municipality = airport?.metadata?.municipality;
  return [code, name, municipality].filter((value, index, values) => value && values.indexOf(value) === index).join(" · ");
}

function missingCoordinateDescription(leg) {
  const missing = [];
  if (!airportCoordinate(leg.departure_airport)) {
    missing.push(stringValue(leg.departure_airport?.code, "departure airport"));
  }
  if (!airportCoordinate(leg.arrival_airport)) {
    missing.push(stringValue(leg.arrival_airport?.code, "arrival airport"));
  }
  return `Coordinates unavailable for ${missing.join(" and ")}`;
}

function preferredLiveTime(live, endpoint, booked) {
  if (!live || !booked) {
    return null;
  }
  for (const kind of ["actual", "estimated", "scheduled"]) {
    const value = live[`${kind}_${endpoint}`];
    const formatted = formatInstantAtOffset(value, booked.offsetMinutes, booked.offsetLabel);
    if (!formatted) {
      continue;
    }
    if (kind === "scheduled" && formatted.instant === booked.instant) {
      return null;
    }
    return {...formatted, kind};
  }
  return null;
}

function timingDelaySeconds(live, endpoint, booked, update) {
  if (!live) {
    return null;
  }
  const reported = Number(live[`${endpoint}_delay_seconds`]);
  if (
    live[`${endpoint}_delay_seconds`] !== null &&
    live[`${endpoint}_delay_seconds`] !== undefined &&
    Number.isFinite(reported)
  ) {
    return reported;
  }
  if (booked && update) {
    return Math.round((update.instant - booked.instant) / 1000);
  }
  return null;
}

function distinctOperationalDescription(live) {
  const description = typeof live?.description === "string" ? live.description.trim() : "";
  if (!description) {
    return null;
  }
  const status = typeof live.status === "string" ? live.status.replaceAll("_", " ").trim() : "";
  if (description.localeCompare(status, undefined, {sensitivity: "accent"}) === 0) {
    return null;
  }
  return description;
}

function parseLocalTimestamp(value) {
  if (typeof value !== "string") {
    return null;
  }
  const match = value.match(
    /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::\d{2}(?:\.\d+)?)?(Z|[+-]\d{2}:\d{2})$/,
  );
  if (!match) {
    return null;
  }
  const [, year, month, day, hour, minute, offset] = match;
  const instant = Date.parse(value);
  if (!Number.isFinite(instant)) {
    return null;
  }
  const offsetMinutes = parseOffsetMinutes(offset);
  if (offsetMinutes === null) {
    return null;
  }
  return {
    instant,
    offsetMinutes,
    offsetLabel: offset === "Z" ? "UTC" : offset,
    dateKey: `${year}-${month}-${day}`,
    dateLabel: formatDateLabel(year, month, day, true),
    shortDateLabel: formatDateLabel(year, month, day, false),
    timeLabel: `${hour}:${minute} (${offset === "Z" ? "UTC" : offset})`,
  };
}

function formatInstantAtOffset(value, offsetMinutes, offsetLabel) {
  if (typeof value !== "string") {
    return null;
  }
  const instant = Date.parse(value);
  if (!Number.isFinite(instant)) {
    return null;
  }
  const shifted = new Date(instant + offsetMinutes * 60_000);
  const year = String(shifted.getUTCFullYear()).padStart(4, "0");
  const month = String(shifted.getUTCMonth() + 1).padStart(2, "0");
  const day = String(shifted.getUTCDate()).padStart(2, "0");
  const hour = String(shifted.getUTCHours()).padStart(2, "0");
  const minute = String(shifted.getUTCMinutes()).padStart(2, "0");
  return {
    instant,
    dateKey: `${year}-${month}-${day}`,
    dateLabel: formatDateLabel(year, month, day, true),
    shortDateLabel: formatDateLabel(year, month, day, false),
    timeLabel: `${hour}:${minute} (${offsetLabel})`,
  };
}

function parseOffsetMinutes(value) {
  if (value === "Z") {
    return 0;
  }
  const match = value.match(/^([+-])(\d{2}):(\d{2})$/);
  if (!match) {
    return null;
  }
  const minutes = Number(match[2]) * 60 + Number(match[3]);
  return match[1] === "-" ? -minutes : minutes;
}

function formatDateLabel(year, month, day, includeYear) {
  const date = new Date(`${year}-${month}-${day}T00:00:00Z`);
  return new Intl.DateTimeFormat(undefined, {
    day: "numeric",
    month: "short",
    ...(includeYear ? {year: "numeric"} : {}),
    timeZone: "UTC",
  }).format(date);
}

function gateDescription(terminal, gate) {
  if (!terminal && !gate) {
    return "Not assigned";
  }
  return [terminal ? `Terminal ${terminal}` : null, gate ? `Gate ${gate}` : null]
    .filter(Boolean)
    .join(" · ");
}

function freshnessDescription(freshness) {
  if (!freshness) {
    return "Unavailable";
  }
  const stateLabel = freshness.state === "stale" ? "Stale cache" : "Fresh";
  return `${stateLabel} · fetched ${formatProviderTimestamp(freshness.fetched_at)}`;
}

function positionDescription(position) {
  const parts = [
    `${Number(position.latitude).toFixed(3)}, ${Number(position.longitude).toFixed(3)}`,
  ];
  if (
    position.altitude_feet !== null &&
    position.altitude_feet !== undefined &&
    Number.isFinite(Number(position.altitude_feet))
  ) {
    parts.push(`${Number(position.altitude_feet).toLocaleString()} ft`);
  }
  if (
    position.groundspeed_knots !== null &&
    position.groundspeed_knots !== undefined &&
    Number.isFinite(Number(position.groundspeed_knots))
  ) {
    parts.push(`${Number(position.groundspeed_knots)} kt`);
  }
  return parts.join(" · ");
}

function delayDescription(seconds) {
  const minutes = Math.round(Number(seconds) / 60);
  if (!Number.isFinite(minutes) || minutes === 0) {
    return "On time";
  }
  return minutes > 0 ? `${minutes} min late` : `${Math.abs(minutes)} min early`;
}

function formatLocalTimestamp(value) {
  if (typeof value !== "string") {
    return "Time unavailable";
  }
  const match = value.match(/^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})(?::\d{2}(?:\.\d+)?)?(Z|[+-]\d{2}:\d{2})$/);
  if (!match) {
    return value;
  }
  const [, year, month, day, hour, minute, offset] = match;
  const date = new Date(`${year}-${month}-${day}T00:00:00Z`);
  const dateLabel = new Intl.DateTimeFormat(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
    timeZone: "UTC",
  }).format(date);
  return `${dateLabel} · ${hour}:${minute} (${offset === "Z" ? "UTC" : offset})`;
}

function formatProviderTimestamp(value) {
  if (!value) {
    return "unavailable";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return String(value);
  }
  return new Intl.DateTimeFormat(undefined, {
    day: "numeric",
    month: "short",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
    timeZoneName: "short",
  }).format(date);
}

function formatGeneratedAt(value) {
  if (!value) {
    return "just now";
  }
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) {
    return "just now";
  }
  return new Intl.DateTimeFormat(undefined, {timeStyle: "medium"}).format(date);
}

function formatStatus(value) {
  const status = stringValue(value, "Live update").replaceAll("_", " ");
  return `${status.charAt(0).toUpperCase()}${status.slice(1)}`;
}

function splitFlightNumber(value) {
  const text = stringValue(value, "FL");
  const compact = text.length > 7 ? text.slice(0, 7) : text;
  const spaced = compact.match(/^(\S+)\s+(.+)$/);
  if (spaced) {
    return {carrier: spaced[1], number: spaced[2]};
  }
  if (compact.length > 2) {
    return {carrier: compact.slice(0, 2), number: compact.slice(2)};
  }
  return {carrier: compact, number: ""};
}

function stringValue(value, fallback) {
  return typeof value === "string" && value.length > 0 ? value : fallback;
}

function textSpan(value) {
  const span = document.createElement("span");
  span.textContent = value;
  return span;
}

function classSpan(className, value) {
  const span = textSpan(value);
  span.className = className;
  return span;
}

function emptyFeatureCollection() {
  return {type: "FeatureCollection", features: []};
}

function clamp(value, minimum, maximum) {
  return Math.min(Math.max(value, minimum), maximum);
}

function setLoading(loading) {
  elements["refresh-button"].disabled = loading;
  elements["refresh-button"].querySelector("span").textContent = loading ? "Loading…" : "Refresh";
}

function setRequestStatus(message, error) {
  elements["request-status"].textContent = message;
  elements["request-status"].classList.toggle("error", error);
}

function showFormError(message) {
  elements["form-error"].textContent = message;
  elements["form-error"].hidden = false;
}

function hideFormError() {
  elements["form-error"].hidden = true;
  elements["form-error"].textContent = "";
}

function showMapMessage(message, error) {
  const text = elements["map-message"].querySelector("span:last-child");
  text.textContent = message;
  elements["map-message"].classList.toggle("error", error);
  elements["map-message"].hidden = false;
  const spinner = elements["map-message"].querySelector(".spinner");
  spinner.hidden = error;
}

function hideMapMessage() {
  elements["map-message"].hidden = true;
}
